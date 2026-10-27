//! The digest — the one number in this crate that becomes money.
//!
//! A depot's digest is a `bytes32` committed on chain by `ScryNotary`
//! (`0x0C15fA7829458118e3d26229F58FE0443f8b792c` on 4663), which is what lets
//! a player ask *is the build I just installed the build the house listed?*
//! and answer it from a block explorer without trusting the launcher, the CDN,
//! or us.
//!
//! ⚠ **That makes a second implementation a bug** — `CLAUDE.md` invariant 3
//! applied to this layer, and this file IS a second implementation of
//! `depot.py:digest()`. It is only allowed to exist because it is pinned
//! byte-for-byte against frozen golden vectors by `tests/digest_parity.rs`, which
//! runs both over the same fixtures and compares. If that test is deleted or
//! skipped, this file becomes the bug the invariant is about.

use crate::error::{DepotError, Result};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// Canonical JSON: sorted keys, no whitespace, UTF-8, non-ASCII left raw.
///
/// Must produce the same bytes as Python's
/// `json.dumps(obj, sort_keys=True, separators=(",",":"), ensure_ascii=False)`.
/// Three properties carry that, and each is load-bearing:
///
/// * `serde_json::Value` is backed by `BTreeMap` unless the `preserve_order`
///   feature is on — so keys come out sorted, matching `sort_keys=True`.
///   **Never enable `preserve_order` in this workspace.**
/// * `to_vec` is the compact form, matching `separators=(",",":")`.
/// * serde_json does not escape non-ASCII, matching `ensure_ascii=False`.
///   Both escape the same control set (`\b \f \n \r \t`, else `\u00XX`).
pub fn canonical(value: &Value) -> Result<Vec<u8>> {
    reject_floats(value, "")?;
    serde_json::to_vec(value).map_err(|e| DepotError::new(format!("cannot canonicalize: {e}")))
}

/// Floats are refused rather than hashed, and this is deliberate.
///
/// Python and Rust agree on `1.0` and disagree on shapes like `1e+100` vs
/// `1e100`. A depot has no legitimate float field — sizes are integers, hashes
/// are strings — so the only way one arrives is a malformed depot. Refusing is
/// the failure that gets noticed; hashing it is a digest that silently
/// disagrees with the notarized one, which is the expensive failure.
fn reject_floats(value: &Value, path: &str) -> Result<()> {
    match value {
        Value::Number(n) if n.as_f64().is_some() && !n.is_i64() && !n.is_u64() => {
            Err(DepotError::new(format!(
                "depot field {} is a float ({n}); the digest accepts integers and strings only",
                if path.is_empty() { "(root)" } else { path }
            )))
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                reject_floats(item, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for (k, v) in map {
                let child = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                reject_floats(v, &child)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The `bytes32` that gets notarized. `0x`-prefixed, lowercase.
///
/// `notary` and `signature` are excluded because they are written *about* this
/// digest — including them would make the value unfixable.
pub fn digest(depot: &Value) -> Result<String> {
    let body = match depot {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if k != "notary" && k != "signature" {
                    out.insert(k.clone(), v.clone());
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    };
    let bytes = canonical(&body)?;
    Ok(format!("0x{}", hex_lower(&Sha256::digest(&bytes))))
}

pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// sha256 of a file, streamed. Same chunk size as the Python port so a large
/// build behaves the same way on the same disk.
///
/// `ring`'s SHA-256 rather than `sha2`'s: the same function, in BoringSSL's
/// assembly, and about three times as fast in a release build (measured
/// 2026-10-25: 310 MB/s against 105 on the same 300 MB file). Every byte of an
/// install and a verify goes through here — Gates is 584 MB — so this is the
/// install's CPU cost. Small documents still hash with `sha2`; the parity
/// vectors pin both to the same answer.
pub fn sha256_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|e| DepotError::new(format!("{}: {e}", path.display())))?;
    let mut hasher = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; crate::CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| DepotError::new(format!("{}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(hasher.finish().as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ring` hashes the files and `sha2` hashes the documents, so the two must
    /// agree byte for byte — FIPS 180-2's own vectors, and a file larger than
    /// one read so the chunking is exercised too.
    #[test]
    fn a_file_hashes_to_the_same_sha256_either_library_computes() {
        let dir = tempfile::tempdir().unwrap();
        let abc = dir.path().join("abc");
        std::fs::write(&abc, b"abc").unwrap();
        assert_eq!(sha256_file(&abc).unwrap(),
                   "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(sha256_file(&empty).unwrap(),
                   "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let big: Vec<u8> = (0..crate::CHUNK * 3 + 17).map(|i| (i * 31 % 251) as u8).collect();
        let path = dir.path().join("big");
        std::fs::write(&path, &big).unwrap();
        assert_eq!(sha256_file(&path).unwrap(), hex_lower(&Sha256::digest(&big)));
    }
}
