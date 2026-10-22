//! Which build `elo play` starts, and which manifests `elo` will trust — through
//! the real binary, because both rules are wiring.

use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::path::Path;
use std::process::Command;

const ELO: &str = env!("CARGO_BIN_EXE_elo");

fn sha_hex(body: &[u8]) -> String {
    Sha256::digest(body).iter().map(|b| format!("{b:02x}")).collect()
}

fn elo(games: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(ELO)
        .args(args)
        .arg("--games")
        .arg(games)
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .output()
        .expect("run elo");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn put(games: &Path, build: &str) {
    let body: &[u8] = b"#!/bin/sh\n";
    let raw = json!({
        "depot_version": 1, "slug": "t", "build": build,
        "platform": "linux-x86_64", "root": "https://cdn.example/d",
        "files": [{"path": "run", "sha256": sha_hex(body), "bytes": body.len(), "executable": true}],
        "launch": {"exec": "run", "args": [], "cwd": ".", "env": {}}
    });
    let d = elo_depot::parse_depot(&raw, false).unwrap();
    let fetch = move |_u: &str, dest: &Path| -> Result<(), elo_depot::DepotError> {
        std::fs::write(dest, body).map_err(elo_depot::DepotError::from)
    };
    elo_depot::install(&d, games, &fetch, None, None).unwrap();
}

#[test]
fn play_starts_the_build_installed_last_not_the_one_named_last() {
    // l-depot#1: `rfind` over name order picked `zzz` whatever was installed
    // after it, so a build planted under a name that sorts last always won.
    let games = tempfile::tempdir().unwrap();
    put(games.path(), "zzz");
    put(games.path(), "aaa");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::open(games.path().join("t").join("zzz"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    let manifest = games.path().join("t.json");
    std::fs::write(
        &manifest,
        json!({"manifest_version": 1, "slug": "t", "name": "T", "builds": []}).to_string(),
    )
    .unwrap();
    let (code, out) = elo(games.path(), &["play", manifest.to_str().unwrap(), "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("/t/aaa/run"), "play must start the newest build:\n{out}");
}

#[test]
fn play_never_starts_a_build_another_origin_installed_under_the_same_name() {
    // l-depot#1, second half: whatever the install order, a build that came
    // from attacker.example is not the one a manifest from somewhere else
    // starts. The receipt is where the client wrote down where it came from.
    let games = tempfile::tempdir().unwrap();
    put(games.path(), "0.5.0");
    let receipt = games.path().join("t").join("0.5.0").join(elo_depot::install::RECEIPT_NAME);
    let mut r: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&receipt).unwrap()).unwrap();
    r["origin"] = json!("https://attacker.example");
    std::fs::write(&receipt, r.to_string()).unwrap();
    let manifest = games.path().join("t.json");
    std::fs::write(
        &manifest,
        json!({"manifest_version": 1, "slug": "t", "name": "T", "builds": []}).to_string(),
    )
    .unwrap();
    let (code, out) = elo(games.path(), &["play", manifest.to_str().unwrap(), "--dry-run"]);
    assert!(!out.contains("/t/0.5.0/run"), "another origin's build must not start:\n{out}");
    assert_ne!(code, 0, "{out}");
}

#[test]
fn an_http_manifest_is_refused_without_allow_insecure() {
    // l-depot#3: a manifest read over http names the depot url, so a MITM on
    // that hop picks the hashes. Port 9 is discard: the refusal must come
    // before any fetch.
    let games = tempfile::tempdir().unwrap();
    let (code, out) = elo(games.path(), &["status", "http://127.0.0.1:9/m.json"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("--allow-insecure"), "the refusal names the flag:\n{out}");
}
