//! `elo signin` with no code, `elo profile`, and `elo agent init` — an
//! account, its name and its face, set from a terminal with nobody at a
//! browser.
//!
//! The site already signs a wallet in with no browser: the store prepares a
//! sign-in text (`/api/account/siwe/prepare`), the wallet signs it, and the
//! wallet grant (`/api/oauth/token`) answers with the account and a site
//! token. The first sign-in makes the account. The token lives in this
//! process and nowhere else: nothing here writes a session to disk, so each
//! command signs in again, which costs one signature and no browser.
//!
//! With that token the name is one call (`/api/account/name`) and the face is
//! two (`/api/face/prepare`, then `/api/face`): the face route takes the
//! account's sign-in in place of a second signature when the account names
//! the wallet, which a wallet's own sign-in always sets.
//!
//! `agent init` runs the lot for an agent on its owner's machine: the
//! machine's own keystore, never a paired browser, made if there is none,
//! signed in, named, given a face, and linked for chat.

use crate::args::Args;
use crate::chatkey;
use crate::friends::told;
use crate::prompt;
use crate::signer::{self, CliSigner};
use elo_broker::signer::Signer;
use elo_depot::DepotError;
use elo_hive::render::safe_line;
use elo_net::Net;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The store's wallet grant (`meter/oauth.py` `WALLET_GRANT`).
pub(crate) const WALLET_GRANT: &str = "urn:elopros:params:oauth:grant-type:wallet";
/// `meter/faces.py` `PIC_MAX_BYTES`.
const PIC_MAX_BYTES: usize = 256 * 1024;

fn err(e: impl Into<String>) -> DepotError {
    DepotError::new(e.into())
}

/// A signed-in account: its id, and the header that carries its token.
pub(crate) struct Session {
    pub account: String,
    pub wallet: String,
    pub bearer: String,
}

/// Sign this wallet in to the store: the store's text, signed, traded for a
/// site token. Refuses to sign a text that does not name this wallet and the
/// nonce it came with.
pub(crate) fn sign_in(net: &Net, s: &mut CliSigner, host: &str, why: &str) -> Result<Session, DepotError> {
    let me = s.address().ok_or_else(|| err("the signer names no address"))?.to_lowercase();
    let prep = told(net.post_json_told(&format!("{host}/api/account/siwe/prepare"), &json!({"wallet": me})))?;
    let msg = prep.get("message").and_then(Value::as_str).unwrap_or("");
    let nonce = prep.get("nonce").and_then(Value::as_str).unwrap_or("");
    if !msg.to_lowercase().contains(&me) || nonce.is_empty() || !msg.contains(nonce) {
        return Err(err("the store's sign-in text does not name this wallet and its nonce — nothing was signed"));
    }
    let signed = s.sign(msg, why).map_err(|r| err(r.reason))?;
    let sig = signed.get("signature").and_then(Value::as_str).unwrap_or("");
    let tok = told(net.post_json_told(&format!("{host}/api/oauth/token"),
                                      &json!({"grant_type": WALLET_GRANT, "nonce": nonce, "signature": sig})))?;
    let token = tok.get("access_token").and_then(Value::as_str).unwrap_or("");
    if token.is_empty() {
        return Err(err("the store signed this wallet in and sent no token"));
    }
    Ok(Session {
        account: tok.get("account").and_then(Value::as_str).unwrap_or("").to_string(),
        wallet: me,
        bearer: format!("Bearer {token}"),
    })
}

/// One call with the session's token; the body as JSON, or the store's refusal.
fn call(net: &Net, sess: &Session, method: &str, url: &str, body: Option<&Value>) -> Result<Value, DepotError> {
    let payload = body.map(Value::to_string);
    let mut headers = vec![("authorization", sess.bearer.as_str())];
    if payload.is_some() {
        headers.push(("content-type", "application/json"));
    }
    let (code, _hs, bytes) = net
        .request_full(method, url, &headers, payload.as_deref().map(str::as_bytes), 1 << 20)
        .map_err(err)?;
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !(200..300).contains(&code) {
        let why = v.get("error").and_then(Value::as_str).map(str::to_string)
            .unwrap_or_else(|| format!("the store answered {code}"));
        return Err(err(safe_line(&why)));
    }
    Ok(v)
}

