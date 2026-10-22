//! A relay on localhost, over real TLS and a real WebSocket, so the read path
//! is tested without the network and without speaking in a public room.
//!
//! The live relay's rooms are all empty today (measured 2026-10-20: zero
//! stored events in each of the six), so nothing there can exercise what this
//! does: a stored history with a forgery in it, and a live message that lands
//! while the client is waiting on the relay's ruling about its own note.
//!
//! ⚠ **The certificates are made here, at test time, and never committed.** A
//! PEM private key in the tree is what `deploy/hooks/pre-commit` (and the CI
//! scan of history) exists to stop, and a fixture is not an exception to that.
//! So [`test_pki`] makes a throwaway Ed25519 CA and a `localhost` leaf it
//! signs, with `ring` — already in the tree under rustls — and a few lines of
//! DER. `SSL_CERT_FILE` adds the CA as an EXTRA root (`relay.rs`
//! `extra_roots`), the same door a corporate proxy uses.

use elo_hive::relay::{Polled, Relay};
use elo_hive::voice::{Event, UnsignedEvent, Voice};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{json, Value};
use std::net::TcpListener;
use std::sync::{Arc, OnceLock};
use tungstenite::Message;

// ── a minimal DER writer, enough for two certificates ───────────────────────

fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = body.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes: Vec<u8> = n.to_be_bytes().iter().copied().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(body);
    out
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

/// OID 1.3.101.112 (Ed25519) and friends, already encoded.
const OID_ED25519: &[u8] = &[0x2b, 0x65, 0x70];
const OID_CN: &[u8] = &[0x55, 0x04, 0x03];
const OID_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];
const OID_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11];
const OID_EKU: &[u8] = &[0x55, 0x1d, 0x25];
const OID_SERVER_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];

fn oid(o: &[u8]) -> Vec<u8> {
    tlv(0x06, o)
}

fn name(cn: &str) -> Vec<u8> {
    seq(&[tlv(0x31, &seq(&[oid(OID_CN), tlv(0x0c, cn.as_bytes())]))])
}

fn ext(id: &[u8], critical: bool, value: Vec<u8>) -> Vec<u8> {
    let mut parts = vec![oid(id)];
    if critical {
        parts.push(tlv(0x01, &[0xff]));
    }
    parts.push(tlv(0x04, &value));
    seq(&parts)
}

/// One certificate for `subject_key`, signed by `issuer_key`.
fn cert(serial: u8, issuer: &str, subject: &str, subject_key: &Ed25519KeyPair,
        issuer_key: &Ed25519KeyPair, extensions: Vec<Vec<u8>>) -> Vec<u8> {
    let alg = seq(&[oid(OID_ED25519)]);
    let mut spki_bits = vec![0u8];
    spki_bits.extend_from_slice(subject_key.public_key().as_ref());
    let tbs = seq(&[
        tlv(0xa0, &tlv(0x02, &[2])),                     // v3
        tlv(0x02, &[serial]),
        alg.clone(),
        name(issuer),
        seq(&[tlv(0x17, b"250101000000Z"), tlv(0x18, b"20991231235959Z")]),
        name(subject),
        seq(&[alg.clone(), tlv(0x03, &spki_bits)]),
        tlv(0xa3, &seq(&extensions)),
    ]);
    let mut sig_bits = vec![0u8];
    sig_bits.extend_from_slice(issuer_key.sign(&tbs).as_ref());
    seq(&[tbs, alg, tlv(0x03, &sig_bits)])
}

