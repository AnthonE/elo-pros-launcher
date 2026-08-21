//! Signing — the backends that hold no key.
//!
//! `CLAUDE.md`'s ladder is the shape:
//!
//! ```text
//! rung 0  the human signs each action   → Browser   (default)
//! rung 1  bounded standing authority    → mandate.py, behind the arca
//! rung 2  the same, around a real key   → Arca
//! rung 3  a renounced vault             → RESERVED, not here
//! ```
//!
//! **Four backends are here and the key-holding one is not**, because it lives
//! a layer up: `elo_vault::LocalSigner` holds the account, and `elo-gui`
//! wires it to `Host::sign` and `Host::prove` directly. This file is only the
//! backends that reach a signer somebody *else* runs, which is why invariant 7
//! holds trivially in it (below).
//!
//! ⚠ **`arca` and `external` cannot `prove`.** `prove` bypasses the family
//! allowlist by construction, and the arca's own copy of that list — the one
//! beside the key — does not know the word. A player on either gets a readable
//! refusal, and teaching an arca to prove is an edit to a security-relevant
//! allowlist that wants its own operator sentence rather than a quiet change.
//! `local` is the backend `prove` works on.
//!
//! **`CLAUDE.md` invariant 7 holds trivially in this file: no branch here
//! reads a key.** Browser hands off, arca asks a process that has one,
//! external asks a program the *player* named, and none refuses. That is a
//! property of the file, not a promise about it — `tests/signer.rs` asserts it
//! against the source.

use crate::protocol::Refusal;
use serde_json::{json, Value};
use std::io::Write;
// The line reader is the arca client's, and the arca is a unix program — so
// these are unix-only imports, not unconditional ones.
#[cfg(unix)]
use std::io::{BufRead, BufReader};

/// What every backend answers.
pub trait Signer: Send {
    /// `browser` · `arca` · `external` · `none`. A game reads this from
    /// `hello` and shapes its UI: with `none` there is nothing to ask for, and
    /// with `browser` expect a handoff rather than a signature.
    fn kind(&self) -> &'static str;

    /// One line a player reads in the Signing window. Never a promise — where
    /// a backend cannot answer, this says so.
    fn status(&self) -> String;

    /// The address this backend would sign as, if it knows one.
    fn address(&self) -> Option<String>;

    /// Sign, or refuse with a reason. **The three non-signature outcomes are
    /// not the same outcome** (`sdk/PROTOCOL.md`): a player's refusal, a
    /// handoff, and a signer that said no are distinguishable by the caller.
    fn sign(&mut self, text: &str, why: &str) -> Result<Value, Refusal>;
}

/// No signer configured. Refuses, and says what would change that.
pub struct NoSigner;

impl Signer for NoSigner {
    fn kind(&self) -> &'static str { "none" }
    fn status(&self) -> String {
        "no signer is configured — acts that need a signature will be refused".into()
    }
    fn address(&self) -> Option<String> { None }
    fn sign(&mut self, _text: &str, _why: &str) -> Result<Value, Refusal> {
        Err(Refusal::new(
            "no signer is configured. Choose one in Signing, or play without \
             one — playing anonymously is a normal state.",
        ))
    }
}

/// rung 0 — the launcher builds a link and stops.
///
/// It collects nothing and listens on nothing. That is not a limitation worked
/// around: a launcher that collected signatures would need a socket, an origin
/// check and a threat model, and would still be strictly worse at all three
/// than the browser the player's wallet is already in.
pub struct BrowserSigner {
    pub host: String,
    pub wallet: Option<String>,
}

impl BrowserSigner {
    /// Where this family is finished. A message the launcher cannot place goes
    /// to the wallet page rather than to a guessed route — **a 404 in a
    /// browser is a worse answer than a general page.**
    pub fn page_for(text: &str) -> &'static str {
        let family = crate::protocol::sign_family(text).unwrap_or_default();
        match family.as_str() {
            "play" => "play.html",
            "review" => "reviews.html",
            "vow" => "vows.html",
            "hive" | "braid" => "hive.html",
            "store" => "manage.html",
            _ => "wallet.html",
        }
    }
}

