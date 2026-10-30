//! `LocalSigner` — the launcher IS the signer, and this is the only backend
//! in the client that holds a key.
//!
//! # Why it lives HERE and not beside the other four
//!
//! `elo-broker::signer` is keyless **by construction**: browser hands off,
//! arca asks a process that holds the key, external asks a program the player
//! named, none refuses. `elo-broker/tests/signer.rs` asserts that against the
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
//! backend exists because the alternative — no account at all until you have
//! gone and installed a wallet elsewhere — loses most players at the door, not
//! because it is the safe one.

use crate::phrase::Phrase;
use crate::solana::{SolAccount, Transaction as SolTx};
use crate::tx::{Signed, Tx};
use crate::{keystore, Account};
use elo_broker::protocol::Refusal;
use elo_broker::signer::Signer;
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
    /// The Solana half's address, read in the clear from its own file
    /// (`crate::solana_path_for`) — `None` when this account has no Solana
    /// half yet.
    sol_address: Option<String>,
    /// When the unlocked key last DID something — set by `unlock`, refreshed
    /// by every `sign`. What the GUI's idle relock reads: an account busy
    /// signing for a three-hour session is never idle, and one forgotten
    /// after a sign-in is relocked on the clock rather than at close.
    last_used: Option<std::time::Instant>,
    /// Why the keystore, or its Solana half, is there and could not be read.
    /// `None` when both read cleanly or are simply absent.
    unreadable: Option<String>,
}

impl LocalSigner {
    /// Point at a keystore without opening it. A locked signer is a normal
    /// state and says so rather than erroring.
    pub fn at(path: impl Into<PathBuf>) -> LocalSigner {
        let path = path.into();
        // The address is public and lives in the file in the clear, so it can
        // be shown while locked. That is the whole reason the format keeps it.
        let (address, unreadable) = match clear_address(&path) {
            // Checksummed, NOT the file's lowercase — see `checksum_address`.
            // One account must render as one string whether it is locked or not.
            Ok(a) => (a.and_then(|a| crate::checksum_address(&a)), None),
            Err(why) => (None, Some(why)),
        };
        let (sol_address, sol_unreadable) = match clear_address(&crate::solana_path_for(&path)) {
            Ok(a) => (
                a.and_then(|a| crate::solana::parse_address(&a).ok())
                    .map(|raw| crate::solana::base58_encode(&raw)),
                None,
            ),
            Err(why) => (None, Some(why)),
        };
        LocalSigner {
            path,
            account: None,
            address,
            sol_address,
            last_used: None,
            unreadable: unreadable.or(sol_unreadable),
        }
    }

    /// Why a file of this account is on disk and could not be read — a
    /// permission, a damaged file — or `None`. Said rather than folded into
    /// "no address", because the two send a player opposite ways: one makes
    /// a new account, the other recovers the one they have.
    pub fn unreadable(&self) -> Option<&str> {
        self.unreadable.as_deref()
    }

    /// This account's Solana address, shown locked or unlocked.
    pub fn sol_address(&self) -> Option<String> {
        self.sol_address.clone()
    }

    /// Where the Solana half is (or would be) kept.
    pub fn solana_path(&self) -> PathBuf {
        crate::solana_path_for(&self.path)
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
        let account = Account::from_secret(secret.as_ref())?;
        self.address = Some(account.address());
        self.account = Some(account);
        self.last_used = Some(std::time::Instant::now());
        Ok(())
    }

    /// Forget the key. Called on close, by the GUI's idle relock, and
    /// available to a player who wants it gone without quitting. Dropping the
    /// `Account` drops its `SigningKey`, whose scalar zeroizes on drop
    /// (RustCrypto's `ZeroizeOnDrop`) — locking is forgetting, not hiding.
    /// That holds because every other copy is `Zeroizing` too: the decrypted
    /// bytes, the scrypt output, and `Account::secret`.
    pub fn lock(&mut self) {
        self.account = None;
        self.last_used = None;
    }

