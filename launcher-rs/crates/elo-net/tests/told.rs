//! A refusal's SENTENCE has to survive the wire.
//!
//! `post_json` folds every non-2xx into a bare status code and throws the body
//! away. That is right for a depot — a 404 on a build file is a 404 and there
//! is nothing to read. It is wrong for the store desk, whose whole design is
//! that a refusal names the field and the reason ("a silently dropped field is
//! an operator believing they published something they did not"). A client that
//! printed `the origin answered 422` would be discarding the only part of the
//! answer worth having.
//!
//! So `post_json_told` exists, and this is what makes it a rule rather than a
//! method: the two behaviours are asserted side by side, on the same reply.

use elo_net::Net;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

/// A tiny HTTP/1.1 server, hand-written for the same reason `transport.rs`
/// writes its own: pulling a web framework into the dev-dependencies to test a
/// launcher would grow the tree this workspace's policy exists to keep small.
struct Server {
    base: String,
    _stop: mpsc::Sender<()>,
}

fn serve(code: u16, body: &'static str, ctype: &'static str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if rx.try_recv().is_ok() {
                return;
            }
            let Ok(mut stream) = stream else { continue };
            let _ = drain(&mut stream);
            let head = format!(
                "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
            let _ = stream.flush();
        }
    });
    Server { base: format!("http://127.0.0.1:{port}"), _stop: tx }
}

/// Read the request head and whatever body it declares — a server that replies
/// without draining gets a broken pipe on the client's write instead of the
/// status we are trying to test.
fn drain(stream: &mut TcpStream) -> Option<()> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
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
    reader.read_exact(&mut body).ok();
    Some(())
}

#[test]
fn a_refusals_reason_survives_and_the_old_post_drops_it() {
    let s = serve(
        422,
        r#"{"error":"'band' is the house's field, not the title's"}"#,
        "application/json",
    );
    let net = Net::new();
    let url = format!("{}/api/store/dev/post", s.base);

    let told = net.post_json_told(&url, &json!({"game": "gates"}));
    assert!(told.reachable, "the origin answered, so it was reached");
    assert!(!told.ok, "a 422 is not ok, however readable its body is");
    assert_eq!(told.status, Some(422));
    assert_eq!(told.why, "'band' is the house's field, not the title's");
    assert!(told.value.is_some(), "the body is parsed and handed back");

    // The contrast, asserted rather than described: the same reply through the
    // old method keeps the code and loses the sentence.
    let bare = net.post_json(&url, &json!({"game": "gates"}));
    assert_eq!(bare.status, Some(422));
    assert!(bare.value.is_none(), "post_json drops the body — that is why the other exists");
}

#[test]
fn a_success_is_ok_and_carries_the_body() {
    let s = serve(201, r#"{"ok":true,"id":"gates-1"}"#, "application/json");
    let got = Net::new().post_json_told(&format!("{}/x", s.base), &json!({}));
    assert!(got.ok && got.why.is_empty());
    assert_eq!(got.status, Some(201));
    assert_eq!(got.value.unwrap()["id"], "gates-1");
}

#[test]
fn a_refusal_with_no_json_body_still_says_the_code_rather_than_nothing() {
    let s = serve(503, "the desk is down", "text/plain");
    let got = Net::new().post_json_told(&format!("{}/x", s.base), &json!({}));
    assert!(!got.ok && got.reachable);
    assert_eq!(got.status, Some(503));
    assert!(got.why.contains("503"), "{}", got.why);
}

#[test]
fn an_origin_that_is_not_there_is_unreachable_and_not_a_refusal() {
    // The distinction this whole crate exists for: "we could not look" is never
    // "it said no". A client that confused them would tell a dev their edit was
    // refused when their wifi dropped.
    let got = Net::new().post_json_told("http://127.0.0.1:9/x", &json!({}));
    assert!(!got.reachable);
    assert_eq!(got.status, None);
}

#[test]
fn bytes_go_up_as_bytes_and_the_reply_still_reads() {
    let s = serve(201, r#"{"ok":true,"sha256":"abc"}"#, "application/json");
    let got = Net::new().post_bytes_told(
        &format!("{}/api/store/dev/upload", s.base),
        "application/octet-stream",
        vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10],
    );
    assert!(got.ok, "{}", got.why);
    assert_eq!(got.value.unwrap()["sha256"], "abc");
}