impl Signer for BrowserSigner {
    fn kind(&self) -> &'static str { "browser" }
    fn status(&self) -> String {
        "acts open in your browser, where your wallet already is".into()
    }
    fn address(&self) -> Option<String> { self.wallet.clone() }

    fn sign(&mut self, text: &str, why: &str) -> Result<Value, Refusal> {
        let host = self.host.trim_end_matches('/');
        let mut url = format!("{host}/{}", Self::page_for(text));
        let mut q: Vec<String> = Vec::new();
        if let Some(w) = &self.wallet {
            q.push(format!("wallet={}", urlencode(w)));
        }
        if !why.is_empty() {
            q.push(format!("for={}", urlencode(why)));
        }
        if !q.is_empty() {
            url.push('?');
            url.push_str(&q.join("&"));
        }
        // NOT an error. `sdk/PROTOCOL.md`: a game that treats `handoff` as one
        // tells the player something broke when nothing did.
        Err(Refusal {
            reason: "this act needs a signature, and this program has no key \
                     and no way to receive one — finish it in your browser"
                .into(),
            refused_by_player: false,
            handoff: Some(url),
        })
    }
}

/// Percent-encode a query value. Small on purpose: a URL crate for two fields
/// would be a dependency the budget does not owe.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// rung 2 — talks to `arca.py` over its UNIX socket.
///
/// **Only the client half is here**, in about thirty lines, so the launcher
/// stays standalone. The recogniser, the daily count, the journal and the
/// mandate are all on the far side of the socket with the key, which is the
/// entire point: *a cap enforced by the caller is not a cap.* A check on this
/// side would be a check the caller performs on its own request.
pub struct ArcaSigner {
    pub socket_path: String,
}

impl ArcaSigner {
    #[cfg(unix)]
    fn call(&self, req: Value) -> Value {
        use std::os::unix::net::UnixStream;
        if self.socket_path.is_empty() {
            return json!({"ok": false, "reason": "no arca socket configured"});
        }
        let stream = match UnixStream::connect(&self.socket_path) {
            Ok(s) => s,
            Err(e) => {
                return json!({"ok": false, "reason":
                    format!("cannot reach the arca at {}: {e}", self.socket_path)})
            }
        };
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(20)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(20)));
        let mut w = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => return json!({"ok": false, "reason": e.to_string()}),
        };
        if w.write_all((req.to_string() + "\n").as_bytes()).is_err() {
            return json!({"ok": false, "reason": "the arca closed while being asked"});
        }
        let mut line = String::new();
        if BufReader::new(stream).read_line(&mut line).is_err() || line.trim().is_empty() {
            return json!({"ok": false, "reason": "the arca closed without replying"});
        }
        serde_json::from_str(line.trim()).unwrap_or_else(|_| {
            json!({"ok": false, "reason": "the arca replied with something that is not JSON"})
        })
    }

    /// The arca is a UNIX-socket program. On Windows there is nothing to reach
    /// yet, and saying so beats a connection error a player cannot act on.
    #[cfg(not(unix))]
    fn call(&self, _req: Value) -> Value {
        json!({"ok": false, "reason":
            "the arca is a unix program and this is Windows — use the browser \
             signer, or an external command"})
    }

    pub fn card(&self) -> Value {
        self.call(json!({"op": "card"}))
    }
}

impl Signer for ArcaSigner {
    fn kind(&self) -> &'static str { "arca" }

    fn status(&self) -> String {
        let card = self.card();
        if card["ok"] != json!(true) {
            return card["reason"].as_str().unwrap_or("the arca did not answer").into();
        }
        let body = &card["card"];
        let msgs = &body["messages"];
        let boundary = card["boundary"].as_str().unwrap_or("?");
        let mut line = format!(
            "{} · boundary: {boundary} · {}/{} messages left today",
            body["address"].as_str().unwrap_or("?"),
            msgs["remaining_today"], msgs["per_day"],
        );
        if body["armed"] != json!(true) {
            line.push_str(" · UNARMED (it will refuse to sign)");
        }
        if boundary != "separate uid" {
            // The arca says this itself on every reply; repeating it is
            // deliberate, because this is the line the player reads.
            line.push_str(" · same uid — a lint pass, not a boundary");
        }
        line
    }

