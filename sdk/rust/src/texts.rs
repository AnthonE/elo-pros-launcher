//! The exact texts a wallet signs, rebuilt offline.
//!
//! A client that signs whatever a server hands it has given that server its
//! signature. These functions build the text from what was asked for, so
//! [`crate::Client`] can refuse a server that offers anything else. Each one
//! is pinned to the meter's own code by `sdk/testdata/` (friends_texts.json,
//! face.json, buddy_texts.json, board_texts.json, entitle_texts.json), which
//! `meter/test_sdk_fixtures.py` rebuilds on the server side.

use serde_json::Value;

use crate::overlay::{play_message, utc_day};

/// The `action:` line of a friends change (`meter/friends.py`).
pub const FRIENDS_ACTION: &str = "friends";
/// The `action:` line of a face change (`meter/faces.py`).
pub const FACE_ACTION: &str = "profile";
/// The `action:` line of a change to a wallet's agents (`meter/buddy.py`).
pub const BUDDY_ACTION: &str = "buddy";

/// Every op `/buddy/prepare` takes (`meter/buddy.py` `OPS`).
pub const BUDDY_OPS: [&str; 10] = [
    "trial", "create", "update", "face", "pause", "resume", "retire", "token", "fund", "gift",
];

/// Each friends op and the one argument it takes, a wallet
/// (`meter/friends.py` `OPS`).
pub const OPS: [(&str, &str); 8] = [
    ("ask", "to"),
    ("accept", "from"),
    ("decline", "from"),
    ("cancel", "to"),
    ("remove", "wallet"),
    ("block", "wallet"),
    ("unblock", "wallet"),
    ("invite", "to"),
];

/// The argument a friends op takes, or None for an op the server does not know.
pub fn op_key(op: &str) -> Option<&'static str> {
    OPS.iter().find(|(o, _)| *o == op).map(|(_, k)| *k)
}

