//! Your profile — the name and picture the store, the game and chat show for
//! your wallet (`meter/faces.py`), read and changed from the launcher.
//!
//! A profile is the wallet's, not the launcher's: a change is one EIP-191
//! signature by the account, over the exact text the origin prepares
//! (`POST /api/face/prepare` → sign → `POST /api/face`). This module is the
//! part with no window and no key — the rules a change must pass before anyone
//! is asked for a passphrase, and the two origin calls, behind a seam so the
//! flow is testable with no network.
//!
//! **The rules here are the server's, checked early, never instead of it.**
//! `faces.py` re-checks everything. These exist so a player who picked a
//! 600-pixel JPEG hears why in one line, before a passphrase prompt, rather
//! than after typing it.

use serde_json::Value;

/// The origin's caps (`faces.py` `NAME_MAX`, `PIC_MIN`, `PIC_MAX`,
/// `PIC_MAX_BYTES`).
pub const NAME_MAX: usize = 16;
pub const PIC_MIN: u32 = 64;
pub const PIC_MAX: u32 = 512;
pub const PIC_MAX_BYTES: usize = 256 * 1024;

/// Why a name would be refused, or `None`. `""` is not checked here: it means
/// "clear my name", which the origin accepts.
pub fn name_error(name: &str) -> Option<String> {
    let n = name.trim();
    if n.is_empty() {
        return None;
    }
    if n.chars().count() > NAME_MAX {
        return Some(format!("a name is at most {NAME_MAX} characters"));
    }
    if n.chars().any(|c| !(' '..='~').contains(&c)) {
        return Some("a name uses plain letters, digits, spaces and punctuation only".into());
    }
    if n.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("0x")) || n.starts_with('#') {
        // What a game shows when there is no name — a name may not pose as it.
        return Some("a name may not start with 0x or #".into());
    }
    None
}

/// Why a picture would be refused, or `None` — the header half of the
/// origin's `png_error` (it also walks every chunk's checksum).
pub fn png_error(data: &[u8]) -> Option<String> {
    if data.len() > PIC_MAX_BYTES {
        return Some(format!(
            "the picture is {} KB and the cap is {} KB",
            data.len() / 1024,
            PIC_MAX_BYTES / 1024
        ));
    }
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("the picture must be a PNG".into());
    }
    // The first chunk must be a 13-byte IHDR: width, height, depth, colour,
    // compression, filter, interlace.
    if data.len() < 33 || &data[12..16] != b"IHDR" || data[8..12] != [0, 0, 0, 13] {
        return Some("the PNG has no IHDR header".into());
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    let (depth, colour, interlace) = (data[24], data[25], data[28]);
    if w != h {
        return Some(format!("the picture must be square (it is {w}×{h})"));
    }
    if !(PIC_MIN..=PIC_MAX).contains(&w) {
        return Some(format!("the picture must be {PIC_MIN} to {PIC_MAX} px across (it is {w})"));
    }
    if depth != 8 || !matches!(colour, 2 | 6) {
        return Some("the PNG must be 8-bit RGB or RGBA".into());
    }
    if interlace != 0 {
        return Some("the PNG must not be interlaced".into());
    }
    None
}

/// Standard base64 with padding — what `faces.py` decodes with
/// `b64decode(validate=True)`. Twenty lines rather than a crate: the
/// dependency budget is a posture (`supply_chain.rs`), and this is the only
/// place the client encodes anything.
pub fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                A[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// What a change asks for. `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// `Some("")` clears the name.
    pub name: Option<String>,
    /// PNG bytes, already through [`png_error`].
    pub picture: Option<Vec<u8>>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.picture.is_none()
    }
}

/// The origin's face routes, injectable so the flow runs with no network.
pub trait FaceHttp {
    /// `GET /api/face/{wallet}` — the current name and picture path.
    fn face(&self, host: &str, wallet: &str) -> elo_net::Fetched<Value>;
    /// The picture bytes at an `/api/face/…` path the origin named.
    fn picture(&self, host: &str, path: &str) -> elo_net::Fetched<Vec<u8>>;
    /// `POST /api/face/prepare` — the exact text to sign.
    fn prepare(&self, host: &str, body: &Value) -> elo_net::Fetched<Value>;
    /// `POST /api/face` — the signed change.
    fn save(&self, host: &str, body: &Value) -> elo_net::Fetched<Value>;
}

pub struct RealFaceHttp;