fn pem(der: &[u8]) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b = String::new();
    for chunk in der.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            b.push(if i <= chunk.len() { B64[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    let lines: Vec<&str> = b.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
    format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", lines.join("\n"))
}

struct Pki {
    leaf: Vec<u8>,
    leaf_key: Vec<u8>,
}

/// A CA and a `localhost` leaf, made once per test binary, with the CA
/// written where `SSL_CERT_FILE` points.
fn test_pki() -> &'static Pki {
    static PKI: OnceLock<Pki> = OnceLock::new();
    PKI.get_or_init(|| {
        let rng = ring::rand::SystemRandom::new();
        let ca_doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let ca_key = Ed25519KeyPair::from_pkcs8(ca_doc.as_ref()).unwrap();
        let leaf_doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let leaf_key = Ed25519KeyPair::from_pkcs8(leaf_doc.as_ref()).unwrap();
        let ca = cert(1, "elo test relay CA", "elo test relay CA", &ca_key, &ca_key, vec![
            ext(OID_BASIC_CONSTRAINTS, true, seq(&[tlv(0x01, &[0xff])])),
            ext(OID_KEY_USAGE, true, tlv(0x03, &[0x02, 0x04])), // keyCertSign
        ]);
        let leaf = cert(2, "elo test relay CA", "localhost", &leaf_key, &ca_key, vec![
            ext(OID_BASIC_CONSTRAINTS, true, seq(&[])),
            ext(OID_KEY_USAGE, true, tlv(0x03, &[0x07, 0x80])), // digitalSignature
            ext(OID_EKU, false, seq(&[oid(OID_SERVER_AUTH)])),
            ext(OID_SAN, false, seq(&[tlv(0x82, b"localhost")])),
        ]);
        let dir = std::env::temp_dir().join(format!("elo-hive-fake-relay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ca.pem");
        std::fs::write(&path, pem(&ca)).unwrap();
        std::env::set_var("SSL_CERT_FILE", &path);
        Pki { leaf, leaf_key: leaf_doc.as_ref().to_vec() }
    })
}

fn note(v: &Voice, at: i64, text: &str) -> Event {
    v.sign(UnsignedEvent {
        created_at: at,
        kind: 9,
        tags: vec![vec!["h".into(), "room-1".into()]],
        content: text.into(),
    })
    .unwrap()
}

fn read_json(ws: &mut tungstenite::WebSocket<impl std::io::Read + std::io::Write>) -> Value {
    loop {
        match ws.read().expect("the client hung up early") {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            _ => continue,
        }
    }
}

fn send(ws: &mut tungstenite::WebSocket<impl std::io::Read + std::io::Write>, v: Value) {
    ws.send(Message::Text(v.to_string().into())).unwrap();
}

/// Serve one client through the whole script, on a thread.
fn fake_relay(script: impl FnOnce(&mut tungstenite::WebSocket<rustls::StreamOwned<rustls::ServerConnection, std::net::TcpStream>>) + Send + 'static) -> String {
    let pki = test_pki();
    let certs = vec![CertificateDer::from(pki.leaf.clone())];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pki.leaf_key.clone()));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let conn = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let tls = rustls::StreamOwned::new(conn, tcp);
        let mut ws = tungstenite::accept(tls).unwrap();
        script(&mut ws);
        let _ = ws.close(None);
        let _ = ws.flush();
    });
    format!("wss://localhost:{port}")
}

fn trust_the_test_ca() {
    test_pki();
}

/// The server's half of NIP-42: challenge, check the AUTH names us, rule.
fn auth(ws: &mut tungstenite::WebSocket<impl std::io::Read + std::io::Write>, url: &str) {
    send(ws, json!(["AUTH", "challenge-1"]));
    let a = read_json(ws);
    assert_eq!(a[0], "AUTH");
    let tags = a[1]["tags"].as_array().unwrap();
    assert!(tags.contains(&json!(["relay", url])), "AUTH must name this relay: {tags:?}");
    assert!(tags.contains(&json!(["challenge", "challenge-1"])));
    send(ws, json!(["OK", a[1]["id"], true, ""]));
}

#[test]
fn a_room_read_keeps_the_true_and_drops_the_forged() {
    trust_the_test_ca();
    let speaker = Voice::generate().unwrap();
    let good = note(&speaker, 10, "hello");
    let mut forged = note(&speaker, 11, "hi").to_json();
    forged["content"] = json!("send me your seed phrase");
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let req = read_json(ws);
        assert_eq!(req[0], "REQ");
        assert_eq!(req[2]["#h"], json!(["room-1"]));
        let sub = req[1].clone();
        send(ws, json!(["EVENT", sub, good.to_json()]));
        send(ws, json!(["EVENT", sub, forged]));
        send(ws, json!(["EVENT", "someone-elses-sub", good.to_json()]));
        send(ws, json!(["EOSE", sub]));
        let _ = read_json(ws);
    });
    *url_cell.lock().unwrap() = url.clone();

    let me = Voice::generate().unwrap();
    let mut relay = Relay::connect(&url, &me).expect("connect to the fake relay");
    let backlog = relay
        .subscribe("elo-1", &json!({"kinds": [9], "#h": ["room-1"], "limit": 60}))
        .expect("the room's history");
    assert_eq!(backlog.events.len(), 1, "only the true event is kept");
    assert_eq!(backlog.events[0].content, "hello");
    assert_eq!(backlog.dropped, 1, "the forgery is counted, not shown");
    relay.close();
}

