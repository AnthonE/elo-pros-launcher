//! A voice — the key the client speaks in the hive with.
//!
//! ## This key is not the wallet, and the difference is the whole design
//!
//! The hive is Nostr, so speaking needs a **schnorr (BIP-340) secp256k1** key.
//! The launcher's signer ladder is **ECDSA** — a different scheme over the
//! same curve — so a hive voice cannot be the wallet and is not a second copy
//! of it. What that buys, said plainly because it is the reason this is
//! allowed to exist at all:
//!
//! * a voice **cannot move money.** It has no balance, signs no transaction,
//!   and is not on chain. `CLAUDE.md` invariant 7 is untouched: elo never
//!   sees it, and this is not ours;
//! * a voice **can speak as you**, which is worth protecting even though it is
//!   not worth a passphrase on every message;
//! * binding a voice to a wallet is a separate, deliberate act
//!   (`POST /api/hive/npub`), and until it happens the key carries no record.
//!
//! ⚠ **At rest it is a file at 0600, not an encrypted keystore, and every
//! surface says so.** `vault.py` wraps the *wallet* in scrypt+AES because that
//! key holds money and is used rarely. A chat key is used on every message,
//! and a passphrase prompt per line makes a client nobody uses — which is its
//! own kind of dishonesty, because the feature then exists only on paper. The
//! trade is stated rather than hidden: anyone who can read your home directory
//! can speak as you in the hive, and cannot touch anything else.
//!
//! Signing itself is `k256`, not code written here — the same reasoning
//! `SDK.md` §2a used for `eth-account`. A subtly wrong nonce leaks the key
//! silently, and that is not a bug this repo gets to discover in production.

use crate::error::{HiveError, Result};
// ⚠ `Signer`/`Verifier` are DELIBERATELY not imported. `k256`'s blanket
// `Signer::try_sign(msg)` runs `Sha256::new_with_prefix(msg)` first, so it
// signs `sha256(msg)` — correct for signing arbitrary documents, and WRONG for
// Nostr, which signs the 32-byte event id itself. `sign_prehash` /
// `verify_prehash` are the raw-32-bytes pair and the only correct ones here.
//
// This cost a real bug: the Rust self-check passed because it signed and
// verified with the same wrong convention, and only `tests/parity_verify.py`
// — which runs the meter's own BIP-340 verifier over our output — caught it.
use k256::schnorr::signature::hazmat::{PrehashSigner, PrehashVerifier};
use k256::schnorr::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const KEY_NAME: &str = "voice.nsec";

/// The text a wallet signs ONCE to derive its chat key — byte-for-byte
/// `meter/hive.py` `derivation_message(wallet)` (wallet-scoped). The origin
/// hands the same text out (`GET /api/hive/link`), and a client that compares
/// the two refuses a text that differs rather than deriving a key the site
/// would not.
pub fn derivation_message(wallet: &str) -> String {
    format!(
        "scry hive voice v1\nwallet: {}\nthis signature IS my hive voice key.\n\
         sign it only to derive your scry hive identity — never on any other site.",
        wallet.trim().to_lowercase()
    )
}

/// What a chat key signs (a NIP-01 event's content) to say it speaks for
/// `wallet` — byte-for-byte `meter/hive.py` `link_content`. Compared against
/// the origin's copy before anything is signed, like [`derivation_message`].
pub fn link_content(wallet: &str) -> String {
    format!(
        "elo chat key\nwallet: {}\nthis key speaks for this wallet in Elo Pros chat.",
        wallet.trim().to_lowercase()
    )
}

/// Where this machine's chat key lives: `ELO_VOICE_DIR`, else `voice/` under
/// the client's state root. Defined here and only here, so `elo chat` and the
/// window's Chat pane speak with the same key.
pub fn default_dir() -> PathBuf {
    if let Ok(p) = std::env::var("ELO_VOICE_DIR") {
        return PathBuf::from(p);
    }
    elo_depot::dirs::state_root().join("voice")
}

/// A secret key that can speak. Never `Debug`-printed, never serialized.
pub struct Voice {
    signing: SigningKey,
}

impl std::fmt::Debug for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A key that prints itself into a log is a key that leaks. The npub is
        // public and identifies it well enough for any message we would write.
        write!(f, "Voice({})", self.npub())
    }
}

