//! The local account — made on your machine, kept on your machine.
//!
//! # What this changes, said before anything else
//!
//! `WALLETS.md` §0 carries *"Elo never creates an account, holds a seed, or
//! accepts a recovery phrase."* Read the sentence it opens: **"Website rule."**
//! It is scoped to a browser visitor and the reasoning is sound there — a web
//! page is the worst place in computing to hold a key (XSS, a supply chain you
//! do not control, no OS isolation), and the visitor already has a wallet
//! extension next to it. That is why MetaMask is an extension and not a page.
//!
//! **A desktop binary on the user's own disk is the one surface where none of
//! that reasoning transfers**, and the surface where the rule costs most:
//! making a stranger go install a wallet elsewhere before they can have an
//! account is the thing that loses the most players at the door.
//!
//! So the carve-out is narrow and stated: **the desktop client may create a
//! self-custody account; the website still may not.** What is unchanged is the
//! part that was ever about custody — `CLAUDE.md` invariant 7 is that *we* hold
//! no user keys, and elo the service never sees this one. It is generated from
//! the OS entropy pool on the holder's box, encrypted with their passphrase,
//! written to their disk, and transmitted nowhere.
//!
//! # Nothing here is hand-rolled
//!
//! Operator, 2026-08-06: *"don't hand roll anything."* `vault.py` opens with
//! the same rule and it is the reason this crate has dependencies at all:
//!
//! | | |
//! |---|---|
//! | secp256k1 + recoverable ECDSA | `k256` (RustCrypto) |
//! | keccak-256 | `sha3` (RustCrypto) |
//! | scrypt | `scrypt` (RustCrypto) |
//! | AES-128-CTR | `aes` + `ctr` (RustCrypto) |
//! | entropy | `getrandom` — the OS pool, never a userspace PRNG |
//!
//! Writing novel ECDSA or a novel KDF is how a key leaks with no symptom until
//! somebody else has the money. The one thing this file *does* implement is the
//! **assembly**: which bytes go into which primitive, in the order Web3 Secret
//! Storage V3 and EIP-191 specify. That part is testable against published
//! vectors and is tested that way.

use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use sha3::{Digest, Keccak256};
use zeroize::Zeroize;
pub use zeroize::Zeroizing;

pub mod harden;
pub mod keystore;
pub mod local;
pub mod masked;
pub mod phrase;
pub mod seaport;
pub mod solana;
pub mod tx;

pub use harden::harden_process;
pub use keystore::{decrypt, encrypt, KdfParams, Keystore};
pub use local::LocalSigner;
pub use phrase::Phrase;
pub use solana::SolAccount;

/// Where the local keystore lives, and **this is the only definition**.
///
/// Both binaries ask here. That matters more than it looks: `elo account new`
/// and the GUI's Account window must be the same account, and a second copy of
/// this logic is how they would quietly become two — one client making a key
/// the other cannot see, with no error anywhere to say so.
///
/// Beside the settings, not beside the games: a games root is a cache a player
/// may delete to reclaim disk, and a key is the one file in this program that
/// must never live in something disposable.
pub fn keystore_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("ELO_KEYSTORE") {
        return std::path::PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(base).join("elo").join("keystore.json")
    }
    #[cfg(not(windows))]
    {
        // ⚠ Only an ABSOLUTE `XDG_CONFIG_HOME` counts, which is what the XDG
        // spec says and not pedantry: an empty one (`XDG_CONFIG_HOME= elo …`,
        // or a desktop session that exports it blank) joined to `elo/` is a
        // path under whatever directory the launcher was started in, so the
        // key landed in the player's Downloads folder, or a game's, and the
        // next run from somewhere else found no account at all.
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|p| std::path::Path::new(p).is_absolute())
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_default();
                if home.is_empty() { ".".into() } else { format!("{home}/.config") }
            });
        std::path::PathBuf::from(base).join("elo").join("keystore.json")
    }
}

/// Where a browser pairing is remembered — beside the keystore, never in it.
///
/// **This file holds no key and cannot be made to hold one.** It remembers a
/// pairing code, the launcher's poll secret and the address a browser wallet
/// proved, and none of the three can sign anything: every signature is asked
/// for over the wire and approved by a human in another window
/// (`meter/browser_signer.py`). It lives here rather than in `elo-net` for the
/// reason [`keystore_path`] gives about itself — the CLI and the GUI must find
/// the SAME pairing, and a second copy of the directory logic is how they would
/// quietly become two.
///
/// It is still a bearer credential: whoever holds it may QUEUE asks the
/// paired browser will be shown. Written 0600 on unix for that reason, and
/// `elo pair forget` deletes it.
pub fn pairing_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("ELO_PAIRING") {
        return std::path::PathBuf::from(p);
    }
    keystore_path().with_file_name("pairing.json")
}