impl FaceHttp for RealFaceHttp {
    fn face(&self, host: &str, wallet: &str) -> elo_net::Fetched<Value> {
        elo_net::Net::new().get_json_quick(&format!("{host}/api/face/{wallet}"))
    }
    fn picture(&self, host: &str, path: &str) -> elo_net::Fetched<Vec<u8>> {
        // Only a path on our own origin is ever fetched — never a url the
        // reply could point elsewhere.
        if !path.starts_with("/api/face/") {
            return elo_net::Fetched::bad("not a face path on this origin");
        }
        elo_net::Net::new().art(&format!("{host}{path}"))
    }
    fn prepare(&self, host: &str, body: &Value) -> elo_net::Fetched<Value> {
        elo_net::Net::new().post_json_told(&format!("{host}/api/face/prepare"), body)
    }
    fn save(&self, host: &str, body: &Value) -> elo_net::Fetched<Value> {
        elo_net::Net::new().post_json_told(&format!("{host}/api/face"), body)
    }
}

/// The current profile, as the Account window shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shown {
    pub name: Option<String>,
    pub picture: Option<Vec<u8>>,
}

/// Read a wallet's profile. `Err` is "could not look", which the window says
/// as such rather than as "no profile".
pub fn read(http: &dyn FaceHttp, host: &str, wallet: &str) -> Result<Shown, String> {
    let got = http.face(host, wallet);
    let Some(v) = got.value else {
        return Err(if got.reachable { got.why } else { format!("could not reach {host}") });
    };
    let name = v.get("name").and_then(Value::as_str).map(str::to_string);
    let picture = v
        .get("picture")
        .and_then(Value::as_str)
        .and_then(|p| http.picture(host, p).value);
    Ok(Shown { name, picture })
}

/// The text a profile change signs, rebuilt here and held against the one the
/// origin offered — the same check `friendsop::checked_text` makes for a
/// friends change. The format is `meter/faces.py`'s and deterministic: an
/// `elo play` message whose detail is the sorted, compact JSON of `at` and the
/// fields being changed. Only the day is taken from the offered text, and only
/// when it is shaped like one.
///
/// Signing whatever `sign_this` said meant a breached or buggy origin could put
/// any `elo play` text under this account's key behind a profile edit; the
/// prompt shows the text, but a player changing their name does not read a
/// detail line of JSON.
pub fn checked_profile_text(offered: &str, wallet: &str, at: i64, name: Option<&str>,
                            picture_sha256: Option<&str>) -> Result<(), String> {
    let day = offered
        .lines()
        .nth(3)
        .and_then(|l| l.strip_prefix("day: "))
        .filter(|d| d.len() == 10 && d.bytes().enumerate()
            .all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() }))
        .ok_or("the origin's text to sign is not a profile change — nothing was signed")?;
    let mut detail = serde_json::Map::new();
    detail.insert("at".into(), Value::from(at));
    if let Some(n) = name {
        detail.insert("name".into(), Value::from(n));
    }
    if let Some(h) = picture_sha256 {
        detail.insert("picture_sha256".into(), Value::from(h));
    }
    let wanted = format!(
        "elo play\naction: profile\nwallet: {}\nday: {day}\ndetail: {}",
        wallet.to_lowercase(),
        ascii_json(&Value::Object(detail).to_string())
    );
    if wanted == offered {
        Ok(())
    } else {
        Err("the origin asked to sign something other than this change — nothing was signed"
            .into())
    }
}

/// Python's `json.dumps` default: every non-ASCII character as `\uXXXX`, a
/// character past U+FFFF as its UTF-16 surrogate pair. serde_json leaves them
/// raw, so a name like "Zoë" would otherwise rebuild as different bytes.
fn ascii_json(compact: &str) -> String {
    let mut out = String::with_capacity(compact.len());
    for c in compact.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for u in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{u:04x}"));
            }
        }
    }
    out
}

