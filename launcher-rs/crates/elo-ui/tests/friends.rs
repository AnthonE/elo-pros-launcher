//! What the Friends window OFFERS must match what the list SAYS.
//!
//! One `#[test]`, every window built in sequence on one thread — FLTK owns
//! the first thread that touches it, and `shards.rs` carries the reasoning.

use elo_net::friends::{self, View};
use elo_ui::windows::{self, Friends};
use fltk::prelude::*;
use serde_json::{json, Value};

const ME: &str = "0x00000000000000000000000000000000000000aa";
const ASH: &str = "0x00000000000000000000000000000000000000bb";
const BO: &str = "0x00000000000000000000000000000000000000cc";

fn row(wallet: &str, name: &str, join: Value) -> Value {
    json!({
        "wallet": wallet, "since": 1, "name": name, "pic": null, "agent": false,
        "online": true, "last_online": 2,
        "playing": {"game": "gates", "title": "Gates", "server": "eu-1",
                    "server_name": "Gates EU", "since": 1, "join": join},
    })
}

fn view(rows: Vec<Value>, reachable: bool) -> View {
    friends::parse(&json!({
        "wallet": ME, "friends": rows,
        "asks_in": [{"wallet": BO, "at": 1, "name": "Bo"}], "asks_out": [],
        "presence": {"reachable": reachable, "why": if reachable { Value::Null }
                     else { json!("ledger down") }},
    }))
    .expect("a list the contract describes must parse")
}