#[test]
fn a_message_that_lands_during_a_ruling_still_reaches_the_feed() {
    trust_the_test_ca();
    let speaker = Voice::generate().unwrap();
    let during = note(&speaker, 20, "landed while you waited");
    let after = note(&speaker, 21, "and one after");
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let req = read_json(ws);
        let sub = req[1].clone();
        send(ws, json!(["EOSE", sub]));
        // The client's own note: a room message arrives BEFORE the ruling.
        let ev = read_json(ws);
        assert_eq!(ev[0], "EVENT");
        send(ws, json!(["EVENT", sub, during.to_json()]));
        send(ws, json!(["OK", ev[1]["id"], true, ""]));
        send(ws, json!(["EVENT", sub, after.to_json()]));
        let _ = read_json(ws);
    });
    *url_cell.lock().unwrap() = url.clone();

    let me = Voice::generate().unwrap();
    let mut relay = Relay::connect(&url, &me).unwrap();
    relay.set_wait(std::time::Duration::from_millis(200)).unwrap();
    assert!(relay.subscribe("elo-1", &json!({"#h": ["room-1"]})).unwrap().events.is_empty());
    let mine = note(&me, 19, "my note");
    relay.publish(&mine).expect("the relay ruled true");

    let mut got = Vec::new();
    for _ in 0..20 {
        match relay.poll("elo-1").unwrap() {
            Some(Polled::Event(e)) => got.push(e.content),
            Some(Polled::Dropped) => panic!("nothing here was forged"),
            None if got.len() == 2 => break,
            None => {}
        }
    }
    assert_eq!(got, ["landed while you waited", "and one after"], "in order, none lost");
    relay.close();
}

#[test]
fn a_closed_room_is_a_sentence_not_a_hang() {
    trust_the_test_ca();
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let req = read_json(ws);
        send(ws, json!(["CLOSED", req[1], "restricted: members only"]));
        let _ = read_json(ws);
    });
    *url_cell.lock().unwrap() = url.clone();

    let me = Voice::generate().unwrap();
    let mut relay = Relay::connect(&url, &me).unwrap();
    let e = relay.subscribe("elo-1", &json!({"#h": ["room-1"]})).unwrap_err();
    assert!(e.to_string().contains("members only"), "{e}");
    relay.close();
}

// ── a hostile relay: it can drop a message, and it can do no more ───────────

use elo_hive::relay::checked;
use std::time::{Duration, Instant};

/// A `pubkey` that is not hex but is an even number of BYTES, with the id
/// recomputed over it so the id check passes and the decode is reached.
fn poison_pubkey_event() -> Value {
    let pubkey = "a\u{e9}a";
    let unsigned = UnsignedEvent {
        created_at: 1_760_000_000,
        kind: 9,
        tags: vec![vec!["h".into(), "room-1".into()]],
        content: "hi".into(),
    };
    let id = unsigned.id(pubkey);
    json!({
        "id": id, "pubkey": pubkey, "created_at": 1_760_000_000, "kind": 9,
        "tags": [["h", "room-1"]], "content": "hi", "sig": "00".repeat(64),
    })
}

/// A genuine note with only its `sig` swapped — the id does not cover it.
fn poison_sig_event() -> Value {
    let speaker = Voice::generate().unwrap();
    let mut ev = note(&speaker, 1_760_000_000, "a real note").to_json();
    ev["sig"] = json!("a\u{e9}a");
    ev
}

#[test]
fn a_non_hex_key_or_signature_is_a_dropped_event_not_a_crash() {
    // The release profile is panic = "abort": a panic here is the whole
    // launcher gone, so a relay's bad bytes must come back as an Err.
    assert!(checked(&poison_pubkey_event()).is_err());
    assert!(checked(&poison_sig_event()).is_err());

    trust_the_test_ca();
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let req = read_json(ws);
        let sub = req[1].clone();
        send(ws, json!(["EVENT", sub, poison_pubkey_event()]));
        send(ws, json!(["EVENT", sub, poison_sig_event()]));
        send(ws, json!(["EOSE", sub]));
        let _ = ws.read();
    });
    *url_cell.lock().unwrap() = url.clone();
    let me = Voice::generate().unwrap();
    let mut relay = Relay::connect(&url, &me).unwrap();
    let backlog = relay.subscribe("elo-1", &json!({"#h": ["room-1"], "limit": 60})).unwrap();
    assert!(backlog.events.is_empty());
    assert_eq!(backlog.dropped, 2, "both counted, neither shown");
    relay.close();
}