/// `0x` and 40 hex characters, either case.
pub fn is_wallet(s: &str) -> bool {
    s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// The UTC day (`YYYY-MM-DD`) of a unix time, with no `chrono`: Howard
/// Hinnant's civil-from-days, the same arithmetic as the overlay's
/// `utc_day`. A text keyed to `day_of(at)` signed at 23:59 still verifies
/// at 00:00, because the server keys it the same way (`playauth.day_of`).
pub fn day_of(at: i64) -> String {
    let z = at.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// The signed detail of a friends change: sorted compact JSON of its args,
/// `at` and op (`friends.detail_of`). The other wallet is lowercased. None for
/// an op the server does not know or an argument that is not a wallet.
pub fn friends_detail(op: &str, other: &str, at: u64) -> Option<String> {
    let key = op_key(op)?;
    if !is_wallet(other.trim()) {
        return None;
    }
    let other = other.trim().to_ascii_lowercase();
    Some(format!(
        r#"{{"args":{{"{key}":"{other}"}},"at":{at},"op":"{op}"}}"#
    ))
}

/// The whole text a friends change signs: `playauth.play_message("friends",
/// me, detail, day_of(at))`, both wallets lowercased.
pub fn friends_sign_text(me: &str, op: &str, other: &str, at: u64) -> Option<String> {
    if !is_wallet(me.trim()) {
        return None;
    }
    let detail = friends_detail(op, other, at)?;
    let day = day_of(i64::try_from(at).ok()?);
    Some(play_message(FRIENDS_ACTION, me.trim(), &detail, Some(&day)))
}

/// A JSON string exactly as Python's `json.dumps` writes it (`ensure_ascii`):
/// quote and backslash escaped, control characters and anything past ASCII
/// as `\uXXXX`. The server signs Python's bytes, so a client must build the same.
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The signed detail of a face change (`faces._detail`): sorted compact JSON
/// of `at` and only the fields being changed. `Some("")` clears a field;
/// `None` leaves it as it is and keeps it out of the text.
pub fn face_detail(
    at: u64,
    name: Option<&str>,
    picture_sha256: Option<&str>,
    kind: Option<&str>,
    harness: Option<&str>,
    model: Option<&str>,
) -> String {
    // Sorted by key, as `json.dumps(sort_keys=True)` sorts them.
    let fields = [
        ("harness", harness),
        ("kind", kind),
        ("model", model),
        ("name", name),
        ("picture_sha256", picture_sha256),
    ];
    let mut out = format!(r#"{{"at":{at}"#);
    for (k, v) in fields {
        if let Some(v) = v {
            out.push_str(&format!(r#","{k}":{}"#, json_string(v)));
        }
    }
    out.push('}');
    out
}

/// The day and detail of an `elo play` text, when it is one for this action
/// and wallet and nothing else: five lines, the detail on one. None for any
/// other text.
pub fn read_play(text: &str, action: &str, wallet: &str) -> Option<(String, String)> {
    let w = wallet.trim();
    if !is_wallet(w) {
        return None;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() != 5 {
        return None;
    }
    let day = lines[3].strip_prefix("day: ")?;
    let detail = lines[4].strip_prefix("detail: ")?;
    let b = day.as_bytes();
    let day_ok = b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        });
    if !day_ok || text != play_message(action, w, detail, Some(day)) {
        return None;
    }
    Some((day.to_string(), detail.to_string()))
}

/// A change to a wallet's agents, read back out of the text that signs it.
#[derive(Debug, Clone, PartialEq)]
pub struct BuddyChange {
    pub day: String,
    pub op: String,
    pub args: Value,
    pub at: u64,
}

/// The change a buddy text signs (its detail is `buddy.detail_of`'s JSON),
/// or None when the text is not a buddy text for this wallet.
pub fn buddy_change(text: &str, wallet: &str) -> Option<BuddyChange> {
    let (day, detail) = read_play(text, BUDDY_ACTION, wallet)?;
    let d: Value = serde_json::from_str(&detail).ok()?;
    let o = d.as_object()?;
    if o.len() != 3 || !o.get("args")?.is_object() {
        return None;
    }
    Some(BuddyChange {
        day,
        op: o.get("op")?.as_str()?.to_string(),
        args: o["args"].clone(),
        at: o.get("at")?.as_u64()?,
    })
}

fn hexish(s: &str) -> bool {
    is_wallet(s) || (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A plain decimal dollar string (`buddy._USD_RE`) as a number.
fn cents(s: &str) -> Option<f64> {
    let (whole, frac) = s.split_once('.').unwrap_or((s, "1"));
    let digits = |p: &str, max: usize| {
        !p.is_empty() && p.len() <= max && p.bytes().all(|b| b.is_ascii_digit())
    };
    if digits(whole, 9) && digits(frac, 6) {
        s.parse().ok()
    } else {
        None
    }
}

fn same_value(asked: &Value, signed: &Value) -> bool {
    match (asked, signed) {
        (Value::Bool(a), Value::Bool(s)) => a == s,
        (Value::Bool(_), _) | (_, Value::Bool(_)) => false,
        (_, Value::Number(s)) => {
            let a = match asked {
                Value::Number(a) => a.as_f64(),
                Value::String(a) => cents(a.trim()),
                _ => None,
            };
            matches!((a, s.as_f64()), (Some(a), Some(s)) if (a - s).abs() <= 0.005 + 1e-9)
        }
        (Value::Number(a), Value::String(s)) => (a.is_i64() || a.is_u64()) && a.to_string() == *s,
        (Value::String(a), Value::String(s)) => {
            let a = a.split_whitespace().collect::<Vec<_>>().join(" ");
            a == *s || (hexish(&a) && a.eq_ignore_ascii_case(s))
        }
        (_, Value::Object(_)) => same_args(asked, signed),
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

/// Is the change signed the change asked for? The same keys, and each value
/// the same: a string once its whitespace is collapsed (a wallet or a sha256
/// in either case), a dollar amount to the cent, a number given as an id the
/// same digits. What the server cleans; nothing it could swap.
pub fn same_args(asked: &Value, signed: &Value) -> bool {
    match (asked.as_object(), signed.as_object()) {
        (Some(a), Some(s)) => {
            a.len() == s.len()
                && s.iter()
                    .all(|(k, v)| a.get(k).is_some_and(|x| same_value(x, v)))
        }
        _ => false,
    }
}

/// The text a board claim signs: the bounty's id is the detail.
pub fn claim_text(wallet: &str, quest_id: &str, day: Option<&str>) -> String {
    play_message("claim", wallet, quest_id, day)
}

/// A delivery's signed detail (`munus.submit_detail`): the bounty, the first
/// 16 hex of the url's sha256, and the wallet's 0-based delivery index on
/// that bounty. (Feature `http`, for its sha256.)
#[cfg(feature = "http")]
pub fn submit_detail(quest_id: &str, url: &str, n: u64) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, url.as_bytes());
    let hex: String = d.as_ref()[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{quest_id} {hex} #{n}")
}

/// The text a board delivery signs. (Feature `http`.)
#[cfg(feature = "http")]
pub fn submit_text(wallet: &str, quest_id: &str, url: &str, n: u64, day: Option<&str>) -> String {
    play_message("submit", wallet, &submit_detail(quest_id, url, n), day)
}

/// The text a download grant signs (`tickets.entitle_message`): the game,
/// the wallet lowercased, and the UTC day (today when `None`).
pub fn entitle_text(game: &str, wallet: &str, day: Option<&str>) -> String {
    let day = day.map(str::to_string).unwrap_or_else(utc_day);
    let wallet = wallet.trim().to_ascii_lowercase();
    format!("elo entitle\ngame: {game}\nwallet: {wallet}\nday: {day}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_of_is_utc_and_exact_at_midnight() {
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(1_761_004_799), "2025-10-20");
        assert_eq!(day_of(1_761_004_800), "2025-10-21");
        assert_eq!(day_of(951_782_400), "2000-02-29");
        assert_eq!(day_of(-1), "1969-12-31");
    }

    #[test]
    fn an_unknown_op_or_a_bad_wallet_builds_nothing() {
        assert_eq!(
            friends_detail("poke", "0x1111111111111111111111111111111111111111", 1),
            None
        );
        assert_eq!(friends_detail("ask", "0x123", 1), None);
        assert_eq!(
            friends_sign_text("me", "ask", "0x1111111111111111111111111111111111111111", 1),
            None
        );
    }

    #[test]
    fn json_string_matches_python() {
        assert_eq!(json_string(r#"Doc "W" \o/"#), r#""Doc \"W\" \\o/""#);
        assert_eq!(json_string("é😀"), r#""\u00e9\ud83d\ude00""#);
    }
}
