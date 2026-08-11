//! `scry dev` — the desk a listing's own developer stands at.
//!
//! The origin half is `meter/devdesk.py`; this is the hand that reaches it.
//! Every write is the same three beats and there is no fourth: **prepare, sign,
//! submit.**
//!
//! ```text
//!   prepare  →  the origin validates the exact edit and hands back the text
//!               to sign, plus every refusal and warning it has
//!   sign     →  the account on THIS machine signs that text (EIP-191)
//!   submit   →  the same fields go back with the signature attached
//! ```
//!
//! Prepare is not a nicety. A dev should never learn a field was refused
//! *after* typing a passphrase for it, and the origin's refusals are written as
//! sentences precisely so a client can print them — which is why the desk's
//! POSTs go through [`Net::post_json_told`] and not `post_json`, whose refusal
//! is a bare status code.
//!
//! **The key never leaves this machine and never reaches scry.** The signature
//! does. That is the same wall `scry account sign` stands on, and the reason a
//! studio's build agent can be handed a *delegation* — its own address, its own
//! key, its own scoped standing — instead of a copy of the studio's key.
//!
//! ## Why a CLI first, and maybe only
//!
//! Operator, 2026-08-09: *"cli and api first idk if i care about a frontend for
//! any of this for devs when we all just AI."* A store's dev portal is
//! traditionally a web console because the dev is a person with a mouse. Here
//! the dev is as likely to be an agent with a shell, and everything below is
//! `--json` clean: an agent reads `scry dev --json`, learns which titles it may
//! touch and what a post may contain, and posts one. No page had to exist for
//! that to work, and adding one later changes nothing here.

use crate::args::Args;
use crate::prompt;
use scry_broker::signer::Signer;
use scry_depot::{sha256_file, DepotError};
use scry_net::{Fetched, Net};
use serde_json::{json, Map, Value};
use std::path::Path;

pub const USAGE: &str = "\
scry dev — a listing's own desk: the row, the art, the updates

  scry dev                          where this account stands, on every title
  scry dev show   <slug>            the row as the store serves it
  scry dev edit   <slug> --set k=v  edit your own row (repeat --set, or --from f.json)
  scry dev image  <slug> <file>     upload art; prints the url to paste into a field
  scry dev post   <slug> --title T  post a community update
  scry dev posts  <slug>            the updates already posted
  scry dev delegate <slug> <address>   let another wallet act for you
  scry dev revoke   <slug> <address>   take that back (or resign, with your own)

  --set k=v        a field to write; repeat it. A value that parses as JSON is
                   used as JSON, so --set tags='[\"survival\"]' is a list
  --from FILE      read the fields from a JSON object instead
  --slot NAME      which slot the art is for: capsule | hero | media.src | record.art
  --title T        the post's headline          --kind K   update|patch|release|news|event
  --body B         the post's text              --body-file F   …read it from a file
  --art URL        art for the post (upload it first)   --link URL   where it points
  --scope S        a delegation's scopes, comma separated: post,media,edit
  --days N         make the delegation expire in N days
  --dry-run        prepare only: every refusal and warning, nothing signed
  --json           machine-readable output
";

/// The address at this machine's account, without unlocking anything — it lives
/// in the keystore in the clear. Needed *before* signing, because the origin
/// picks which allowlist to validate against from who is asking.
fn my_address() -> Option<String> {
    scry_vault::LocalSigner::at(scry_vault::keystore_path()).address()
}

/// Unlock once, sign once. The passphrase is prompted for the specific text
/// being signed and is never held — the vault's own rule, kept here.
fn sign(message: &str) -> Result<(String, String), DepotError> {
    let path = scry_vault::keystore_path();
    let mut signer = scry_vault::LocalSigner::at(&path);
    if !signer.exists() {
        return Err(DepotError::new(format!(
            "no account at {} — `scry account new` makes one, and the key stays there",
            path.display()
        )));
    }
    crate::warn_if_echoing();
    let pass = prompt::secret("passphrase: ").map_err(DepotError::new)?;
    signer.unlock(&pass).map_err(DepotError::new)?;
    let out = signer
        .sign(message, "scry dev")
        .map_err(|r| DepotError::new(r.reason))?;
    let addr = out
        .get("address")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let sig = out
        .get("signature")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if addr.is_empty() || sig.is_empty() {
        return Err(DepotError::new("the signer returned no signature"));
    }
    Ok((addr, sig))
}

