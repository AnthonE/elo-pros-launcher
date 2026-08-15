//! The client replacing the client, through the real binary.
//!
//! Everything here runs the compiled `scry` as a subprocess against a stub of
//! the origin's `/api/launcher` card — the same discipline as `origin.rs`, and
//! for the same reason: the risk in a self-updater is the wiring (what gets
//! fetched, what gets checked, what lands on disk and in what order), and none
//! of that is reachable from a unit test.
//!
//! The install being replaced is a scratch directory named through
//! `SCRY_SELF_DIR`, which is the one seam `selfup.rs` offers tests: it grants
//! nothing file permissions did not already grant, and without it this suite
//! would be overwriting the binary cargo is currently executing.

use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

const SCRY: &str = env!("CARGO_BIN_EXE_scry");
/// What the stub publishes. Any real workspace version sorts below it.
const NEWER: &str = "9.9.9";

fn sha_hex(body: &[u8]) -> String {
    Sha256::digest(body).iter().map(|b| format!("{b:02x}")).collect()
}

// ── a real artifact, built the way build_release.py builds one ───────────────

/// One tar member: 512-byte ustar header, octal size, data padded to 512.
fn tar_entry(name: &str, data: &[u8], typeflag: u8) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..107].copy_from_slice(b"0000755");
    h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
    h[156] = typeflag;
    h[257..262].copy_from_slice(b"ustar");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    h[148..154].copy_from_slice(format!("{sum:06o}").as_bytes());
    h[154] = 0;
    h[155] = b' ';
    let mut out = h;
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(512) * 512, 0);
    out
}

/// `scry-<v>-linux-x86_64.tar.gz` with the two binaries, as gzip'd tar.
/// The trailer's CRC is zeros: the client pins the artifact by sha256 against
/// the card and deliberately does not re-check container checksums.
fn artifact(cli: &[u8], gui: &[u8]) -> Vec<u8> {
    let root = format!("scry-{NEWER}-linux-x86_64");
    let mut tar = Vec::new();
    tar.extend(tar_entry(&format!("./{root}/"), b"", b'5'));
    tar.extend(tar_entry(&format!("./{root}/scry"), cli, b'0'));
    tar.extend(tar_entry(&format!("./{root}/scry-gui"), gui, b'0'));
    tar.extend(tar_entry(&format!("./{root}/COPYRIGHT"), b"MIT\n", b'0'));
    tar.extend_from_slice(&[0u8; 1024]);
    let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    gz.extend(miniz_oxide::deflate::compress_to_vec(&tar, 6));
    gz.extend_from_slice(&[0u8; 8]);
    gz
}

const NEW_CLI: &[u8] = b"#!/bin/sh\necho 9.9.9 cli\nexit 0\n";
const NEW_GUI: &[u8] = b"#!/bin/sh\necho 9.9.9 gui\nexit 0\n";
const OLD_CLI: &[u8] = b"#!/bin/sh\necho old cli\n";
const OLD_GUI: &[u8] = b"#!/bin/sh\necho old gui\n";

// ── the stub origin (the `origin.rs` harness, unchanged) ─────────────────────

struct Origin {
    base: String,
    _stop: mpsc::Sender<()>,
}

