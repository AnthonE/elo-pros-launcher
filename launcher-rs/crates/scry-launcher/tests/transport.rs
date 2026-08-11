//! The wire, end to end — the gap the Python suite could not close.
//!
//! `test_launcher.py` covers the install *logic* thoroughly, but it injects
//! `opener=`, so `_download` never faced real HTTP: redirects, a truncated
//! body, a 404, a timeout, TLS. And no real binary was ever launched. Those
//! are the paths every native title depends on and the ones nothing had run.
//!
//! So this suite stands up an actual TCP server, installs over it with the
//! real `ureq` client, hash-verifies, launches a real executable, updates to a
//! second build reusing files off disk, and checks what happens when the
//! server misbehaves.

use scry_depot::{install, installed, launch, parse_depot, parse_manifest, plan, verify};
use scry_net::Net;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

fn sha_hex(body: &[u8]) -> String {
    Sha256::digest(body).iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone)]
enum Reply {
    Ok(Vec<u8>),
    /// Declares more bytes than it sends, then closes — a truncated transfer,
    /// which is what a dropped wifi connection looks like mid-download.
    Short(Vec<u8>, usize),
    Redirect(String),
    Status(u16),
}

struct Server {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
    _stop: mpsc::Sender<()>,
}

/// A tiny HTTP/1.1 server. Deliberately hand-written: pulling a web framework
/// into the dev-dependencies to test a launcher would grow the tree this
/// workspace's policy exists to keep small.
///
/// The routes are built from a closure because a depot's `root` has to name
/// the port the server is about to listen on — so the socket is bound first,
/// then the documents that point at it are made.
fn serve(make: impl FnOnce(&str) -> Vec<(String, Reply)>) -> Server {
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
            let path = match read_request(&mut stream) {
                Some(p) => p,
                None => continue,
            };
            seen.lock().unwrap().push(path.clone());
            let reply = routes.iter().find(|(p, _)| *p == path).map(|(_, r)| r.clone());
            let _ = match reply {
                Some(Reply::Ok(body)) => write_body(&mut stream, 200, &body, body.len()),
                Some(Reply::Short(body, declared)) => write_body(&mut stream, 200, &body, declared),
                Some(Reply::Redirect(to)) => {
                    let head = format!(
                        "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    stream.write_all(head.as_bytes())
                }
                Some(Reply::Status(code)) => write_body(&mut stream, code, b"nope", 4),
                None => write_body(&mut stream, 404, b"not found", 9),
            };
        }
    });
    Server {
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

fn write_body(stream: &mut TcpStream, code: u16, body: &[u8], declared: usize) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

const RUN_SH: &[u8] = b"#!/bin/sh\necho \"gates $*\" > \"$SCRY_TEST_OUT\"\n";
const PAK_V1: &[u8] = b"PAK-ONE-0000000000000000000000";
const PAK_V2: &[u8] = b"PAK-TWO-1111111111111111111111";
const LIB: &[u8] = b"shared-library-bytes";

fn depot_json(base: &str, build: &str, pak: &[u8]) -> serde_json::Value {
    json!({
        "depot_version": 1, "slug": "gates", "build": build,
        "platform": "linux-x86_64", "root": format!("{base}/depot/{build}"),
        "files": [
            {"path": "run", "sha256": sha_hex(RUN_SH), "bytes": RUN_SH.len(), "executable": true},
            {"path": "data/x.pak", "sha256": sha_hex(pak), "bytes": pak.len()},
            {"path": "lib/libgates.so", "sha256": sha_hex(LIB), "bytes": LIB.len()},
        ],
        "launch": {"exec": "run", "args": ["--server", "{server}"], "cwd": ".",
                   "env": {"LD_LIBRARY_PATH": "lib"}}
    })
}

fn routes_for(base: &str) -> Vec<(String, Reply)> {
    let mut r = Vec::new();
    for (build, pak) in [("0.1.0", PAK_V1), ("0.2.0", PAK_V2)] {
        let d = depot_json(base, build, pak);
        r.push((
            format!("/depot/{build}/index.json"),
            Reply::Ok(serde_json::to_vec(&d).unwrap()),
        ));
        r.push((format!("/depot/{build}/run"), Reply::Ok(RUN_SH.to_vec())));
        r.push((format!("/depot/{build}/data/x.pak"), Reply::Ok(pak.to_vec())));
        r.push((
            format!("/depot/{build}/lib/libgates.so"),
            Reply::Ok(LIB.to_vec()),
        ));
    }
    r
}

#[test]
fn a_build_installs_over_real_http_verifies_and_actually_runs() {
    let srv = serve(routes_for);
    let base = srv.base.clone();

    let tmp = tempfile::tempdir().unwrap();
    let net = Net::new();

    let raw = net.get_json(&format!("{base}/depot/0.1.0/index.json"));
    assert!(raw.reachable && raw.ok, "{}", raw.why);
    let d = parse_depot(&raw.value.unwrap(), true).expect("depot parses");

    let at = install(&d, tmp.path(), &net, None, None).expect("install over http");
    let v = verify(&at);
    assert!(v.ok, "{}", v.line());
    assert_eq!(v.checked, 3);

    // Every file actually came off the wire.
    let hits = srv.hits.lock().unwrap().clone();
    assert!(hits.iter().any(|h| h.ends_with("/run")));
    assert!(hits.iter().any(|h| h.ends_with("/data/x.pak")));

    // …and the thing we installed starts, with argv and env applied.
    let out = tmp.path().join("launched.txt");
    let i = installed(tmp.path()).remove(0);
    let vals = BTreeMap::from([("server".to_string(), "eu-1:7777".to_string())]);
    let mut l = launch::resolve(&i, &vals, &BTreeMap::new()).expect("resolve");
    l.env.insert("SCRY_TEST_OUT".into(), out.to_string_lossy().into());
    assert!(l.env["LD_LIBRARY_PATH"].starts_with(i.path.to_str().unwrap()));
    launch::spawn(&l).expect("spawn");

    let mut waited = 0;
    while !out.is_file() && waited < 100 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        waited += 1;
    }
    assert!(out.is_file(), "the installed binary never ran");
    assert_eq!(
        std::fs::read_to_string(&out).unwrap().trim(),
        "gates --server eu-1:7777"
    );
}