/// Write a file only its owner may read — the keystore, a pairing, a grant.
///
/// ⚠ **0600 from its first byte, never written wide and then narrowed.**
/// `fs::write` then `chmod` leaves a window in which another local user who
/// can see the directory opens the file, and a descriptor opened in that
/// window stays readable after the chmod lands. So the file is created with
/// its mode, and a directory this call has to make is made 0700.
///
/// `fresh` is the keystore's rule: refuse anything already at the path — a
/// file, or a symlink, dangling or not, which `exists()` cannot see and
/// `fs::write` would follow. Otherwise the new bytes go to a 0600 sibling
/// that is renamed over the old file, so a reader holding the old one never
/// sees the new contents and a link at the path is replaced, not followed.
///
/// ⚠ **A fresh file appears whole or not at all.** It is written and synced
/// under a temp name first, then hard-linked to its real name. A crash
/// partway through a direct write left a truncated `keystore.json`, and every
/// create after it refused that stump as "may be the only copy of a key" — a
/// player locked out of making an account by a file holding no key. `link`
/// is the one call that is atomic AND refuses an existing name (rename
/// replaces), so the no-overwrite rule above survives the move. The
/// directory is synced after, so the name outlives a power cut as well as
/// the bytes.
///
/// On Windows the file inherits the user's profile ACL, which is the
/// platform's equivalent and is not something this should second-guess.
pub fn write_private(path: &std::path::Path, body: &[u8], fresh: bool) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(dir) = dir {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir)?;
    }
    let open = |at: &std::path::Path| {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(at)
    };
    // Unique per call, not per process: two threads saving the same file
    // (the GUI re-pairing twice) must not share, or remove, one temp file.
    static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{n}.tmp", std::process::id()));
    let tmp = path.with_file_name(name);
    let _ = std::fs::remove_file(&tmp);
    let wrote = open(&tmp).and_then(|mut f| {
        f.write_all(body)?;
        f.sync_all()
    });
    if fresh {
        let made = wrote.and_then(|()| {
            std::fs::hard_link(&tmp, path).or_else(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    return Err(e);
                }
                // A disk with no hard links (FAT or exFAT — where a key kept
                // on a USB stick through `ELO_KEYSTORE` lives) refuses the link
                // for a reason that has nothing to do with the path. The
                // exclusive create is still the refusal there; only the
                // whole-or-nothing promise is lost, and only on that disk. A
                // failed write still removes what this call made.
                let mut f = open(path)?;
                f.write_all(body).and_then(|()| f.sync_all()).inspect_err(|_| {
                    let _ = std::fs::remove_file(path);
                })
            })
        });
        // The temp name goes whatever happened: on success the real name holds
        // the same file, and on failure nothing of this call is left behind.
        let _ = std::fs::remove_file(&tmp);
        made?;
        sync_dir(dir);
        return Ok(());
    }
    match wrote.and_then(|()| std::fs::rename(&tmp, path)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Make a new name in `dir` durable. Best effort, and deliberately silent: it
/// runs after the file is already in place, so failing here would report an
/// account as not made while its keystore sits on disk — and the next try
/// would refuse that keystore as "may be the only copy of a key". Unix only;
/// Windows cannot open a directory as a file and journals the name itself.
fn sync_dir(dir: Option<&std::path::Path>) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir.unwrap_or(std::path::Path::new("."))) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// The Solana half of an account, beside its keystore: `keystore.json` →
/// `keystore.solana.json`. The same V3 shape and the same passphrase, so one
/// unlock answers for both chains; its `address` is base58 and its `path`
/// says where the key came from. Named from the keystore rather than fixed so
/// a keystore somewhere else (`ELO_KEYSTORE`) keeps its Solana half beside it.
pub fn solana_path_for(keystore: &std::path::Path) -> std::path::PathBuf {
    let stem = keystore.file_stem().map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "keystore".into());
    keystore.with_file_name(format!("{stem}.solana.json"))
}

/// The addresses this launcher has sent to, beside the keystore. Public data,
/// kept so a send to a new or lookalike address can say so
/// (`elo_net::wallet`). One definition, for [`keystore_path`]'s reason.
pub fn recipients_path() -> std::path::PathBuf {
    keystore_path().with_file_name("recipients.json")
}

/// An account this machine holds. **Never serialise this**; the encrypted
/// [`Keystore`] is the on-disk form.
pub struct Account {
    key: SigningKey,
}

