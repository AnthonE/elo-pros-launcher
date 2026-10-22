//! What a hostile input or a co-resident user can do to this crate, asserted.
//!
//! `vectors.rs` checks the assembly against published answers; this file
//! checks the edges an attacker reaches: a string that is not hex, a path that
//! is not a plain file, and how long key material outlives the lock.

use elo_vault::{keystore, Account, Zeroizing};

/// **A non-ASCII signature is an error, never a panic.** `unhex` sliced a
/// `&str` two bytes at a time, so `é` straddling a slice boundary panicked,
/// and the release profile's `panic = "abort"` turned a malformed manifest
/// into a dead `elo games` for everyone it was served to.
#[test]
fn a_multibyte_signature_is_refused_not_a_crash() {
    // 130 BYTES after the prefix, so the length check passes and the decode
    // is reached: "a", then a two-byte "é", then 127 more.
    let sig = format!("0xa\u{e9}{}", "a".repeat(127));
    assert_eq!(sig.len() - 2, 130);
    let got = std::panic::catch_unwind(|| elo_vault::recover_signer("m", &sig));
    let got = got.expect("recover_signer panicked on a multibyte character");
    assert!(got.is_err(), "non-hex must be refused: {got:?}");

    // The address path shares `unhex`: 40 bytes, one of them inside a char.
    let addr = format!("0xa\u{e9}{}", "a".repeat(37));
    assert_eq!(addr.len() - 2, 40);
    let got = std::panic::catch_unwind(|| elo_vault::checksum_address(&addr));
    assert_eq!(got.expect("checksum_address panicked"), None);
}

/// **The decrypted key and the KDF output are wiped when dropped.** Locking is
/// forgetting only if no plain `[u8; 32]` copy outlives it, so the types carry
/// the promise: a caller cannot get the secret back as a bare array.
#[test]
fn key_material_comes_back_zeroizing() {
    let acct = Account::from_secret(&[3u8; 32]).expect("account");
    let secret: Zeroizing<[u8; 32]> = acct.secret();
    let doc = keystore::encrypt(&secret, "p", &acct.address()).expect("encrypt");
    let back: Zeroizing<[u8; 32]> = keystore::decrypt(&doc, "p").expect("decrypt");
    assert_eq!(*back, [3u8; 32]);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;

    /// Long enough for `local::MIN_PASSPHRASE`, so a refusal is the one tested.
    const PASS: &str = "correct horse battery";

    fn mode(p: &std::path::Path) -> u32 {
        std::fs::metadata(p).expect("stat").permissions().mode() & 0o777
    }

    /// **A directory the keystore needed is made 0700, not 0755.** A
    /// traversable parent is what lets another user watch it and open the
    /// file in the moment it exists.
    #[test]
    fn a_new_keystore_directory_is_private() {
        let root = tempfile::tempdir().expect("tmp");
        let dir = root.path().join("elo");
        let path = dir.join("keystore.json");
        if let Err(e) = elo_vault::LocalSigner::import(&path, PASS, &[5u8; 32]) {
            panic!("import: {e}");
        }
        assert_eq!(mode(&dir), 0o700, "the keystore's own directory is the holder's alone");
        assert_eq!(mode(&path), 0o600);
    }

    /// **A symlink at the keystore path is refused, dangling or not.**
    /// `exists()` is false for a dangling link, and `fs::write` follows it —
    /// so the key went wherever the link pointed.
    #[test]
    fn a_symlink_at_the_keystore_path_is_not_followed() {
        let root = tempfile::tempdir().expect("tmp");
        let target = root.path().join("elsewhere.json");
        let path = root.path().join("keystore.json");
        std::os::unix::fs::symlink(&target, &path).expect("symlink");
        // A passphrase long enough to pass, so the refusal is the link's.
        let made = elo_vault::LocalSigner::import(&path, PASS, &[5u8; 32]);
        let e = made.err().expect("a link at the keystore path must not be written through");
        assert!(e.contains("already exists"), "refused for the link, not something else: {e}");
        assert!(!target.exists(), "nothing may land at the link's target");
    }

    /// The shared writer: created at 0600 from its first byte, and a fresh
    /// write refuses an existing path rather than truncating it.
    #[test]
    fn the_private_writer_is_0600_and_fresh_means_fresh() {
        let root = tempfile::tempdir().expect("tmp");
        let path = root.path().join("sub").join("grants.json");
        elo_vault::write_private(&path, b"{}", false).expect("write");
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        // Overwrite is allowed when asked for, and keeps the mode tight even
        // if the file had been loosened.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        elo_vault::write_private(&path, b"{\"a\":1}", false).expect("rewrite");
        assert_eq!(mode(&path), 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
        assert!(elo_vault::write_private(&path, b"x", true).is_err(), "fresh must not clobber");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
    }

    /// Two threads saving one file at once (the GUI re-pairing twice) both
    /// land. A temp name shared by every thread of a process let one remove
    /// the other's half-written file, and that save failed.
    #[test]
    fn concurrent_saves_of_one_file_all_land() {
        let root = tempfile::tempdir().expect("tmp");
        let path = std::sync::Arc::new(root.path().join("pairing.json"));
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let path = std::sync::Arc::clone(&path);
                std::thread::spawn(move || {
                    (0..200)
                        .filter(|n| {
                            let body = format!("{{\"t\":{t},\"n\":{n}}}");
                            elo_vault::write_private(&path, body.as_bytes(), false).is_err()
                        })
                        .count()
                })
            })
            .collect();
        let lost: usize = threads.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(lost, 0, "{lost} saves were lost to another thread's temp file");
        assert_eq!(mode(&path), 0o600);
        let left = std::fs::read_dir(root.path()).unwrap().count();
        assert_eq!(left, 1, "a temp file was left behind");
    }
}
