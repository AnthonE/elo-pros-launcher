//! `LocalSigner` — the launcher IS the signer, and this is the only backend
//! in the client that holds a key.
//!
//! # Why it lives HERE and not beside the other four
//!
//! `scry-broker::signer` is keyless **by construction**: browser hands off,
//! arca asks a process that holds the key, external asks a program the player
//! named, none refuses. `scry-broker/tests/signer.rs` asserts that against the
//! source, and that assertion is only worth having while it is cheap to keep
//! true. Putting this backend in that file would have forced the test to be
//! re-scoped into a per-backend scan — which is exactly what the Python module
//! had to do, and it says so.
//!
//! So the split is the answer instead: **one crate holds keys and every other
//! crate provably does not.** That is a stronger statement than the Python
//! client can make, and it costs nothing but a file boundary.
//!
//! # The rung, stated plainly
//!
//! This is rung 0 with worse isolation than the browser, not rung 2. There is
//! no far side: the process that draws the window also holds the key, so a
//! compromise of the launcher is a compromise of the key. `ArcaSigner` is the
//! one with a real boundary — its own uid, its own ledger, `SO_PEERCRED`. This
//! backend exists because `STEAM.md` §4 is right that the alternative loses
//! most players at the door, not because it is the safe one.

use crate::{keystore, Account};
use scry_broker::protocol::Refusal;
use scry_broker::signer::Signer;
use serde_json::{json, Value};
use std::path::PathBuf;

/// A local account, unlocked for as long as the launcher is open.
///
/// **The passphrase is never stored and the decrypted key never reaches disk.**
/// Locking is dropping this value; there is no "remember me", because a
/// remembered passphrase is a passphrase the player did not type for the
/// signature that spent their money.
pub struct LocalSigner {
    path: PathBuf,
    account: Option<Account>,
    address: Option<String>,
    /// When the unlocked key last DID something — set by `unlock`, refreshed
    /// by every `sign`. What the GUI's idle relock reads: an account busy
    /// signing for a three-hour session is never idle, and one forgotten
    /// after a sign-in is relocked on the clock rather than at close.
    last_used: Option<std::time::Instant>,
}

impl LocalSigner {
    /// Point at a keystore without opening it. A locked signer is a normal
    /// state and says so rather than erroring.
    pub fn at(path: impl Into<PathBuf>) -> LocalSigner {
        let path = path.into();
        // The address is public and lives in the file in the clear, so it can
        // be shown while locked. That is the whole reason the format keeps it.
        let address = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.get("address").and_then(Value::as_str).map(str::to_string))
            // Checksummed, NOT the file's lowercase — see `checksum_address`.
            // One account must render as one string whether it is locked or not.
            .and_then(|a| crate::checksum_address(&a));
        LocalSigner { path, account: None, address, last_used: None }
    }

    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    pub fn is_unlocked(&self) -> bool {
        self.account.is_some()
    }

    /// Unlock with a passphrase. Wrong is *wrong passphrase*, never "corrupt".
    pub fn unlock(&mut self, passphrase: &str) -> Result<(), String> {
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("cannot read {}: {e}", self.path.display()))?;
        let doc: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("{} is not JSON: {e}", self.path.display()))?;
        let secret = keystore::decrypt(&doc, passphrase)?;
        let account = Account::from_secret(&secret)?;
        self.address = Some(account.address());
        self.account = Some(account);
        self.last_used = Some(std::time::Instant::now());
        Ok(())
    }

    /// Forget the key. Called on close, by the GUI's idle relock, and
    /// available to a player who wants it gone without quitting. Dropping the
    /// `Account` drops its `SigningKey`, whose scalar zeroizes on drop
    /// (RustCrypto's `ZeroizeOnDrop`) — locking is forgetting, not hiding.
    pub fn lock(&mut self) {
        self.account = None;
        self.last_used = None;
    }

    /// How long the unlocked key has sat unused — `None` while locked.
    pub fn idle(&self) -> Option<std::time::Duration> {
        self.account.as_ref()?;
        self.last_used.map(|t| t.elapsed())
    }

    /// Make a new account and write it encrypted. **Refuses to overwrite**: a
    /// keystore is the only copy of a key, and a launcher that clobbered one
    /// on a stray click would be unforgivable.
    pub fn create(path: impl Into<PathBuf>, passphrase: &str) -> Result<LocalSigner, String> {
        Self::write_new(path.into(), passphrase, Account::generate()?)
    }

    /// Adopt a key the holder already has, from its 32 raw bytes.
    ///
    /// The counterpart to `create`, and the reason both exist: a player with a
    /// wallet elsewhere should not have to abandon it to use this client. The
    /// V3 file this writes opens in geth and MetaMask, so the move is
    /// reversible in both directions — **a launcher you cannot leave is the
    /// thing this client refuses to be.**
    pub fn import(
        path: impl Into<PathBuf>,
        passphrase: &str,
        secret: &[u8],
    ) -> Result<LocalSigner, String> {
        Self::write_new(path.into(), passphrase, Account::from_secret(secret)?)
    }

    /// The shared write half. Both doors refuse the same two things, in the
    /// same order, because a refusal that depends on which verb you typed is a
    /// refusal a caller can route around.
    fn write_new(
        path: PathBuf,
        passphrase: &str,
        account: Account,
    ) -> Result<LocalSigner, String> {
        if path.exists() {
            return Err(format!(
                "{} already exists — refusing to overwrite a keystore, which \
                 may be the only copy of a key",
                path.display()
            ));
        }
        if passphrase.is_empty() {
            return Err("a keystore needs a passphrase — an empty one encrypts nothing".into());
        }
        let address = account.address();
        let doc = keystore::encrypt(&account.secret(), passphrase, &address)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        restrict(&path);
        Ok(LocalSigner {
            path,
            account: Some(account),
            address: Some(address),
            last_used: Some(std::time::Instant::now()),
        })
    }
}

/// 0600 where the OS has a notion of it. On Windows the file inherits the
/// user's profile ACL, which is the platform's equivalent and is not something
/// this file should second-guess.
#[cfg(unix)]
fn restrict(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path) {}

impl Signer for LocalSigner {
    fn kind(&self) -> &'static str { "local" }

    fn status(&self) -> String {
        if !self.exists() {
            return "no account on this machine — make one in Account".into();
        }
        match (&self.address, self.is_unlocked()) {
            (Some(a), true) => format!("{a} · unlocked for this session"),
            (Some(a), false) => format!("{a} · locked — a passphrase unlocks it"),
            (None, _) => "a keystore is there but names no address".into(),
        }
    }

    fn address(&self) -> Option<String> { self.address.clone() }

    fn sign(&mut self, text: &str, _why: &str) -> Result<Value, Refusal> {
        let account = self.account.as_ref().ok_or_else(|| {
            Refusal::new(
                "the local account is locked — unlock it in Signing before a \
                 game asks for a signature",
            )
        })?;
        let signature = account.sign_message(text).map_err(Refusal::new)?;
        self.last_used = Some(std::time::Instant::now());
        Ok(json!({
            "signature": signature,
            "address": account.address(),
            "backend": "local",
        }))
    }
}