impl Account {
    /// A new account from the OS entropy pool.
    ///
    /// `getrandom` and nothing else: a userspace PRNG seeded badly produces a
    /// key that looks fine and is guessable, which is the failure mode with no
    /// symptom. An error here is fatal and is returned rather than papered
    /// over — a "random" key from a failed draw is the one outcome worse than
    /// no key at all.
    pub fn generate() -> Result<Account, String> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *bytes).map_err(|e| format!("no OS entropy: {e}"))?;
        let key = SigningKey::from_slice(bytes.as_slice())
            .map_err(|e| format!("the drawn bytes are not a valid key: {e}"))?;
        Ok(Account { key })
    }

    pub fn from_secret(bytes: &[u8]) -> Result<Account, String> {
        SigningKey::from_slice(bytes)
            .map(|key| Account { key })
            .map_err(|e| format!("not a secp256k1 key: {e}"))
    }

    /// The 32 secret bytes. Only [`encrypt`] should want these.
    ///
    /// ⚠ `Zeroizing`, so the copy is wiped when the caller drops it. The
    /// `SigningKey` wipes its own scalar on drop; a bare `[u8; 32]` handed out
    /// here would be a copy that outlived the lock and sat in freed memory for
    /// a core dump or a swap file to find.
    pub fn secret(&self) -> Zeroizing<[u8; 32]> {
        let mut bytes = self.key.to_bytes();
        let mut out = Zeroizing::new([0u8; 32]);
        out.copy_from_slice(&bytes);
        bytes.as_mut_slice().zeroize();
        out
    }

    /// `0x…` checksummed per EIP-55.
    pub fn address(&self) -> String {
        let pubkey = self.key.verifying_key().to_encoded_point(false);
        // Drop the 0x04 prefix: the address is over the 64 coordinate bytes.
        let hash = Keccak256::digest(&pubkey.as_bytes()[1..]);
        to_checksum(&hash[12..])
    }

    /// Sign an EIP-191 `personal_sign` message.
    ///
    /// The prefix is not decoration: it is what stops a signature over a
    /// message from ever being a valid signature over a *transaction*. A
    /// signer that omitted it could be asked to "sign a greeting" and produce
    /// something that moves money.
    pub fn sign_message(&self, message: &str) -> Result<String, String> {
        let digest = eip191_digest(message.as_bytes());
        let (sig, recid): (Signature, RecoveryId) = self
            .key
            .sign_prehash_recoverable(&digest)
            .map_err(|e| format!("signing failed: {e}"))?;
        let mut out = Vec::with_capacity(65);
        out.extend_from_slice(&sig.r().to_bytes());
        out.extend_from_slice(&sig.s().to_bytes());
        // Ethereum's v is the recovery id plus 27. Not 0/1, and not 35+chain —
        // that is EIP-155, which is for transactions and not for this.
        out.push(recid.to_byte() + 27);
        Ok(format!("0x{}", hex(&out)))
    }
}

/// Who signed this message — the inverse of [`Account::sign_message`].
///
/// # Why this is in the client at all
///
/// Signing is what a holder does with their own key; **recovering is how
/// anyone checks somebody else's**, and until this existed the client could
/// produce signatures and verify none. That is the gap that made `publisher`
/// in a manifest a string somebody typed: a depot could be hash-verified
/// byte-for-byte and still say nothing about *who* published it.
///
/// # It costs no dependency, and that decided the scheme
///
/// `k256` is already here with `ecdsa`, so secp256k1 recovery is free while an
/// ed25519 verifier measured **+10 packages against 4 of headroom** — over the
/// budget in `supply_chain.rs`, which the workspace manifest makes an operator
/// decision rather than a shrug. Secp256k1 is also the better key for this
/// job: a publisher identified by an **Ethereum address** is one the chain,
/// a contract, a merkle drop and every wallet already understand, where a
/// second ed25519 identity would be legible only to us.
///
/// Takes the signature as `0x`-prefixed or bare hex, 65 bytes, `v` as 27/28
/// (what every wallet emits) or 0/1.
pub fn recover_signer(message: &str, signature: &str) -> Result<String, String> {
    use k256::ecdsa::VerifyingKey;

    let raw = signature.trim();
    let raw = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")).unwrap_or(raw);
    if raw.len() != 130 {
        // The value is a public signature, not a secret, but saying the shape
        // is still the more useful error.
        return Err(format!(
            "a signature is 65 bytes (130 hex characters); that was {}",
            raw.len()
        ));
    }
    let bytes = unhex(raw)?;

    // v last, and both conventions accepted. 27/28 is what wallets emit;
    // 0/1 is the raw recovery id, and refusing it would reject signatures that
    // are perfectly valid for a reason no caller could guess from the message.
    let v = bytes[64];
    let recid = match v {
        27 | 28 => v - 27,
        0 | 1 => v,
        // ⚠ NOT accepted: EIP-155's 35+2*chain_id. That encoding belongs to
        // transactions, and quietly taking it here would mean a signature
        // authorising a transfer could be replayed as a signature over a
        // manifest. The prefix in `eip191_digest` exists to keep those two
        // apart; this keeps the other end of it apart too.
        other => return Err(format!(
            "unexpected recovery byte {other} — expected 27/28, or 0/1"
        )),
    };
    let recid = RecoveryId::from_byte(recid).ok_or("bad recovery id")?;
    let sig = Signature::from_slice(&bytes[..64]).map_err(|e| format!("bad signature: {e}"))?;

    let digest = eip191_digest(message.as_bytes());
    let key = VerifyingKey::recover_from_prehash(&digest, &sig, recid)
        .map_err(|e| format!("no key recovers from that signature: {e}"))?;
    let point = key.to_encoded_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    Ok(to_checksum(&hash[12..]))
}

