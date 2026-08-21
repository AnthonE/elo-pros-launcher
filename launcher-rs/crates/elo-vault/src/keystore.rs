//! Web3 Secret Storage V3 — scrypt + AES-128-CTR, the format every Ethereum
//! tool already reads.
//!
//! **The format is chosen so the account is not trapped here.** A keystore this
//! writes opens in geth, in MetaMask's import, in `cast wallet`. A launcher
//! that invented its own encrypted-key format would be a launcher you cannot
//! leave, which is the same moat this whole client refuses elsewhere.
//!
//! Every primitive is a library call (`scrypt`, `aes`, `ctr`, `sha3`). What is
//! implemented here is the **assembly** — which bytes go where, in the order
//! the spec says — and that is the part with published vectors, so that is the
//! part the tests check.
//!
//! ⚠ **The MAC is checked before the plaintext is used, and a mismatch is a
//! wrong passphrase rather than a corrupt file.** Those are the same bytes and
//! different sentences; telling a player their file is corrupt when they simply
//! mistyped is how someone deletes the only copy of their key.

use crate::{hex, unhex};
use aes::cipher::{KeyIvInit, StreamCipher};
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};

type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

/// scrypt work factor. geth's "standard" is n = 262144, and MetaMask uses the
/// same. **Lowering it is a decision**: it is the only thing standing between a
/// stolen keystore file and a passphrase-guessing rig.
pub const SCRYPT_LOG_N: u8 = 18; // 2^18 = 262144
pub const SCRYPT_R: u32 = 8;
pub const SCRYPT_P: u32 = 1;
pub const DKLEN: usize = 32;

#[derive(Debug, Clone)]
pub struct KdfParams {
    pub log_n: u8,
    pub r: u32,
    pub p: u32,
    pub salt: Vec<u8>,
}

/// A decrypted-and-forgotten keystore document.
pub struct Keystore {
    pub json: Value,
}

/// Encrypt a secret under a passphrase, producing a V3 document.
pub fn encrypt(secret: &[u8; 32], passphrase: &str, address: &str) -> Result<Value, String> {
    let mut salt = [0u8; 32];
    let mut iv = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| format!("no OS entropy: {e}"))?;
    getrandom::fill(&mut iv).map_err(|e| format!("no OS entropy: {e}"))?;

    let dk = derive(passphrase, &salt, SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P)?;
    let mut ciphertext = *secret;
    Aes128Ctr::new(dk[..16].into(), (&iv).into()).apply_keystream(&mut ciphertext);
    let mac = mac_of(&dk[16..32], &ciphertext);

    Ok(json!({
        "version": 3,
        // ⚠ **`id` is not decoration, and leaving it out cost us the whole
        // portability claim above.** This file's own header says a keystore it
        // writes opens in geth, MetaMask and `cast wallet`; without `id` it
        // opened in NONE of them, because their shared `eth-keystore` decoder
        // types the field as required and fails deserialisation before it ever
        // reaches the passphrase — `Failed to decrypt keystore: missing field
        // 'id'`, which reads like a bad passphrase and is not one.
        //
        // Nothing in this crate reads it back, which is exactly why it was
        // missed: every test here round-trips through `decrypt` in this same
        // file, and a field only a FOREIGN reader requires is invisible to a
        // round-trip. Found by opening a real keystore with `cast`, and
        // `tests/vectors.rs` now asserts the field rather than the round trip.
        "id": uuid_v4()?,
        // Lowercase and unprefixed, which is what the format says. The
        // checksummed form lives in the launcher's settings, not in here.
        "address": address.trim_start_matches("0x").to_lowercase(),
        "crypto": {
            "cipher": "aes-128-ctr",
            "cipherparams": {"iv": hex(&iv)},
            "ciphertext": hex(&ciphertext),
            "kdf": "scrypt",
            "kdfparams": {
                "dklen": DKLEN,
                "n": 1u32 << SCRYPT_LOG_N,
                "p": SCRYPT_P,
                "r": SCRYPT_R,
                "salt": hex(&salt),
            },
            "mac": hex(&mac),
        },
    }))
}

