//! The signer that holds no key — asserted, not promised.
//!
//! Two halves. The first is a source-level property (nothing in `pairing.rs`
//! reads key material), carried over from `elo-broker/tests/signer.rs` because
//! this backend belongs to that family and only lives in another crate to
//! avoid a dependency cycle. The second drives the real `Signer` impl against
//! a stub of `meter/browser_signer.py`'s routes, so the wire this side speaks
//! is checked rather than described.

use elo_broker::signer::Signer;
use elo_net::pairing::{self, Pairing, PairedBrowserSigner};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// `CLAUDE.md` invariant 7 — *we hold no user keys* — asserted against the
/// source rather than promised in a comment.
///
/// This backend works by ASKING somebody else, exactly like the four in
/// `elo-broker::signer`, so the same file-level property holds and is
/// checkable the same way. If a key ever lands here, this test must be
/// re-scoped on purpose rather than deleted.
#[test]
fn no_branch_in_pairing_reads_a_key() {
    let src = include_str!("../src/pairing.rs");
    for needle in [
        "private_key", "privatekey", "secret_key", "seed_phrase",
        "mnemonic", "keystore", "SigningKey", "sign_message",
    ] {
        // Prose may name the thing it forbids; code may not.
        for (n, line) in src.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("*") {
                continue;
            }
            assert!(
                !code.contains(needle),
                "pairing.rs:{} reads like key material ({needle:?}): {code}\n\
                 This backend holds a code, a secret and an address, and none \
                 of the three can sign. If that is changing, re-scope this \
                 test on purpose.",
                n + 1
            );
        }
    }
}

#[test]
fn a_pairing_round_trips_and_drops_what_it_does_not_know() {
    let p = Pairing {
        host: "https://elopros.com".into(),
        code: "ABCD2345".into(),
        secret: "0123456789abcdef0123456789abcdef".into(),
        address: "0x00000000000000000000000000000000000000ab".into(),
        expires: 4_000_000_000,
    };
    let back = Pairing::from_json(&p.to_json()).expect("round trip");
    assert_eq!(back, p);
    assert_eq!(back.show_code(), "ABCD-2345");

    // A hand-edited file with a key-shaped field must round-trip to a
    // `Pairing` that has no such field — the settings file's rule, applied to
    // the one other thing this client writes beside its keystore.
    let mut v = p.to_json();
    v.as_object_mut()
        .unwrap()
        .insert("private_key".into(), json!("0xdeadbeef"));
    let back = Pairing::from_json(&v).expect("still parses");
    assert_eq!(back, p, "a field this type does not name is dropped, not carried");

    for missing in ["host", "code", "secret", "address"] {
        let mut v = p.to_json();
        v.as_object_mut().unwrap().remove(missing);
        assert!(
            Pairing::from_json(&v).is_none(),
            "a pairing with no {missing} is not a pairing"
        );
    }
}

#[test]
fn an_expired_pairing_reads_as_none_unless_you_ask_for_it() {
    let dir = std::env::temp_dir().join(format!("elo-pair-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pairing.json");
    let stale = Pairing {
        host: "https://elopros.com".into(),
        code: "ABCD2345".into(),
        secret: "0123456789abcdef0123456789abcdef".into(),
        address: "0x00000000000000000000000000000000000000ab".into(),
        expires: 1,
    };
    pairing::save(&path, &stale).expect("save");

    // The default read filters it, so no caller has to remember to check —
    // the failure mode of forgetting is a launcher confidently queueing asks
    // nobody will ever see.
    assert!(pairing::load(&path).is_none(), "an expired pairing reads as none");
    assert!(
        pairing::load_even_if_expired(&path).is_some(),
        "and `elo pair show` can still tell expired from never-paired"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "a bearer credential gets the keystore's mode");
    }

    assert!(pairing::forget(&path).unwrap(), "forget removes it");
    assert!(!pairing::forget(&path).unwrap(), "and says so when there was none");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── a stub of meter/browser_signer.py's relay routes ─────────────────────────
//
// Hand-written for the same reason `elo-launcher/tests/origin.rs`'s is: a web
// framework in the dev-dependencies of a client whose whole dependency policy
// is a tree budget would be an odd trade.
struct Stub {
    base: String,
    asks: Arc<AtomicUsize>,
}

fn serve(answer: Value) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let asks = Arc::new(AtomicUsize::new(0));
    let counted = asks.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let body: Value =
                serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));

            let (code, reply) = if method == "POST" && path.ends_with("/ask") {
                counted.fetch_add(1, Ordering::SeqCst);
                // The origin refuses an ask with no stated reason. Mirrored so
                // the client's own early refusal is checked against the real
                // rule and not against nothing.
                if body.get("why").and_then(Value::as_str).unwrap_or("").is_empty() {
                    (422, json!({"error": "send `why`"}))
                } else if body.get("secret").and_then(Value::as_str) != Some("s".repeat(32).as_str())
                {
                    (403, json!({"error": "wrong secret"}))
                } else {
                    (200, json!({"id": "abc123"}))
                }
            } else if method == "GET" && path.contains("/ask/") {
                (200, answer.clone())
            } else {
                (404, json!({"error": "no such route"}))
            };
            let payload = reply.to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.flush();
        }
    });
    Stub { base: format!("http://127.0.0.1:{port}"), asks }
}

