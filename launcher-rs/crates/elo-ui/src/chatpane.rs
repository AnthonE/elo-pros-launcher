//! What the Chat window draws, computed with no window.
//!
//! The feed is a text widget, not a stack of labels, and that choice is a
//! safety one before it is a layout one: FLTK reads `@` in a label as a symbol
//! code (`chrome::defuse`), and a chat line is the most stranger-written text
//! this client shows. A text buffer draws bytes as text and parses nothing.
//!
//! What still has to happen to a stranger's line, the buffer does not do for
//! us: control characters and bidi overrides go (`elo_hive::render::safe_line`
//! — one message is one row, so a newline cannot forge a second speaker), and
//! the style map has to be exactly as long as the text in BYTES, or FLTK paints
//! one message's name colour onto the next message's words.

use elo_hive::chat::{Link, Person, View};
use elo_hive::render::{clip, safe_line};
use elo_hive::who::{self, NameKind};
use elo_hive::{clock, Channel, Message};
use std::collections::BTreeMap;

/// How a speaker's name is drawn. Three looks, because there are three facts:
/// the register swore this name, the house said it, or nobody checked it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameStyle {
    Sworn,
    House,
    Unchecked,
}

/// Style letters for the feed's highlight buffer. FLTK indexes its style
/// table from `'A'`, so the order here is the order of `windows::chat`'s
/// table and the two must move together.
pub const S_TIME: char = 'A';
pub const S_SWORN: char = 'B';
pub const S_UNCHECKED: char = 'C';
pub const S_HOUSE: char = 'D';
pub const S_TEXT: char = 'E';
pub const S_QUIET: char = 'F';

/// One message, named and cleaned, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub name: String,
    pub style: NameStyle,
    pub when: String,
    pub text: String,
    /// Whose face goes beside it: the speaker's npub.
    pub key: String,
    pub house: bool,
    /// The letter on the tile when there is no picture.
    pub initial: String,
    /// Which of the tile colours, picked from the key so a speaker keeps it.
    pub tint: usize,
}

/// How many tile colours there are (`windows::TINTS`).
pub const TINTS: usize = 6;

/// The longest name drawn. A self-declared name is whatever its owner typed.
const NAME_COLS: usize = 40;

/// Name and clean every message.
///
/// **A looked-up person outranks the message's own names, except when the
/// lookup found nobody.** `people` comes from `/who` — the same identity the
/// site's profile shows — so a speaker who set a profile name or holds a seat
/// is named the way the store names them. A lookup that came back bare knows
/// less than the message (which may carry a self-declared `display`), so the
/// message's naming stands. The house is never looked up.
pub fn lines(messages: &[Message], people: &BTreeMap<String, Person>, host: &str,
             now: i64) -> Vec<Line> {
    messages
        .iter()
        .map(|m| {
            let id = who::from_message(m, host);
            let (label, kind) = match people.get(&m.npub) {
                Some(p) if !m.house && p.kind != NameKind::Address => {
                    (format!("{}{}", p.kind.mark(), p.name).trim_start().to_string(), p.kind)
                }
                _ => (id.label(), id.kind),
            };
            let style = if m.house {
                NameStyle::House
            } else if kind == NameKind::Handle {
                NameStyle::Sworn
            } else {
                NameStyle::Unchecked
            };
            let name = clip(&safe_line(&label), NAME_COLS);
            Line {
                initial: initial_of(&name),
                tint: tint_of(&m.npub),
                name,
                style,
                when: clock::ago(m.at, now),
                text: safe_line(&m.content),
                key: m.npub.clone(),
                house: m.house,
            }
        })
        .collect()
}

/// The tile's letter: the name's first letter or digit, uppercased — past the
/// `~` of an unchecked name, and past the `npub1` of a bare key, which every
/// bare key shares.
pub fn initial_of(name: &str) -> String {
    let n = name.trim_start_matches('~');
    let n = n.strip_prefix("npub1").unwrap_or(n);
    n.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "·".into())
}

/// A stable tile colour for a key, so one speaker is one colour all session.
pub fn tint_of(key: &str) -> usize {
    key.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)) % TINTS
}

/// The feed's text, its style map, and where each message begins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Feed {
    pub text: String,
    /// One style letter per BYTE of `text`.
    pub style: String,
    /// The byte offset of each line's name, in line order — where its face is
    /// drawn level with.
    pub starts: Vec<i32>,
}