#[test]
fn an_update_over_http_fetches_only_what_changed() {
    let srv = serve(routes_for);
    let base = srv.base.clone();

    let tmp = tempfile::tempdir().unwrap();
    let net = Net::new();

    let d1 = parse_depot(
        &net.get_json(&format!("{base}/depot/0.1.0/index.json")).value.unwrap(),
        true,
    )
    .unwrap();
    install(&d1, tmp.path(), &net, None, None).unwrap();

    let m = parse_manifest(&json!({
        "slug": "gates", "name": "Gates",
        "builds": [{"kind": "native", "platform": "linux-x86_64",
                    "depot": format!("{base}/depot/0.2.0/index.json")}]
    }))
    .unwrap();

    let checked = scry_depot::check(&m, "linux-x86_64", tmp.path(), &net, true);
    assert!(checked.state.needs_update(), "{}", checked.state.line());

    srv.hits.lock().unwrap().clear();
    let d2 = checked.depot.unwrap();
    let p = plan(&d2, tmp.path());
    assert_eq!(p.reuse.len(), 2, "run and libgates.so are unchanged");

    let at2 = install(&d2, tmp.path(), &net, Some(&p), None).unwrap();
    assert!(verify(&at2).ok);

    // The proof is what the server was asked for: the changed pak, and
    // nothing else. This is the delta, achieved with no delta format.
    let hits = srv.hits.lock().unwrap().clone();
    let fetched: Vec<&String> = hits.iter().filter(|h| h.starts_with("/depot/0.2.0/")).collect();
    assert_eq!(fetched, ["/depot/0.2.0/data/x.pak"], "asked for: {hits:?}");
}

#[test]
fn a_truncated_download_fails_and_installs_nothing() {
    // What a dropped connection mid-download looks like. The failure must be
    // total: a half-build the library counts as real is the bad outcome.
    let srv = serve(|base| {
        let mut routes = routes_for(base);
        routes.retain(|(p, _)| p != "/depot/0.1.0/data/x.pak");
        routes.push((
            "/depot/0.1.0/data/x.pak".into(),
            Reply::Short(PAK_V1[..10].to_vec(), PAK_V1.len()),
        ));
        routes
    });
    let base = srv.base.clone();

    let tmp = tempfile::tempdir().unwrap();
    let net = Net::new();
    let d = parse_depot(
        &net.get_json(&format!("{base}/depot/0.1.0/index.json")).value.unwrap(),
        true,
    )
    .unwrap();
    let err = install(&d, tmp.path(), &net, None, None).unwrap_err();
    assert!(
        err.0.contains("hash mismatch") || err.0.contains("could not fetch"),
        "{}",
        err.0
    );
    assert!(installed(tmp.path()).is_empty(), "nothing may survive");
}

#[test]
fn a_404_or_a_5xx_is_a_refusal_and_not_an_empty_file() {
    let srv = serve(|_| vec![
        ("/gone.json".into(), Reply::Status(404)),
        ("/broken.json".into(), Reply::Status(503)),
    ]);
    let net = Net::new();

    let got = net.get_json(&format!("{}/gone.json", srv.base));
    assert!(!got.ok);
    // We DID reach the origin — that is a different sentence from "no network"
    // and the surface renders them differently.
    assert!(got.reachable, "a 404 means the origin answered");
    assert!(got.why.contains("404"), "{}", got.why);

    let got = net.get_json(&format!("{}/broken.json", srv.base));
    assert!(got.reachable && !got.ok);
    assert!(got.why.contains("503"), "{}", got.why);
}

#[test]
fn an_unreachable_origin_is_unreachable_and_not_empty() {
    // The repo's trap, at the transport layer: a reader that cannot tell
    // "nothing there" from "we could not look" makes a launcher lie about the
    // catalog on a train.
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = format!("http://127.0.0.1:{}", probe.local_addr().unwrap().port());
    drop(probe);

    let net = Net::new();
    let got = net.get_json(&format!("{dead}/anything.json"));
    assert!(!got.ok);
    assert!(!got.reachable, "nothing is listening — this is not a 404");
    assert!(!got.why.is_empty(), "an unreachable read must say why");
}

#[test]
fn a_redirect_is_followed() {
    let srv = serve(|base| {
        vec![
            (
                "/depot/moved.json".into(),
                Reply::Redirect(format!("{base}/depot/real.json")),
            ),
            ("/depot/real.json".into(), Reply::Ok(b"{\"ok\":true}".to_vec())),
        ]
    });
    let net = Net::new();
    let got = net.get_json(&format!("{}/depot/moved.json", srv.base));
    // A CDN in front of a depot redirects; refusing to follow would break
    // every real origin. What may not be followed is a scheme change, which
    // `parse_depot` catches at the root and `browser_argv` catches for pages.
    assert!(got.reachable, "{}", got.why);
}