    /// How long the unlocked key has sat unused — `None` while locked.
    pub fn idle(&self) -> Option<std::time::Duration> {
        self.account.as_ref()?;
        self.last_used.map(|t| t.elapsed())
    }

    /// Make a new account from a fresh recovery phrase and write it
    /// encrypted. Returns the phrase so a caller can show it; a caller that
    /// wants the words checked BEFORE anything is written uses
    /// [`Phrase::generate`] and [`LocalSigner::restore`] instead, which is
    /// what both binaries do.
    ///
    /// **Refuses to overwrite**: a keystore may be the only copy of a key.
    pub fn create(path: impl Into<PathBuf>, passphrase: &str) -> Result<(LocalSigner, Phrase), String> {
        let phrase = Phrase::generate()?;
        let signer = Self::restore(path, passphrase, &phrase)?;
        Ok((signer, phrase))
    }

    /// Write the account a recovery phrase derives (`m/44'/60'/0'/0/0`).
    /// Used for a new account after its words are checked, and to bring an
    /// account back from words written down elsewhere.
    pub fn restore(
        path: impl Into<PathBuf>,
        passphrase: &str,
        phrase: &Phrase,
    ) -> Result<LocalSigner, String> {
        Self::write_new(path.into(), passphrase, phrase.account()?, Some(phrase.solana()?))
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
        Self::write_new(path.into(), passphrase, Account::from_secret(secret)?, None)
    }