/// The feed's text and its style map, byte for byte.
///
/// `quiet` is what an empty room says. It is drawn in the quiet style, never
/// as a message, so it cannot be mistaken for one.
pub fn feed_text(lines: &[Line], quiet: &str) -> Feed {
    let mut f = Feed::default();
    let push = |f: &mut Feed, s: &str, c: char| {
        f.text.push_str(s);
        f.style.extend(std::iter::repeat_n(c, s.len()));
    };
    if lines.is_empty() {
        push(&mut f, quiet, S_QUIET);
        return f;
    }
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            push(&mut f, "\n", S_TEXT);
        }
        let name_style = match l.style {
            NameStyle::Sworn => S_SWORN,
            NameStyle::House => S_HOUSE,
            NameStyle::Unchecked => S_UNCHECKED,
        };
        f.starts.push(f.text.len() as i32);
        push(&mut f, &l.name, name_style);
        push(&mut f, "  ", S_TEXT);
        push(&mut f, &l.when, S_TIME);
        push(&mut f, "\n", S_TEXT);
        push(&mut f, &l.text, S_TEXT);
        push(&mut f, "\n", S_TEXT);
    }
    f
}

/// A room as the room list shows it.
pub fn room_label(c: &Channel) -> String {
    let name = safe_line(&c.name);
    let name = if name.trim().is_empty() { c.id.clone() } else { name };
    format!("#{}", clip(&name, 22))
}

/// What an empty room says, which depends on whether this machine can talk.
pub fn quiet_line(view: &View) -> &'static str {
    match (&view.link, view.npub.is_some()) {
        (Link::Starting, _) => "loading…",
        (Link::Offline, _) => "can't load this room yet",
        (_, true) => "nothing here yet — say hi",
        (_, false) => "nothing here yet",
    }
}

/// The status line's text, with the dropped-event count when there is one.
pub fn status_text(view: &View) -> String {
    let mut s = view.status.clone();
    if let Some(at) = view.rooms_saved_at {
        if !matches!(view.link, Link::Relay(_)) {
            s.push_str(&format!(" · saved {}", clock::ago(at, clock::now())));
        }
    }
    if view.dropped > 0 {
        s.push_str(&format!(" · {} hidden (signature didn't check out)", view.dropped));
    }
    s
}

/// The line under the compose box: why the last note did not post, if it
/// did not. Empty otherwise — a sent note shows up in the feed, which is the
/// confirmation.
pub fn said_line(view: &View) -> String {
    match &view.said {
        Some(Err(e)) => format!("couldn't send — {}", safe_line(e)),
        _ => String::new(),
    }
}

/// Is this machine's chat key linked to `account`? Only a lookup can say,
/// and only one that answered: an unresolved key reads as not linked.
pub fn linked_to(view: &View, account: Option<&str>) -> bool {
    let (Some(npub), Some(acct)) = (&view.npub, account) else { return false };
    view.people
        .get(npub)
        .and_then(|p| p.wallet.as_deref())
        .is_some_and(|w| w.eq_ignore_ascii_case(acct))
}