/// `elo signin` with no code: this machine's signer signs in, and says as whom.
pub fn signin(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let net = Net::new();
    let mut s = signer::choose(a, host).map_err(err)?;
    let sess = sign_in(&net, &mut s, host, "sign in to the store")?;
    let me = call(&net, &sess, "GET", &format!("{host}/api/account/me"), None).unwrap_or(Value::Null);
    let name = me.get("name").and_then(Value::as_str).unwrap_or("");
    if json_out {
        println!("{}", json!({"account": sess.account, "wallet": sess.wallet, "name": name}));
        return Ok(0);
    }
    println!("signed in  {}", safe_line(&sess.account));
    println!("wallet     {}", sess.wallet);
    println!("name       {}", if name.is_empty() { "none yet — `elo profile --name <name>`".into() } else { safe_line(name) });
    Ok(0)
}

/// What a profile change asks for, read from the flags.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Change {
    pub name: Option<String>,
    pub face: Option<String>,
    pub kind: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
}

impl Change {
    fn from(a: &Args) -> Self {
        let f = |n: &str| a.flag(n).map(str::to_string);
        Change { name: f("--name"), face: f("--face"), kind: f("--kind"), harness: f("--harness"), model: f("--model") }
    }
    fn is_empty(&self) -> bool {
        *self == Change::default()
    }
}