fn signer_at(base: &str) -> PairedBrowserSigner {
    PairedBrowserSigner::new(Pairing {
        host: base.to_string(),
        code: "ABCD2345".into(),
        secret: "s".repeat(32),
        address: "0x00000000000000000000000000000000000000ab".into(),
        expires: pairing::now() + 3600,
    })
}

#[test]
fn it_asks_the_browser_and_carries_the_signature_back() {
    let sig = format!("0x{}", "11".repeat(65));
    let stub = serve(json!({"id": "abc123", "state": "signed", "signature": sig}));
    let mut s = signer_at(&stub.base);

    assert_eq!(s.kind(), "browser-paired");
    assert_eq!(
        s.address().as_deref(),
        Some("0x00000000000000000000000000000000000000ab")
    );
    assert!(s.status().contains("browser"), "{}", s.status());

    let out = s
        .sign("elo play\naction: buy\n", "buy a copy of gates")
        .expect("the stub signs");
    assert_eq!(out["signature"], json!(sig));
    assert_eq!(out["backend"], json!("browser-paired"));
    assert_eq!(
        out["address"],
        json!("0x00000000000000000000000000000000000000ab"),
        "the address is the PAIRED one — the origin already checked the \
         signature recovers to it"
    );
}

#[test]
fn a_refusal_by_the_person_stays_a_refusal_by_the_person() {
    let stub = serve(json!({"id": "abc123", "state": "refused", "refusal": "not today"}));
    let mut s = signer_at(&stub.base);
    let r = s.sign("elo play\nx", "testing").expect_err("refused");
    assert!(r.refused_by_player, "a person saying no is not a transport error");
    assert!(r.reason.contains("not today"), "{}", r.reason);
    assert!(r.handoff.is_none(), "nothing to hand off — the browser was already asked");
}

#[test]
fn a_lapsed_ask_says_so_rather_than_hanging() {
    let stub = serve(json!({"id": "abc123", "state": "expired"}));
    let mut s = signer_at(&stub.base);
    let r = s.sign("elo play\nx", "testing").expect_err("lapsed");
    assert!(!r.refused_by_player, "nobody said no — nobody said anything");
    assert!(r.reason.contains("lapsed"), "{}", r.reason);
}

#[test]
fn an_ask_with_no_reason_never_reaches_the_wire() {
    let stub = serve(json!({"id": "abc123", "state": "signed"}));
    let mut s = signer_at(&stub.base);
    let r = s.sign("elo play\nx", "   ").expect_err("refused");
    assert!(r.reason.contains("stated reason"), "{}", r.reason);
    assert_eq!(
        stub.asks.load(Ordering::SeqCst),
        0,
        "refused here, not round-tripped for a 422 — the caller reads the \
         reason either way and one of the two costs a network hop"
    );
}

#[test]
fn an_expired_pairing_refuses_before_it_asks() {
    let stub = serve(json!({"id": "abc123", "state": "signed"}));
    let mut s = signer_at(&stub.base);
    s.pairing.expires = 1;
    let r = s.sign("elo play\nx", "testing").expect_err("expired");
    assert!(r.reason.contains("expired"), "{}", r.reason);
    assert!(r.reason.contains("elo pair"), "and names the way out: {}", r.reason);
    assert_eq!(stub.asks.load(Ordering::SeqCst), 0, "no wire traffic for a dead pairing");
}

#[test]
fn the_origins_own_refusal_survives_to_the_caller() {
    let stub = serve(json!({}));
    let mut s = signer_at(&stub.base);
    s.pairing.secret = "0".repeat(32); // the stub's `wrong secret` branch
    let r = s.sign("elo play\nx", "testing").expect_err("403");
    assert!(
        r.reason.contains("wrong secret"),
        "the door's own sentence, not `the origin answered 403`: {}",
        r.reason
    );
}
