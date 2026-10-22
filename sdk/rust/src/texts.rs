//! The exact texts a wallet signs, rebuilt offline.
//!
//! A client that signs whatever a server hands it has given that server its
//! signature. These functions build the text from what was asked for, so
//! [`crate::Client`] can refuse a server that offers anything else. Each one
//! is pinned to the meter's own code by `sdk/testdata/` (friends_texts.json,
//! face.json), which `meter/test_sdk_fixtures.py` rebuilds on the server side.

use crate::overlay::play_message;

/// The `action:` line of a friends change (`meter/friends.py`).
pub const FRIENDS_ACTION: &str = "friends";
/// The `action:` line of a face change (`meter/faces.py`).
pub const FACE_ACTION: &str = "profile";

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
