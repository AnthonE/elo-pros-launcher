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

pub mod keystore;
pub mod local;

pub use keystore::{decrypt, encrypt, KdfParams, Keystore};
pub use local::LocalSigner;

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
        let base = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
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
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| format!("no OS entropy: {e}"))?;
        let key = SigningKey::from_slice(&bytes)
            .map_err(|e| format!("the drawn bytes are not a valid key: {e}"))?;
        Ok(Account { key })
    }

    pub fn from_secret(bytes: &[u8]) -> Result<Account, String> {
        SigningKey::from_slice(bytes)
            .map(|key| Account { key })
            .map_err(|e| format!("not a secp256k1 key: {e}"))
    }

    /// The 32 secret bytes. Only [`encrypt`] should want these.
    pub fn secret(&self) -> [u8; 32] {
        self.key.to_bytes().into()
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
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}