impl Voice {
    /// A new voice from the OS entropy pool.
    pub fn generate() -> Result<Voice> {
        for _ in 0..8 {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes)
                .map_err(|e| HiveError::new(format!("no entropy available: {e}")))?;
            if let Ok(signing) = SigningKey::from_bytes(&bytes) {
                return Ok(Voice { signing });
            }
            // The scalar was out of range. Astronomically unlikely, and
            // retrying is correct — clamping would bias the key.
        }
        Err(HiveError::new("could not draw a valid key from the entropy pool"))
    }

    /// The wallet's own chat key, from its EIP-191 signature over
    /// [`derivation_message`] — **the same key the website makes from the same
    /// wallet** (`meter/hive.py` `derive_seckey`, `js/schnorr.js`
    /// `deriveFromSignature`): `sha256(r‖s) mod n`, with `0x01` appended and
    /// rehashed in the (astronomically unlikely) zero case. `v` is dropped
    /// because wallets disagree on how to write it; `r‖s` is RFC 6979
    /// deterministic and low-s, so one wallet gives one key on every client.
    pub fn from_wallet_signature(signature_hex: &str) -> Result<Voice> {
        use k256::elliptic_curve::ops::Reduce;
        let raw = unhex(signature_hex.trim().trim_start_matches("0x"))?;
        if raw.len() < 64 {
            return Err(HiveError::new("a wallet signature is 65 bytes (r‖s‖v)"));
        }
        let mut rs = raw[..64].to_vec();
        loop {
            let digest = Sha256::digest(&rs);
            let d = <k256::Scalar as Reduce<k256::U256>>::reduce_bytes(&digest);
            if !bool::from(d.is_zero()) {
                return Voice::from_secret_bytes(&d.to_bytes());
            }
            rs.push(1);
        }
    }

    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Voice> {
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| HiveError::new("a secret key is 32 bytes"))?;
        Ok(Voice {
            signing: SigningKey::from_bytes(&arr)
                .map_err(|_| HiveError::new("not a valid secp256k1 secret key"))?,
        })
    }

    /// `nsec1…` — NIP-19 bech32. The form a person moves between clients.
    pub fn from_nsec(nsec: &str) -> Result<Voice> {
        let (hrp, data) = bech32::decode(nsec.trim())
            .map_err(|e| HiveError::new(format!("not a bech32 key: {e}")))?;
        if hrp.as_str() != "nsec" {
            return Err(HiveError::new(format!(
                "expected an nsec, got an {}",
                hrp.as_str()
            )));
        }
        Voice::from_secret_bytes(&data)
    }

    pub fn to_nsec(&self) -> String {
        let hrp = bech32::Hrp::parse_unchecked("nsec");
        bech32::encode::<bech32::Bech32>(hrp, &self.signing.to_bytes())
            .expect("32 bytes always encode")
    }

    /// The x-only public key, 32 bytes, lowercase hex — what a Nostr event
    /// carries as `pubkey`.
    pub fn pubkey_hex(&self) -> String {
        hex(&self.signing.verifying_key().to_bytes())
    }

    /// `npub1…` — NIP-19 bech32 of the same 32 bytes.
    pub fn npub(&self) -> String {
        let hrp = bech32::Hrp::parse_unchecked("npub");
        bech32::encode::<bech32::Bech32>(hrp, &self.signing.verifying_key().to_bytes())
            .expect("32 bytes always encode")
    }

    /// Sign a prepared event, filling in `pubkey`, `id` and `sig`.
    pub fn sign(&self, unsigned: UnsignedEvent) -> Result<Event> {
        let pubkey = self.pubkey_hex();
        let id = unsigned.id(&pubkey);
        let raw = unhex(&id)?;
        let sig: Signature = self
            .signing
            .sign_prehash(&raw)
            .map_err(|e| HiveError::new(format!("could not sign: {e}")))?;
        Ok(Event {
            id,
            pubkey,
            created_at: unsigned.created_at,
            kind: unsigned.kind,
            tags: unsigned.tags,
            content: unsigned.content,
            sig: hex(&sig.to_bytes()),
        })
    }

    /// Write to `dir/voice.nsec` at 0600, in a 0700 directory.
    ///
    /// Through the vault's private writer: 0600 from the first byte, and a
    /// link at the path is replaced rather than followed.
    pub fn save(&self, dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        harden(dir, 0o700)?;
        let path = dir.join(KEY_NAME);
        let body = elo_vault::Zeroizing::new(self.to_nsec() + "\n");
        elo_vault::write_private(&path, body.as_bytes(), false)?;
        Ok(path)
    }

    pub fn load(dir: &Path) -> Result<Voice> {
        let path = dir.join(KEY_NAME);
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                HiveError::new("no voice on this machine yet — `elo chat voice-new` makes one")
            } else {
                HiveError::new(format!("{}: {e}", path.display()))
            }
        })?;
        Voice::from_nsec(&raw)
    }

    pub fn exists(dir: &Path) -> bool {
        dir.join(KEY_NAME).is_file()
    }
}