/// The picture, read and checked before anyone signs: a PNG under the store's
/// cap. The store checks the rest (square, 64 to 512 px, 8-bit).
fn picture(path: &Path) -> Result<Vec<u8>, DepotError> {
    let data = std::fs::read(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(err(format!("{} is not a PNG", path.display())));
    }
    if data.len() > PIC_MAX_BYTES {
        return Err(err(format!("{} is {} KB and the cap is {} KB", path.display(),
                               data.len() / 1024, PIC_MAX_BYTES / 1024)));
    }
    Ok(data)
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Apply a change for a signed-in session: the account's name, then the face
/// (its name too, so rooms and the store agree, plus picture and labels).
fn apply(net: &Net, host: &str, sess: &Session, ch: &Change) -> Result<(), DepotError> {
    if let Some(k) = &ch.kind {
        if k != "agent" && k != "human" {
            return Err(err("--kind is agent or human"));
        }
    }
    let pic = ch.face.as_deref().map(|p| picture(Path::new(p))).transpose()?;
    if let Some(n) = &ch.name {
        call(net, sess, "POST", &format!("{host}/api/account/name"), Some(&json!({"name": n})))?;
    }
    let mut fields = serde_json::Map::new();
    fields.insert("wallet".into(), json!(sess.wallet));
    for (k, v) in [("name", &ch.name), ("kind", &ch.kind), ("harness", &ch.harness), ("model", &ch.model)] {
        if let Some(v) = v {
            fields.insert(k.into(), json!(v));
        }
    }
    if fields.len() == 1 && pic.is_none() {
        return Ok(());
    }
    let mut prep = fields.clone();
    if let Some(p) = &pic {
        prep.insert("picture_sha256".into(), json!(format!("{:x}", Sha256::digest(p))));
        fields.insert("picture_b64".into(), json!(b64(p)));
    }
    let got = call(net, sess, "POST", &format!("{host}/api/face/prepare"), Some(&Value::Object(prep)))?;
    let at = got.get("at").and_then(Value::as_i64).ok_or_else(|| err("the store prepared no face change"))?;
    fields.insert("at".into(), json!(at));
    call(net, sess, "POST", &format!("{host}/api/face"), Some(&Value::Object(fields)))?;
    Ok(())
}

/// `elo profile` — show, or with flags, change: --name, --face <png>,
/// --kind agent|human, --harness, --model.
pub fn profile(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let net = Net::new();
    let ch = Change::from(a);
    let mut s = signer::choose(a, host).map_err(err)?;
    if ch.is_empty() {
        let w = s.address().ok_or_else(|| err("the signer names no address"))?.to_lowercase();
        let got = told(net.get_json_told(&format!("{host}/api/face/{w}")))?;
        if json_out {
            println!("{got}");
        } else {
            for k in ["name", "kind", "harness", "model"] {
                println!("{k:<8} {}", got.get(k).and_then(Value::as_str).map(safe_line).unwrap_or_else(|| "—".into()));
            }
            println!("picture  {}", if got.get("picture").map_or(false, |p| !p.is_null()) { "set" } else { "none" });
        }
        return Ok(0);
    }
    let sess = sign_in(&net, &mut s, host, "sign in to change your profile")?;
    apply(&net, host, &sess, &ch)?;
    if json_out {
        println!("{}", json!({"account": sess.account, "wallet": sess.wallet, "changed": true}));
    } else {
        println!("profile changed for {}", sess.wallet);
    }
    Ok(0)
}

/// `elo agent init` — from nothing to a named agent that speaks in chat.
pub fn init(a: &Args, host: &str, voice_dir: &Path) -> Result<i32, DepotError> {
    let name = a.flag("--name").ok_or_else(|| err("elo agent init --name <name> [--face <png>]"))?;
    // The agent's own key, whatever its owner has paired.
    let mut a = a.clone();
    if !a.has("--local") {
        a.switches.push("--local".into());
    }
    if !a.has("--yes") {
        a.switches.push("--yes".into());
    }
    let path = elo_vault::keystore_path();
    if !elo_vault::LocalSigner::at(&path).exists() {
        make_account(&a, &path)?;
    }
    let net = Net::new();
    let mut s = signer::choose(&a, host).map_err(err)?;
    let sess = sign_in(&net, &mut s, host, "sign the agent in to the store")?;
    let mut ch = Change::from(&a);
    ch.kind.get_or_insert_with(|| "agent".into());
    apply(&net, host, &sess, &ch)?;
    println!("account  {}", safe_line(&sess.account));
    println!("wallet   {}", sess.wallet);
    println!("name     {}", safe_line(name));
    // Chat last: a refusal here leaves a working account behind, and says so.
    if let Err(e) = chatkey::run(&a, host, voice_dir, false) {
        eprintln!("chat not linked — {e}; `elo chat link --local` tries again");
        return Ok(1);
    }
    println!("\nSet ELO_SIGNER=local where this agent runs, so every command signs as it.");
    Ok(0)
}

/// A new keystore for an agent: the recovery words to `--words-file` (made
/// owner-only, never over an existing file), so nobody has to retype three of
/// them, and the passphrase from `ELO_PASSPHRASE_FILE` or asked twice.
fn make_account(a: &Args, path: &Path) -> Result<(), DepotError> {
    let words = a.flag("--words-file").ok_or_else(|| err(format!(
        "no account at {} — give --words-file <path> and agent init makes one, writing its \
         recovery words there (owner-only), or run `elo account new` yourself",
        path.display())))?;
    let words = Path::new(words);
    if words.exists() {
        return Err(err(format!("{} already exists — this never writes over a file", words.display())));
    }
    let phrase = elo_vault::Phrase::generate().map_err(err)?;
    let pass = match std::env::var_os("ELO_PASSPHRASE_FILE").filter(|p| !p.is_empty()) {
        Some(_) => prompt::passphrase("").map_err(err)?,
        None => crate::passphrase_twice()?,
    };
    if pass.chars().count() < elo_vault::local::MIN_PASSPHRASE {
        return Err(err(format!("the passphrase is under {} characters — nothing was written",
                               elo_vault::local::MIN_PASSPHRASE)));
    }
    write_private(words, &format!("{}\n", phrase.words().join(" ")))?;
    let signer = match elo_vault::LocalSigner::restore(path, &pass, &phrase) {
        Ok(s) => s,
        Err(e) => {
            // No keystore, so no words for one.
            let _ = std::fs::remove_file(words);
            return Err(err(e));
        }
    };
    println!("made     {}", signer.address().unwrap_or_default());
    println!("  recovery words in {} — move them somewhere offline", words.display());
    Ok(())
}

fn write_private(path: &Path, body: &str) -> Result<(), DepotError> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    f.write_all(body.as_bytes()).map_err(|e| err(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_vectors() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="),
                       ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(b64(i.as_bytes()), o);
        }
    }

    #[test]
    fn no_flags_is_no_change() {
        let a = crate::args::parse(&["profile".into()]).unwrap();
        assert!(Change::from(&a).is_empty());
        let a = crate::args::parse(&["profile".into(), "--name".into(), "Tarshish".into()]).unwrap();
        assert_eq!(Change::from(&a).name.as_deref(), Some("Tarshish"));
    }

    #[test]
    fn a_picture_that_is_not_a_png_is_refused_before_signing() {
        let p = std::env::temp_dir().join(format!("elo-face-{}.jpg", std::process::id()));
        std::fs::write(&p, b"\xff\xd8\xff not a png").unwrap();
        assert!(picture(&p).unwrap_err().to_string().contains("not a PNG"));
    }

    #[test]
    fn the_words_file_is_never_written_over() {
        let p = std::env::temp_dir().join(format!("elo-words-{}", std::process::id()));
        write_private(&p, "one\n").unwrap();
        assert!(write_private(&p, "two\n").is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}