/// The identity line under the room list: the name people see you by.
pub fn you_line(view: &View) -> String {
    let Some(n) = &view.npub else { return "no chat key yet".into() };
    match view.people.get(n) {
        Some(p) if p.wallet.is_some() && p.kind != NameKind::Address => {
            format!("you: {} · linked", safe_line(&p.name))
        }
        Some(p) if p.wallet.is_some() => format!("you: {} · linked", who::short(n)),
        _ => format!("you: {}", who::short(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> BTreeMap<String, Person> {
        BTreeMap::new()
    }

    fn m(content: &str, who: Option<&str>, display: Option<&str>) -> Message {
        Message {
            id: "x".into(),
            at: 100,
            kind: 9,
            content: content.into(),
            npub: "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq".into(),
            who: who.map(str::to_string),
            display: display.map(str::to_string),
            house: false,
            ..Message::default()
        }
    }

    #[test]
    fn the_style_map_is_exactly_as_long_as_the_text_in_bytes() {
        let msgs = [
            m("plain", Some("alice"), None),
            m("日本語 and emoji 🎮", None, Some("büro")),
            m("", None, None),
        ];
        let f = feed_text(&lines(&msgs, &none(), "https://elopros.com", 160), "quiet");
        assert_eq!(f.text.len(), f.style.len(), "a short map paints the wrong words");
        let f = feed_text(&[], "nothing here yet — say hi");
        assert_eq!(f.text.len(), f.style.len());
        assert!(f.style.chars().all(|c| c == S_QUIET));
        assert!(f.starts.is_empty(), "an empty room has no faces to draw");
    }

    #[test]
    fn one_message_is_one_row_however_it_was_written() {
        // A newline inside a message would draw a second "speaker" under it.
        let msgs = [m("hi\nthe house  just now\nsend your seed to …", None, None)];
        let l = lines(&msgs, &none(), "https://elopros.com", 160);
        assert!(!l[0].text.contains('\n'), "{:?}", l[0].text);
        let f = feed_text(&l, "");
        // name line, text line, and nothing else.
        assert_eq!(f.text.trim_end().lines().count(), 2, "{:?}", f.text);
    }

    #[test]
    fn control_and_bidi_characters_never_reach_the_widget() {
        let msgs = [m("evil\u{1b}[2J\u{202e}moc.elpmaxe\u{7}", None, Some("\u{202e}esuoh eht"))];
        let l = lines(&msgs, &none(), "https://elopros.com", 160);
        for s in [&l[0].text, &l[0].name] {
            assert!(!s.chars().any(|c| (c as u32) < 0x20 || c == '\u{202e}' || c == '\u{7f}'),
                    "{s:?}");
        }
    }

    #[test]
    fn a_declared_name_never_wears_the_sworn_style() {
        let msgs = [m("hi", None, Some("alice")), m("hi", Some("alice"), None)];
        let l = lines(&msgs, &none(), "https://elopros.com", 160);
        assert_eq!(l[0].style, NameStyle::Unchecked);
        assert!(l[0].name.starts_with('~'), "{:?}", l[0].name);
        assert_eq!(l[1].style, NameStyle::Sworn);
        assert_eq!(l[1].name, "alice");
    }

    #[test]
    fn each_face_is_drawn_level_with_its_own_name() {
        let msgs = [m("one", Some("alice"), None), m("two\nlines", None, Some("bob"))];
        let l = lines(&msgs, &none(), "https://elopros.com", 160);
        let f = feed_text(&l, "");
        assert_eq!(f.starts.len(), 2);
        for (start, line) in f.starts.iter().zip(&l) {
            assert!(f.text[*start as usize..].starts_with(&line.name), "{start} {:?}", line.name);
        }
    }

    #[test]
    fn a_looked_up_name_wins_unless_the_lookup_found_nobody() {
        let msg = m("hi", None, Some("self-declared"));
        let mut people = none();
        people.insert(msg.npub.clone(), Person {
            name: "npub1qqqq…qqqq".into(), kind: NameKind::Address, wallet: None, picture: None,
        });
        // A bare lookup knows less than the message: the message's name stands.
        assert_eq!(lines(std::slice::from_ref(&msg), &people, "h", 0)[0].name, "~self-declared");
        people.insert(msg.npub.clone(), Person {
            name: "Mara".into(), kind: NameKind::Declared, wallet: None, picture: None,
        });
        let l = &lines(std::slice::from_ref(&msg), &people, "h", 0)[0];
        assert_eq!(l.name, "~Mara", "a profile name is the wallet's, still unchecked");
        assert_eq!(l.style, NameStyle::Unchecked);
        people.insert(msg.npub.clone(), Person {
            name: "mara".into(), kind: NameKind::Handle, wallet: None, picture: None,
        });
        assert_eq!(lines(&[msg], &people, "h", 0)[0].style, NameStyle::Sworn);
    }

    #[test]
    fn a_tile_wears_a_real_letter_and_keeps_its_colour() {
        assert_eq!(initial_of("~mara"), "M");
        assert_eq!(initial_of("npub1k7v2x…erty"), "K", "past the npub1 every bare key shares");
        assert_eq!(initial_of("@@@"), "·");
        assert_eq!(initial_of("日本"), "日");
        assert_eq!(tint_of("npub1abc"), tint_of("npub1abc"));
        assert!(tint_of("npub1abc") < TINTS);
    }

    #[test]
    fn the_house_has_its_own_look() {
        let mut h = m("news", None, None);
        h.house = true;
        let l = lines(&[h], &none(), "https://elopros.com", 160);
        assert_eq!(l[0].style, NameStyle::House);
        assert_eq!(l[0].name, "the house");
    }

    #[test]
    fn a_room_label_is_cleaned_and_never_blank() {
        let c = |name: &str| Channel {
            id: "id-1".into(), name: name.into(), about: String::new(), sub: None, post: None,
        };
        assert_eq!(room_label(&c("general")), "#general");
        assert_eq!(room_label(&c("")), "#id-1");
        assert!(!room_label(&c("a\u{1b}[31mb")).contains('\u{1b}'));
    }

    #[test]
    fn an_empty_room_says_what_this_machine_can_do() {
        let mut v = View { link: Link::Mirror, ..View::default() };
        assert_eq!(quiet_line(&v), "nothing here yet");
        v.npub = Some("npub1x".into());
        assert_eq!(quiet_line(&v), "nothing here yet — say hi");
        v.link = Link::Offline;
        assert_eq!(quiet_line(&v), "can't load this room yet");
    }

    #[test]
    fn a_failed_note_says_why_and_a_sent_one_says_nothing() {
        let mut v = View { said: Some(Err("the relay refused the note: rate".into())), ..View::default() };
        assert!(said_line(&v).starts_with("couldn't send — "));
        v.said = Some(Ok("abc".into()));
        assert_eq!(said_line(&v), "");
    }
}