/// Run `f` on a thread and report how long it took, or `None` past `budget`.
fn within<T: Send + 'static>(budget: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<(T, Duration)> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let t = Instant::now();
        let r = f();
        let _ = tx.send((r, t.elapsed()));
    });
    rx.recv_timeout(budget).ok()
}

#[test]
fn a_relay_that_never_challenges_cannot_hold_connect() {
    trust_the_test_ca();
    let url = fake_relay(move |ws| {
        for _ in 0..1200 {
            if ws.send(Message::Text(json!(["NOTICE", "hold on"]).to_string().into())).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let got = within(Duration::from_secs(30), move || {
        Relay::connect(&url, &Voice::generate().unwrap()).map(|_| ()).map_err(|e| e.to_string())
    });
    let (r, _took) = got.expect("connect must give up on a relay that only talks");
    assert!(r.is_err(), "a relay that never challenged or ruled is not connected");
}

#[test]
fn a_chatty_relay_cannot_hold_one_poll() {
    trust_the_test_ca();
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let req = read_json(ws);
        send(ws, json!(["EOSE", req[1]]));
        for _ in 0..1200 {
            if ws.send(Message::Text(json!(["NOTICE", "x"]).to_string().into())).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    *url_cell.lock().unwrap() = url.clone();
    let mut relay = Relay::connect(&url, &Voice::generate().unwrap()).unwrap();
    relay.set_wait(Duration::from_millis(250)).unwrap();
    assert!(relay.subscribe("elo-1", &json!({"#h": ["room-1"]})).unwrap().events.is_empty());
    let got = within(Duration::from_secs(10), move || format!("{:?}", relay.poll("elo-1")));
    let (r, took) = got.expect("one poll must come back while the relay chatters");
    assert_eq!(r, "Ok(None)");
    assert!(took < Duration::from_secs(3), "poll took {took:?} on a 250 ms wait");
}

#[test]
fn an_oversized_frame_ends_the_connection() {
    trust_the_test_ca();
    let big = "x".repeat(1 << 20);
    let url = fake_relay(move |ws| {
        send(ws, json!(["AUTH", "challenge-1"]));
        let a = read_json(ws);
        let _ = ws.send(Message::Text(json!(["NOTICE", big]).to_string().into()));
        let _ = ws.send(Message::Text(json!(["OK", a[1]["id"], true, ""]).to_string().into()));
        let _ = ws.flush();
        let _ = ws.read();
    });
    let e = Relay::connect(&url, &Voice::generate().unwrap()).map(|_| ()).unwrap_err();
    assert!(e.to_string().contains("relay"), "{e}");
}

#[test]
fn what_waits_for_poll_is_bounded_by_bytes_not_only_frames() {
    trust_the_test_ca();
    const N: usize = 40;
    let speaker = Voice::generate().unwrap();
    let fat: Vec<Event> = (0..N).map(|i| note(&speaker, i as i64, &"y".repeat(100 << 10))).collect();
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let req = read_json(ws);
        let sub = req[1].clone();
        send(ws, json!(["EOSE", sub]));
        let ev = read_json(ws);
        for f in &fat {
            send(ws, json!(["EVENT", sub, f.to_json()]));
        }
        send(ws, json!(["OK", ev[1]["id"], true, ""]));
        let _ = ws.read();
    });
    *url_cell.lock().unwrap() = url.clone();
    let me = Voice::generate().unwrap();
    let mut relay = Relay::connect(&url, &me).unwrap();
    relay.set_wait(Duration::from_millis(200)).unwrap();
    relay.subscribe("elo-1", &json!({"#h": ["room-1"]})).unwrap();
    relay.publish(&note(&me, 1, "mine")).unwrap();
    let mut kept = 0;
    while let Some(Polled::Event(_)) = relay.poll("elo-1").unwrap() {
        kept += 1;
    }
    assert!(kept > 0, "some of what landed during the ruling is kept");
    assert!(kept < N, "{kept} of {N} fat frames held — the stash has no byte bound");
    relay.close();
}

#[test]
fn a_room_history_keeps_no_more_than_it_asked_for() {
    trust_the_test_ca();
    let speaker = Voice::generate().unwrap();
    let many: Vec<Event> = (0..50).map(|i| note(&speaker, i, "n")).collect();
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let req = read_json(ws);
        let sub = req[1].clone();
        for e in &many {
            send(ws, json!(["EVENT", sub, e.to_json()]));
        }
        send(ws, json!(["EOSE", sub]));
        let _ = ws.read();
    });
    *url_cell.lock().unwrap() = url.clone();
    let mut relay = Relay::connect(&url, &Voice::generate().unwrap()).unwrap();
    let backlog = relay.subscribe("elo-1", &json!({"#h": ["room-1"], "limit": 10})).unwrap();
    assert_eq!(backlog.events.len(), 10, "a relay that ignores `limit` does not grow us");
    relay.close();
}

