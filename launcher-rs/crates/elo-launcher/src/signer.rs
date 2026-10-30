//! Which signer this machine reaches, decided in ONE place.
//!
//! # The gap this closes
//!
//! `elo-broker::signer` has held a `Signer` trait and four keyless backends
//! since the Rust port, and `LAUNCHER.md` §4 described choosing between them
//! as a settled feature. It was not: **every CLI verb constructed
//! `elo_vault::LocalSigner` directly**, the four keyless backends were
//! referenced by nothing outside their own test file, and there was no
//! settings file to choose from. The launcher had exactly one signer, and it
//! was the local keystore — so a player with a wallet in their browser and no
//! wish for a second key on disk could not run `elo entitle` at all.
//!
//! This module is the seam the doc already claimed. Every verb that needs a
//! signature asks [`choose`] and then talks to a `dyn Signer`, so a backend
//! arrives by being listed here rather than by being threaded through each
//! verb again.
//!
//! # Precedence, and why a live pairing wins
//!
//! 1. `--local`, or `ELO_SIGNER=local` in the environment — the operator said
//!    which one. Nothing outranks that. The variable is for an agent that
//!    shares a machine with its owner's paired browser: set once where the
//!    agent runs, it signs as the agent on every command.
//! 2. a browser pairing that has not expired.
//! 3. the local keystore.
//!
//! ⚠ Rule 2 above rule 3 is the surprising one, so here is the reasoning: a
//! keystore sits on disk forever and says nothing about intent, while a
//! pairing had to be established BY HAND in the last twelve hours, at a
//! browser, by someone typing a code. The newer, deliberate act is the better
//! guess at what its owner wants right now — and it is the only rule of the
//! two that can be undone in one command (`elo pair forget`). `--local` exists
//! so nobody has to.
//!
//! # What is NOT decided here
//!
//! Consent. The local backend prompts for a passphrase and the paired backend
//! waits for a human in a browser; both are the backend's own act. This
//! module picks, and never signs.

use elo_broker::protocol::Refusal;
use elo_broker::signer::Signer;
use elo_net::pairing::{self, PairedBrowserSigner};
use serde_json::Value;

use crate::args::Args;
use crate::prompt;

/// One signer, whichever kind it turned out to be.
///
/// A newtype rather than `Box<dyn Signer>` because the local backend needs
/// something the trait does not describe: an unlock, and therefore a
/// passphrase prompt, and therefore a terminal. Doing it lazily inside `sign`
/// is what lets a caller hold "a signer" without knowing which door it opens
/// — and is why `--statement` paths still cost no passphrase.
pub struct CliSigner {
    inner: Inner,
}

enum Inner {
    Paired(Box<PairedBrowserSigner>),
    Local(Box<elo_vault::LocalSigner>),
}

/// Where a paired signature request says it came from, on the browser's screen.
pub(crate) fn tell(line: &str) {
    // stderr, not stdout: `--json` callers pipe stdout, and a progress line in
    // the middle of a JSON document is a parse error rather than a courtesy.
    eprintln!("  {line}");
}

/// Pick a signer, or say what would make one exist.
pub fn choose(a: &Args, host: &str) -> Result<CliSigner, String> {
    let ks = elo_vault::keystore_path();
    let local = elo_vault::LocalSigner::at(&ks);
    let forced_local = a.has("--local") || env_local()?;
    let paired = pairing::load(&elo_vault::pairing_path());

    if forced_local || paired.is_none() {
        if local.exists() {
            return Ok(CliSigner { inner: Inner::Local(Box::new(local)) });
        }
        if forced_local {
            return Err(format!(
                "--local, and there is no account at {} — `elo account new` \
                 makes one (or `import`)",
                ks.display()
            ));
        }
        return Err(format!(
            "no signer on this machine. Two doors, and neither needs the other:\n  \
             elo pair          use the wallet in your browser — nothing is \
             written to disk that can sign\n  \
             elo account new   make an account here — generated on this box, \
             transmitted nowhere\n\
             the origin is {host}",
        ));
    }

    // Unwrap is sound: the `is_none` arm above returned.
    let mut p = paired.expect("a pairing, checked above");
    // ⚠ The pairing remembers the host it was made against, and that one wins
    // over `--host`. A code and a secret are meaningless at a different
    // origin, and quietly asking the wrong box would fail as "no such
    // pairing" — a sentence that sends its reader to look at the pairing
    // rather than at the flag that actually caused it.
    if !host.is_empty() && host.trim_end_matches('/') != p.host.trim_end_matches('/') {
        tell(&format!(
            "the pairing was made against {} — asking there, not {host}",
            p.host
        ));
    }
    p.host = p.host.trim_end_matches('/').to_string();
    Ok(CliSigner {
        inner: Inner::Paired(Box::new(PairedBrowserSigner::new(p).telling(tell))),
    })
}

/// `ELO_SIGNER`: unset or empty is the default order, `local` forces the
/// keystore. Anything else is a refusal, like an unknown flag: a caller who
/// believes they chose a signer must not be quietly given another.
fn env_local() -> Result<bool, String> {
    signer_setting(std::env::var("ELO_SIGNER").ok().as_deref())
}

fn signer_setting(v: Option<&str>) -> Result<bool, String> {
    match v.map(str::trim).unwrap_or("") {
        "" => Ok(false),
        "local" => Ok(true),
        other => Err(format!("ELO_SIGNER is {other:?}; the one value it takes is \"local\"")),
    }
}

impl CliSigner {
    /// One line for `elo account` and friends.
    pub fn line(&self) -> String {
        format!("{} · {}", self.kind(), self.status())
    }

    /// Will the next `sign` stop for a human? A paired browser asks every
    /// time; the local keystore asks only for its passphrase, once, so a
    /// second signature in one command goes unasked unless the caller asks.
    pub fn will_ask(&self) -> bool {
        match &self.inner {
            Inner::Paired(_) => true,
            Inner::Local(l) => !l.is_unlocked(),
        }
    }
}

impl Signer for CliSigner {
    fn kind(&self) -> &'static str {
        match &self.inner {
            Inner::Paired(p) => p.kind(),
            Inner::Local(l) => l.kind(),
        }
    }

    fn status(&self) -> String {
        match &self.inner {
            Inner::Paired(p) => p.status(),
            Inner::Local(l) => l.status(),
        }
    }

    fn address(&self) -> Option<String> {
        match &self.inner {
            Inner::Paired(p) => p.address(),
            Inner::Local(l) => l.address(),
        }
    }

    fn sign(&mut self, text: &str, why: &str) -> Result<Value, Refusal> {
        match &mut self.inner {
            Inner::Paired(p) => p.sign(text, why),
            Inner::Local(l) => {
                // The CLI unlocks per command and forgets at exit — the shape
                // `elo account sign` already had, moved here so every verb
                // gets it rather than each one re-typing the prompt.
                if !l.is_unlocked() {
                    crate::warn_if_echoing();
                    let pass = prompt::passphrase("passphrase: ").map_err(Refusal::new)?;
                    // A wrong passphrase is a WRONG PASSPHRASE and never
                    // "corrupt" — the vault draws that line and this must not
                    // blur it back by rewording the error.
                    l.unlock(&pass).map_err(Refusal::new)?;
                }
                l.sign(text, why)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::signer_setting;

    #[test]
    fn elo_signer_takes_local_or_nothing() {
        assert_eq!(signer_setting(None), Ok(false));
        assert_eq!(signer_setting(Some("")), Ok(false));
        assert_eq!(signer_setting(Some("local")), Ok(true));
        assert!(signer_setting(Some("paired")).is_err());
    }
}
