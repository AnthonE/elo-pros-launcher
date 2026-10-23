//! A passphrase held for a while — by the order runner, which must sign when an
//! order fires and nobody is at the keyboard — without sitting in memory as
//! itself.
//!
//! It is split in two: a random pad, and the passphrase XOR the pad, in two
//! separately allocated buffers. Neither half is the passphrase, and the two
//! are joined only for the instant of a signature (`with`), into a buffer that
//! is wiped when the closure returns. On unix both halves are `mlock`ed, so the
//! OS never writes them to swap, and both are zeroed on drop.
//!
//! **What this beats:** a crash dump, a swap file, a hibernation image, and a
//! scraper looking for passphrase-shaped strings in memory. **What it does
//! not:** malware that targets this program — it can read both halves, or call
//! `with` itself. Against that the defences are the same-user ptrace and core
//! dump refusals in `harden.rs`, and not running the runner on a machine you
//! do not trust. The honest fix for "no secret held at all" is a person at the
//! keyboard when an order fires.

use zeroize::{Zeroize, Zeroizing};

pub struct Masked {
    pad: Zeroizing<Vec<u8>>,
    body: Zeroizing<Vec<u8>>,
}

#[cfg(unix)]
fn lock(b: &[u8]) {
    if !b.is_empty() {
        // Best effort: a refused lock (RLIMIT_MEMLOCK) still leaves the split.
        unsafe { libc::mlock(b.as_ptr() as *const libc::c_void, b.len()) };
    }
}
#[cfg(unix)]
fn unlock(b: &[u8]) {
    if !b.is_empty() {
        unsafe { libc::munlock(b.as_ptr() as *const libc::c_void, b.len()) };
    }
}
#[cfg(not(unix))]
fn lock(_: &[u8]) {}
#[cfg(not(unix))]
fn unlock(_: &[u8]) {}

impl Masked {
    /// Split `secret` and wipe the caller's copy as it goes.
    pub fn new(mut secret: Zeroizing<String>) -> Result<Masked, String> {
        let n = secret.len();
        let mut pad = Zeroizing::new(vec![0u8; n]);
        getrandom::fill(&mut pad).map_err(|e| format!("no OS entropy: {e}"))?;
        let mut body = Zeroizing::new(vec![0u8; n]);
        for (i, b) in secret.as_bytes().iter().enumerate() {
            body[i] = b ^ pad[i];
        }
        secret.zeroize();
        lock(&pad);
        lock(&body);
        Ok(Masked { pad, body })
    }

    /// Join the halves for `f`, and wipe the joined copy when it returns.
    pub fn with<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        let mut joined = Zeroizing::new(vec![0u8; self.body.len()]);
        for i in 0..joined.len() {
            joined[i] = self.body[i] ^ self.pad[i];
        }
        // The halves came from a String, so this is UTF-8; a passphrase that is
        // not would have failed at `new`'s caller long before.
        let s = Zeroizing::new(String::from_utf8(joined.to_vec()).unwrap_or_default());
        joined.zeroize();
        f(&s)
    }
}

impl Drop for Masked {
    fn drop(&mut self) {
        unlock(&self.pad);
        unlock(&self.body);
        // Zeroizing wipes both buffers as they drop.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_passphrase_comes_back_whole() {
        let m = Masked::new(Zeroizing::new("correct horse battery".to_string())).unwrap();
        assert_eq!(m.with(|p| p.to_string()), "correct horse battery");
        assert_eq!(m.with(|p| p.len()), 21);
    }

    #[test]
    fn neither_half_is_the_passphrase() {
        let secret = "a passphrase long enough to tell";
        let m = Masked::new(Zeroizing::new(secret.to_string())).unwrap();
        assert_ne!(&m.body[..], secret.as_bytes());
        assert_ne!(&m.pad[..], secret.as_bytes());
        assert!(!m.body.windows(8).any(|w| secret.as_bytes().windows(8).any(|s| s == w)));
    }

    #[test]
    fn spaces_at_the_ends_survive() {
        let m = Masked::new(Zeroizing::new(" padded ".to_string())).unwrap();
        assert_eq!(m.with(|p| p.to_string()), " padded ");
    }
}
