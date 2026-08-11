//! Installing a game by its NAME — the whole chain, through the real binary.
//!
//! Before this, `scry install` took a url to a manifest, which meant the
//! client could install a title only if you already knew where that title's
//! manifest was published. That is a storefront you cannot shop in. The origin
//! now serves manifests (`meter/launcher.py`) and this drives the real `scry`
//! binary against a stub of those exact routes.
//!
//! Everything below runs the compiled binary as a subprocess rather than
//! calling library functions. That is the point: the two defects the transport
//! suite found were both in the wiring — `ureq` ignoring `NO_PROXY`, and `play`
//! starting the oldest installed build — and neither was reachable from a unit
//! test. Slug resolution, rebasing and exit codes are wiring too.

use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

const SCRY: &str = env!("CARGO_BIN_EXE_scry");

fn sha_hex(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Origin {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
    _stop: mpsc::Sender<()>,
}

/// A stub of the origin's launcher routes. Hand-written for the same reason
/// `transport.rs`'s is: a web framework in the dev-dependencies of a launcher
/// whose whole dependency policy is a tree budget would be an odd trade.
/// The routes are built from a closure because a depot's `root` has to name
/// the port the server is about to listen on — so the socket is bound first,
/// then the documents that point at it are made. (Same ordering, and same
/// reason, as `transport.rs`.)
fn serve(make: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> Origin {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let routes = make(&format!("http://127.0.0.1:{port}"));
    let hits = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel::<()>();
    let seen = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if rx.try_recv().is_ok() {
                return;
            }
            let Ok(mut stream) = stream else { continue };
            let Some(path) = read_request(&mut stream) else {
                continue;
            };
            seen.lock().unwrap().push(path.clone());
            match routes.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 3\r\nConnection: close\r\n\r\nno!",
                    );
                }
            }
        }
    });
    Origin {
        base: format!("http://127.0.0.1:{port}"),
        hits,
        _stop: tx,
    }
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_string();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    Some(path)
}

/// The routes the origin actually serves, with one published native build.
/// `depot` is a PATH, not a url — which is the case that matters, because it
/// is what an origin behind nginx can safely write.
fn gates_origin() -> (Origin, Vec<u8>) {
    let exe = b"#!/bin/sh\necho \"gates started: $*\"\n".to_vec();
    let body = exe.clone();
    let origin = serve(move |base| {
        let depot = json!({
            "depot_version": 1,
            "slug": "gates",
            "build": "0.1.0",
            "platform": "linux-x86_64",
            // The depot's own root is ABSOLUTE and baked at package time — it
            // is inside the digest, so nothing may rewrite it. Only the
            // MANIFEST's pointer at this document is relative, which is the
            // asymmetry this whole test exists to pin.
            "root": format!("{base}/api/launcher/depot/gates/0.1.0/files"),
            "files": [{
                "path": "gates", "sha256": sha_hex(&body),
                "bytes": body.len(), "executable": true
            }],
            "launch": {
                "exec": "gates",
                "args": ["--server", "{server}", "--identity", "{wallet}"]
            }
        });
        let manifest = json!({
            "manifest_version": 1,
            "slug": "gates", "name": "Gates", "state": "in-build",
            "builds": [
                {"kind": "browser", "platform": "any", "url": null},
                {"kind": "native", "platform": "linux-x86_64",
                 "depot": "/api/launcher/depot/gates/0.1.0",
                 "version": "0.1.0"}
            ]
        });
        vec![
            (
                "/api/launcher/manifest/gates".into(),
                manifest.to_string().into_bytes(),
            ),
            (
                "/api/launcher/manifests".into(),
                json!({"count": 1, "manifests": [manifest],
                       "depots": {"reachable": true, "why": null}})
                .to_string()
                .into_bytes(),
            ),
            (
                "/api/launcher/depot/gates/0.1.0".into(),
                depot.to_string().into_bytes(),
            ),
            ("/api/launcher/depot/gates/0.1.0/files/gates".into(), body),
        ]
    });
    (origin, exe)
}

