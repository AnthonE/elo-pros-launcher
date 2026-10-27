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

const AT: u64 = 1_761_003_600;

/// buddy.detail_of's bytes, as Python writes them (5.0, not 5).
fn buddy_text(op: &str, args_json: &str) -> String {
    elo_sdk::play_message(
        texts::BUDDY_ACTION,
        ME,
        &format!(r#"{{"args":{args_json},"at":{AT},"op":"{op}"}}"#),
        Some(&elo_sdk::utc_day()),
    )
}

type Acted = Arc<Mutex<Vec<Value>>>;

/// A stub that prepares `offered` at `prepare` and records each body POSTed
/// to `act`.
fn acting(
    prepare: &'static str,
    offered: Value,
    act: String,
) -> (String, Arc<Mutex<Vec<String>>>, Acted) {
    let acted: Acted = Arc::default();
    let log = Arc::clone(&acted);
    let (origin, seen) = stub(Box::new(move |path, body| {
        if path == prepare {
            (200, offered.to_string())
        } else if path == act {
            log.lock().unwrap().push(body.clone());
            (200, json!({"ok": true, "grant": "g.1"}).to_string())
        } else {
            (404, json!({"error": "no"}).to_string())
        }
    }));
    (origin, seen, acted)
}

fn signer(into: &mut Vec<String>) -> impl FnMut(&str) -> Result<String, String> + '_ {
    |t: &str| {
        into.push(t.to_string());
        Ok("0xsig".to_string())
    }
}

