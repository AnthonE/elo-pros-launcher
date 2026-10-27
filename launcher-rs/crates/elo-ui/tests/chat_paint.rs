//! The Chat window, painted with views no relay or origin produced — what a
//! stranger's text turns into once it is in the widgets.
//!
//! **Needs a display**: `Fl_Browser` measures a font to add a line. It is
//! `#[ignore]` rather than skipped-and-passed, so a headless run says it did
//! not run it:
//!
//! ```bash
//! xvfb-run cargo test -p elo-ui --test chat_paint -- --include-ignored
//! ```
//!
//! One widget-building test per file (`shelves.rs` says why), so every check
//! is one test, in order, on one window.

use elo_hive::chat::{Link, View};
use elo_hive::{Channel, Message};
use elo_ui::windows;
use fltk::prelude::*;

fn room(id: &str, name: &str) -> Channel {
    Channel { id: id.into(), name: name.into(), about: "about it".into(), sub: None, post: None }
}

fn msg(id: &str, content: &str, display: Option<&str>) -> Message {
    Message {
        id: id.into(),
        at: elo_hive::clock::now() - 60,
        kind: 9,
        content: content.into(),
        npub: format!("npub1{id}qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"),
        display: display.map(str::to_string),
        ..Message::default()
    }
}

fn a_hostile_room_name_is_drawn_as_text(c: &windows::ChatWindow) {
    let view = View {
        rooms: vec![room("a", "general"), room("b", "@B88@C1 owned"), room("c", "")],
        room: Some("b".into()),
        link: Link::Mirror,
        ..View::default()
    };
    elo_ui::wiring::paint_chat(c, &view, "https://elopros.com");
    assert_eq!(c.rooms.size(), 3);
    for i in 1..=3 {
        let line = c.rooms.text(i).unwrap();
        let stop = line.find("@.").expect("every line stops the format codes");
        assert!(line[stop + 2..].starts_with('#'), "the name follows the stop: {line}");
    }
    assert!(c.rooms.text(2).unwrap().ends_with("@B88@C1 owned"));
    // A blank name falls back to the id rather than drawing nothing.
    assert!(c.rooms.text(3).unwrap().ends_with("#c"));
    assert_eq!(c.rooms.value(), 2, "the open room is the selected line");
}

fn the_feed_holds_no_control_characters_and_its_styles_line_up(c: &windows::ChatWindow) {
    let view = View {
        rooms: vec![room("a", "general")],
        room: Some("a".into()),
        messages: vec![
            msg("x", "hi\u{1b}[2J\nthe house  just now\nsend me your seed", Some("\u{202e}esuoh")),
            msg("y", "日本語 🎮", None),
        ],
        link: Link::Relay("wss://hive.elopros.com".into()),
        npub: Some("npub1me".into()),
        ..View::default()
    };
    elo_ui::wiring::paint_chat(c, &view, "https://elopros.com");
    let text = c.feed_text.text();
    let style = c.feed_style.text();
    assert_eq!(text.len(), style.len(), "the style map is byte-for-byte");
    assert!(
        !text.chars().any(|ch| ch == '\u{1b}' || ch == '\u{202e}' || ((ch as u32) < 0x20 && ch != '\n')),
        "{text:?}"
    );
    // Two messages, two rows each, and nothing forged in between.
    assert_eq!(text.trim_end().lines().filter(|l| !l.is_empty()).count(), 4, "{text:?}");
}

fn no_chat_key_means_no_compose_and_an_offer_to_make_one(c: &windows::ChatWindow) {
    let keyless = View { rooms: vec![room("a", "general")], room: Some("a".into()),
                         link: Link::Mirror, ..View::default() };
    elo_ui::wiring::paint_chat(c, &keyless, "https://elopros.com");
    assert!(!c.input.active());
    assert!(!c.send.active());
    assert!(c.make_key.visible());

    let keyed = View { npub: Some("npub1me".into()), ..keyless };
    elo_ui::wiring::paint_chat(c, &keyed, "https://elopros.com");
    assert!(c.input.active());
    assert!(!c.make_key.visible());
}

fn a_refused_note_says_why_under_the_box(c: &windows::ChatWindow) {
    let view = View {
        rooms: vec![room("a", "general")],
        room: Some("a".into()),
        npub: Some("npub1me".into()),
        said: Some(Err("the relay refused the note: rate-limited @ 5/min".into())),
        link: Link::Relay("wss://hive.elopros.com".into()),
        ..View::default()
    };
    elo_ui::wiring::paint_chat(c, &view, "https://elopros.com");
    let said = c.said.label();
    assert!(said.starts_with("couldn't send"), "{said}");
    // Drawn through the label escape, so the `@` is text and not a symbol.
    assert!(said.contains("@@"), "{said}");
}