fn serve(make: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> Origin {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let routes = make(&format!("http://127.0.0.1:{port}"));
    let hits = Arc::new(Mutex::new(Vec::<String>::new()));
    let (tx, rx) = mpsc::channel::<()>();
    let seen = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if rx.try_recv().is_ok() {
                return;
            }
            let Ok(mut stream) = stream else { continue };
            let Some(path) = read_request(&mut stream) else { continue };
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

/// A card + artifact origin. `sha` lets a test publish a LIE about the hash.
fn update_origin(version: &str, sha: Option<String>) -> (Origin, Vec<u8>) {
    let body = artifact(NEW_CLI, NEW_GUI);
    let name = format!("scry-{NEWER}-linux-x86_64.tar.gz");
    let hash = sha.unwrap_or_else(|| sha_hex(&body));
    let served = body.clone();
    let version = version.to_string();
    let origin = serve(move |_base| {
        let card = json!({
            "client": { "version": version },
            "get_it": [
                { "name": name, "path": format!("/dl/{name}"),
                  "sha256": hash, "bytes": served.len() },
                { "name": format!("scry_{NEWER}_amd64.deb"),
                  "path": format!("/dl/scry_{NEWER}_amd64.deb"),
                  "sha256": "ab".repeat(32) },
            ],
        });
        vec![
            ("/api/launcher".into(), card.to_string().into_bytes()),
            (format!("/dl/{name}"), served.clone()),
        ]
    });
    (origin, body)
}

/// A scratch "install": the two binaries as executable scripts.
fn fake_install(gui_too: bool) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tmp");
    for (name, bytes, there) in [("scry", OLD_CLI, true), ("scry-gui", OLD_GUI, gui_too)] {
        if !there {
            continue;
        }
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

fn self_update(install: &Path, host: &str, extra: &[&str]) -> (i32, String) {
    let out = Command::new(SCRY)
        .arg("self-update")
        .args(extra)
        .arg("--host")
        .arg(host)
        .arg("--allow-insecure")
        .env("SCRY_SELF_DIR", install)
        // Same reason as origin.rs: a proxy ANSWERS for a dead port, which
        // would defeat the could-not-look assertions below.
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

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

// ── the proofs ───────────────────────────────────────────────────────────────

#[test]
fn a_published_newer_client_replaces_both_binaries_and_keeps_the_old_pair() {
    let install = fake_install(true);
    let (origin, _body) = update_origin(NEWER, None);

    let (code, out) = self_update(install.path(), &origin.base, &["--yes"]);
    assert_eq!(code, 0, "{out}");

    assert_eq!(std::fs::read(install.path().join("scry")).unwrap(), NEW_CLI);
    assert_eq!(std::fs::read(install.path().join("scry-gui")).unwrap(), NEW_GUI);
    // The rollback is real files, not a promise.
    assert_eq!(std::fs::read(install.path().join("scry.old")).unwrap(), OLD_CLI);
    assert_eq!(std::fs::read(install.path().join("scry-gui.old")).unwrap(), OLD_GUI);
    // And nothing staged is left behind.
    assert!(!install.path().join("scry.new").exists(), "{out}");
    // The new binary is executable — the whole point of the exercise.
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(install.path().join("scry")).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "not executable: {mode:o}");
    assert!(out.contains("replaced"), "{out}");
}

#[test]
fn a_tampered_artifact_is_refused_and_the_install_is_untouched() {
    let install = fake_install(true);
    // The card publishes a hash the artifact does not have — a lying mirror,
    // or bytes edited in flight.
    let (origin, _body) = update_origin(NEWER, Some("ee".repeat(32)));

    let (code, out) = self_update(install.path(), &origin.base, &["--yes"]);
    assert_ne!(code, 0, "a bad hash must not exit 0:\n{out}");
    assert!(out.contains("does not match"), "{out}");

    assert_eq!(std::fs::read(install.path().join("scry")).unwrap(), OLD_CLI);
    assert_eq!(
        files_in(install.path()),
        vec!["scry", "scry-gui"],
        "no .new, no .old, no litter:\n{out}"
    );
}

#[test]
fn could_not_look_exits_3_and_never_says_current() {
    let install = fake_install(true);
    // A port nobody is listening on.
    let (code, out) = self_update(install.path(), "http://127.0.0.1:9", &[]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("could not"), "{out}");
    assert!(!out.to_lowercase().contains("current"), "{out}");
    assert_eq!(std::fs::read(install.path().join("scry")).unwrap(), OLD_CLI);
}

#[test]
fn the_same_version_is_current_and_nothing_moves() {
    let install = fake_install(true);
    // The binary under test carries the workspace version.
    let (origin, _body) = update_origin(env!("CARGO_PKG_VERSION"), None);
    let (code, out) = self_update(install.path(), &origin.base, &["--yes"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("current"), "{out}");
    assert_eq!(files_in(install.path()), vec!["scry", "scry-gui"]);
}

#[test]
fn a_card_with_no_artifact_for_this_platform_says_so_by_name() {
    let install = fake_install(true);
    let origin = serve(|_| {
        vec![(
            "/api/launcher".into(),
            json!({
                "client": { "version": NEWER },
                "get_it": [ { "name": format!("scry-{NEWER}-windows-x86_64.zip"),
                              "path": "/dl/x.zip", "sha256": "aa".repeat(32) } ],
            })
            .to_string()
            .into_bytes(),
        )]
    });
    let (code, out) = self_update(install.path(), &origin.base, &["--yes"]);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("linux-x86_64") && out.contains("windows"),
        "the refusal names both sides:\n{out}"
    );
}

#[test]
fn a_row_without_a_hash_is_refused_before_a_byte_is_fetched() {
    let install = fake_install(true);
    let origin = serve(|_| {
        vec![(
            "/api/launcher".into(),
            json!({
                "client": { "version": NEWER },
                "get_it": [ { "name": format!("scry-{NEWER}-linux-x86_64.tar.gz"),
                              "path": "/dl/a.tar.gz" } ],
            })
            .to_string()
            .into_bytes(),
        )]
    });
    let (code, out) = self_update(install.path(), &origin.base, &["--yes"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("cannot be checked"), "{out}");
    assert_eq!(files_in(install.path()), vec!["scry", "scry-gui"]);
}

#[test]
fn dry_run_states_the_plan_and_writes_nothing() {
    let install = fake_install(true);
    let (origin, _body) = update_origin(NEWER, None);
    let (code, out) = self_update(install.path(), &origin.base, &["--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(NEWER) && out.contains("artifact"), "{out}");
    assert_eq!(files_in(install.path()), vec!["scry", "scry-gui"]);
}

#[test]
fn no_answer_at_the_prompt_keeps_what_is_installed() {
    // `Command::output` closes the child's stdin, so the y/N prompt reads
    // EOF — which must read as "no", the same default every destroyer here
    // takes. Nothing downloaded, nothing changed, exit 0: a chosen no is
    // not a failure.
    let install = fake_install(true);
    let (origin, _body) = update_origin(NEWER, None);
    let (code, out) = self_update(install.path(), &origin.base, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("kept"), "{out}");
    assert_eq!(std::fs::read(install.path().join("scry")).unwrap(), OLD_CLI);
}

#[test]
fn a_new_binary_that_fails_its_smoke_run_is_refused_without_litter() {
    // The hash can prove the bytes are the published bytes and still not
    // prove they RUN here. A staged binary that fails `help` must leave the
    // old pair in place and clean its own staging up.
    let install = fake_install(true);
    let body = artifact(b"#!/bin/sh\nexit 9\n", NEW_GUI);
    let name = format!("scry-{NEWER}-linux-x86_64.tar.gz");
    let hash = sha_hex(&body);
    let served = body.clone();
    let origin = serve(move |_| {
        vec![
            (
                "/api/launcher".into(),
                json!({
                    "client": { "version": NEWER },
                    "get_it": [ { "name": name, "path": format!("/dl/{name}"),
                                  "sha256": hash } ],
                })
                .to_string()
                .into_bytes(),
            ),
            (format!("/dl/{name}"), served.clone()),
        ]
    });

    let (code, out) = self_update(install.path(), &origin.base, &["--yes"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("smoke"), "{out}");
    assert_eq!(std::fs::read(install.path().join("scry")).unwrap(), OLD_CLI);
    assert_eq!(
        files_in(install.path()),
        vec!["scry", "scry-gui"],
        "no .new litter after the refusal:\n{out}"
    );
}

#[test]
fn an_install_without_the_window_does_not_gain_one() {
    // A CLI-only install (a server, an agent's harness) updates the CLI and
    // must NOT install scry-gui out of the same artifact: an updater that
    // adds programs has changed jobs.
    let install = fake_install(false);
    let (origin, _body) = update_origin(NEWER, None);
    let (code, out) = self_update(install.path(), &origin.base, &["--yes"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(std::fs::read(install.path().join("scry")).unwrap(), NEW_CLI);
    assert_eq!(
        files_in(install.path()),
        vec!["scry", "scry.old"],
        "no scry-gui appeared:\n{out}"
    );
}

#[test]
fn json_reports_the_update_machine_readably() {
    let install = fake_install(true);
    let (origin, _body) = update_origin(NEWER, None);
    let (code, out) = self_update(install.path(), &origin.base, &["--yes", "--json"]);
    assert_eq!(code, 0, "{out}");
    let line = out
        .lines()
        .skip_while(|l| !l.trim_start().starts_with('{'))
        .collect::<Vec<_>>()
        .join("\n");
    let v: serde_json::Value = serde_json::from_str(&line).expect("json out");
    assert_eq!(v["state"], "updated");
    assert_eq!(v["to"], NEWER);
}