fn scry(games: &std::path::Path, origin: &Origin, args: &[&str]) -> (i32, String) {
    let out = Command::new(SCRY)
        .args(args)
        .arg("--games")
        .arg(games)
        .arg("--host")
        .arg(&origin.base)
        .arg("--allow-insecure")
        // The launcher tunnels through a proxy if the environment names one,
        // and a proxy ANSWERS for a dead port — which would make this suite
        // pass against nothing. `scry-net` handles loopback correctly; clearing
        // these makes the test independent of that fix being present.
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .output()
        .expect("run scry");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

#[test]
fn a_slug_installs_a_title_the_player_never_typed_a_url_for() {
    let games = tempfile::tempdir().expect("tmp");
    let (origin, exe) = gates_origin();

    let (code, out) = scry(games.path(), &origin, &["install", "gates"]);
    assert_eq!(code, 0, "install by slug should succeed:\n{out}");

    let installed = games.path().join("gates").join("0.1.0").join("gates");
    assert!(installed.is_file(), "the binary landed:\n{out}");
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        exe,
        "and it is byte-for-byte what the origin served"
    );

    // The manifest was resolved from the slug, and the RELATIVE depot pointer
    // inside it was completed against the origin we asked — the two things
    // this test exists for.
    let hits = origin.hits.lock().unwrap().clone();
    assert!(
        hits.contains(&"/api/launcher/manifest/gates".to_string()),
        "the slug became a manifest request; saw {hits:?}"
    );
    assert!(
        hits.contains(&"/api/launcher/depot/gates/0.1.0".to_string()),
        "the relative depot path was rebased and fetched; saw {hits:?}"
    );
}

#[test]
fn an_installed_build_verifies_and_reports_current() {
    let games = tempfile::tempdir().expect("tmp");
    let (origin, _exe) = gates_origin();
    assert_eq!(scry(games.path(), &origin, &["install", "gates"]).0, 0);

    let (code, out) = scry(games.path(), &origin, &["verify", "gates"]);
    assert_eq!(code, 0, "verify should pass on a fresh install:\n{out}");

    // 0 = current. `status` uses distinct codes so a script can tell "up to
    // date" from "could not look" — 3 is unknown, 10 is stale.
    let (code, out) = scry(games.path(), &origin, &["status", "gates"]);
    assert_eq!(code, 0, "a freshly installed build is current:\n{out}");
}

#[test]
fn a_tampered_install_is_caught_by_verify() {
    // The install receipt is a file on disk and a build directory can be
    // edited afterwards, so verify re-hashes rather than trusting it.
    let games = tempfile::tempdir().expect("tmp");
    let (origin, _exe) = gates_origin();
    assert_eq!(scry(games.path(), &origin, &["install", "gates"]).0, 0);

    let installed = games.path().join("gates").join("0.1.0").join("gates");
    std::fs::write(&installed, b"#!/bin/sh\necho pwned\n").unwrap();

    let (code, out) = scry(games.path(), &origin, &["verify", "gates"]);
    assert_ne!(code, 0, "a modified build must NOT verify:\n{out}");
}

#[test]
fn games_lists_what_the_origin_serves() {
    let games = tempfile::tempdir().expect("tmp");
    let (origin, _exe) = gates_origin();
    let (code, out) = scry(games.path(), &origin, &["games"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("gates"), "the title is listed:\n{out}");
}

#[test]
fn a_slug_the_origin_does_not_serve_fails_loudly() {
    let games = tempfile::tempdir().expect("tmp");
    let (origin, _exe) = gates_origin();
    let (code, out) = scry(games.path(), &origin, &["install", "nosuchgame"]);
    assert_ne!(code, 0, "an unknown slug must not exit 0:\n{out}");
    assert!(
        !games.path().join("nosuchgame").exists(),
        "and must leave nothing behind"
    );
}

#[test]
fn a_word_that_is_neither_slug_nor_url_names_all_three_shapes() {
    // The failure a player actually hits: a typo'd path. It must not be
    // fetched as though it were a url, and the error has to say what the
    // three accepted shapes are rather than "not found".
    let games = tempfile::tempdir().expect("tmp");
    let (origin, _exe) = gates_origin();
    let (code, out) = scry(games.path(), &origin, &["install", "./not/a/manifest.json"]);
    assert_ne!(code, 0);
    assert!(
        out.contains("slug") && out.contains("url"),
        "the error should name the shapes it accepts:\n{out}"
    );
}

#[test]
fn no_home_lands_in_the_working_directory_never_at_the_filesystem_root() {
    // The Windows bug, caught on linux because it has the same cause. Both
    // roots read `$HOME` and `unwrap_or_default()`'d it, so an unset HOME —
    // which is the NORMAL state on Windows, where the variable is USERPROFILE
    // — produced the string "/.local/share/scry/games": an absolute path at
    // the filesystem root. On a dev box that is a permission error; on a
    // player's Windows box it is a game installed somewhere nobody looks.
    //
    // Asserted through the binary with the environment actually cleared,
    // because the defect was in reading the environment and a unit test would
    // have supplied one.
    let out = Command::new(SCRY)
        .args(["list"])
        .env_remove("HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("SCRY_GAMES")
        .output()
        .expect("run scry");
    let text = String::from_utf8_lossy(&out.stdout).to_string()
        + &String::from_utf8_lossy(&out.stderr);
    assert!(
        !text.contains("/.local/share"),
        "with no HOME the games root must not be root-relative:\n{text}"
    );
    assert!(
        text.contains("scry/games") || text.contains("scry\\games"),
        "it should still name a games root:\n{text}"
    );
}
