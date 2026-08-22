//! The signer that holds no key and still answers: the browser's wallet.
//!
//! # Why this crate and not `elo-broker`
//!
//! `elo-broker::signer` is the home of the keyless backends, and this one
//! belongs beside them by shape. It cannot live there: that crate depends on
//! `serde_json` and nothing else — deliberately, and `elo-depot`'s manifest
//! says so in as many words — and `elo-net` depends on it transitively, so an
//! HTTP-speaking signer in `elo-broker` would be a dependency cycle. It lives
//! in the crate that already talks to the origin, which is also the only thing
//! it does.
//!
//! # What it is
//!
//! [`BrowserSigner`] (in `elo-broker`) opens a page and stops — *"builds a
//! link and stops"* is its own doc line, and for a game handing off mid-play
//! that is the right shape. It is the wrong shape for the launcher's own acts:
//! `elo entitle` needs the signature BACK, and there was no road back, so a
//! launcher with no local account could not finish a single one.
//!
//! This is that road. The origin is the bridge, the same way it is for
//! `meter/signin.py` in the other direction — the browser cannot knock on a
//! Unix socket and the launcher cannot reach into a tab, but both already talk
//! to `elopros.com`:
//!
//! ```text
//! elo pair          POST /api/signer/start        → CODE + this side's secret
//! the player        TYPES the code at elopros.com/pair.html
//! the browser       signs `elo pair …` with MetaMask, claims the code
//! this side         GET  /api/signer/{code}?secret=…   → the paired address
//!
//! then, per act:
//! this side         POST /api/signer/{code}/ask   {secret, text, why}
//! the browser       shows the text VERBATIM, MetaMask shows it again, y/N
//! this side         GET  /api/signer/{code}/ask/{id}?secret=…
//! ```
//!
//! # What it holds, and what it can therefore do
//!
//! A code, a secret, and an address. **None of the three can sign anything.**
//! Holding this file lets a program QUEUE an ask; a human in another window
//! still approves each one, in a wallet this process cannot reach. That is the
//! whole security story and it is why `CLAUDE.md` invariant 7 is not merely
//! unbroken here but trivially true: there is no key on this machine at all.
//!
//! ⚠ **`prove` does not work on this backend, and the refusal is not a gap.**
//! `prove_message` is EIP-4361, the origin refuses SIWE on the relay so this
//! door can never be used to phish a login, and that wall does not have an
//! exception shaped like us. A player proving to a game server wants a local
//! account or a wallet in that browser. `arca` and `external` cannot prove
//! either, for a neighbouring reason — this is the third member of that set,
//! not a new limitation.

use crate::Net;
use elo_broker::protocol::Refusal;
use elo_broker::signer::Signer;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long to wait for a human to answer one ask.
///
/// Under the origin's own `ASK_OPEN_TTL_S` (600s) on purpose: the ask must
/// still be open when we stop asking about it, so a caller that gives up
/// leaves a lapsing ask rather than a mystery.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(180);
/// How long to wait for a browser to claim a fresh code — the origin's own
/// pending window.
pub const CLAIM_TIMEOUT: Duration = Duration::from_secs(600);
const POLL_EVERY: Duration = Duration::from_millis(1500);

/// What `elo pair` writes and every later act reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Pairing {
    pub host: String,
    pub code: String,
    pub secret: String,
    pub address: String,
    /// Unix seconds. The origin is the authority on expiry; this is what we
    /// were told, so the client can say "expired" without a round trip.
    pub expires: u64,
}

impl Pairing {
    pub fn to_json(&self) -> Value {
        json!({
            "host": self.host, "code": self.code, "secret": self.secret,
            "address": self.address, "expires": self.expires,
        })
    }