    /// Give an account made before Solana — or adopted from a raw key — its
    /// Solana half.
    ///
    /// Two ways in, both checked before anything is written:
    /// - **the recovery words**, which must derive THIS account's `0x`
    ///   address, so the Solana address that appears is the one Phantom shows
    ///   for the same words and never some other account's;
    /// - **a Solana private key** as Phantom exports it, for a holder whose
    ///   Solana account lives elsewhere.
    ///
    /// The passphrase must open this keystore: the Solana half is encrypted
    /// under the same one, so one passphrase answers for both chains.
    /// **Refuses to overwrite** a Solana half that is already there.
    pub fn add_solana(&mut self, passphrase: &str, words_or_key: &str) -> Result<String, String> {
        let at = self.solana_path();
        if at.exists() {
            if let (None, Some(why)) = (&self.sol_address, &self.unreadable) {
                return Err(format!("{} is there and cannot be read ({why}) — it may be the \
                                    only copy of a Solana key, so nothing replaces it",
                                   at.display()));
            }
            return Err(format!("{} already exists — this account already has a Solana address",
                               at.display()));
        }
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("cannot read {}: {e}", self.path.display()))?;
        let doc: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("{} is not JSON: {e}", self.path.display()))?;
        keystore::decrypt(&doc, passphrase)?;
        let sol = if words_or_key.split_whitespace().count() > 1 {
            let phrase = Phrase::parse(words_or_key)?;
            if Some(phrase.account()?.address()) != self.address {
                return Err("Those words are a different account from this one, so no Solana \
                            address was added. Check them against what you wrote down.".into());
            }
            phrase.solana()?
        } else {
            SolAccount::from_typed(words_or_key)?
        };
        write_solana(&at, passphrase, &sol, true)?;
        self.sol_address = Some(sol.address());
        Ok(sol.address())
    }

    /// Sign a Solana transaction, with the passphrase asked for THIS one —
    /// the same rule as [`LocalSigner::sign_transaction`]: decrypted fresh,
    /// used once, dropped. The key must be the address the file names, and the
    /// transaction must be asking that address to sign
    /// ([`crate::solana::Transaction`] refuses otherwise).
    pub fn sign_solana_transaction(&self, passphrase: &str, tx: &mut SolTx) -> Result<(), String> {
        let at = self.solana_path();
        // Only a file that is NOT THERE is "no Solana address". One that is
        // there and unreadable is a permission or a damaged disk, and telling
        // that player to add an address sends them off to make a second one.
        let raw = std::fs::read_to_string(&at).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "this account has no Solana address yet — add one in Account".to_string()
            } else {
                format!("cannot read {}: {e}", at.display())
            }
        })?;
        let doc: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("{} is not JSON: {e}", at.display()))?;
        let secret = keystore::decrypt(&doc, passphrase)?;
        let sol = SolAccount::from_secret(secret.as_ref())?;
        if self.sol_address.as_deref() != Some(sol.address().as_str()) {
            return Err("this Solana keystore's key is not the address it names — refusing to sign".into());
        }
        tx.sign_with(&sol)
    }

    /// Sign a transaction, with the passphrase asked for THIS transaction.
    ///
    /// The keystore is decrypted fresh, used once and dropped, whether or not
    /// the account is unlocked. An unlocked session answers games' sign-in
    /// messages; it never moves money on its own. A game cannot reach this at
    /// all: the door's `sign` verb is EIP-191 only.
    pub fn sign_transaction(&self, passphrase: &str, tx: &Tx) -> Result<Signed, String> {
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("cannot read {}: {e}", self.path.display()))?;
        let doc: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("{} is not JSON: {e}", self.path.display()))?;
        let secret = keystore::decrypt(&doc, passphrase)?;
        let account = Account::from_secret(secret.as_ref())?;
        // The file names an address in the clear, and that is what every
        // screen showed as "from". A keystore whose key is some other account
        // would sign from an address nobody reviewed.
        if self.address.as_deref() != Some(account.address().as_str()) {
            return Err("this keystore's key is not the address it names — refusing to sign".into());
        }
        account.sign_tx(tx)
    }

    /// Sign a Seaport 1.6 listing (`crate::seaport`), with the passphrase asked
    /// for THIS listing, like a transaction. The vault refuses any order that is
    /// not one item offered by this account and paid to it first in ETH.
    pub fn sign_seaport_listing(&self, passphrase: &str, order: &crate::seaport::Order) -> Result<String, String> {
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("cannot read {}: {e}", self.path.display()))?;
        let doc: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("{} is not JSON: {e}", self.path.display()))?;
        let secret = keystore::decrypt(&doc, passphrase)?;
        let account = Account::from_secret(secret.as_ref())?;
        if self.address.as_deref() != Some(account.address().as_str()) {
            return Err("this keystore's key is not the address it names — refusing to sign".into());
        }
        account.sign_listing(order)
    }

    /// Sign a USDG transfer authorization for a credit pack (`crate::usdg`),
    /// with the passphrase asked for THIS payment, like a transaction. The
    /// vault refuses any authorization that is not from this account, to the
    /// credits address, for at most one pack, valid now and briefly.
    pub fn sign_usdg_authorization(&self, passphrase: &str, a: &crate::usdg::Authorization, now: u64)
                                   -> Result<String, String> {
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("cannot read {}: {e}", self.path.display()))?;
        let doc: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("{} is not JSON: {e}", self.path.display()))?;
        let secret = keystore::decrypt(&doc, passphrase)?;
        let account = Account::from_secret(secret.as_ref())?;
        if self.address.as_deref() != Some(account.address().as_str()) {
            return Err("this keystore's key is not the address it names — refusing to sign".into());
        }
        account.sign_usdg(a, now)
    }

    /// The shared write half. Every door refuses the same things, in the
    /// same order, because a refusal that depends on which verb you typed is a
    /// refusal a caller can route around.
    fn write_new(
        path: PathBuf,
        passphrase: &str,
        account: Account,
        sol: Option<SolAccount>,
    ) -> Result<LocalSigner, String> {
        let occupied = || {
            format!(
                "{} already exists — refusing to overwrite a keystore, which \
                 may be the only copy of a key",
                path.display()
            )
        };
        if path.exists() {
            return Err(occupied());
        }
        // A Solana half with no keystore beside it is a leftover from an
        // account whose keystore went — and may be the only copy of that key.
        //
        // ⚠ Refused whatever `sol` is. An import writes no Solana half, and
        // checking only when it would was how a player who moved
        // `keystore.json` aside (as the overwrite refusal tells them to) and
        // imported a raw key was shown the OLD account's Solana address as
        // theirs — read back from this leftover by `LocalSigner::at`.
        let sol_at = crate::solana_path_for(&path);
        if sol_at.exists() {
            return Err(format!(
                "{} already exists — move it somewhere safe first: it may be the only \
                 copy of a Solana key", sol_at.display()));
        }
        if passphrase.chars().count() < MIN_PASSPHRASE {
            return Err(format!(
                "a passphrase needs at least {MIN_PASSPHRASE} characters — it is all that \
                 stands between a copied keystore file and the money in it"
            ));
        }
        let address = account.address();
        let doc = keystore::encrypt(&account.secret(), passphrase, &address)?;
        let body = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        // `fresh`: the check above is a courtesy, and THIS is the refusal — an
        // exclusive create, so two launchers racing to make an account cannot
        // both pass the check and have the second truncate the first. The
        // file is 0600 from its first byte, a directory it makes is 0700, and
        // a failed write leaves no partial file (`crate::write_private`).
        crate::write_private(&path, body.as_bytes(), true).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                occupied()
            } else {
                format!("cannot write {}: {e}", path.display())
            }
        })?;
        let sol_address = match &sol {
            Some(s) => {
                if let Err(e) = write_solana(&sol_at, passphrase, s, true) {
                    // A failed create leaves nothing. The keystore is this
                    // call's own — the exclusive create above made it — and
                    // kept, it is half an account whose words were never
                    // handed back, blocking the retry as "may be the only
                    // copy of a key".
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
                Some(s.address())
            }
            None => None,
        };
        Ok(LocalSigner {
            path,
            account: Some(account),
            address: Some(address),
            sol_address,
            last_used: Some(std::time::Instant::now()),
            unreadable: None,
        })
    }
}