#[test]
fn the_window_offers_exactly_what_the_list_states() {
    // ── a friend with a link gets Join and Invite; one without gets neither ─
    let w = windows::friends(
        &Friends::Listed {
            view: view(vec![
                row(ASH, "Ash", json!("elo://join/gates/eu-1.example.test:61234")),
                row(BO, "Bo", Value::Null),
            ], true),
        },
        100,
    );
    assert_eq!(w.rows.len(), 1, "only a friend whose server is known can be joined");
    assert_eq!(w.rows[0].wallet, ASH);
    assert_eq!(w.rows[0].link, "elo://join/gates/eu-1.example.test:61234");
    let back = elo_depot::deeplink::parse(&w.rows[0].link).expect("the drawn link must parse");
    assert_eq!(back.addr, "eu-1.example.test:61234");
    assert!(w.rows[0].join.active() && w.rows[0].copy.active());
    assert_eq!(w.rows[0].join.label(), "Join");
    assert_eq!(w.rows[0].copy.label(), "Invite");

    // ── presence nobody could read offers nothing to follow ─────────────────
    //
    // A link in a document whose ledger did not read describes a place nobody
    // measured, so it is not pressable.
    let w = windows::friends(
        &Friends::Listed {
            view: view(vec![row(ASH, "Ash", json!("elo://join/gates/eu-1.example.test:61234"))],
                       false),
        },
        100,
    );
    assert!(w.rows.is_empty(), "an unread ledger must offer no Join");

    // ── the rowless states are four different windows ───────────────────────
    for (what, state) in [
        ("no wallet", Friends::NoWallet),
        ("unreachable", Friends::Unreadable {
            url: friends::url("https://elo.test", ME),
            why: "dns error".into(),
            reachable: false,
        }),
        ("unparseable", Friends::Unreadable {
            url: friends::url("https://elo.test", ME),
            why: "the friends list names no wallet".into(),
            reachable: true,
        }),
        ("empty", Friends::Listed { view: view(vec![], true) }),
    ] {
        let w = windows::friends(&state, 0);
        assert!(w.rows.is_empty(), "{what}: a rowless state must offer no Join");
        assert!(w.refresh.active(), "{what}: Refresh is always there");
        assert!(w.window.width() > 0, "{what}: no width");
        assert!(w.window.height() > 100, "{what}: no body");
    }

    // ── a hostile row cannot take the window down ───────────────────────────
    let mut hostile = row(ASH, &"W".repeat(96), json!("elo://join/gates/eu-1.example.test:61234"));
    hostile["playing"]["title"] = json!("あ@".repeat(40));
    hostile["playing"]["server_name"] = json!("@2> x");
    let w = windows::friends(&Friends::Listed { view: view(vec![hostile], true) }, 0);
    assert_eq!(w.rows.len(), 1, "a hostile row must still be drawn");
    assert!(w.rows[0].join.active());

    // ── a long list scrolls rather than growing past the desktop ────────────
    let many: Vec<Value> = (0..40)
        .map(|i| row(&format!("0x{:040x}", i + 1), &format!("friend {i}"), Value::Null))
        .collect();
    let w = windows::friends(&Friends::Listed { view: view(many, true) }, 0);
    assert!(w.window.height() <= 520 + 160, "the well is a viewport, not a cut");
    assert_eq!(w.acts.len(), 40 + 2, "every row keeps its buttons, scrolled to or not");

    // ── the changes: each row offers exactly the ones its state allows ──────
    //
    // The list the meter serves, from the fixture its own suite writes: three
    // friends, one buddy, an ask in and an ask out.
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../sdk/testdata/friends_view.json")).unwrap();
    let full = friends::parse(&fixture).expect("the fixture parses");
    let w = windows::friends(&Friends::Listed { view: full.clone() }, 1_761_003_600);
    let acts: Vec<(&str, &str, String)> =
        w.acts.iter().map(|a| (a.op, a.wallet.as_str(), a.button.label())).collect();
    let rook = "0x6666666666666666666666666666666666666666";
    let asked = "0x7777777777777777777777777777777777777777";
    assert_eq!(acts, vec![
        ("accept", rook, "Accept".to_string()),
        ("decline", rook, "Decline".to_string()),
        ("remove", "0x2222222222222222222222222222222222222222", "Remove".to_string()),
        ("remove", "0x3333333333333333333333333333333333333333", "Remove".to_string()),
        ("remove", "0x4444444444444444444444444444444444444444", "Remove".to_string()),
        ("cancel", asked, "Cancel".to_string()),
    ], "asks in, friends, then your own asks — and no change on a buddy");
    assert_eq!(w.acts[0].called, "rook", "a change says who it was about by name");
    let buddy = "0x5555555555555555555555555555555555555555";
    assert!(w.acts.iter().all(|a| a.wallet != buddy), "a buddy is never removed here");
    // Join and Invite: Ash and the buddy have a server with a link; Doctor
    // Wood is playing where no link is known, and the third is offline.
    let joins: Vec<(&str, bool)> = w.rows.iter().map(|r| (r.wallet.as_str(), r.buddy)).collect();
    assert_eq!(joins, vec![("0x2222222222222222222222222222222222222222", false), (buddy, true)]);
    assert!(w.acts.iter().all(|a| a.button.active()));
    // The add box, empty and ready, with the list's own wallet to act as.
    assert!(w.add.active() && w.add_input.value().is_empty());
    assert_eq!(w.add.label(), "Ask");
    assert_eq!(w.wallet.as_deref(), Some("0x1111111111111111111111111111111111111111"));
    assert!(w.status.label().is_empty(), "nothing has happened yet");

    // A list we could not read offers no change on a row — but asking still
    // works, as it does with no wallet (where it is finished on the website).
    for state in [Friends::NoWallet, Friends::Unreadable {
        url: friends::url("https://elo.test", ME), why: "dns error".into(), reachable: false }] {
        let w = windows::friends(&state, 0);
        assert!(w.acts.is_empty() && w.wallet.is_none());
        assert!(w.add.active());
    }

    // An older meter, with no `buddies` at all, draws the same rows minus one.
    let mut old = fixture.clone();
    old.as_object_mut().unwrap().remove("buddies");
    let w = windows::friends(&Friends::Listed { view: friends::parse(&old).unwrap() }, 0);
    assert_eq!(w.acts.len(), 6);
    assert_eq!(w.rows.len(), 1);
}
