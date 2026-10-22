//! A friends change made from the window — ask, accept, decline, cancel,
//! remove — with no window and no key: the origin's routes behind a seam, and
//! the one rule every change passes before anything is signed.
//!
//! The shape of [`crate::profile`]. `meter/friends.py` prepares the exact text
//! (`POST /api/friends/prepare`), the account signs it, and the signed change
//! goes back (`POST /api/friends/act`) — the dev desk's three beats, as
//! `elo friends` takes them. `wiring::change_friend` is the flow around this:
//! which signer, which wallet a typed name means, and the passphrase prompt.
//!
//! **What is signed is what was asked for.** [`checked_text`] rebuilds the
//! text from the change the player pressed (`elo_net::friends::sign_text`, the
//! CLI's builder too) and refuses an origin that offers any other bytes —
//! another op, another wallet — before a signer is ever asked.

use elo_net::friends::{self as roster, View};
use elo_net::{Fetched, Net};
use serde_json::{json, Value};

/// The origin's friends routes, injectable so the flow runs with no network.
pub trait FriendsHttp {
    /// `GET /api/who` — the wallet a typed name stands for.
    fn who(&self, host: &str, handle: &str) -> Fetched<elo_hive::who::Identity>;
    /// `POST /api/friends/prepare` — the exact text to sign, and its `at`.
    fn prepare(&self, host: &str, body: &Value) -> Fetched<Value>;
    /// `POST /api/friends/act` — the signed change, and the list after it.
    fn act(&self, host: &str, body: &Value) -> Fetched<Value>;
}

pub struct RealFriendsHttp;

impl FriendsHttp for RealFriendsHttp {
    fn who(&self, host: &str, handle: &str) -> Fetched<elo_hive::who::Identity> {
        elo_hive::who::resolve(&Net::new(), host, handle)
    }
    fn prepare(&self, host: &str, body: &Value) -> Fetched<Value> {
        Net::new().post_json_told(&format!("{host}/api/friends/prepare"), body)
    }
    fn act(&self, host: &str, body: &Value) -> Fetched<Value> {
        Net::new().post_json_told(&format!("{host}/api/friends/act"), body)
    }
}

/// Said when the origin offers a text other than the change asked for.
pub const MISMATCH: &str =
    "the origin prepared a different change from the one asked for — nothing was signed";

/// The text to sign and its `at`, or why it must not be signed.
///
/// Rebuilt from `me`, `op` and `other` — never taken on the origin's word —
/// and compared byte for byte with the `sign_this` the origin prepared.
pub fn checked_text(
    me: &str,
    op: &str,
    other: &str,
    prepared: &Value,
) -> Result<(String, i64), String> {
    let key = roster::op_key(op).ok_or_else(|| format!("{op:?} is not a friends change"))?;
    let offered = prepared
        .get("sign_this")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or("the origin's reply named nothing to sign")?;
    let at = prepared
        .get("at")
        .and_then(Value::as_i64)
        .filter(|at| *at >= 0)
        .ok_or("the origin's reply carries no `at` to sign")?;
    match roster::sign_text(me, op, key, other, at as u64) {
        Some(text) if text == offered => Ok((text, at)),
        _ => Err(MISMATCH.into()),
    }
}

/// Prepare, check, sign, submit — one change, as `me`, about `other`.
///
/// `sign` is asked only once [`checked_text`] has passed, so an origin that
/// will not prepare, or prepares something else, costs no prompt and no
/// signature. It returns the `0x…` signature, or why there is none.
pub fn submit(
    http: &dyn FriendsHttp,
    host: &str,
    me: &str,
    op: &str,
    other: &str,
    sign: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Result<View, String> {
    let key = roster::op_key(op).ok_or_else(|| format!("{op:?} is not a friends change"))?;
    let args = json!({ key: other });
    let prepared = told(
        host,
        http.prepare(host, &json!({"wallet": me, "op": op, "args": args})),
    )?;
    let (text, at) = checked_text(me, op, other, &prepared)?;
    let signature = sign(&text)?;
    if signature.is_empty() {
        return Err("the signer returned no signature".into());
    }
    let done = told(
        host,
        http.act(
            host,
            &json!({"wallet": me, "op": op, "args": args, "at": at, "signature": signature}),
        ),
    )?;
    done.get("view")
        .and_then(|v| roster::parse(v).ok())
        .ok_or_else(|| {
            "the change was made, and the list that came back did not read — Refresh shows it"
                .into()
        })
}

/// A refusal in the origin's own words, kept apart from "could not reach".
fn told(host: &str, got: Fetched<Value>) -> Result<Value, String> {
    if got.ok {
        return Ok(got.value.unwrap_or(Value::Null));
    }
    Err(if got.reachable {
        got.why
    } else {
        format!("could not reach {host} — {}", got.why)
    })
}

/// The heading a change's prompt carries: what pressing the button does.
pub fn heading(op: &str) -> &'static str {
    match op {
        "ask" => "Ask to be friends",
        "accept" => "Accept a friend",
        "decline" => "Decline an ask",
        "cancel" => "Take back your ask",
        "remove" => "Remove a friend",
        "block" => "Block a wallet",
        "unblock" => "Unblock a wallet",
        _ => "Change your friends",
    }
}

