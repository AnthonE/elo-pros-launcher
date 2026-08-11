//! What the Servers window OFFERS must match what the shard list SAYS.
//!
//! The same rule `rows.rs` exists for, on the surface where getting it wrong
//! is worse. A Games row that offers the wrong button costs a player a stale
//! build; a Servers row that offers the wrong button sends them somewhere.
//!
//! ## Why this is one `#[test]` and not six
//!
//! **FLTK asserts a single UI thread** (`app::is_ui_thread`), and the thread
//! that touches it first owns it for the process. Cargo runs `#[test]`s on a
//! thread pool, so six window tests in one binary means five of them panic
//! inside `Window::new` — and *which* five varies per run, which is the worst
//! possible shape: a flaky failure that looks like a bug in the window.
//!
//! `credit.rs` builds a window and does not hit this only because it is the
//! only test in its binary that builds one. Rather than depend on that, every
//! window this file builds is built in sequence on one thread. The sections
//! are named in their assertion messages, so a failure still says which claim
//! broke.

use fltk::prelude::*;
use scry_depot::shardlist::Shard;
use scry_ui::windows::{self, Shards};

fn shard(id: &str, players: Option<u32>, max_players: Option<u32>) -> Shard {
    Shard {
        id: id.into(),
        name: format!("Gates {id}"),
        addr: format!("{id}.example.test:61234"),
        players,
        max_players,
        map: None,
        ping_ms: None,
        status_url: None,
    }
}

fn listed(rows: Vec<Shard>) -> Shards {
    Shards::Listed { url: "https://example.test/servers.json".into(), rows }
}

#[test]
fn the_window_offers_exactly_what_the_list_states() {
    // ── every listed shard gets a control, and an invite that parses ────────
    //
    // A shard drawn with no Join is a server a player can see and cannot
    // reach; an invite the parser would refuse is a link that fails for the
    // friend rather than for the sender.
    let w = windows::servers(
        "gates",
        &listed(vec![shard("eu-1", Some(47), Some(100)), shard("us-1", None, None)]),
    );
    assert_eq!(w.rows.len(), 2, "every listed shard must get a control");
    assert_eq!(w.rows[0].addr, "eu-1.example.test:61234");
    assert_eq!(
        w.rows[0].link, "scry://join/gates/eu-1.example.test:61234",
        "the invite must be the canonical form"
    );
    let back = scry_depot::deeplink::parse(&w.rows[0].link).expect("the drawn link must parse");
    assert_eq!(back.addr, w.rows[0].addr, "the drawn link must carry the row's address");
    assert_eq!(back.slug, "gates");

    // ── a full shard is drawn and not pressable ─────────────────────────────
    //
    // Dropping it would make a busy server look like one that does not exist,
    // which is the opposite of what a browser is for.
    let w = windows::servers(
        "gates",
        &listed(vec![shard("eu-1", Some(100), Some(100)), shard("eu-2", Some(99), Some(100))]),
    );
    assert_eq!(w.rows.len(), 2, "a full shard must still be drawn");
    assert!(!w.rows[0].join.active(), "a full shard must not be joinable");
    assert!(w.rows[1].join.active(), "a shard with room must be joinable");
    assert!(
        !w.rows[0].link.is_empty(),
        "a full shard must still be invitable — sending a friend a link to a \
         server you cannot get into right now is normal"
    );

    // ── an unknown count is never treated as full ───────────────────────────
    //
    // `?` must not read as closed. A shard nobody could measure is joinable
    // and the door decides.
    let w = windows::servers("gates", &listed(vec![shard("eu-1", None, None)]));
    assert!(w.rows[0].join.active(), "an unmeasured shard must stay joinable");
    assert!(!shard("eu-1", None, None).full());
    assert_eq!(shard("eu-1", None, None).population(), "?");
    // A measured zero is a measurement, and must not render as unmeasured.
    assert_eq!(shard("eu-1", Some(0), Some(100)).population(), "0/100");

    // ── the four rowless states are four different windows ──────────────────
    //
    // None draws a control, which is the easy half. The half that matters is
    // that a player can tell "nobody is playing" from "we could not look" —
    // the repo's own `reachable`-is-not-`empty` trap, arriving at a window.
    for (what, state) in [
        ("unpublished", Shards::Unpublished),
        ("unreachable", Shards::Unreadable {
            url: "https://example.test/servers.json".into(),
            why: "dns error".into(),
            reachable: false,
        }),
        ("unparseable", Shards::Unreadable {
            url: "https://example.test/servers.json".into(),
            why: "the shard list did not parse".into(),
            reachable: true,
        }),
        ("empty", listed(vec![])),
    ] {
        let w = windows::servers("gates", &state);
        assert!(w.rows.is_empty(), "{what}: a rowless state must offer no Join");
        // Still a window with a body, not a slot — acres of empty olive read
        // as a failed draw rather than a finished screen.
        assert!(w.window.width() > 0, "{what}: no width");
        assert!(w.window.height() > 100, "{what}: no body");
    }

    // ── a hostile row cannot take the window down ───────────────────────────
    //
    // Names and maps come off a document a title serves. `label_untrusted` is
    // what keeps the text inside the well; what is pinned here is that such a
    // row still builds, still draws, and is still joinable — a launcher must
    // not be crashable by a title naming a shard in Japanese.
    let mut s = shard("eu-1", Some(1), Some(100));
    s.name = "W".repeat(96);
    s.map = Some("あ".repeat(32));
    let w = windows::servers("gates", &listed(vec![s]));
    assert_eq!(w.rows.len(), 1, "a hostile row must still be drawn");
    assert!(w.rows[0].join.active());
}