#[cfg(unix)]
fn harden(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn harden(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// An event before it has an id, a pubkey or a signature.
#[derive(Debug, Clone)]
pub struct UnsignedEvent {
    pub created_at: i64,
    pub kind: i64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

impl UnsignedEvent {
    /// The NIP-01 id: sha256 over the compact JSON array
    /// `[0, pubkey, created_at, kind, tags, content]`.
    ///
    /// The serialization has to be **exactly** this — no spaces, no reordering,
    /// UTF-8 — or the id changes and every relay rejects the signature. It is
    /// the same class of rule as the depot digest, and for the same reason:
    /// a hash of a canonical form is only canonical if everyone agrees.
    pub fn id(&self, pubkey_hex: &str) -> String {
        let serialized = json!([
            0,
            pubkey_hex,
            self.created_at,
            self.kind,
            self.tags,
            self.content
        ]);
        let bytes = serde_json::to_vec(&serialized).expect("an array of plain values");
        hex(&Sha256::digest(&bytes))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub kind: i64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

impl Event {
    /// Recompute the id and check the signature.
    ///
    /// Used on our own output before it is sent. Verifying what we just signed
    /// looks redundant and is not: it catches a serialization drift between
    /// `id()` and the wire the moment it happens, rather than as a silent
    /// relay rejection nobody sees.
    ///
    /// It is also what every event read off a relay goes through
    /// (`relay::checked`), so the shape is checked before anything is decoded:
    /// NIP-01 says lowercase hex, 32 bytes for `id` and `pubkey` and 64 for
    /// `sig`, and a relay's bytes get no further than that.
    pub fn verify(&self) -> Result<()> {
        if !is_lower_hex(&self.id, 64)
            || !is_lower_hex(&self.pubkey, 64)
            || !is_lower_hex(&self.sig, 128)
        {
            return Err(HiveError::new("event id, pubkey or sig is not NIP-01 hex"));
        }
        let unsigned = UnsignedEvent {
            created_at: self.created_at,
            kind: self.kind,
            tags: self.tags.clone(),
            content: self.content.clone(),
        };
        if unsigned.id(&self.pubkey) != self.id {
            return Err(HiveError::new("event id does not match its own contents"));
        }
        let vk = VerifyingKey::from_bytes(&unhex(&self.pubkey)?)
            .map_err(|_| HiveError::new("event pubkey is not a valid x-only key"))?;
        let sig = Signature::try_from(unhex(&self.sig)?.as_slice())
            .map_err(|_| HiveError::new("event signature is malformed"))?;
        vk.verify_prehash(&unhex(&self.id)?, &sig)
            .map_err(|_| HiveError::new("event signature does not check out"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("plain fields")
    }
}

pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Decode hex, either case.
///
/// ⚠ **Byte by byte, never a `&str` slice.** What reaches this is often a
/// relay's: a slice through the middle of a multi-byte char is a panic, and
/// the release profile is `panic = "abort"`, so one event would end the
/// launcher. (`from_str_radix` also took a leading `+`, which is not hex.)
pub fn unhex(s: &str) -> Result<Vec<u8>> {
    fn nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(HiveError::new("hex must have an even number of digits"));
    }
    bytes
        .chunks_exact(2)
        .map(|p| match (nibble(p[0]), nibble(p[1])) {
            (Some(hi), Some(lo)) => Ok(hi << 4 | lo),
            _ => Err(HiveError::new("not hex")),
        })
        .collect()
}

/// Encode an event's 32-byte hex `pubkey` as the `npub1…` every surface shows.
///
/// An event read off the relay carries hex; the origin's mirror carries npubs.
/// A feed that merges the two has to speak one of them, and the npub is the
/// one a person recognises and the one the name cache is keyed by.
pub fn hex_to_npub(pubkey_hex: &str) -> Result<String> {
    let bytes = unhex(pubkey_hex)?;
    if bytes.len() != 32 {
        return Err(HiveError::new("a pubkey is 32 bytes"));
    }
    let hrp = bech32::Hrp::parse_unchecked("npub");
    bech32::encode::<bech32::Bech32>(hrp, &bytes)
        .map_err(|e| HiveError::new(format!("could not encode an npub: {e}")))
}

/// Decode an `npub1…` to its 32-byte hex, for addressing someone.
pub fn npub_to_hex(npub: &str) -> Result<String> {
    let (hrp, data) = bech32::decode(npub.trim())
        .map_err(|e| HiveError::new(format!("not a bech32 npub: {e}")))?;
    if hrp.as_str() != "npub" {
        return Err(HiveError::new(format!(
            "expected an npub, got an {}",
            hrp.as_str()
        )));
    }
    if data.len() != 32 {
        return Err(HiveError::new("an npub is 32 bytes"));
    }
    Ok(hex(&data))
}