/// A random UUID v4, formatted 8-4-4-4-12.
///
/// Hand-written because it is a layout, not a primitive: sixteen bytes from the
/// same OS pool the key came from, with the version and variant nibbles pinned
/// per RFC 4122 §4.4. A `uuid` crate would be another package on a budget
/// (`supply_chain.rs`) to print thirty-two hex characters with four dashes.
fn uuid_v4() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| format!("no OS entropy: {e}"))?;
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 10xx
    let h = hex(&b);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]
    ))
}

/// Decrypt a V3 document, or say why not.
///
/// Refuses anything it does not fully understand rather than guessing: a
/// keystore with a cipher or KDF this does not implement is an error, never a
/// best effort. Guessing here would produce 32 bytes that are not the key, and
/// the first symptom would be a signature nobody can verify.
pub fn decrypt(doc: &Value, passphrase: &str) -> Result<[u8; 32], String> {
    if doc.get("version").and_then(Value::as_i64) != Some(3) {
        return Err(format!(
            "this reads Web3 Secret Storage v3; the file says version {}",
            doc.get("version").unwrap_or(&json!("(absent)"))
        ));
    }
    let c = doc.get("crypto").ok_or("no `crypto` section")?;
    let cipher = c.get("cipher").and_then(Value::as_str).unwrap_or("");
    if cipher != "aes-128-ctr" {
        return Err(format!("unsupported cipher {cipher:?} — this reads aes-128-ctr"));
    }
    let kdf = c.get("kdf").and_then(Value::as_str).unwrap_or("");
    if kdf != "scrypt" {
        return Err(format!(
            "unsupported kdf {kdf:?} — this reads scrypt. A pbkdf2 keystore \
             opens in geth, which can rewrite it."
        ));
    }
    let p = c.get("kdfparams").ok_or("no kdfparams")?;
    let salt = unhex(p.get("salt").and_then(Value::as_str).ok_or("no salt")?)?;
    let n = p.get("n").and_then(Value::as_u64).ok_or("no n")?;
    if !n.is_power_of_two() || n < 2 {
        return Err(format!("scrypt n must be a power of two, got {n}"));
    }
    let log_n = n.trailing_zeros() as u8;
    let r = p.get("r").and_then(Value::as_u64).ok_or("no r")? as u32;
    let pp = p.get("p").and_then(Value::as_u64).ok_or("no p")? as u32;

    let iv = unhex(
        c.get("cipherparams")
            .and_then(|x| x.get("iv"))
            .and_then(Value::as_str)
            .ok_or("no iv")?,
    )?;
    let ciphertext = unhex(c.get("ciphertext").and_then(Value::as_str).ok_or("no ciphertext")?)?;
    let want_mac = unhex(c.get("mac").and_then(Value::as_str).ok_or("no mac")?)?;

    let dk = derive(passphrase, &salt, log_n, r, pp)?;

    // The MAC first, ALWAYS. Decrypting before checking would hand a caller
    // plaintext derived from the wrong passphrase, and there is no later point
    // at which that becomes detectable.
    let got = mac_of(&dk[16..32], &ciphertext);
    if got.as_slice() != want_mac.as_slice() {
        return Err("wrong passphrase (the file's MAC does not match)".into());
    }

    if ciphertext.len() != 32 {
        return Err(format!(
            "the ciphertext is {} bytes; a secp256k1 key is 32",
            ciphertext.len()
        ));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&ciphertext);
    if iv.len() != 16 {
        return Err(format!("the iv is {} bytes; aes-128-ctr wants 16", iv.len()));
    }
    Aes128Ctr::new(dk[..16].into(), iv.as_slice().into()).apply_keystream(&mut out);
    Ok(out)
}

/// `keccak256(derived[16..32] || ciphertext)` — the V3 MAC.
fn mac_of(half: &[u8], ciphertext: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(half);
    h.update(ciphertext);
    h.finalize().into()
}

fn derive(passphrase: &str, salt: &[u8], log_n: u8, r: u32, p: u32) -> Result<[u8; 32], String> {
    let params = scrypt::Params::new(log_n, r, p, DKLEN)
        .map_err(|e| format!("bad scrypt parameters: {e}"))?;
    let mut dk = [0u8; DKLEN];
    scrypt::scrypt(passphrase.as_bytes(), salt, &params, &mut dk)
        .map_err(|e| format!("scrypt failed: {e}"))?;
    Ok(dk)
}