#[test]
fn a_chatty_relay_cannot_hold_a_room_history_open() {
    trust_the_test_ca();
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let _req = read_json(ws);
        // Never EOSE; one NOTICE well inside every per-message timeout.
        for _ in 0..240 {
            if ws.send(Message::Text(json!(["NOTICE", "x"]).to_string().into())).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    *url_cell.lock().unwrap() = url.clone();
    let mut relay = Relay::connect(&url, &Voice::generate().unwrap()).unwrap();
    let got = within(Duration::from_secs(35), move || {
        relay
            .subscribe("elo-1", &json!({"#h": ["room-1"], "limit": 60}))
            .map(|_| ())
            .map_err(|e| e.to_string())
    });
    let (r, took) = got.expect("a room's history must come back while the relay chatters");
    assert!(r.is_err(), "a relay that never sent EOSE did not finish the history");
    assert!(took < Duration::from_secs(30), "subscribe took {took:?}");
}

// ── a hostile relay's WORDS: it can say why, and it cannot drive the terminal ─

/// OSC 52 (write the clipboard) then clear the screen — what a relay put in
/// its refusal reason to swap the address a player was about to paste.
const HOSTILE_WHY: &str = "\x1b]52;c;MHhBVFRBQ0tFUg==\x07\x1b[2J\x1b[Hall good";

fn no_terminal_control(e: &str) {
    assert!(!e.contains('\x1b'), "a relay's text reached the error raw: {e:?}");
    assert!(!e.contains('\x07'), "a relay's text reached the error raw: {e:?}");
    assert!(e.chars().count() < 400, "a relay's text is clipped: {} chars", e.chars().count());
}

#[test]
fn a_refused_auth_reason_cannot_drive_the_terminal() {
    trust_the_test_ca();
    let url = fake_relay(move |ws| {
        send(ws, json!(["AUTH", "challenge-1"]));
        let a = read_json(ws);
        let why = format!("{HOSTILE_WHY}{}", "!".repeat(4000));
        send(ws, json!(["OK", a[1]["id"], false, why]));
        let _ = ws.read();
    });
    let e = Relay::connect(&url, &Voice::generate().unwrap()).map(|_| ()).unwrap_err().to_string();
    assert!(e.contains("refused this key") && e.contains("all good"), "the reason is still said: {e}");
    no_terminal_control(&e);
}

#[test]
fn a_refused_note_and_a_closed_room_cannot_drive_the_terminal() {
    trust_the_test_ca();
    let url_cell = Arc::new(std::sync::Mutex::new(String::new()));
    let url_for_server = Arc::clone(&url_cell);
    let url = fake_relay(move |ws| {
        let url = url_for_server.lock().unwrap().clone();
        auth(ws, &url);
        let ev = read_json(ws);
        send(ws, json!(["OK", ev[1]["id"], false, HOSTILE_WHY]));
        let req = read_json(ws);
        send(ws, json!(["CLOSED", req[1], HOSTILE_WHY]));
        let _ = read_json(ws);
    });
    *url_cell.lock().unwrap() = url.clone();
    let me = Voice::generate().unwrap();
    let mut relay = Relay::connect(&url, &me).unwrap();
    let e = relay.publish(&note(&me, 30, "hi")).unwrap_err().to_string();
    assert!(e.contains("refused the note"), "{e}");
    no_terminal_control(&e);
    let e = relay.subscribe("elo-1", &json!({"#h": ["room-1"]})).unwrap_err().to_string();
    assert!(e.contains("closed the room"), "{e}");
    no_terminal_control(&e);
    relay.close();
}

#[test]
fn a_close_frame_reason_cannot_drive_the_terminal() {
    trust_the_test_ca();
    let url = fake_relay(move |ws| {
        send(ws, json!(["AUTH", "challenge-1"]));
        let _ = read_json(ws);
        let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Policy,
            reason: HOSTILE_WHY.into(),
        }));
        let _ = ws.flush();
        let _ = ws.read();
    });
    let e = Relay::connect(&url, &Voice::generate().unwrap()).map(|_| ()).unwrap_err().to_string();
    assert!(e.contains("closed the connection"), "{e}");
    no_terminal_control(&e);
}