/// A minimal PNG header of `size`×`size` that passes [`png_error`], for tests
/// elsewhere in the crate.
#[cfg(test)]
pub(crate) fn tests_png(size: u32) -> Vec<u8> {
    tests::png(size, size, 8, 6, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG header with the given IHDR fields and nothing after it — enough
    /// for the header rules, which are all this module checks.
    pub(crate) fn png(w: u32, h: u32, depth: u8, colour: u8, interlace: u8) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend([0, 0, 0, 13]);
        b.extend(b"IHDR");
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([depth, colour, 0, 0, interlace]);
        b.extend([0, 0, 0, 0]);
        b
    }

    /// The bytes `meter/faces.py` hands out, character for character —
    /// `json.dumps(..., sort_keys=True, separators=(",", ":"))`, so non-ASCII
    /// is escaped — and anything else refused.
    #[test]
    fn a_profile_change_signs_only_the_text_it_describes() {
        let w = "0xAbC0000000000000000000000000000000000001";
        let ok = "elo play\naction: profile\nwallet: 0xabc0000000000000000000000000000000000001\n\
                  day: 2026-10-25\ndetail: {\"at\":1793000000,\"name\":\"Zo\\u00eb \\ud83d\\ude00\",\
                  \"picture_sha256\":\"ab\"}";
        assert_eq!(checked_profile_text(ok, w, 1_793_000_000, Some("Zoë 😀"), Some("ab")), Ok(()));
        // Another name, another action, another wallet, a picture nobody chose.
        for bad in [
            ok.replace("Zo", "Xo"),
            ok.replace("action: profile", "action: grant"),
            ok.replace("0xabc0", "0xdef0"),
            ok.replace(",\"picture_sha256\":\"ab\"", ""),
            ok.replace("day: 2026-10-25", "day: tomorrow!!"),
        ] {
            assert!(checked_profile_text(&bad, w, 1_793_000_000, Some("Zoë 😀"), Some("ab")).is_err(),
                    "{bad}");
        }
        assert!(checked_profile_text(ok, w, 1_793_000_001, Some("Zoë 😀"), Some("ab")).is_err());
    }

    #[test]
    fn a_name_follows_the_games_wire_rule() {
        assert_eq!(name_error("Mara"), None);
        assert_eq!(name_error("  "), None, "blank clears the name; the origin takes it");
        assert!(name_error("seventeen chars!!").is_some());
        assert!(name_error("0xdeadbeef").is_some(), "may not pose as an address");
        assert!(name_error("0Xdead").is_some());
        assert!(name_error("#123").is_some());
        assert!(name_error("mära").is_some(), "plain ASCII only, as the game draws it");
        assert!(name_error("tab\there").is_some());
    }

    #[test]
    fn a_picture_is_checked_before_anyone_types_a_passphrase() {
        assert_eq!(png_error(&png(128, 128, 8, 6, 0)), None);
        assert_eq!(png_error(&png(64, 64, 8, 2, 0)), None);
        assert!(png_error(&png(128, 96, 8, 6, 0)).unwrap().contains("square"));
        assert!(png_error(&png(32, 32, 8, 6, 0)).unwrap().contains("64 to 512"));
        assert!(png_error(&png(1024, 1024, 8, 6, 0)).unwrap().contains("64 to 512"));
        assert!(png_error(&png(128, 128, 16, 6, 0)).unwrap().contains("8-bit"));
        assert!(png_error(&png(128, 128, 8, 3, 0)).unwrap().contains("8-bit"), "no palettes");
        assert!(png_error(&png(128, 128, 8, 6, 1)).unwrap().contains("interlaced"));
        assert!(png_error(b"\xff\xd8\xff\xe0 a jpeg").unwrap().contains("must be a PNG"));
        assert!(png_error(&vec![0u8; PIC_MAX_BYTES + 1]).unwrap().contains("cap"));
        assert!(png_error(b"\x89PNG\r\n\x1a\n").unwrap().contains("IHDR"));
    }

    #[test]
    fn base64_matches_the_standard_vectors() {
        // RFC 4648 §10.
        for (raw, enc) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"),
                           ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(b64(raw.as_bytes()), enc, "{raw:?}");
        }
        assert_eq!(b64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    struct Fixed(Value, Option<Vec<u8>>);
    impl FaceHttp for Fixed {
        fn face(&self, _: &str, _: &str) -> elo_net::Fetched<Value> {
            elo_net::Fetched::good(self.0.clone())
        }
        fn picture(&self, _: &str, _: &str) -> elo_net::Fetched<Vec<u8>> {
            match &self.1 {
                Some(b) => elo_net::Fetched::good(b.clone()),
                None => elo_net::Fetched::bad("404"),
            }
        }
        fn prepare(&self, _: &str, _: &Value) -> elo_net::Fetched<Value> { unreachable!() }
        fn save(&self, _: &str, _: &Value) -> elo_net::Fetched<Value> { unreachable!() }
    }

    #[test]
    fn reading_a_profile_fetches_its_picture_and_keeps_could_not_look_apart() {
        let both = Fixed(serde_json::json!({"name": "Mara", "picture": "/api/face/0xa/pic.png?v=1"}),
                         Some(png(64, 64, 8, 6, 0)));
        let s = read(&both, "https://h", "0xa").unwrap();
        assert_eq!(s.name.as_deref(), Some("Mara"));
        assert!(s.picture.is_some());

        let none = Fixed(serde_json::json!({"name": null, "picture": null}), None);
        assert_eq!(read(&none, "https://h", "0xa").unwrap(), Shown::default());
    }
}