/// A read that says `reachable` before it says anything else — the trap this
/// whole crate is built around (`scry-net`'s module note).
fn read(net: &Net, url: &str) -> Result<Value, DepotError> {
    let got = net.get_json(url);
    match got.value {
        Some(v) if got.ok => Ok(v),
        _ if !got.reachable => Err(DepotError::new(format!("could not reach {url}: {}", got.why))),
        Some(v) => Ok(v),
        None => Err(DepotError::new(format!("{url}: {}", got.why))),
    }
}

fn say_warnings(v: &Value) {
    for w in v.get("warnings").and_then(Value::as_array).into_iter().flatten() {
        if let Some(s) = w.as_str() {
            eprintln!("  ⚠ {s}");
        }
    }
}

/// Turn a refusal into an error carrying the origin's own sentence. A store
/// desk that refuses in prose and a client that prints `422` are a bad pair.
fn told(got: Fetched<Value>, what: &str) -> Result<Value, DepotError> {
    if got.ok {
        return Ok(got.value.unwrap_or(Value::Null));
    }
    if !got.reachable {
        return Err(DepotError::new(format!("could not reach the origin: {}", got.why)));
    }
    Err(DepotError::new(format!("{what} refused: {}", got.why)))
}

/// prepare → warnings → sign → submit. Every write here goes through it, so
/// there is exactly one place that decides what a dev sees before a passphrase.
fn perform(
    net: &Net,
    a: &Args,
    prepare_url: &str,
    prepare_body: Value,
    submit_url: &str,
    mut submit_body: Value,
    json_out: bool,
) -> Result<i32, DepotError> {
    let prepared = told(net.post_json_told(prepare_url, &prepare_body), "the edit")?;
    say_warnings(&prepared);
    let message = prepared
        .get("sign_this")
        .and_then(Value::as_str)
        .ok_or_else(|| DepotError::new("the origin returned no text to sign"))?;
    if a.has("--dry-run") {
        if json_out {
            println!("{}", serde_json::to_string_pretty(&prepared).unwrap_or_default());
        } else {
            println!("would sign:\n{message}");
            println!("\nnothing was sent — drop --dry-run to do it");
        }
        return Ok(0);
    }
    let (address, signature) = sign(message)?;
    if let Some(map) = submit_body.as_object_mut() {
        map.insert("wallet".into(), json!(address));
        map.insert("signature".into(), json!(signature));
    }
    let done = told(net.post_json_told(submit_url, &submit_body), "the act")?;
    say_warnings(&done);
    if json_out {
        println!("{}", serde_json::to_string_pretty(&done).unwrap_or_default());
    } else if let Some(note) = done.get("note").and_then(Value::as_str) {
        println!("done — {note}");
    } else {
        println!("done");
    }
    Ok(0)
}

