//! A game server's half: the sign-in check, the roster sweep and the nonce,
//! against a local stub server that answers the way the meter does.
#![cfg(feature = "http")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use elo_sdk::{new_nonce, Client, Look, SignInCheck, ROSTER_PAGE};
use serde_json::{json, Value};

const SOLW: &str = "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB";

type Route = Box<dyn Fn(&str, &Value) -> (u16, String) + Send + Sync>;
type Seen = Arc<Mutex<Vec<(String, Value)>>>;

/// Serve `route` on a local port and record every request's path and body.
fn stub(route: Route) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
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
            log.lock().unwrap().push((path.clone(), body.clone()));
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

fn case(i: usize) -> Value {
    let doc: Value =
        serde_json::from_str(include_str!("../../testdata/signin_check.json")).unwrap();
    doc["cases"][i].clone()
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap().to_string()
}

fn w(i: u32) -> String {
    format!("0x{i:040x}")
}

#[test]
fn check_pass_sends_pass_game_domain_and_nonce() {
    let reply = json!({"ok": true, "player": "acct_aaaaaaaaaaaaaaaaaaaa", "account": "acct_aaaaaaaaaaaaaaaaaaaa",
                       "wallet": null, "name": "Bob", "named": true, "game": "my-game",
                       "domain": "play.example", "exp": 1900000000u64})
    .to_string();
    let (origin, seen) = stub(Box::new(move |path, _| match path {
        "/api/account/play/check" => (200, reply.clone()),
        _ => (404, r#"{"error":"no"}"#.into()),
    }));
    let nonce = "a".repeat(32);
    let c = client(&origin);
    let got = c.check_pass("pass.sig", "my-game", "play.example", &nonce);
    let Look::Answer(p) = got else {
        panic!("{got:?}")
    };
    assert_eq!(p.player, "acct_aaaaaaaaaaaaaaaaaaaa");
    assert_eq!(p.name, "Bob");
    assert_eq!(p.exp, 1900000000);
    let (path, body) = seen.lock().unwrap()[0].clone();
    assert_eq!(path, "/api/account/play/check");
    assert_eq!(
        body,
        json!({"token": "pass.sig", "game": "my-game", "domain": "play.example", "nonce": nonce})
    );
    assert!(matches!(
        c.check_pass("pass.sig", "my-game", "play.example:443", &nonce),
        Look::Refused(_)
    ));
    assert!(matches!(
        c.check_pass("pass.sig", "my-game", "play.example", &"a".repeat(65)),
        Look::Refused(_)
    ));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn check_sign_in_sends_message_signature_domain_and_nonce() {
    let evm = case(0);
    let addr = s(&evm["expect"], "address");
    let reply = json!({"ok": true, "address": addr, "chain": "evm", "chain_id": "4663",
                       "domain": "game.example", "nonce": evm["nonce"], "uri": "https://game.example",
                       "issued_at": "x"})
    .to_string();
    let (origin, seen) = stub(Box::new(move |path, _| match path {
        "/api/signin/check" => (200, reply.clone()),
        _ => (404, r#"{"error":"no"}"#.into()),
    }));
    let mut check = SignInCheck::new(&s(&evm, "domain"), &s(&evm, "nonce"));
    check.chain_id = Some("4663".into());
    check.max_age_s = Some(300);
    let got = client(&origin).check_sign_in(&s(&evm, "message"), &s(&evm, "signature"), &check);
    let Look::Answer(signin) = got else {
        panic!("{got:?}")
    };
    assert_eq!(signin.address, addr);
    assert_eq!(signin.chain, "evm");
    let (path, body) = seen.lock().unwrap()[0].clone();
    assert_eq!(path, "/api/signin/check");
    assert_eq!(
        body,
        json!({"message": evm["message"], "signature": evm["signature"], "domain": evm["domain"],
               "nonce": evm["nonce"], "chain_id": "4663", "max_age_s": 300})
    );
}

#[test]
fn a_failed_check_is_refused_with_the_field() {
    let sol = case(1);
    let (origin, _) = stub(Box::new(|_, _| {
        (
            401,
            r#"{"ok":false,"code":"stale","error":"this sign-in was made 900s ago"}"#.into(),
        )
    }));
    let got = client(&origin).check_sign_in(
        &s(&sol, "message"),
        &s(&sol, "signature"),
        &SignInCheck::new(&s(&sol, "domain"), &s(&sol, "nonce")),
    );
    let Look::Refused(r) = got else {
        panic!("{got:?}")
    };
    assert_eq!(r.status, 401);
    assert_eq!(r.body["code"], "stale");
}

#[test]
fn an_unreachable_chain_is_could_not_look() {
    let evm = case(0);
    let (origin, _) = stub(Box::new(|_, _| {
        (503, r#"{"ok":false,"reachable":false,"code":"unreachable","error":"could not reach the chain"}"#.into())
    }));
    let got = client(&origin).check_sign_in(
        &s(&evm, "message"),
        &s(&evm, "signature"),
        &SignInCheck::new(&s(&evm, "domain"), &s(&evm, "nonce")),
    );
    assert!(
        matches!(&got, Look::CouldNotLook(why) if why.contains("could not reach the chain")),
        "{got:?}"
    );
}

#[test]
fn a_200_that_is_not_a_checked_sign_in_is_could_not_look() {
    let evm = case(0);
    let (origin, _) = stub(Box::new(|_, _| (200, r#"{"ok":true}"#.into())));
    let got = client(&origin).check_sign_in(
        &s(&evm, "message"),
        &s(&evm, "signature"),
        &SignInCheck::new(&s(&evm, "domain"), &s(&evm, "nonce")),
    );
    assert!(got.is_could_not_look(), "{got:?}");
}

#[test]
fn check_sign_in_refuses_before_asking_without_a_domain_or_nonce() {
    let evm = case(0);
    let (origin, seen) = stub(Box::new(|_, _| (200, r#"{"ok":true}"#.into())));
    let c = client(&origin);
    let (m, sig) = (s(&evm, "message"), s(&evm, "signature"));
    let mut aged = SignInCheck::new("game.example", "Q8n2Vx41abc");
    aged.max_age_s = Some(0);
    for check in [
        SignInCheck::new("", "Q8n2Vx41abc"),
        SignInCheck::new("game.example", ""),
        SignInCheck::new("https://game.example", "Q8n2Vx41abc"),
        SignInCheck::new("game.example/play", "Q8n2Vx41abc"),
        SignInCheck::new("game.example:99999x", "Q8n2Vx41abc"),
        SignInCheck::new("game.example", "550e8400-e29b-41d4"),
        aged,
    ] {
        let got = c.check_sign_in(&m, &sig, &check);
        assert!(
            matches!(&got, Look::Refused(r) if r.status == 0),
            "{check:?} {got:?}"
        );
    }
    assert!(c
        .check_sign_in("", &sig, &SignInCheck::new("game.example", "Q8n2Vx41abc"))
        .is_refused());
    assert!(seen.lock().unwrap().is_empty(), "nothing was sent");
}

#[test]
fn check_roster_pages_the_roster_and_keys_every_wallet() {
    let mut roster: Vec<String> = (1..=(ROSTER_PAGE as u32 + 5)).map(w).collect();
    roster.push(w(1).to_uppercase().replace("0X", "0x"));
    roster.push(SOLW.into());
    let three = w(3);
    let (origin, seen) = stub(Box::new(move |_, body| {
        let mut rows = serde_json::Map::new();
        for x in body["wallets"].as_array().unwrap() {
            let x = x.as_str().unwrap();
            rows.insert(x.into(), json!({"entitled": x != three}));
        }
        (200, json!({"game": "gates", "wallets": rows}).to_string())
    }));
    let refs: Vec<&str> = roster.iter().map(String::as_str).collect();
    let got = client(&origin).check_roster("gates", &refs);
    let Look::Answer(r) = got else {
        panic!("{got:?}")
    };
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].1["wallets"].as_array().unwrap().len(), ROSTER_PAGE);
    assert!(seen.iter().all(|(p, _)| p == "/api/ticket/gates/check"));
    assert_eq!(
        r.entitled.len(),
        ROSTER_PAGE + 6,
        "a repeated wallet is swept once"
    );
    assert_eq!(r.entitled[&w(3)], Some(false));
    assert_eq!(r.entitled[&w(4)], Some(true));
    assert_eq!(r.entitled[SOLW], Some(true));
    assert!(r.ticketed);
}

#[test]
fn a_page_that_could_not_be_read_is_none_never_a_kick() {
    let calls = Arc::new(Mutex::new(0));
    let n = Arc::clone(&calls);
    let (origin, _) = stub(Box::new(move |_, body| {
        let mut k = n.lock().unwrap();
        *k += 1;
        if *k == 2 {
            return (502, r#"{"error":"rpc down"}"#.into());
        }
        let rows: serde_json::Map<String, Value> = body["wallets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (x.as_str().unwrap().to_string(), json!({"entitled": false})))
            .collect();
        (200, json!({"game": "gates", "wallets": rows}).to_string())
    }));
    let roster: Vec<String> = (1..=(ROSTER_PAGE as u32 + 2)).map(w).collect();
    let refs: Vec<&str> = roster.iter().map(String::as_str).collect();
    let Look::Answer(r) = client(&origin).check_roster("gates", &refs) else {
        panic!("not an answer")
    };
    assert_eq!(r.entitled[&w(1)], Some(false));
    assert_eq!(r.entitled[&w(ROSTER_PAGE as u32 + 1)], None);
    assert_eq!(r.entitled[&w(ROSTER_PAGE as u32 + 2)], None);
}

#[test]
fn a_row_left_out_or_unread_is_none() {
    let one = w(1);
    let (origin, _) = stub(Box::new(move |_, _| {
        (
            200,
            json!({"game": "gates", "wallets": {one.clone(): {"entitled": null}}}).to_string(),
        )
    }));
    let (a, b) = (w(1), w(2));
    let Look::Answer(r) = client(&origin).check_roster("gates", &[&a, &b]) else {
        panic!("not an answer")
    };
    assert_eq!(r.entitled[&a], None);
    assert_eq!(r.entitled[&b], None);
}

#[test]
fn no_page_read_is_could_not_look_and_a_free_title_is_everyones() {
    let (origin, _) = stub(Box::new(|_, _| (503, r#"{"error":"down"}"#.into())));
    let a = w(1);
    assert!(client(&origin)
        .check_roster("gates", &[&a])
        .is_could_not_look());
    let (origin, _) = stub(Box::new(|_, _| {
        (
            200,
            r#"{"game":"free-game","ticketed":false,"every_wallet_entitled":true}"#.into(),
        )
    }));
    let Look::Answer(r) = client(&origin).check_roster("free-game", &[&a, SOLW]) else {
        panic!("not an answer")
    };
    assert!(!r.ticketed);
    assert_eq!(r.entitled[&a], Some(true));
    assert_eq!(r.entitled[SOLW], Some(true));
}

#[test]
fn a_bad_roster_is_refused_before_asking() {
    let (origin, seen) = stub(Box::new(|_, _| (200, "{}".into())));
    let c = client(&origin);
    let a = w(1);
    assert!(matches!(c.check_roster("gates", &[]), Look::Refused(r) if r.status == 0));
    assert!(matches!(c.check_roster("gates", &[&a, "0xnope"]), Look::Refused(r) if r.status == 0));
    assert!(matches!(c.check_roster("Not A Slug", &[&a]), Look::Refused(r) if r.status == 0));
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn owns_takes_a_solana_wallet_as_given() {
    let (origin, seen) = stub(Box::new(|_, _| {
        (
            200,
            json!({"game": "gates", "wallet": SOLW, "entitled": false}).to_string(),
        )
    }));
    assert_eq!(client(&origin).owns("gates", SOLW), Look::Answer(false));
    assert_eq!(
        seen.lock().unwrap()[0].0,
        format!("/api/ticket/gates/of/{SOLW}")
    );
}

#[test]
fn new_nonce_is_eip4361s_alphabet_the_length_asked_for_and_fresh() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..200 {
        let n = new_nonce(16).unwrap();
        assert_eq!(n.len(), 16);
        assert!(n.bytes().all(|c| c.is_ascii_alphanumeric()), "{n}");
        seen.insert(n);
    }
    assert_eq!(seen.len(), 200);
    assert_eq!(new_nonce(8).unwrap().len(), 8);
    assert!(new_nonce(7).is_none());
    assert!(new_nonce(129).is_none());
}
