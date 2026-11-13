//! `elo report-crash`, through the real binary: what it asks, and the one
//! POST it makes when (and only when) the player says yes.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const ELO: &str = env!("CARGO_BIN_EXE_elo");

/// A stub origin that records every request's path and JSON body and answers
/// `/api/bugs` the way the meter does.
fn origin() -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).is_err() {
                continue;
            }
            let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            let _ = reader.read_exact(&mut body);
            log.lock().unwrap().push((path, serde_json::from_slice(&body).unwrap_or(Value::Null)));
            let text = json!({"ok": true, "id": "20261102-120000-abcdef12", "seen": 1,
                              "shot": false, "dump": true,
                              "dump_contains": "memory of the game process",
                              "dump_expires": "2026-12-02T12:00:00Z"}).to_string();
            let _ = write!(stream, "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\n\
                                   Content-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
        }
    });
    (base, seen)
}

/// An installed build of `gates` with a dump in its crash folder.
fn installed_with_dump(games: &std::path::Path, name: &str, body: &[u8]) -> std::path::PathBuf {
    let build = games.join("gates").join("0.5.0-abcdef1");
    std::fs::create_dir_all(build.join("crashes")).unwrap();
    std::fs::write(build.join("gates"), b"#!/bin/sh\n").unwrap();
    let receipt = json!({"slug": "gates", "build": "0.5.0-abcdef1", "platform": "linux-x86_64",
                         "files": [{"path": "gates", "sha256": "00", "bytes": 10, "executable": true}],
                         "launch": {"exec": "gates", "args": []}});
    std::fs::write(build.join(".elo-install.json"), receipt.to_string()).unwrap();
    let dump = build.join("crashes").join(name);
    std::fs::write(&dump, body).unwrap();
    dump
}

fn elo(games: &std::path::Path, state: &std::path::Path, host: &str, args: &[&str], stdin: &str)
    -> (i32, String) {
    let mut child = Command::new(ELO)
        .args(args)
        .arg("--games").arg(games)
        .arg("--host").arg(host)
        .env("ELO_STATE", state)
        .env_remove("HTTP_PROXY").env_remove("HTTPS_PROXY").env_remove("ALL_PROXY")
        .env_remove("http_proxy").env_remove("https_proxy")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

#[test]
fn it_says_what_the_dump_holds_and_sends_nothing_without_a_yes() {
    let (games, state) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dump = installed_with_dump(games.path(), "1.dmp", b"MDMP\x93\xa7rest");
    let (host, seen) = origin();
    let (code, out) = elo(games.path(), state.path(), &host, &["report-crash", "gates"], "\n");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("memory of the game process") && out.contains("we keep it 30 days"), "{out}");
    assert!(out.contains("not sent"), "{out}");
    assert!(seen.lock().unwrap().is_empty(), "Enter is a no, and a no sends nothing");
    assert!(dump.is_file());
    let (_, out) = elo(games.path(), state.path(), &host, &["report-crash", "gates", "--dry-run"], "");
    assert!(out.contains("nothing was sent") && seen.lock().unwrap().is_empty(), "{out}");
}

#[test]
fn a_yes_sends_the_dump_with_its_report_once() {
    let (games, state) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dump = installed_with_dump(games.path(), "panic.txt", b"thread 'main' panicked at a.rs:1\n");
    let (host, seen) = origin();
    let (code, out) = elo(games.path(), state.path(), &host,
                          &["report-crash", "gates", "--body", "fell through the floor"], "y\n");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("2026-12-02T12:00:00Z"), "the answer's expiry is shown: {out}");
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    let (path, body) = &got[0];
    assert_eq!(path, "/api/bugs");
    assert_eq!(body["game"], "gates");
    assert_eq!(body["dump"]["format"], "backtrace");
    assert_eq!(body["dump"]["consent"], true);
    assert_eq!(body["report"]["kind"], "crash");
    assert_eq!(body["report"]["body"], "fell through the floor");
    assert!(!dump.exists() && dump.with_file_name("panic.txt.sent").is_file());
    let (code, out) = elo(games.path(), state.path(), &host, &["report-crash", "gates", "--yes"], "");
    assert_ne!(code, 0, "a sent dump is not offered again: {out}");
    assert!(out.contains("no crash dump"), "{out}");
    assert_eq!(seen.lock().unwrap().len(), 1);
}