/// `--set k=v`, repeated. A value that parses as JSON is used as JSON, so a
/// list or a boolean can be typed without a second flag; everything else is a
/// string, which is what a blurb is.
fn fields_from(a: &Args) -> Result<Value, DepotError> {
    let mut out = Map::new();
    if let Some(file) = a.flag("--from") {
        let text = std::fs::read_to_string(file)
            .map_err(|e| DepotError::new(format!("{file}: {e}")))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => out.extend(m),
            Ok(_) => return Err(DepotError::new(format!("{file} is not a JSON object"))),
            Err(e) => return Err(DepotError::new(format!("{file}: {e}"))),
        }
    }
    for pair in a.all("--set") {
        let (key, raw) = pair
            .split_once('=')
            .ok_or_else(|| DepotError::new(format!("--set wants k=v, got {pair:?}")))?;
        if key.is_empty() {
            return Err(DepotError::new("--set wants a field name before the ="));
        }
        let value = serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!(raw));
        out.insert(key.to_string(), value);
    }
    if out.is_empty() {
        return Err(DepotError::new(
            "nothing to write — use --set field=value (repeatable) or --from file.json",
        ));
    }
    Ok(Value::Object(out))
}

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let net = Net::new();
    let sub = a.positional.first().map(String::as_str).unwrap_or("whoami");
    // `scry dev gates` reads as "the gates desk", so a slug in the first
    // position is not an error — it is the show verb with the word left out.
    let (sub, slug_first) = match sub {
        "whoami" | "show" | "edit" | "image" | "post" | "posts" | "delegate" | "revoke"
        | "help" => (sub, false),
        _ => ("show", true),
    };
    let slug_at = if slug_first { 0 } else { 1 };
    let game = || -> Result<String, DepotError> {
        a.positional
            .get(slug_at)
            .cloned()
            .ok_or_else(|| DepotError::new("which game? — a slug, like `gates`"))
    };

    match sub {
        "help" => {
            print!("{USAGE}");
            Ok(0)
        }

        // Where do I stand, and on whose word. The first thing an arriving
        // agent asks, and the reason it is the default with no verb at all.
        "whoami" => {
            let me = my_address();
            let url = match &me {
                Some(addr) => format!("{host}/api/store/dev?wallet={addr}"),
                None => format!("{host}/api/store/dev"),
            };
            let v = read(&net, &url)?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(0);
            }
            match &me {
                Some(addr) => println!("{addr}"),
                // Not an error, and not "you have no standing" either — those
                // are different sentences and only one of them is about a key.
                None => println!("no account on this machine — `scry account new` makes one"),
            }
            let standing = v.get("standing").and_then(Value::as_array).cloned().unwrap_or_default();
            if standing.is_empty() {
                println!(
                    "  {}",
                    v.get("why_not").and_then(Value::as_str).unwrap_or("no standing on any title")
                );
            }
            for s in &standing {
                let owns = s.get("owns").and_then(Value::as_bool).unwrap_or(false);
                println!(
                    "  {:<16} {:<9} {}{}",
                    s.get("game").and_then(Value::as_str).unwrap_or("?"),
                    s.get("role").and_then(Value::as_str).unwrap_or("?"),
                    s.get("scopes")
                        .and_then(Value::as_array)
                        .map(|xs| xs
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(","))
                        .unwrap_or_default(),
                    if owns { "  · yours" } else { "" }
                );
            }
            if !v.get("armed").and_then(Value::as_bool).unwrap_or(false) {
                println!("  ⚠ the desk is not armed on this origin — reads work, writes refuse");
            }
            Ok(0)
        }

        "show" => {
            let slug = game()?;
            let cat = read(&net, &format!("{host}/api/store/catalog"))?;
            let row = cat
                .get("titles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|t| t.get("slug").and_then(Value::as_str) == Some(slug.as_str()))
                .cloned()
                .ok_or_else(|| {
                    DepotError::new(format!("'{slug}' is not on the shelf — `scry games` lists it"))
                })?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&row).unwrap_or_default());
                return Ok(0);
            }
            for key in ["name", "kind", "band", "state", "blurb", "capsule", "hero", "play_url"] {
                if let Some(v) = row.get(key) {
                    if !v.is_null() {
                        println!("{key:<10} {}", v.as_str().map(str::to_string)
                            .unwrap_or_else(|| v.to_string()));
                    }
                }
            }
            let recs = row.get("record").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            println!("record     {recs} entr{}", if recs == 1 { "y" } else { "ies" });
            Ok(0)
        }

        "edit" => {
            let slug = game()?;
            let fields = fields_from(a)?;
            let mut prep = json!({"game": slug, "fields": fields, "act": "save"});
            if let Some(addr) = my_address() {
                // Sent so the origin quotes the DEV allowlist back rather than
                // the house's — a refusal that names the wrong field set is
                // worse than no refusal.
                prep["wallet"] = json!(addr);
            }
            perform(
                &net,
                a,
                &format!("{host}/api/store/prepare"),
                prep,
                &format!("{host}/api/store/title"),
                json!({"game": slug, "fields": fields}),
                json_out,
            )
        }

        // Art without a commit. The file is hashed HERE, the hash is what gets
        // signed, and the origin re-hashes what actually arrived — so a
        // signature travels with exactly the bytes it was made for.
        "image" => {
            let slug = game()?;
            let file = a
                .positional
                .get(slug_at + 1)
                .ok_or_else(|| DepotError::new("which file? — a path to a png, jpg, webp, gif or svg"))?;
            let path = Path::new(file);
            let bytes = std::fs::read(path).map_err(|e| DepotError::new(format!("{file}: {e}")))?;
            let sha = sha256_file(path)?;
            let prepared = told(
                net.post_json_told(
                    &format!("{host}/api/store/dev/prepare"),
                    &json!({"act": "upload", "game": slug, "fields": {"sha256": sha}}),
                ),
                "the upload",
            )?;
            let message = prepared
                .get("sign_this")
                .and_then(Value::as_str)
                .ok_or_else(|| DepotError::new("the origin returned no text to sign"))?;
            if a.has("--dry-run") {
                println!("would upload {} ({} bytes, sha256 {sha})", file, bytes.len());
                println!("would sign:\n{message}");
                return Ok(0);
            }
            let (address, signature) = sign(message)?;
            let slot = a.flag("--slot").unwrap_or("");
            let url = format!(
                "{host}/api/store/dev/upload?game={slug}&wallet={address}&signature={signature}\
                 &slot={slot}&name={}",
                urlish(path.file_name().and_then(|s| s.to_str()).unwrap_or("art"))
            );
            let done = told(
                net.post_bytes_told(&url, "application/octet-stream", bytes),
                "the upload",
            )?;
            say_warnings(&done);
            if json_out {
                println!("{}", serde_json::to_string_pretty(&done).unwrap_or_default());
                return Ok(0);
            }
            let asset = done.get("url").and_then(Value::as_str).unwrap_or("");
            println!("{asset}");
            if let Some(size) = done.get("size").and_then(Value::as_array) {
                let fits = done
                    .get("fits")
                    .and_then(Value::as_array)
                    .map(|xs| xs.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                println!(
                    "  {}x{} · fits: {}",
                    size.first().and_then(Value::as_i64).unwrap_or(0),
                    size.get(1).and_then(Value::as_i64).unwrap_or(0),
                    if fits.is_empty() { "no declared slot".into() } else { fits }
                );
            }
            println!("  wire it up: scry dev edit {slug} --set capsule={asset}");
            Ok(0)
        }

        "post" => {
            let slug = game()?;
            let body = match (a.flag("--body"), a.flag("--body-file")) {
                (Some(_), Some(_)) => {
                    return Err(DepotError::new("--body and --body-file are two answers to one question"))
                }
                (Some(b), None) => b.to_string(),
                (None, Some(f)) => std::fs::read_to_string(f)
                    .map_err(|e| DepotError::new(format!("{f}: {e}")))?,
                (None, None) => {
                    return Err(DepotError::new(
                        "what does it say? — --body \"…\" or --body-file notes.md",
                    ))
                }
            };
            let title = a
                .flag("--title")
                .ok_or_else(|| DepotError::new("--title is the headline the store shows"))?;
            let mut fields = Map::new();
            fields.insert("title".into(), json!(title));
            fields.insert("body".into(), json!(body));
            if let Some(k) = a.flag("--kind") {
                fields.insert("kind".into(), json!(k));
            }
            if let Some(art) = a.flag("--art") {
                fields.insert("art".into(), json!(art));
            }
            if let Some(link) = a.flag("--link") {
                fields.insert("href".into(), json!(link));
            }
            let fields = Value::Object(fields);
            perform(
                &net,
                a,
                &format!("{host}/api/store/dev/prepare"),
                json!({"act": "post", "game": slug, "fields": fields}),
                &format!("{host}/api/store/dev/post"),
                json!({"game": slug, "fields": fields}),
                json_out,
            )
        }

        "posts" => {
            let slug = game()?;
            let limit = a.flag("--limit").unwrap_or("20");
            let v = read(&net, &format!("{host}/api/store/posts?game={slug}&limit={limit}"))?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(0);
            }
            let posts = v.get("posts").and_then(Value::as_array).cloned().unwrap_or_default();
            if posts.is_empty() {
                // A real count of a real read, and it says so — this is not the
                // "we could not look" answer, which `read` already raised.
                println!("no updates posted for {slug} yet");
                return Ok(0);
            }
            for p in posts {
                println!(
                    "{:<11} {:<8} {}",
                    p.get("at").and_then(Value::as_str).unwrap_or("")[..10.min(
                        p.get("at").and_then(Value::as_str).unwrap_or("").len()
                    )]
                    .to_string(),
                    p.get("kind").and_then(Value::as_str).unwrap_or("update"),
                    p.get("title").and_then(Value::as_str).unwrap_or("")
                );
            }
            Ok(0)
        }

        "delegate" => {
            let slug = game()?;
            let who = a
                .positional
                .get(slug_at + 1)
                .ok_or_else(|| DepotError::new("which wallet? — the 0x address to delegate to"))?;
            let mut fields = Map::new();
            fields.insert("wallet".into(), json!(who));
            if let Some(scopes) = a.flag("--scope") {
                fields.insert(
                    "scopes".into(),
                    json!(scopes
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()),
                );
            }
            if let Some(days) = a.flag("--days") {
                let n: i64 = days
                    .parse()
                    .map_err(|_| DepotError::new(format!("--days wants a number, got {days:?}")))?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                fields.insert("until".into(), json!(now + n * 86_400));
            }
            let fields = Value::Object(fields);
            perform(
                &net,
                a,
                &format!("{host}/api/store/dev/prepare"),
                json!({"act": "delegate", "game": slug, "fields": fields}),
                &format!("{host}/api/store/dev/delegate"),
                json!({"game": slug, "fields": fields}),
                json_out,
            )
        }

        "revoke" => {
            let slug = game()?;
            // Defaulting to your own address is what makes resigning easy, and
            // resigning is the one act on this desk nobody else can block.
            let who = match a.positional.get(slug_at + 1) {
                Some(w) => w.clone(),
                None => my_address().ok_or_else(|| {
                    DepotError::new("which wallet? — a 0x address, or none to resign your own")
                })?,
            };
            let fields = json!({"wallet": who});
            perform(
                &net,
                a,
                &format!("{host}/api/store/dev/prepare"),
                json!({"act": "revoke", "game": slug, "fields": fields}),
                &format!("{host}/api/store/dev/revoke"),
                json!({"game": slug, "fields": fields}),
                json_out,
            )
        }

        other => Err(DepotError::new(format!(
            "unknown dev command {other:?} — `scry dev help` lists them"
        ))),
    }
}