/// What happened, in a line for the window's status. An ask to someone who
/// already asked you is a friendship at once, so it reads two ways. `other`
/// is what was asked about — a wallet, or the name that was typed.
pub fn done_line(op: &str, called: &str, view: &View, other: &str) -> String {
    let other = other.trim();
    let friends_now = view.friends.iter().any(|f| {
        f.wallet.eq_ignore_ascii_case(other)
            || f.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(other))
    });
    match op {
        "ask" if friends_now => format!("you and {called} are friends now"),
        "ask" => format!("asked {called} — friends lists are public, like playtime"),
        "accept" => format!("you and {called} are friends now"),
        "decline" => format!("declined {called}"),
        "cancel" => format!("took back your ask to {called}"),
        "remove" => format!("{called} is off your list"),
        "block" => format!("blocked {called}"),
        "unblock" => format!("unblocked {called}"),
        _ => "done".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const ME: &str = "0x1111111111111111111111111111111111111111";
    const ROOK: &str = "0x6666666666666666666666666666666666666666";
    const VIEW: &str = include_str!("../../../../sdk/testdata/friends_view.json");

    /// Answers prepare from a script and records every call; `act` answers
    /// the fixture list, and panics when the test says nothing may be sent.
    struct Desk {
        prepared: Fetched<Value>,
        may_act: bool,
        sent: RefCell<Vec<(&'static str, Value)>>,
    }

    impl FriendsHttp for Desk {
        fn who(&self, _: &str, _: &str) -> Fetched<elo_hive::who::Identity> {
            panic!("a wallet needs no lookup")
        }
        fn prepare(&self, _: &str, body: &Value) -> Fetched<Value> {
            self.sent.borrow_mut().push(("prepare", body.clone()));
            self.prepared.clone()
        }
        fn act(&self, _: &str, body: &Value) -> Fetched<Value> {
            assert!(
                self.may_act,
                "a change was sent that should have stopped first"
            );
            self.sent.borrow_mut().push(("act", body.clone()));
            let view: Value = serde_json::from_str(VIEW).unwrap();
            Fetched::good(json!({"ok": true, "op": "accept", "view": view}))
        }
    }

    fn prepared(text: &str, at: i64) -> Fetched<Value> {
        Fetched::good(json!({"sign_this": text, "at": at, "op": "accept", "args": {"from": ROOK}}))
    }

    fn real_text(op: &str, other: &str, at: u64) -> String {
        roster::sign_text(ME, op, roster::op_key(op).unwrap(), other, at).unwrap()
    }

    fn never_sign() -> impl FnMut(&str) -> Result<String, String> {
        |text: &str| panic!("the signer was asked to sign {text:?}")
    }

    #[test]
    fn a_mismatched_text_is_refused_and_never_reaches_the_signer() {
        let at = 1_761_003_601;
        for (why, text) in [
            ("another op", real_text("block", ROOK, at)),
            (
                "another wallet",
                real_text("accept", "0x7777777777777777777777777777777777777777", at),
            ),
            ("another at", real_text("accept", ROOK, at + 1)),
            (
                "a trailing byte",
                format!("{}\n", real_text("accept", ROOK, at)),
            ),
            ("not a play text at all", "sign me".into()),
        ] {
            let desk = Desk {
                prepared: prepared(&text, at as i64),
                may_act: false,
                sent: RefCell::default(),
            };
            let e =
                submit(&desk, "https://x", ME, "accept", ROOK, &mut never_sign()).expect_err(why);
            assert_eq!(e, MISMATCH, "{why}");
            assert_eq!(
                desk.sent.borrow().len(),
                1,
                "{why}: prepared, and nothing after"
            );
        }
        // The rule on its own, for the same inputs.
        assert_eq!(
            checked_text(
                ME,
                "accept",
                ROOK,
                &prepared(&real_text("block", ROOK, at), at as i64)
                    .value
                    .unwrap()
            ),
            Err(MISMATCH.to_string())
        );
    }

    #[test]
    fn a_reply_with_nothing_to_sign_or_a_refusal_costs_no_signature() {
        for got in [
            Fetched::good(json!({"at": 5})),
            Fetched::good(json!({"sign_this": real_text("accept", ROOK, 5)})),
            Fetched::good(json!({"sign_this": real_text("accept", ROOK, 5), "at": -5})),
            Fetched::unreachable("dns error"),
            Fetched {
                value: Some(json!({"error": "no ask from that wallet"})),
                ok: false,
                reachable: true,
                status: Some(404),
                why: "no ask from that wallet".into(),
            },
        ] {
            let desk = Desk {
                prepared: got,
                may_act: false,
                sent: RefCell::default(),
            };
            let e = submit(&desk, "https://x", ME, "accept", ROOK, &mut never_sign()).unwrap_err();
            assert!(!e.is_empty());
        }
        let desk = Desk {
            prepared: Fetched::unreachable("dns error"),
            may_act: false,
            sent: RefCell::default(),
        };
        let e = submit(&desk, "https://x", ME, "accept", ROOK, &mut never_sign()).unwrap_err();
        assert_eq!(e, "could not reach https://x — dns error");
    }

    #[test]
    fn a_matching_text_is_signed_once_and_the_list_comes_back() {
        let at = 1_761_003_601;
        let text = real_text("accept", ROOK, at);
        let desk = Desk {
            prepared: prepared(&text, at as i64),
            may_act: true,
            sent: RefCell::default(),
        };
        let signed = RefCell::new(Vec::new());
        let mut sign = |t: &str| {
            signed.borrow_mut().push(t.to_string());
            Ok(format!("0x{}", "ab".repeat(65)))
        };
        let view = submit(&desk, "https://x", ME, "accept", ROOK, &mut sign).unwrap();
        assert_eq!(
            signed.borrow().as_slice(),
            [text],
            "exactly the rebuilt text, once"
        );
        assert_eq!(
            view.buddies.len(),
            1,
            "the list after the change, as the origin sent it"
        );
        let sent = desk.sent.borrow();
        assert_eq!(
            sent[0].1,
            json!({"wallet": ME, "op": "accept", "args": {"from": ROOK}})
        );
        assert_eq!(sent[1].0, "act");
        assert_eq!(
            sent[1].1["args"],
            json!({"from": ROOK}),
            "the act sends the args prepared"
        );
        assert_eq!(sent[1].1["at"], at);
        assert_eq!(sent[1].1["signature"], format!("0x{}", "ab".repeat(65)));
    }

    #[test]
    fn a_signer_that_says_no_stops_the_change() {
        let at = 7;
        let desk = Desk {
            prepared: prepared(&real_text("accept", ROOK, at), at as i64),
            may_act: false,
            sent: RefCell::default(),
        };
        let mut refuse = |_: &str| Err("Cancelled — nothing was signed.".to_string());
        let e = submit(&desk, "https://x", ME, "accept", ROOK, &mut refuse).unwrap_err();
        assert!(e.contains("Cancelled"), "{e}");
        let mut empty = |_: &str| Ok(String::new());
        assert!(submit(&desk, "https://x", ME, "accept", ROOK, &mut empty).is_err());
    }

    #[test]
    fn every_op_has_a_heading_and_a_line() {
        let view = roster::parse(&serde_json::from_str(VIEW).unwrap()).unwrap();
        for (op, _) in roster::OPS {
            assert_ne!(heading(op), "Change your friends", "{op}");
            assert_ne!(done_line(op, "rook", &view, ROOK), "done", "{op}");
        }
        let ash = "0x2222222222222222222222222222222222222222";
        assert_eq!(
            done_line("ask", "ash", &view, ash),
            "you and ash are friends now"
        );
        assert_eq!(
            done_line("ask", "Ash", &view, " Ash "),
            "you and Ash are friends now",
            "a typed name finds the friend it made"
        );
        assert!(done_line("ask", "rook", &view, ROOK).starts_with("asked rook"));
    }
}
