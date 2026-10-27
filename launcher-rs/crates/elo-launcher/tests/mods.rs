//! `elo mods`, through the real binary, against a stub of the origin.
//!
//! The routes are the contract `meter/workshop.py` serves: `GET
//! /api/workshop/{game}/subscribed/{wallet}`, `POST /api/workshop/prepare`,
//! `POST /api/workshop/act`. Downloads are `elo_depot::workshop`'s and are
//! tested there with a fake fetcher, since a file url must be a github.com
//! release; here a sync is driven with lists that need no download, which is
//! where its removals and its refusal to act on a failed read live.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};

const ELO: &str = env!("CARGO_BIN_EXE_elo");
const ME: &str = "0x00000000000000000000000000000000000000aa";
const AT: u64 = 1_760_000_000;
const DAY: &str = "2025-10-09";

type Seen = Arc<Mutex<Vec<(String, String, String)>>>;
type Route = Box<dyn Fn(&str, &str, &str) -> Option<(u16, String)> + Send>;

struct Origin {
    base: String,
    seen: Seen,
    _stop: mpsc::Sender<()>,
}

/// The hand-written HTTP/1.1 stub `friends.rs` uses, for its reason.
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

fn elo(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut c = Command::new(ELO);
    c.args(args)
        .env("ELO_KEYSTORE", dir.join("keystore.json"))
        .env("ELO_PAIRING", dir.join("pairing.json"))
        .env("ELO_STATE", dir.join("state"))
        .env("ELO_GAMES", dir.join("games"))
        .env("ELO_MODS", dir.join("mods"))
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

/// A subscribed list as the origin answers it.
fn subscribed(mods: Value) -> Value {
    json!({
        "game": "gates", "wallet": ME, "open": true, "runs_code": false,
        "count": mods.as_array().map(Vec::len).unwrap_or(0), "mods": mods,
        "withheld": [{"mod": "old-run", "why": "taken down"}],
        "check": "download each file from its url and check its sha256",
        "takedowns": {"reachable": true, "why": null},
    })
}

fn desert() -> Value {
    json!({"id": "desert-run", "name": "Desert Run", "kind": "map", "version": "1.2.0",
           "requires": [], "author": {"owner": "ash", "repo": "desert-run",
                                      "url": "https://github.com/ash/desert-run"},
           "license": "MIT", "url": "/api/workshop/gates/desert-run", "subscribed_at": AT,
           "files": [{"name": "map.pak",
                      "url": "https://github.com/ash/desert-run/releases/download/v1.2.0/map.pak",
                      "sha256": "ab".repeat(32), "size": 2048}]})
}

/// The text `meter/workshop.py` offers: `playauth.play_message("workshop",
/// …)` over `detail_of(op, game, mod, at)`. serde_json's map is sorted and
/// `to_string` compact, which is the meter's `json.dumps`.
fn meter_text(b: &Value) -> String {
    let detail = json!({"at": AT, "game": b["game"], "mod": b["mod"], "op": b["op"]}).to_string();
    format!("elo play\naction: workshop\nwallet: {}\nday: {DAY}\ndetail: {detail}",
            b["wallet"].as_str().unwrap_or(""))
}

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
fn the_list_says_what_is_subscribed_and_what_is_held_back() {
    let o = serve(Box::new(|m, p, _| {
        (m == "GET" && p == format!("/api/workshop/gates/subscribed/{ME}"))
            .then(|| (200, subscribed(json!([desert()])).to_string()))
    }));
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = elo(dir.path(), &["mods", "gates", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("1 mod") && out.contains("desert-run"), "{out}");
    assert!(out.contains("github.com/ash/desert-run"), "{out}");
    assert!(out.contains("held back") && out.contains("taken down"), "{out}");
    assert!(out.contains("elo mods sync gates"), "{out}");

    let (code, out, err) =
        elo(dir.path(), &["mods", "gates", "--host", &o.base, "--json"], &[("ELO_WALLET", ME)]);
    assert_eq!(code, 0, "{out}{err}");
    let v: Value = serde_json::from_str(&out).expect("--json is JSON");
    assert_eq!(v["count"], 1);
    assert_eq!(v["mods"][0]["files"][0]["sha256"], "ab".repeat(32));
    assert_eq!(v["withheld"][0]["mod"], "old-run");
}

#[test]
fn a_failed_read_changes_nothing_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join("mods/gates/old-run");
    std::fs::create_dir_all(&stale).unwrap();
    let (code, out, err) = elo(dir.path(),
        &["mods", "sync", "gates", "--wallet", ME, "--host", "http://127.0.0.1:9"], &[]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(err.contains("could not reach"), "{err}");
    assert!(stale.is_dir(), "an unread list is not an empty one");

    // A 404 in the origin's words, and a document that is not a list.
    let o = serve(Box::new(|_, p, _| {
        if p.contains("/nope/") {
            Some((404, json!({"error": "'nope' is not a listed game"}).to_string()))
        } else {
            Some((200, json!({"game": "gates"}).to_string()))
        }
    }));
    let (code, _, err) = elo(dir.path(), &["mods", "nope", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("'nope' is not a listed game"), "{err}");
    let (code, _, err) =
        elo(dir.path(), &["mods", "sync", "gates", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("did not read"), "{err}");
    assert!(stale.is_dir());

    let (code, _, err) = elo(dir.path(), &["mods", "sync", "../etc", "--wallet", ME], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("not a game slug"), "{err}");
    let (code, _, err) = elo(dir.path(), &["mods", "gates", "--host", "http://127.0.0.1:9"], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("whose mods?"), "{err}");
}

#[test]
fn a_sync_removes_what_was_dropped_and_writes_the_index() {
    let o = serve(Box::new(|m, _, _| (m == "GET").then(|| (200, subscribed(json!([])).to_string()))));
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("mods/gates");
    std::fs::create_dir_all(game.join("old-run")).unwrap();
    std::fs::write(game.join("old-run/x.pak"), b"x").unwrap();

    let (code, out, err) = elo(dir.path(),
        &["mods", "sync", "gates", "--wallet", ME, "--host", &o.base, "--dry-run"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("would remove old-run"), "{out}");
    assert!(game.join("old-run/x.pak").exists(), "a dry run removes nothing");

    let (code, out, err) =
        elo(dir.path(), &["mods", "sync", "gates", "--wallet", ME, "--host", &o.base], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("removed old-run"), "{out}");
    assert!(out.contains(&game.display().to_string()), "it says where a game loads them: {out}");
    assert!(!game.join("old-run").exists());
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(game.join("mods.json")).unwrap()).unwrap();
    assert_eq!(index["game"], "gates");
    assert_eq!(index["mods"], json!([]));
}

#[test]
fn subscribe_is_prepared_signed_and_submitted_as_the_signer() {
    let o = serve(Box::new(|m, p, body| {
        let b: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        match (m, p) {
            // An origin that turns an unsubscribe into a subscribe is refused.
            ("POST", "/api/workshop/prepare") if b["op"] == "unsubscribe" => {
                let mut other = b.clone();
                other["op"] = json!("subscribe");
                Some((200, json!({"sign_this": meter_text(&other), "at": AT}).to_string()))
            }
            ("POST", "/api/workshop/prepare") if b["mod"] == "closed" => Some((409, json!({
                "error": "'gates' has closed its workshop\x1b[2J"
            }).to_string())),
            ("POST", "/api/workshop/prepare") => Some((200, json!({
                "sign_this": meter_text(&b), "at": AT, "op": b["op"], "game": b["game"],
                "mod": b["mod"],
            }).to_string())),
            ("POST", "/api/signer/ABCD2345/ask") => Some((200, r#"{"id":"q1"}"#.into())),
            ("GET", p) if p.starts_with("/api/signer/ABCD2345/ask/q1") => {
                Some((200, json!({"state": "signed", "signature": "0xfeed"}).to_string()))
            }
            ("POST", "/api/workshop/act") => Some((200, json!({
                "ok": true, "op": "subscribe", "game": "gates", "mod": "desert-run",
                "subscribed": ["desert-run"],
            }).to_string())),
            _ => None,
        }
    }));
    let dir = tempfile::tempdir().unwrap();
    pair(dir.path(), &o.base);
    let (code, out, err) =
        elo(dir.path(), &["mods", "subscribe", "gates", "desert-run", "--host", &o.base], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("subscribed to desert-run in gates"), "{out}");

    let seen = o.seen.lock().unwrap().clone();
    let body = |path: &str| -> Value {
        let row = seen.iter().find(|(m, p, _)| m == "POST" && p == path).expect(path);
        serde_json::from_str(&row.2).unwrap()
    };
    assert_eq!(body("/api/workshop/prepare"),
               json!({"wallet": ME, "op": "subscribe", "game": "gates", "mod": "desert-run"}));
    assert_eq!(body("/api/signer/ABCD2345/ask")["text"], format!(
        "elo play\naction: workshop\nwallet: {ME}\nday: {DAY}\n\
         detail: {{\"at\":{AT},\"game\":\"gates\",\"mod\":\"desert-run\",\"op\":\"subscribe\"}}"
    ));
    assert_eq!(body("/api/workshop/act"),
               json!({"wallet": ME, "op": "subscribe", "game": "gates", "mod": "desert-run",
                      "at": AT, "signature": "0xfeed"}));

    // Refused before the signer is asked: another change, and the origin's no.
    let before = o.seen.lock().unwrap().len();
    let (code, _, err) =
        elo(dir.path(), &["mods", "unsubscribe", "gates", "desert-run", "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("nothing was signed"), "{err}");
    let (code, _, err) =
        elo(dir.path(), &["mods", "subscribe", "gates", "closed", "--host", &o.base], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("closed its workshop") && !err.contains('\x1b'), "{err:?}");
    let (code, out, _) = elo(dir.path(),
        &["mods", "subscribe", "gates", "desert-run", "--dry-run", "--host", &o.base], &[]);
    assert_eq!(code, 0);
    assert!(out.contains("would sign") && out.contains("nothing was sent"), "{out}");
    let after: Vec<_> = o.seen.lock().unwrap()[before..].iter()
        .filter(|(_, p, _)| p.contains("/api/signer/") || p == "/api/workshop/act")
        .cloned()
        .collect();
    assert!(after.is_empty(), "{after:?}");
}