/// Percent-encode the few characters that would break a query string. Not a URL
/// library: this escapes a filename we are passing as a label, and a label is
/// all it is — the origin derives the extension from the bytes.
fn urlish(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => out.push(c),
            _ => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args;

    fn argv(parts: &[&str]) -> args::Args {
        args::parse(&parts.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn set_is_repeatable_and_builds_one_object() {
        let a = argv(&["dev", "edit", "gates", "--set", "blurb=a line", "--set", "kind=survival"]);
        let f = fields_from(&a).unwrap();
        assert_eq!(f["blurb"], "a line");
        assert_eq!(f["kind"], "survival");
    }

    #[test]
    fn a_value_that_is_json_is_used_as_json() {
        // The alternative is a second flag for "this one is a list", which is a
        // worse thing to have to remember than quoting.
        let a = argv(&["dev", "edit", "gates", "--set", r#"tags=["survival","pvp"]"#,
                       "--set", "repo_public=true"]);
        let f = fields_from(&a).unwrap();
        assert_eq!(f["tags"], serde_json::json!(["survival", "pvp"]));
        assert_eq!(f["repo_public"], serde_json::json!(true));
    }

    #[test]
    fn a_value_that_only_looks_like_a_word_stays_a_string() {
        let a = argv(&["dev", "edit", "gates", "--set", "blurb=100 players, one shard"]);
        assert_eq!(fields_from(&a).unwrap()["blurb"], "100 players, one shard");
    }

    #[test]
    fn an_equals_inside_the_value_belongs_to_the_value() {
        let a = argv(&["dev", "edit", "gates", "--set", "play_url=https://x.example/?a=b"]);
        assert_eq!(fields_from(&a).unwrap()["play_url"], "https://x.example/?a=b");
    }

    #[test]
    fn a_set_without_an_equals_is_a_refusal_never_a_guess() {
        let a = argv(&["dev", "edit", "gates", "--set", "blurb"]);
        assert!(fields_from(&a).is_err());
    }

    #[test]
    fn nothing_to_write_says_so_rather_than_sending_an_empty_edit() {
        // An empty edit would be refused by the origin anyway — but after a
        // round trip and, worse, after a passphrase prompt.
        assert!(fields_from(&argv(&["dev", "edit", "gates"])).is_err());
    }

    #[test]
    fn from_a_file_and_set_compose_with_set_winning() {
        let dir = std::env::temp_dir().join("scry-dev-test-fields");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("row.json");
        std::fs::write(&f, r#"{"blurb":"from the file","kind":"survival"}"#).unwrap();
        let a = argv(&["dev", "edit", "gates", "--from", f.to_str().unwrap(),
                       "--set", "blurb=typed later"]);
        let got = fields_from(&a).unwrap();
        assert_eq!(got["kind"], "survival");
        assert_eq!(got["blurb"], "typed later", "the flag typed last wins");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn a_filename_label_cannot_break_out_of_the_query_string() {
        assert_eq!(urlish("my capsule&x=1.png"), "my%20capsule%26x%3D1.png");
        assert_eq!(urlish("capsule.png"), "capsule.png");
    }
}