#[test]
fn a_buddy_change_is_signed_once_as_cleaned_and_acted() {
    let offered = buddy_text("gift", &format!(r#"{{"to":"{THEM}","usd":5.0}}"#));
    let (origin, _, acted) = acting(
        "/api/buddy/prepare",
        json!({"sign_this": offered, "at": AT, "op": "gift"}),
        "/api/buddy/act".into(),
    );
    let mut signed = Vec::new();
    let asked = json!({"to": THEM, "usd": "5"});
    let got = client(&origin)
        .buddy(ME, "gift", &asked, None, &mut signer(&mut signed))
        .unwrap();
    assert!(got.is_answer());
    assert_eq!(signed, vec![offered]);
    assert_eq!(
        acted.lock().unwrap().clone(),
        vec![json!({"wallet": ME, "op": "gift", "args": asked, "at": AT, "signature": "0xsig"})]
    );
}

#[test]
fn a_gift_to_someone_else_or_for_more_is_never_signed() {
    for args in [
        format!(r#"{{"to":"0x{}","usd":5.0}}"#, "88".repeat(20)),
        format!(r#"{{"to":"{THEM}","usd":50.0}}"#),
        format!(r#"{{"memo":"x","to":"{THEM}","usd":5.0}}"#),
    ] {
        let offered = buddy_text("gift", &args);
        let (origin, seen, _) = acting(
            "/api/buddy/prepare",
            json!({"sign_this": offered, "at": AT}),
            "/api/buddy/act".into(),
        );
        let mut signed = Vec::new();
        let r = client(&origin).buddy(
            ME,
            "gift",
            &json!({"to": THEM, "usd": 5}),
            None,
            &mut signer(&mut signed),
        );
        assert!(matches!(r, Err(ActError::Mismatch { .. })), "{args}");
        assert!(signed.is_empty(), "the signer was never asked");
        assert_eq!(seen.lock().unwrap().len(), 1, "nothing was acted");
    }
    let c = client("http://127.0.0.1:9");
    let mut never = |_: &str| -> Result<String, String> { panic!("nothing should be signed") };
    assert!(matches!(
        c.buddy(ME, "poke", &json!({}), None, &mut never),
        Err(ActError::BadInput(_))
    ));
    assert!(matches!(
        c.buddy(ME, "pause", &json!({"id": "a"}), Some(b"png"), &mut never),
        Err(ActError::BadInput(_))
    ));
}

#[test]
fn a_delivery_signs_this_bounty_and_this_url_and_nothing_else() {
    let url = "https://github.com/AnthonE/Gates/pull/412";
    let day = elo_sdk::utc_day();
    let run = |op: &str, detail: String| {
        let offered = elo_sdk::play_message(op, ME, &detail, Some(&day));
        let (origin, _, acted) = acting(
            "/api/munus/gates-pr/prepare",
            json!({"op": op, "detail": detail, "sign_this": offered}),
            format!("/api/munus/gates-pr/{op}"),
        );
        let mut signed = Vec::new();
        let c = client(&origin);
        let got = if op == "submit" {
            c.board_submit(ME, "gates-pr", url, Some("hi"), &mut signer(&mut signed))
        } else {
            c.board_claim(ME, "gates-pr", &mut signer(&mut signed))
        };
        let acted = acted.lock().unwrap().clone();
        (got, signed, acted)
    };
    let (got, signed, acted) = run("submit", texts::submit_detail("gates-pr", url, 2));
    assert!(got.unwrap().is_answer());
    assert_eq!(
        signed,
        vec![texts::submit_text(ME, "gates-pr", url, 2, Some(&day))]
    );
    assert_eq!(
        acted,
        vec![json!({"wallet": ME, "signature": "0xsig", "url": url, "note": "hi"})]
    );
    for d in [
        texts::submit_detail("gates-pr", &format!("{url}1"), 0),
        texts::submit_detail("gates-soak", url, 0),
        "gates-pr".to_string(),
    ] {
        assert!(
            matches!(run("submit", d.clone()).0, Err(ActError::Mismatch { .. })),
            "{d}"
        );
    }
    let (got, signed, _) = run("claim", "gates-pr".into());
    assert!(got.unwrap().is_answer());
    assert_eq!(signed, vec![texts::claim_text(ME, "gates-pr", Some(&day))]);
    assert!(matches!(
        run("claim", "gates-soak".into()).0,
        Err(ActError::Mismatch { .. })
    ));
}

#[test]
fn the_entitle_text_is_rebuilt_signed_and_the_grant_comes_back() {
    let day = elo_sdk::utc_day();
    for (whose, ok) in [(ME, true), (THEM, false)] {
        let (origin, _, acted) = acting(
            "/api/ticket/gates/message?wallet=0x1111111111111111111111111111111111111111",
            json!({"message": texts::entitle_text("gates", whose, Some(&day))}),
            "/api/ticket/gates/entitle".into(),
        );
        let mut signed = Vec::new();
        let upper = format!("0x{}", ME[2..].to_ascii_uppercase());
        let got = client(&origin).entitle("gates", &upper, &mut signer(&mut signed));
        if ok {
            assert_eq!(got.unwrap().answer().unwrap()["grant"], "g.1");
            assert_eq!(
                acted.lock().unwrap().clone(),
                vec![json!({"wallet": ME, "signature": "0xsig"})]
            );
        } else {
            assert!(matches!(got, Err(ActError::Mismatch { .. })));
            assert!(signed.is_empty());
        }
    }
}

#[test]
fn each_read_asks_its_path() {
    let (origin, seen) = stub(Box::new(|_, _| (200, json!({"ok": true}).to_string())));
    let c = client(&origin);
    for got in [
        c.home(Some(ME), Some(5)),
        c.home(None, None),
        c.floor(),
        c.buddy_offer(),
        c.buddies(ME),
        c.board(None),
        c.ticket_buy("gates", Some(ME), Some("free")),
    ] {
        assert!(got.is_answer());
    }
    assert_eq!(
        seen.lock().unwrap().clone(),
        vec![
            format!("/api/home?wallet={ME}&since=5"),
            "/api/home".into(),
            "/api/agents/floor".into(),
            "/api/buddy".into(),
            format!("/api/buddy/{ME}"),
            "/api/munus?game=gates".into(),
            format!("/api/ticket/gates/prepare?rail=free&wallet={ME}"),
        ]
    );
    let c = client("http://127.0.0.1:9");
    assert!(c.home(Some("0x12"), None).is_refused());
    assert!(c.buddies("x").is_refused());
    assert!(c.board(Some("../x")).is_refused());
}
