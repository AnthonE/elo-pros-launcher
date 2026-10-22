//! `elo friends`, through the real binary, against a stub of the origin.
//!
//! The routes are the contract `meter/friends.py` serves: `GET
//! /api/friends/{wallet}`, `POST /api/friends/prepare`, `POST
//! /api/friends/act`. The signer is a browser pairing pointed at the same
//! stub, so the whole prepare → sign → submit walk runs with no key on disk
//! and no passphrase — and what reached the origin is asserted, not assumed.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};

const ELO: &str = env!("CARGO_BIN_EXE_elo");
const ME: &str = "0x00000000000000000000000000000000000000aa";
const ASH: &str = "0x00000000000000000000000000000000000000bb";
const BO: &str = "0x00000000000000000000000000000000000000cc";

/// `(method, path with query, body)` of every request the stub saw.
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;
type Route = Box<dyn Fn(&str, &str, &str) -> Option<(u16, String)> + Send>;

struct Origin {
    base: String,
    seen: Seen,
    _stop: mpsc::Sender<()>,
}

/// A hand-written HTTP/1.1 stub, for the reason `origin.rs` gives: a web
/// framework in a launcher's dev-dependencies would spend the tree budget.
fn serve(route: Route) -> Origin {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if rx.try_recv().is_ok() {
                return;
            }
            let Ok(mut stream) = stream else { continue };
            let Some((method, path, body)) = read_request(&mut stream) else { continue };
            log.lock().unwrap().push((method.clone(), path.clone(), body.clone()));
            let (code, reply) = route(&method, &path, &body)
                .unwrap_or((404, r#"{"detail":"Not Found"}"#.to_string()));
            let head = format!(
                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        }
    });
    Origin { base: format!("http://127.0.0.1:{port}"), seen, _stop: tx }
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String, String)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).ok()?;
    Some((method, path, String::from_utf8_lossy(&body).to_string()))
}

/// A list as the origin answers it: Ash playing with a link, Bo asking.
fn list(reachable: bool, ash_name: &str, join: Value) -> Value {
    json!({
        "wallet": ME, "count": 1,
        "friends": [{
            "wallet": ASH, "since": 1_759_000_000, "name": ash_name, "pic": null,
            "agent": false, "online": true,
            "playing": {"game": "gates", "title": "Gates", "server": "eu-1",
                        "server_name": "Gates EU", "since": 1_760_000_000, "join": join},
            "last_online": 1_760_000_100,
        }],
        "asks_in": [{"wallet": BO, "at": 1_760_000_000, "name": "Bo"}],
        "asks_out": [],
        "presence": {"reachable": reachable,
                     "why": if reachable { Value::Null } else { json!("ledger down") }},
        "limits": {"friends_max": 200, "asks_out_max": 50},
    })
}