/// `keccak256("\x19Ethereum Signed Message:\n" || len || message)`.
///
/// Split out because it is the piece worth testing against a published vector:
/// everything else here is a library call, and this is the assembly.
pub fn eip191_digest(message: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(b"\x19Ethereum Signed Message:\n");
    h.update(message.len().to_string().as_bytes());
    h.update(message);
    h.finalize().into()
}

/// EIP-55: the address, with letters cased by the hash of its own hex.
///
/// Worth having rather than lowercasing, because a checksummed address is
/// self-validating — a typo has a 1-in-2^32-ish chance of surviving it, and
/// every wallet a player pastes into will check.
pub fn to_checksum(addr20: &[u8]) -> String {
    let lower = hex(addr20);
    let hash = Keccak256::digest(lower.as_bytes());
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, c) in lower.chars().enumerate() {
        let nibble = if i % 2 == 0 { hash[i / 2] >> 4 } else { hash[i / 2] & 0x0f };
        if c.is_ascii_digit() || nibble < 8 {
            out.push(c);
        } else {
            out.push(c.to_ascii_uppercase());
        }
    }
    out
}

/// Re-case an address string per EIP-55, whatever case it arrives in.
///
/// ⚠ **Exists because of a bug a round trip showed and no test did.** The V3
/// format stores the address lowercase and unprefixed, so a LOCKED account
/// read straight out of the file rendered `0x8e66…` while the same account
/// unlocked rendered `0x8E66…` — one account, two strings, depending on
/// whether a passphrase had been typed. A player comparing that against their
/// wallet sees a mismatch and has no way to know which one is theirs.
/// Returns `None` for anything that is not 20 bytes of hex.
pub fn checksum_address(raw: &str) -> Option<String> {
    let bytes = unhex(raw).ok()?;
    if bytes.len() != 20 {
        return None;
    }
    Some(to_checksum(&bytes))
}

/// An address someone typed or pasted, as 20 bytes.
///
/// Mixed case must be a correct EIP-55 checksum, so a pasted address with one
/// character changed is refused rather than paid. All-lowercase and
/// all-uppercase carry no checksum and are taken as they are.
pub fn parse_address(typed: &str) -> Result<[u8; 20], String> {
    let t = typed.trim();
    let body = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    if body.len() != 40 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("An address is 0x and 40 hex characters.".into());
    }
    let bytes = unhex(body)?;
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    let mixed = body.chars().any(|c| c.is_ascii_lowercase()) && body.chars().any(|c| c.is_ascii_uppercase());
    if mixed && to_checksum(&out)[2..] != *body {
        return Err("That address fails its checksum: a character was changed or mistyped. \
                    Copy it again from where you got it."
            .into());
    }
    Ok(out)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub(crate) fn unhex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() % 2 != 0 {
        return Err("odd-length hex".into());
    }
    // ⚠ Checked as BYTES before anything is sliced. A `&str` sliced two bytes
    // at a time panics when a multibyte character straddles the cut, and under
    // the release profile's `panic = "abort"` a manifest with an `é` in its
    // signature killed the verifier meant to call it malformed.
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("not hex".into());
    }
    s.as_bytes()
        .chunks(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|e| e.to_string())?;
            u8::from_str_radix(pair, 16).map_err(|e| e.to_string())
        })
        .collect()
}