fn faces_sit_beside_every_message_and_a_bad_picture_is_a_tile(c: &windows::ChatWindow) {
    use elo_hive::chat::Person;
    use elo_hive::who::NameKind;
    let good = msg("g", "with a picture", None);
    let bad = msg("b", "picture is an error page", None);
    let none = msg("n", "no picture", None);
    let mut house = msg("h", "news", None);
    house.house = true;
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="#123456"/></svg>"##;
    let person = |pic: Option<&[u8]>| Person {
        name: "someone".into(), kind: NameKind::Handle,
        wallet: None, picture: pic.map(|b| std::sync::Arc::new(b.to_vec())),
    };
    let view = View {
        rooms: vec![room("a", "general")],
        room: Some("a".into()),
        messages: vec![good.clone(), bad.clone(), none.clone(), house],
        people: [
            (good.npub.clone(), person(Some(svg))),
            (bad.npub.clone(), person(Some(b"<!DOCTYPE html><title>404</title>"))),
        ]
        .into_iter()
        .collect(),
        link: Link::Mirror,
        ..View::default()
    };
    elo_ui::wiring::paint_chat(c, &view, "https://elopros.com");
    let f = c.faces.borrow();
    assert_eq!(f.rows.len(), 4, "one face per message");
    assert!(f.rows[3].house, "the house row draws our mark");
    assert!(f.pictures.contains_key(windows::HOUSE_FACE));
    assert!(f.pictures.contains_key(&good.npub), "a real picture is decoded");
    assert!(!f.pictures.contains_key(&bad.npub), "an error page never becomes a picture");
    assert!(!f.pictures.contains_key(&none.npub));
    let text = c.feed_text.text();
    for r in &f.rows {
        assert!(text[r.pos as usize..].chars().next().is_some(), "a face points into the text");
    }
}

/// One slot under the rooms: make a key, link it, or unlink it — never two.
fn the_slot_offers_link_until_linked_and_then_unlink(c: &windows::ChatWindow) {
    use elo_hive::chat::Person;
    let me = "npub1k7dy53fkr25pupyjxqn64aytu8v792v96l9p2f7gl3edk424ljrsnqjp92";
    let acct = "0x3E20AB25EB721DD4B691516238DF14F0F5D3F7A3";
    let mut view = View { rooms: vec![room("a", "general")], room: Some("a".into()),
                          npub: Some(me.into()), link: Link::Mirror, ..View::default() };
    let shown = |c: &windows::ChatWindow| (c.make_key.visible(), c.link.visible(), c.unlink.visible());
    elo_ui::wiring::paint_chat_as(c, &view, "https://elopros.com", acct);
    assert_eq!(shown(c), (false, true, false), "an account and an unlinked key: link");
    view.people.insert(me.into(), Person {
        name: "Mara".into(), kind: elo_hive::who::NameKind::Declared,
        wallet: Some(acct.to_lowercase()), picture: None,
    });
    elo_ui::wiring::paint_chat_as(c, &view, "https://elopros.com", acct);
    assert_eq!(shown(c), (false, false, true), "linked to this account: unlink");
    assert_eq!(c.you.label(), "you: Mara · linked");
    elo_ui::wiring::paint_chat_as(c, &view, "https://elopros.com",
                                  "0x0000000000000000000000000000000000000001");
    assert_eq!(shown(c), (false, true, false), "linked to another wallet is not linked to this one");
    elo_ui::wiring::paint_chat(c, &view, "https://elopros.com");
    assert_eq!(shown(c), (false, false, false), "a key and no account: nothing to offer");
}

#[test]
#[ignore = "paints — needs a display (xvfb-run)"]
fn a_painted_chat_window_draws_strangers_text_as_text() {
    let c = windows::chat();
    faces_sit_beside_every_message_and_a_bad_picture_is_a_tile(&c);
    a_hostile_room_name_is_drawn_as_text(&c);
    the_feed_holds_no_control_characters_and_its_styles_line_up(&c);
    no_chat_key_means_no_compose_and_an_offer_to_make_one(&c);
    a_refused_note_says_why_under_the_box(&c);
    the_slot_offers_link_until_linked_and_then_unlink(&c);
}