    fn address(&self) -> Option<String> {
        self.card()["card"]["address"].as_str().map(str::to_string)
    }

    fn sign(&mut self, text: &str, _why: &str) -> Result<Value, Refusal> {
        let got = self.call(json!({"op": "sign", "text": text}));
        if got["ok"] == json!(true) {
            return Ok(json!({
                "signature": got["signature"],
                "address": got["address"],
                "backend": "arca",
            }));
        }
        Err(Refusal::new(
            got["reason"].as_str().unwrap_or("the arca refused").to_string(),
        ))
    }
}

/// A command the player named: a hardware wallet, `cast wallet sign`, a script.
///
/// The message goes in on stdin; a `0x…` signature is expected on stdout. Two
/// rules, both enforced here:
///
/// * **the command comes from the settings file and nowhere else.** Never from
///   a manifest, a catalog, a depot or a broker client. A game that could name
///   the program that signs for you has already won.
/// * **the output must look like a signature.** 65 bytes of hex or it is a
///   refusal — a signer that prints an error to stdout would otherwise have
///   that error travel *as* a signature and fail somewhere far away with a
///   useless message.
pub struct ExternalSigner {
    pub command: String,
    pub wallet: Option<String>,
}

impl ExternalSigner {
    /// 65 bytes: r (32) + s (32) + v (1), hex, `0x`-prefixed.
    pub fn looks_like_a_signature(s: &str) -> bool {
        let s = s.trim();
        s.len() == 132
            && s.starts_with("0x")
            && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// Split a command line the way a shell would, minus the shell. No `sh -c`
    /// anywhere: the string is the player's, but running it through a shell
    /// would make every metacharacter in it live.
    pub fn argv(command: &str) -> Result<Vec<String>, String> {
        let mut out: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut quote: Option<char> = None;
        let mut any = false;
        for c in command.chars() {
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => cur.push(c),
                None if c == '\'' || c == '"' => {
                    quote = Some(c);
                    any = true;
                }
                None if c.is_whitespace() => {
                    if any || !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                        any = false;
                    }
                }
                None => cur.push(c),
            }
        }
        if quote.is_some() {
            return Err("the signing command has an unclosed quote".into());
        }
        if any || !cur.is_empty() {
            out.push(cur);
        }
        if out.is_empty() {
            return Err("no signing command is set".into());
        }
        Ok(out)
    }
}

impl Signer for ExternalSigner {
    fn kind(&self) -> &'static str { "external" }

    fn status(&self) -> String {
        if self.command.trim().is_empty() {
            return "no command set".into();
        }
        format!("`{}` · stdin → message, stdout → 0x signature", self.command)
    }

    fn address(&self) -> Option<String> { self.wallet.clone() }

    fn sign(&mut self, text: &str, _why: &str) -> Result<Value, Refusal> {
        let argv = ExternalSigner::argv(&self.command).map_err(Refusal::new)?;
        let mut child = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Refusal::new(format!("could not run {:?}: {e}", argv[0])))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let out = child
            .wait_with_output()
            .map_err(|e| Refusal::new(format!("the signing command failed: {e}")))?;
        let printed = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(Refusal::new(format!(
                "the signing command exited {}: {}",
                out.status.code().unwrap_or(-1),
                if err.is_empty() { "(nothing on stderr)".into() } else { err }
            )));
        }
        if !ExternalSigner::looks_like_a_signature(&printed) {
            return Err(Refusal::new(format!(
                "the signing command printed something that is not a 65-byte \
                 signature: {:?}. Refused here rather than passed on, so it \
                 cannot fail somewhere far away as a bad signature.",
                printed.chars().take(80).collect::<String>()
            )));
        }
        Ok(json!({
            "signature": printed,
            "address": self.wallet.clone(),
            "backend": "external",
        }))
    }
}
