//! The HTTP client against a local stub server: a refusal, could-not-look,
//! and the guard that refuses to sign a text the change did not ask for.
#![cfg(feature = "http")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use elo_sdk::{texts, ActError, Client, Look};
use serde_json::{json, Value};

const ME: &str = "0x1111111111111111111111111111111111111111";
const THEM: &str = "0x7777777777777777777777777777777777777777";

type Route = Box<dyn Fn(&str, &Value) -> (u16, String) + Send + Sync>;

/// Serve `route` on a local port for as long as the test runs, and record
/// every request path. Returns the origin and the record.
fn stub(route: Route) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    thread::spawn(move || {
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
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            let _ = reader.read_exact(&mut body);
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            log.lock().unwrap().push(path.clone());
            let (status, text) = route(&path, &body);
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    (origin, seen)
}

fn client(origin: &str) -> Client {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    Client::with_agent(origin, agent)
}

fn view() -> Value {
    serde_json::from_str(include_str!("../../testdata/friends_view.json")).unwrap()
}

#[test]
fn a_text_other_than_the_change_asked_for_is_never_signed() {
    let at = 1_761_003_600u64;
    // Asked to ask THEM; the origin prepares a block instead.
    let offered = texts::friends_sign_text(ME, "block", THEM, at).unwrap();
    let (origin, seen) = stub(Box::new(move |path, _| match path {
        "/api/friends/prepare" => (
            200,
            json!({"sign_this": offered, "at": at, "args": {"wallet": THEM}, "op": "block"})
                .to_string(),
        ),
        _ => (500, "{}".into()),
    }));
    let mut signed = 0;
    let mut sign = |_: &str| -> Result<String, String> {
        signed += 1;
        Ok("0xsig".into())
    };
    let got = client(&origin).friend(ME, "ask", THEM, &mut sign);
    assert!(matches!(got, Err(ActError::Mismatch { .. })), "{got:?}");
    assert_eq!(signed, 0, "the signer was never asked");
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["/api/friends/prepare".to_string()],
        "nothing was acted"
    );
}

#[test]
fn the_rebuilt_text_is_signed_once_and_acted() {
    let at = 1_761_003_600u64;
    let text = texts::friends_sign_text(ME, "ask", THEM, at).unwrap();
    let want = text.clone();
    let (origin, seen) = stub(Box::new(move |path, body| match path {
        "/api/friends/prepare" => (
            200,
            json!({"sign_this": text, "at": at, "args": {"to": THEM}, "op": "ask"}).to_string(),
        ),
        "/api/friends/act"
            if body["signature"] == "0xsig" && body["at"] == at && body["args"]["to"] == THEM =>
        {
            (
                200,
                json!({"ok": true, "op": "ask", "view": view()}).to_string(),
            )
        }
        _ => (401, json!({"error": "bad act"}).to_string()),
    }));
    let mut texts_signed = Vec::new();
    let mut sign = |t: &str| -> Result<String, String> {
        texts_signed.push(t.to_string());
        Ok("0xsig".into())
    };
    let got = client(&origin).friend(ME, "ask", THEM, &mut sign).unwrap();
    let acted = got.answer().expect("an answer");
    assert!(acted.ok);
    assert_eq!(acted.view.unwrap().buddies.len(), 1);
    assert_eq!(texts_signed, vec![want]);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn a_refusal_and_a_dead_origin_are_different_answers() {
    let (origin, _) = stub(Box::new(|path, _| {
        match path {
        p if p.starts_with("/api/friends/") => (404, json!({"error": "not here"}).to_string()),
        p if p.starts_with("/api/ticket/gates/of/") => (
            503,
            json!({"game": "gates", "entitled": null, "reachable": false, "why": "the chain read failed"}).to_string(),
        ),
        _ => (200, "not json".into()),
    }
    }));
    let c = client(&origin);
    match c.friends(ME) {
        Look::Refused(r) => assert_eq!((r.status, r.error.as_str()), (404, "not here")),
        other => panic!("expected a refusal, got {}", other.kind()),
    }
    assert_eq!(
        c.owns("gates", ME),
        Look::CouldNotLook("the chain read failed".into())
    );
    assert!(
        c.face(ME).is_could_not_look(),
        "a body that is not JSON is could-not-look"
    );

    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    assert!(
        client(&dead).friends(ME).is_could_not_look(),
        "nobody listening is could-not-look"
    );
}
