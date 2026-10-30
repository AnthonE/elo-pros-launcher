//! Moving a player between a game's servers: the handoff a server signs and
//! the door the next one admits it at, against the shared fixture.
#![cfg(feature = "http")]

use std::sync::{Arc, Mutex};

use elo_sdk::{handoff_text, read_handoff, HandoffDoor, KeySource, Look, ServerKey};
use serde_json::{json, Value};

fn fx() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../testdata/handoff.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn carry(r: &Value) -> Vec<u8> {
    r["carry_utf8"].as_str().unwrap_or("").as_bytes().to_vec()
}

fn key(f: &Value) -> ServerKey {
    ServerKey::from_seed(s(&f["game"]), s(&f["key_id"]), &hex(s(&f["seed_hex"]))).unwrap()
}

fn pinned(f: &Value) -> HandoffDoor {
    HandoffDoor::pinned(
        s(&f["game"]),
        &[s(&f["to"])],
        &[(s(&f["key_id"]), s(&f["public_b64"]))],
    )
    .unwrap()
}

#[test]
fn the_text_and_the_token_match_the_fixture() {
    let f = fx();
    let k = key(&f);
    assert_eq!(k.public_b64(), s(&f["public_b64"]));
    let (now, exp) = (f["now"].as_u64().unwrap(), f["exp"].as_u64().unwrap());
    for c in f["cases"].as_array().unwrap() {
        let cb = carry(c);
        let cr = (!cb.is_empty()).then_some(cb.as_slice());
        let text = handoff_text(
            s(&f["game"]),
            s(&f["key_id"]),
            s(&c["player"]),
            s(&f["from"]),
            s(&f["to"]),
            s(&c["id"]),
            exp,
            cr,
        );
        assert_eq!(text, s(&c["text"]));
        let tok = k
            .handoff_at(
                s(&c["player"]),
                s(&f["from"]),
                s(&f["to"]),
                cr,
                exp - now,
                now,
                Some(s(&c["id"])),
            )
            .unwrap();
        assert_eq!(tok, s(&c["token"]));
        assert_eq!(read_handoff(&tok).unwrap().player, s(&c["player"]));
    }
}

#[test]
fn every_case_admits_once_then_every_refusal_answers_its_code() {
    let f = fx();
    let now = f["now"].as_u64().unwrap();
    let mut door = pinned(&f);
    for c in f["cases"].as_array().unwrap() {
        assert_eq!(
            door.admit(s(&c["token"]), &carry(c), now).unwrap().player,
            s(&c["player"])
        );
    }
    for r in f["refused"].as_array().unwrap() {
        let got = door.admit(s(&r["token"]), &carry(r), now).unwrap_err();
        assert_eq!(got.code, s(&r["code"]), "{r}");
    }
}

#[test]
fn a_refused_handoff_is_not_used_up_and_the_window_holds() {
    let f = fx();
    let (now, exp) = (f["now"].as_u64().unwrap(), f["exp"].as_u64().unwrap());
    let c = &f["cases"][2];
    let mut door = pinned(&f);
    assert_eq!(
        door.admit(s(&c["token"]), b"tampered", now)
            .unwrap_err()
            .code,
        "carry"
    );
    assert!(door.admit(s(&c["token"]), &carry(c), now).is_ok());
    let t = s(&f["cases"][0]["token"]);
    assert!(pinned(&f).admit(t, b"", exp + 29).is_ok());
    assert_eq!(
        pinned(&f).admit(t, b"", exp + 30).unwrap_err().code,
        "expired"
    );
}

#[test]
fn issuing_refuses_what_no_door_would_take() {
    let f = fx();
    let k = key(&f);
    let p = s(&f["cases"][0]["player"]);
    let (from, to) = (s(&f["from"]), s(&f["to"]));
    assert!(k.handoff("0xABC", from, to, None, 60).is_err());
    assert!(k.handoff(p, "no-port", to, None, 60).is_err());
    assert!(k.handoff(p, from, to, None, 121).is_err());
    assert!(k.handoff(p, from, to, Some(&[0u8; 4097]), 60).is_err());
    assert!(k
        .handoff_at(p, from, to, None, 60, 0, Some("short"))
        .is_err());
    assert!(pinned(&f)
        .admit_now(&k.handoff(p, from, to, None, 60).unwrap(), b"")
        .is_ok());
}

struct Fake {
    rows: Arc<Mutex<Option<Value>>>,
    calls: Arc<Mutex<u32>>,
}

impl KeySource for Fake {
    fn server_keys(&self, game: &str) -> Look<Value> {
        *self.calls.lock().unwrap() += 1;
        match self.rows.lock().unwrap().clone() {
            None => Look::CouldNotLook("unreachable".into()),
            Some(rows) => Look::Answer(json!({"game": game, "keys": rows})),
        }
    }
}

#[test]
fn a_door_reading_the_keys_caches_rotates_and_revokes() {
    let f = fx();
    let k = key(&f);
    let p = s(&f["cases"][0]["player"]).to_string();
    let (from, to) = (s(&f["from"]).to_string(), s(&f["to"]).to_string());
    let fresh = |t: u64| k.handoff_at(&p, &from, &to, None, 60, t, None).unwrap();
    let rows = |id: &str| Some(json!([{"id": id, "pub": s(&f["public_b64"])}]));
    let (cell, calls) = (Arc::new(Mutex::new(rows("old-1"))), Arc::new(Mutex::new(0)));
    let mut door = HandoffDoor::with_source(
        s(&f["game"]),
        &[&to],
        Box::new(Fake {
            rows: cell.clone(),
            calls: calls.clone(),
        }),
    );
    let mut t = f["now"].as_u64().unwrap();
    assert_eq!(
        door.admit(&fresh(t), b"", t).unwrap_err().code,
        "unknown_key"
    );
    assert_eq!(
        door.admit(&fresh(t), b"", t).unwrap_err().code,
        "unknown_key"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        1,
        "an unknown key does not hammer the origin"
    );
    *cell.lock().unwrap() = rows(s(&f["key_id"]));
    t += 60;
    assert!(
        door.admit(&fresh(t), b"", t).is_ok(),
        "a rotated key works on first use"
    );
    *cell.lock().unwrap() = Some(json!([]));
    t += 299;
    assert!(
        door.admit(&fresh(t), b"", t).is_ok(),
        "cached until the refresh"
    );
    t += 1;
    assert_eq!(
        door.admit(&fresh(t), b"", t).unwrap_err().code,
        "unknown_key",
        "revoked"
    );
}

#[test]
fn could_not_look_is_its_own_answer() {
    let f = fx();
    let fake = Fake {
        rows: Arc::new(Mutex::new(None)),
        calls: Arc::new(Mutex::new(0)),
    };
    let mut door = HandoffDoor::with_source(s(&f["game"]), &[s(&f["to"])], Box::new(fake));
    let got = door
        .admit(s(&f["cases"][0]["token"]), b"", f["now"].as_u64().unwrap())
        .unwrap_err();
    assert_eq!(got.code, "no_keys");
}