    /// Parse one back. **A field-by-field read, never a blanket deserialise**:
    /// a hand-edited file with a key-shaped field in it must round-trip to a
    /// `Pairing` with no such field, the same way the settings file drops one.
    pub fn from_json(v: &Value) -> Option<Pairing> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let p = Pairing {
            host: s("host")?,
            code: s("code")?,
            secret: s("secret")?,
            address: s("address")?,
            expires: v.get("expires").and_then(Value::as_u64).unwrap_or(0),
        };
        if p.code.is_empty() || p.secret.is_empty() || p.address.is_empty() {
            return None;
        }
        Some(p)
    }

    pub fn expired(&self, now: u64) -> bool {
        self.expires != 0 && now >= self.expires
    }

    pub fn show_code(&self) -> String {
        if self.code.len() == 8 {
            format!("{}-{}", &self.code[..4], &self.code[4..])
        } else {
            self.code.clone()
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read the pairing at `path`, or None. **An expired one reads as None**, so
/// no caller has to remember to check — the failure mode of forgetting is a
/// launcher confidently queueing asks nobody will ever see.
pub fn load(path: &Path) -> Option<Pairing> {
    let raw = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let p = Pairing::from_json(&v)?;
    if p.expired(now()) {
        return None;
    }
    Some(p)
}

/// The same read WITHOUT the expiry filter, for the one surface that must tell
/// "expired" apart from "never paired" — `elo pair show`. Every other caller
/// wants [`load`].
pub fn load_even_if_expired(path: &Path) -> Option<Pairing> {
    let raw = std::fs::read_to_string(path).ok()?;
    Pairing::from_json(&serde_json::from_str::<Value>(&raw).ok()?)
}

pub fn save(path: &Path, p: &Pairing) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let body = serde_json::to_string_pretty(&p.to_json()).unwrap_or_default();
    std::fs::write(path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    // A bearer credential, so the same mode the keystore gets. Best effort:
    // on a filesystem with no unix modes this is not a reason to fail a
    // pairing that otherwise worked.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn forget(path: &Path) -> Result<bool, String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Start a pairing: mint a code at the origin. The caller shows the code and
/// then calls [`await_claim`].
pub fn start(net: &Net, host: &str) -> Result<(String, String, u64), String> {
    let host = host.trim_end_matches('/');
    let got = net.post_json_told(&format!("{host}/api/signer/start"), &json!({}));
    let Some(v) = got.value else {
        return Err(format!("could not reach {host} to start a pairing — {}", got.why));
    };
    if !got.ok {
        return Err(reason_from(&v, &got.why));
    }
    let code = v.get("code").and_then(Value::as_str).unwrap_or("").to_string();
    let secret = v.get("secret").and_then(Value::as_str).unwrap_or("").to_string();
    if code.is_empty() || secret.is_empty() {
        return Err("the origin started a pairing with no code or no secret".into());
    }
    Ok((code, secret, v.get("expires_at").and_then(Value::as_u64).unwrap_or(0)))
}

/// What one look at a pending pairing found.
///
/// Three states and not two, for the reason the whole `elo-net` crate exists:
/// **still waiting** and **gone** are different answers, and a caller that
/// folded them together would either give up on a player who has not opened
/// their browser yet, or spin forever on a code the origin dropped.
pub enum Claim {
    Pending,
    Paired(Box<Pairing>),
    Gone(String),
}

/// One look. **Never blocks longer than a request** — this is the shape a GUI
/// needs, because a window that sleeps in a poll loop is a window that has
/// stopped repainting. [`await_claim`] is this in a loop, for the CLI.
pub fn poll_claim(net: &Net, host: &str, code: &str, secret: &str) -> Claim {
    let host = host.trim_end_matches('/').to_string();
    let got = net.get_json(&format!("{host}/api/signer/{code}?secret={secret}"));
    if let Some(v) = got.value.as_ref() {
        if v.get("state").and_then(Value::as_str) == Some("paired") {
            let address = v.get("address").and_then(Value::as_str).unwrap_or("");
            if address.is_empty() {
                return Claim::Gone("the origin says paired and names no address".into());
            }
            return Claim::Paired(Box::new(Pairing {
                host,
                code: code.to_string(),
                secret: secret.to_string(),
                address: address.to_string(),
                expires: v.get("expires_at").and_then(Value::as_u64).unwrap_or(0),
            }));
        }
    }
    // ⚠ A 404 here is the code EXPIRING, not a bad url — the origin drops an
    // unclaimed pairing at its pending TTL. Saying "expired" beats "the origin
    // answered 404" for the one person who will read it.
    if got.status == Some(404) {
        return Claim::Gone(
            "the code expired before a browser claimed it — start a fresh one".into(),
        );
    }
    // Anything else — a dropped wifi, a 500 — is PENDING rather than gone. A
    // laptop on a train must not lose a pairing its owner is mid-way through
    // claiming (`elo-net`'s own reachable-is-not-empty rule).
    Claim::Pending
}

/// Poll until a browser claims the code, or the window closes.
///
/// `tick` is called once per poll so a CLI can print a dot; it is a plain `fn`
/// rather than a closure so the whole struct stays `Send` with no boxing.
pub fn await_claim(
    net: &Net,
    host: &str,
    code: &str,
    secret: &str,
    tick: fn(),
) -> Result<Pairing, String> {
    let began = Instant::now();
    loop {
        match poll_claim(net, host, code, secret) {
            Claim::Paired(p) => return Ok(*p),
            Claim::Gone(why) => return Err(why),
            Claim::Pending => {}
        }
        if began.elapsed() >= CLAIM_TIMEOUT {
            return Err("nobody claimed the code in time".into());
        }
        tick();
        std::thread::sleep(POLL_EVERY);
    }
}

/// Reach the wallet in somebody's browser tab.
///
/// **Holds no key, and could not be given one**: the only fields are a code,
/// a secret and an address, and the only thing it can do with them is ask.
pub struct PairedBrowserSigner {
    pub pairing: Pairing,
    /// Printed once when an ask goes out, so a human staring at a terminal
    /// knows to go and look at their browser. A `fn` pointer, not a closure,
    /// so [`Signer`]'s `Send` bound costs nothing.
    pub tell: fn(&str),
    net: Net,
}

fn quiet(_: &str) {}

impl PairedBrowserSigner {
    pub fn new(pairing: Pairing) -> PairedBrowserSigner {
        PairedBrowserSigner { pairing, tell: quiet, net: Net::new() }
    }

    pub fn telling(mut self, tell: fn(&str)) -> PairedBrowserSigner {
        self.tell = tell;
        self
    }

    fn base(&self) -> String {
        let host = self.pairing.host.trim_end_matches('/');
        format!("{host}/api/signer/{}", self.pairing.code)
    }
}

impl Signer for PairedBrowserSigner {
    fn kind(&self) -> &'static str { "browser-paired" }

    fn status(&self) -> String {
        let left = self.pairing.expires.saturating_sub(now());
        if self.pairing.expires != 0 && left == 0 {
            return format!(
                "{} · the pairing expired — `elo pair` again",
                self.pairing.address
            );
        }
        format!(
            "{} · in a browser, approved one message at a time{}",
            self.pairing.address,
            if self.pairing.expires == 0 {
                String::new()
            } else {
                format!(" · {}h left", left / 3600)
            }
        )
    }

    fn address(&self) -> Option<String> {
        Some(self.pairing.address.clone())
    }

    fn sign(&mut self, text: &str, why: &str) -> Result<Value, Refusal> {
        if self.pairing.expired(now()) {
            return Err(Refusal::new(
                "the browser pairing expired — run `elo pair` and claim a \
                 fresh code in your browser",
            ));
        }
        // `why` is REQUIRED by the origin and the reason is worth restating
        // here, where a caller that forgot one will actually read it: the
        // person approving is in another window, possibly on another machine,
        // and a signature request with no stated purpose is one nobody can
        // judge. Refused here rather than round-tripped for a 422.
        let why = why.trim();
        if why.is_empty() {
            return Err(Refusal::new(
                "a browser-relayed signature needs a stated reason — the \
                 person approving it is in another window",
            ));
        }
        let asked = self.net.post_json_told(
            &format!("{}/ask", self.base()),
            &json!({ "secret": self.pairing.secret, "text": text, "why": why }),
        );
        let Some(v) = asked.value else {
            return Err(Refusal::new(format!(
                "could not reach {} to ask for a signature — {}",
                self.pairing.host, asked.why
            )));
        };
        if !asked.ok {
            return Err(Refusal::new(reason_from(&v, &asked.why)));
        }
        let Some(id) = v.get("id").and_then(Value::as_str) else {
            return Err(Refusal::new("the origin queued the ask and named no id"));
        };
        (self.tell)(&format!(
            "waiting for {} to approve this in the browser ({}) …",
            self.pairing.address,
            self.pairing.host.trim_start_matches("https://")
        ));

        let poll = format!("{}/ask/{id}?secret={}", self.base(), self.pairing.secret);
        let began = Instant::now();
        loop {
            let got = self.net.get_json(&poll);
            if let Some(v) = got.value.as_ref() {
                match v.get("state").and_then(Value::as_str).unwrap_or("") {
                    "signed" => {
                        let sig = v.get("signature").and_then(Value::as_str).unwrap_or("");
                        if sig.is_empty() {
                            return Err(Refusal::new(
                                "the origin says signed and carries no signature",
                            ));
                        }
                        return Ok(json!({
                            "signature": sig,
                            "address": self.pairing.address,
                            "backend": "browser-paired",
                        }));
                    }
                    // ⚠ A refusal by the PERSON is not an error, and the
                    // distinction survives to the caller (`sdk/PROTOCOL.md`):
                    // a game that renders "they said no" as a crash tells the
                    // player something broke when nothing did.
                    "refused" => {
                        return Err(Refusal::by_player(
                            v.get("refusal")
                                .and_then(Value::as_str)
                                .unwrap_or("the wallet's holder said no")
                                .to_string(),
                        ))
                    }
                    "expired" => {
                        return Err(Refusal::new(
                            "nobody answered in the browser, and the ask lapsed",
                        ))
                    }
                    _ => {}
                }
            }
            if began.elapsed() >= ANSWER_TIMEOUT {
                return Err(Refusal::new(format!(
                    "no answer from the browser in {}s. The ask is still open \
                     there — approve it and run this again.",
                    ANSWER_TIMEOUT.as_secs()
                )));
            }
            std::thread::sleep(POLL_EVERY);
        }
    }
}

/// The origin's own sentence for a refusal, or the transport's.
///
/// Every door in `meter/` writes its refusals in words and `post_json_told`
/// exists to keep them; printing `the origin answered 422` over the top of one
/// throws away the half of the reply a human can act on.
fn reason_from(v: &Value, fallback: &str) -> String {
    v.get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}