/// Run `elo` with no account and no pairing unless the test made one, and no
/// proxy — a proxy answers for a dead port, which would let this pass against
/// nothing.
fn elo(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut c = Command::new(ELO);
    c.args(args)
        .env("ELO_KEYSTORE", dir.join("keystore.json"))
        .env("ELO_PAIRING", dir.join("pairing.json"))
        .env("ELO_STATE", dir.join("state"))
        .env("ELO_GAMES", dir.join("games"))
        .env_remove("ELO_WALLET")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy");
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("run elo");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The `at` every prepare stub hands back, and its UTC day.
const AT: u64 = 1_760_000_000;
const DAY: &str = "2025-10-09";

/// The text `meter/friends.py` offers for a prepare body: `playauth.play_message`
/// over `detail_of(op, args, at)`, keyed to `day_of(at)`. serde_json's map is
/// sorted and `to_string` is compact, which is the meter's `json.dumps`.
fn meter_text(b: &Value) -> String {
    let detail = json!({"args": b["args"], "at": AT, "op": b["op"]}).to_string();
    format!("elo play\naction: friends\nwallet: {}\nday: {DAY}\ndetail: {detail}",
            b["wallet"].as_str().unwrap_or(""))
}

/// Every request that reached the signer or the act, which a refused change
/// must leave empty.
fn signed_or_sent(seen: &Seen) -> Vec<(String, String, String)> {
    seen.lock().unwrap().iter()
        .filter(|(_, p, _)| p.contains("/api/signer/") || p == "/api/friends/act")
        .cloned()
        .collect()
}

/// A browser pairing for ME, pointed at the stub — the signer with no key.
fn pair(dir: &std::path::Path, base: &str) {
    std::fs::write(
        dir.join("pairing.json"),
        json!({"host": base, "code": "ABCD2345", "secret": "s3cret",
               "address": ME, "expires": 0})
        .to_string(),
    )
    .unwrap();
}

#[test]
fn the_list_says_who_is_playing_where_with_the_link() {
    let o = serve(Box::new(|m, p, _| {
        (m == "GET" && p == format!("/api/friends/{ME}"))
            .then(|| (200, list(true, "Ash", json!("elo://join/gates/eu-1.example.test:61234")).to_string()))
    }));
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = elo(dir.path(), &["friends", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("1 friend"), "{out}");
    assert!(out.contains("Ash") && out.contains("playing Gates · Gates EU"), "{out}");
    assert!(out.contains("elo://join/gates/eu-1.example.test:61234"), "{out}");
    assert!(out.contains("asking you") && out.contains(BO), "{out}");

    // The wallet can come from the environment too, and `--json` is one
    // document a script can read.
    let (code, out, err) =
        elo(dir.path(), &["friends", "--host", &o.base, "--json"], &[("ELO_WALLET", ME)]);
    assert_eq!(code, 0, "{out}{err}");
    let v: Value = serde_json::from_str(&out).expect("--json is JSON");
    assert_eq!(v["count"], 1);
    assert_eq!(v["friends"][0]["playing"]["join"], "elo://join/gates/eu-1.example.test:61234");
    assert_eq!(v["friends"][0]["presence"], "playing Gates · Gates EU");
    assert_eq!(v["presence"]["reachable"], true);
}

#[test]
fn presence_nobody_could_read_is_never_printed_as_a_place() {
    let o = serve(Box::new(|m, _, _| {
        (m == "GET").then(|| (200, list(false, "Ash", Value::Null).to_string()))
    }));
    let dir = tempfile::tempdir().unwrap();
    let (code, out, _) = elo(dir.path(), &["friends", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 0);
    assert!(out.contains("presence could not be read — ledger down"), "{out}");
    assert!(out.contains("—"), "{out}");
    assert!(!out.contains("playing"), "an unread ledger must not say playing: {out}");
}

#[test]
fn an_origin_that_is_not_there_is_not_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) =
        elo(dir.path(), &["friends", "--wallet", ME, "--host", "http://127.0.0.1:9"], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("could not reach"), "{err}");
    assert!(!out.contains("friend"), "nothing about friends may be printed: {out}");

    // A document that is not a list is said to be one, rather than drawn.
    let o = serve(Box::new(|_, _, _| Some((200, json!({"wallet": 7}).to_string()))));
    let (code, _, err) = elo(dir.path(), &["friends", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("the friends list did not read"), "{err}");
}

#[test]
fn no_wallet_anywhere_says_where_one_comes_from() {
    let dir = tempfile::tempdir().unwrap();
    let (code, _, err) = elo(dir.path(), &["friends", "--host", "http://127.0.0.1:9"], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("whose friends?") && err.contains("--wallet"), "{err}");
    let (code, _, err) = elo(dir.path(), &["friends", "--wallet", "ash"], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("not a wallet"), "{err}");
    let (code, _, err) = elo(dir.path(), &["friends", "befriend", ASH], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("unknown friends command"), "{err}");
}

#[test]
fn add_is_prepared_signed_and_submitted_as_the_signer() {
    let o = serve(Box::new(|m, p, body| {
        let b: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        match (m, p) {
            ("POST", "/api/friends/prepare") => Some((200, json!({
                "sign_this": meter_text(&b), "at": AT, "args": b["args"], "op": b["op"],
            }).to_string())),
            ("POST", "/api/signer/ABCD2345/ask") => Some((200, r#"{"id":"q1"}"#.into())),
            ("GET", p) if p.starts_with("/api/signer/ABCD2345/ask/q1") => {
                Some((200, json!({"state": "signed", "signature": "0xfeed"}).to_string()))
            }
            ("POST", "/api/friends/act") => Some((200, json!({
                "ok": true, "op": "ask",
                "view": {"wallet": ME, "friends": [], "asks_in": [], "presence": {"reachable": true},
                         "asks_out": [{"wallet": ASH, "at": 1_760_000_000, "name": null}]},
            }).to_string())),
            _ => None,
        }
    }));
    let dir = tempfile::tempdir().unwrap();
    pair(dir.path(), &o.base);
    let (code, out, err) = elo(dir.path(), &["friends", "add", ASH, "--host", &o.base], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("asked 0x0000…00bb"), "{out}");
    assert!(out.contains("public"), "the one line at the moment it becomes true: {out}");

    let seen = o.seen.lock().unwrap().clone();
    let body = |path: &str| -> Value {
        let row = seen.iter().find(|(m, p, _)| m == "POST" && p == path).expect(path);
        serde_json::from_str(&row.2).unwrap()
    };
    let prep = body("/api/friends/prepare");
    assert_eq!(prep, json!({"wallet": ME, "op": "ask", "args": {"to": ASH}}));
    // The browser was asked to sign exactly the text the meter composes.
    let ask = body("/api/signer/ABCD2345/ask");
    assert_eq!(ask["text"], format!(
        "elo play\naction: friends\nwallet: {ME}\nday: {DAY}\n\
         detail: {{\"args\":{{\"to\":\"{ASH}\"}},\"at\":{AT},\"op\":\"ask\"}}"
    ), "{ask}");
    let act = body("/api/friends/act");
    assert_eq!(act, json!({"wallet": ME, "op": "ask", "args": {"to": ASH},
                           "at": 1_760_000_000u64, "signature": "0xfeed"}));

    // `--json` is one document, rebuilt from the parsed list.
    let (code, out, err) =
        elo(dir.path(), &["friends", "add", ASH, "--json", "--host", &o.base], &[]);
    assert_eq!(code, 0, "{out}{err}");
    let v: Value = serde_json::from_str(&out).expect("--json is JSON");
    assert_eq!(v["op"], "ask");
    assert_eq!(v["wallet"], ASH);
    assert_eq!(v["view"]["asks_out"][0]["wallet"], ASH);
}

#[test]
fn a_refusal_prints_the_origins_words_and_a_dry_run_signs_nothing() {
    let o = serve(Box::new(|m, p, b| {
        let b: Value = serde_json::from_str(b).unwrap_or(Value::Null);
        match (m, p) {
            ("POST", "/api/friends/prepare") if b["op"] == "block" => Some((200, json!({
                "sign_this": meter_text(&b), "at": AT, "args": b["args"], "op": "block",
            }).to_string())),
            // An origin that turns a remove into a block is refused locally.
            ("POST", "/api/friends/prepare") if b["op"] == "remove" => {
                let mut block = b.clone();
                block["op"] = json!("block");
                Some((200, json!({
                    "sign_this": meter_text(&block), "at": AT, "args": b["args"], "op": "block",
                }).to_string()))
            }
            ("POST", "/api/friends/prepare") => Some((403, json!({
                "error": "they are not taking asks\x1b[2J"
            }).to_string())),
            _ => None,
        }
    }));
    let dir = tempfile::tempdir().unwrap();
    pair(dir.path(), &o.base);

    let (code, _, err) = elo(dir.path(), &["friends", "add", ASH, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("they are not taking asks"), "{err}");
    assert!(!err.contains('\x1b'), "the origin's words are printed safely: {err:?}");

    let (code, out, err) =
        elo(dir.path(), &["friends", "block", ASH, "--dry-run", "--host", &o.base], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("would sign") && out.contains("nothing was sent"), "{out}");

    let (code, _, err) = elo(dir.path(), &["friends", "remove", ASH, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("nothing was signed"), "{err}");

    // Nothing above reached the signer or the act.
    let sent = signed_or_sent(&o.seen);
    assert!(sent.is_empty(), "{sent:?}");

    // Your own wallet, and a --wallet that is not the signer's, never leave.
    let (code, _, err) = elo(dir.path(), &["friends", "add", ME, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("your own wallet"), "{err}");
    let (code, _, err) =
        elo(dir.path(), &["friends", "add", ASH, "--wallet", BO, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("drop --wallet"), "{err}");
}

/// The finding this exists for: an origin that echoes the op and the args it
/// was sent, and offers some other text to sign — a governance vote labelled
/// as a friend request. The text is what the signer signs, so it is the text
/// that is checked, and nothing reaches the signer.
#[test]
fn a_text_other_than_the_change_asked_for_is_never_signed() {
    let right = meter_text(&json!({"wallet": ME, "op": "ask", "args": {"to": ASH}}));
    let offers = [
        format!("elo play\naction: vote\nwallet: {ME}\nday: {DAY}\n\
                 detail: {{\"at\":{AT},\"ballot\":\"gates-1\",\"choice\":\"no\"}}"),
        right.replace(&format!("wallet: {ME}"), &format!("wallet: {BO}")),
        right.replace("\"op\":\"ask\"", "\"op\":\"block\""),
        right.replace(&format!("\"to\":\"{ASH}\""), &format!("\"to\":\"{BO}\"")),
        right.replace(DAY, "2025-10-10"),
        right.replacen("elo play", "scry play", 1),
        format!("{right}\n"),
    ];
    for offer in offers {
        assert_ne!(offer, right);
        let text = offer.clone();
        let o = serve(Box::new(move |m, p, body| {
            let b: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            match (m, p) {
                ("POST", "/api/friends/prepare") => Some((200, json!({
                    "sign_this": text, "at": AT, "args": b["args"], "op": b["op"],
                }).to_string())),
                ("POST", "/api/signer/ABCD2345/ask") => Some((200, r#"{"id":"q1"}"#.into())),
                ("GET", p) if p.starts_with("/api/signer/ABCD2345/ask/q1") => {
                    Some((200, json!({"state": "signed", "signature": "0xfeed"}).to_string()))
                }
                ("POST", "/api/friends/act") => Some((200, json!({"ok": true}).to_string())),
                _ => None,
            }
        }));
        let dir = tempfile::tempdir().unwrap();
        pair(dir.path(), &o.base);
        let (code, out, err) = elo(dir.path(), &["friends", "add", ASH, "--host", &o.base], &[]);
        assert_eq!(code, 1, "{offer:?}\n{out}{err}");
        assert!(err.contains("nothing was signed"), "{offer:?}\n{err}");
        assert!(!out.contains("asked"), "{offer:?}\n{out}");
        // A dry run does not print it as the text it would sign either.
        let (code, out, _) =
            elo(dir.path(), &["friends", "add", ASH, "--dry-run", "--host", &o.base], &[]);
        assert_eq!(code, 1, "{offer:?}\n{out}");
        assert!(!out.contains("would sign"), "{out}");
        let sent = signed_or_sent(&o.seen);
        assert!(sent.is_empty(), "{offer:?} reached {sent:?}");
    }
}

#[test]
fn join_follows_a_friends_link_through_the_join_verb() {
    let o = serve(Box::new(|m, p, _| {
        (m == "GET" && p.starts_with("/api/friends/"))
            .then(|| (200, list(true, "Ash", json!("elo://join/gates/eu-1.example.test:61234")).to_string()))
    }));
    let dir = tempfile::tempdir().unwrap();
    // Gates is not installed here, so the walk ends where `elo join` ends for
    // anyone without the game — which proves the link reached it.
    let (code, out, err) =
        elo(dir.path(), &["friends", "join", "ash", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.contains("joining gates on eu-1.example.test:61234"), "{out}{err}");
    assert!(err.contains("elo install gates"), "{err}");

    // In a game with no address known, and not in a game, both say so.
    let o = serve(Box::new(|_, _, _| Some((200, list(true, "Ash", Value::Null).to_string()))));
    let (code, _, err) =
        elo(dir.path(), &["friends", "join", ASH, "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("no join link is known"), "{err}");
    let o = serve(Box::new(|_, _, _| {
        let mut v = list(true, "Ash", Value::Null);
        v["friends"][0]["playing"] = Value::Null;
        Some((200, v.to_string()))
    }));
    let (code, _, err) =
        elo(dir.path(), &["friends", "join", "Ash", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("Ash is not in a game right now"), "{err}");
}

/// Gates as `elo install` leaves it, without the download: a receipt and an
/// executable that the launch template hands `{wallet}` to.
fn fake_install(games: &std::path::Path) {
    let build = games.join("gates").join("0.1.0");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join("gates"), "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(build.join("gates"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    std::fs::write(build.join(".elo-install.json"), json!({
        "slug": "gates", "build": "0.1.0", "platform": "any",
        "files": [{"path": "gates", "sha256": "", "bytes": 10, "executable": true}],
        "launch": {"exec": "gates", "args": ["--server", "{server}", "--identity", "{wallet}"]},
    }).to_string()).unwrap();
}

/// `--wallet` on `friends` picks whose list is read. Joining from it must not
/// start the game as that wallet: the game asks this launcher who is playing.
#[test]
fn joining_from_someone_elses_list_does_not_play_as_them() {
    let o = serve(Box::new(|m, p, _| match (m, p) {
        ("GET", "/api/launcher/manifest/gates") => Some((200, json!({
            "manifest_version": 1, "slug": "gates", "name": "Gates", "state": "in-build",
            "builds": [{"kind": "browser", "platform": "any", "url": null}],
        }).to_string())),
        ("GET", p) if p.starts_with("/api/friends/") => Some((
            200,
            list(true, "Ash", json!("elo://join/gates/eu-1.example.test:61234")).to_string(),
        )),
        _ => None,
    }));
    let dir = tempfile::tempdir().unwrap();
    fake_install(&dir.path().join("games"));

    let (code, out, err) = elo(
        dir.path(),
        &["friends", "join", "ash", "--wallet", BO, "--host", &o.base, "--dry-run"],
        &[],
    );
    assert_eq!(code, 0, "{out}{err}");
    assert!(o.seen.lock().unwrap().iter().any(|(_, p, _)| p == &format!("/api/friends/{BO}")),
            "the list read was Bo's");
    let argv = out.lines().last().unwrap_or("");
    assert!(argv.contains("--server eu-1.example.test:61234"), "{out}");
    assert!(!out.contains(BO), "Bo's list, not Bo's identity: {out}");

    // The same install does take a wallet typed on `elo join` — so the line
    // above is the friends door dropping it, not the template ignoring it.
    let (code, out, err) = elo(
        dir.path(),
        &["join", "elo://join/gates/eu-1.example.test:61234", "--wallet", ME,
          "--host", &o.base, "--dry-run"],
        &[],
    );
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains(&format!("--identity {ME}")), "{out}");
}