/// Write a Solana half: the V3 shape (scrypt + AES-128-CTR, the same
/// passphrase), with the base58 address and the derivation path in the clear.
fn write_solana(at: &std::path::Path, passphrase: &str, sol: &SolAccount, fresh: bool) -> Result<(), String> {
    let mut doc = keystore::encrypt(&sol.secret(), passphrase, "")?;
    // `encrypt` writes an Ethereum address lowercased; base58 is case-sensitive,
    // so the Solana address is set here, as written.
    doc["address"] = json!(sol.address());
    doc["chain"] = json!("solana");
    doc["path"] = json!(crate::solana::PATH_TEXT);
    let body = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    crate::write_private(at, body.as_bytes(), fresh)
        .map_err(|e| format!("cannot write {}: {e}", at.display()))
}

/// The `address` a keystore file names in the clear. `Ok(None)` is a file
/// that is not there — that, and only that, is absence. Anything else that
/// stops the read says what it was.
fn clear_address(at: &std::path::Path) -> Result<Option<String>, String> {
    let raw = match std::fs::read_to_string(at) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", at.display())),
    };
    let v: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("{} is not JSON: {e}", at.display()))?;
    Ok(v.get("address").and_then(Value::as_str).map(str::to_string))
}

/// The shortest passphrase a NEW keystore accepts. Unlocking an existing one
/// takes whatever it was made with.
pub const MIN_PASSPHRASE: usize = 8;

impl Signer for LocalSigner {
    fn kind(&self) -> &'static str { "local" }

    fn status(&self) -> String {
        if !self.exists() {
            return "no account on this machine — make one in Account".into();
        }
        match (&self.address, self.is_unlocked()) {
            (Some(a), true) => format!("{a} · unlocked for this session"),
            (Some(a), false) => format!("{a} · locked — a passphrase unlocks it"),
            (None, _) => self
                .unreadable
                .clone()
                .unwrap_or_else(|| "a keystore is there but names no address".into()),
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
