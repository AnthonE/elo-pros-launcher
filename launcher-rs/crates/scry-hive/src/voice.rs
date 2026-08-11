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
//!   and is not on chain. `CLAUDE.md` invariant 7 is untouched: scry never
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
    pub fn save(&self, dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        harden(dir, 0o700)?;
        let path = dir.join(KEY_NAME);
        std::fs::write(&path, self.to_nsec() + "\n")?;
        harden(&path, 0o600)?;
        Ok(path)
    }

    pub fn load(dir: &Path) -> Result<Voice> {
        let path = dir.join(KEY_NAME);
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                HiveError::new("no voice on this machine yet — `scry hive voice-new` makes one")
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
    pub fn verify(&self) -> Result<()> {
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

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(HiveError::new("hex must have an even number of digits"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| HiveError::new(format!("not hex: {:?}", &s[i..i + 2])))
        })
        .collect()
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
