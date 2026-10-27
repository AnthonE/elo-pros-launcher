//! One identity — the wallet is the account, and everything pulls from it.
//!
//! Operator, 2026-08-05: *"all name stuff need to be unifed: we have a wallet
//! for the steamid and then they can set ONE NAME IN ONE PLACE AND EVERYTHING
//! PULLS FROM THAT SAME WITH PFP."*
//!
//! `GET /api/who?handle=…` is that one place: it takes **a wallet, an npub or
//! a name** and answers with one identity, joined on the wallet (its own name
//! and picture, the seat it holds, its hive link). This module reads it, and
//! every surface calls this module rather than reading names out of whatever
//! it happens to be looking at.
//!
//! ## One name, and the evidence rides ON it
//!
//! There is exactly one `name`, and [`NameKind`] says who checked it:
//!
//! | kind | where it comes from | checked by |
//! |---|---|---|
//! | [`NameKind::Handle`] | the NIP-05 local part of a hive handle | the hive's handle index |
//! | [`NameKind::Declared`] | the wallet's own name (`/api/face`), or a hive display name | **nobody** |
//! | [`NameKind::Address`] | the wallet or npub itself | arithmetic |
//!
//! ⚠ A surface may render all three; it may never render `Declared` in the
//! clothes of `Handle`.

use crate::error::{HiveError, Result};
use elo_net::{Fetched, Net};
use serde::Deserialize;

/// Who checked this name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    /// The NIP-05 local part of a hive handle. Held by one wallet; it cannot
    /// be set by asserting it.
    Handle,
    /// The wallet's own signed name, or a hive display name — *theirs*, but
    /// the string is whatever they typed. Nobody checked that it is true.
    Declared,
    /// No name at all: the address, shortened.
    Address,
}

impl NameKind {
    /// The one-character marker every surface uses. `~` for a name nobody
    /// checked, blank for a held handle.
    pub fn mark(&self) -> char {
        match self {
            NameKind::Handle => ' ',
            NameKind::Declared | NameKind::Address => '~',
        }
    }

    /// True for a held hive handle — the one name somebody checked.
    pub fn checked(&self) -> bool {
        matches!(self, NameKind::Handle)
    }

    pub fn line(&self) -> &'static str {
        match self {
            NameKind::Handle => "a hive handle, held by this wallet",
            NameKind::Declared => "self-declared — nobody checked it",
            NameKind::Address => "no name set; this is the address",
        }
    }
}

/// The one identity. Built by [`resolve`], read by every surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub wallet: Option<String>,
    pub npub: Option<String>,
    pub nip05: Option<String>,
    /// **The one name.**
    pub name: String,
    /// Who checked it.
    pub kind: NameKind,
    /// **The one picture**, absolute — or empty, and then a surface draws the
    /// letter tile. The order is the site's own (`store.js` `faceSrc`): the
    /// wallet's own picture, the Elo Pros seat it holds, a hive picture.
    /// [`Identity::origin_face`] is the one a window may fetch.
    pub face: String,
    /// True when this identity is only an address with nothing set for it.
    pub bare: bool,
    /// How many people answered to this handle. A wallet or an npub names one;
    /// a self-declared name can be shared, and `/who` says so rather than
    /// picking a winner.
    pub matches: u32,
}

impl Identity {
    /// Did this handle name more than one person?
    pub fn ambiguous(&self) -> bool {
        self.matches > 1
    }

    /// The picture, only when it lives on `host` — the one a window may fetch.
    ///
    /// A hive picture is a url its owner chose on any host, and fetching it
    /// would tell that host every reader's IP; the site refuses it with its CSP
    /// (`img-src 'self'`) and the launcher refuses it here. A face on our own
    /// origin is our own bytes.
    pub fn origin_face(&self, host: &str) -> Option<&str> {
        let base = format!("{}/api/", host.trim_end_matches('/'));
        self.face.starts_with(&base).then_some(self.face.as_str())
    }

    /// Everything a surface needs, in one line: `~name` or ` name`.
    pub fn label(&self) -> String {
        format!("{}{}", self.kind.mark(), self.name).trim_start().to_string()
    }

    /// An identity for an address with nothing set for it: a name (the
    /// address, shortened) and a letter tile. No account exists to require.
    ///
    /// `_host` is kept so every caller still reads the same: a bare identity
    /// has no picture on any host, so it no longer builds one.
    pub fn bare(address: &str, _host: &str) -> Identity {
        Identity {
            wallet: address.starts_with("0x").then(|| address.to_string()),
            npub: address.starts_with("npub").then(|| address.to_string()),
            nip05: None,
            name: short(address),
            kind: NameKind::Address,
            matches: 0,
            face: String::new(),
            bare: true,
        }
    }
}

/// `0xabcd…1234` / `npub1abcd…wxyz` — enough to recognise, short enough for a
/// column. The one shortener, so two surfaces cannot disagree about a name.
pub fn short(address: &str) -> String {
    if address.len() <= 16 {
        return address.to_string();
    }
    // Byte slicing below is only safe on ASCII. An address is always ASCII; a
    // string that only claims to be one is clipped by characters instead, so
    // a stranger's value can never take a window down on a char boundary.
    if !address.is_ascii() {
        return crate::render::clip(address, 16);
    }
    let head = if address.starts_with("npub") { 10 } else { 6 };
    format!("{}…{}", &address[..head], &address[address.len() - 4..])
}

// ── the wire ────────────────────────────────────────────────────────────────
//
// Every struct ignores fields it does not name, so a reply carrying more (or
// an older origin's extra keys) still parses.

#[derive(Debug, Clone, Deserialize, Default)]
struct Face {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    picture: Option<String>,
}

/// The wallet's own name and picture (`meter/faces.py`). Wallet-signed, so it
/// is theirs; the name is still whatever they typed.
#[derive(Debug, Clone, Deserialize, Default)]
struct Profile {
    #[serde(default)]
    name: Option<String>,
    /// Already an `/api/face/…` path.
    #[serde(default)]
    picture: Option<String>,
}

/// The Elo Pros seat a wallet holds — the one face a stranger can verify,
/// with `ownerOf`. `src` is a ROUTE path (`/seat/{id}/face.svg`).
#[derive(Debug, Clone, Deserialize, Default)]
struct Seat {
    #[serde(default)]
    src: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct WhoIdentity {
    /// The name `/who` picked: the wallet's own name, else its handle.
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    wallet: Option<String>,
    #[serde(default)]
    npub: Option<String>,
    #[serde(default)]
    nip05: Option<String>,
    /// The wallet's picture as a route path. An API PATH, not an absolute url.
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    face: Option<Face>,
    #[serde(default)]
    profile: Option<Profile>,
    #[serde(default)]
    seat: Option<Seat>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Subject {
    #[serde(default)]
    wallet: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct WhoReply {
    /// What `/api/who` calls the resolved row.
    #[serde(default)]
    found: Option<WhoIdentity>,
    /// The account itself — *a profile is the wallet.*
    #[serde(default)]
    subject: Option<Subject>,
    #[serde(default)]
    matches: Option<u32>,
    #[serde(default)]
    identity: Option<WhoIdentity>,
    #[serde(default)]
    error: Option<String>,
    // A bare `agent`/`wallet` at the top level, for the flatter rows the
    // directory serves. Tried second so a nested row always wins.
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    wallet: Option<String>,
    #[serde(default)]
    npub: Option<String>,
    #[serde(default)]
    nip05: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    face: Option<Face>,
}

fn non_empty(s: Option<&String>) -> Option<String> {
    s.map(|x| x.trim()).filter(|x| !x.is_empty()).map(str::to_string)
}

/// Turn a `/who` reply into the one identity.
///
/// The precedence is the whole point, and it is fixed here so no surface can
/// invent its own: **a held handle, then a declared name, then the address.**
/// Never the other way round.
fn build(reply: &WhoReply, handle: &str, host: &str) -> Identity {
    let id = reply
        .found
        .clone()
        .or_else(|| reply.identity.clone())
        .unwrap_or_default();
    let pick = |a: Option<&String>, b: Option<&String>| non_empty(a).or_else(|| non_empty(b));

    // The wallet is the account, so `subject.wallet` outranks the row's copy.
    let subject = reply.subject.clone().unwrap_or_default();
    let wallet = non_empty(subject.wallet.as_ref())
        .or_else(|| pick(id.wallet.as_ref(), reply.wallet.as_ref()));
    let npub = pick(id.npub.as_ref(), reply.npub.as_ref());
    let nip05 = pick(id.nip05.as_ref(), reply.nip05.as_ref());
    let agent = pick(id.agent.as_ref(), reply.agent.as_ref());
    let avatar = pick(id.avatar.as_ref(), reply.avatar.as_ref());
    let face = id.face.clone().or_else(|| reply.face.clone()).unwrap_or_default();
    let profile = id.profile.clone().unwrap_or_default();
    // The wallet's own signed name leads the hive's display name: it is the
    // name the game and the store show for this account. `agent` is the
    // server's pick of the same.
    let declared = non_empty(profile.name.as_ref())
        .or_else(|| non_empty(face.display_name.as_ref()))
        .or(agent);
    let picture = non_empty(face.picture.as_ref());

    // A handle is the NIP-05 local part, and it is the one name held by a
    // single wallet. It cannot be set by asserting it.
    let held = nip05
        .as_deref()
        .and_then(|n| n.split('@').next())
        .map(str::to_string)
        .filter(|n| !n.is_empty());

    let (name, kind) = match (held, declared) {
        (Some(n), _) => (n, NameKind::Handle),
        (None, Some(d)) => (d, NameKind::Declared),
        (None, None) => (short(handle), NameKind::Address),
    };

    let host = host.trim_end_matches('/');
    let seat = id.seat.as_ref().and_then(|s| non_empty(s.src.as_ref()));
    let face_url = non_empty(profile.picture.as_ref())
        .and_then(|p| on_origin(host, &p))
        .or_else(|| seat.and_then(|p| on_origin(host, &p)))
        // A hive picture is absolute and belongs to whoever posted it — kept
        // as the identity's face, never fetched by a window (`origin_face`).
        .or_else(|| picture.filter(|p| p.starts_with("https://")))
        .or_else(|| avatar.and_then(|a| on_origin(host, &a)))
        .unwrap_or_default();

    Identity {
        bare: kind == NameKind::Address && face_url.is_empty() && npub.is_none(),
        matches: reply.matches.unwrap_or(1),
        wallet,
        npub,
        nip05,
        name,
        kind,
        face: face_url,
    }
}

/// A path the origin wrote, as an absolute url on it — or `None` for anything
/// that is not a plain path on our own host.
///
/// ⚠ **The `/api` is added here because the routes are declared bare.** `/who`
/// answers `seat.src: "/seat/…/face.svg"`, a meter route path; nginx serves it
/// under `/api/`, and at the site root it is a 404 page. The site prefixes it
/// the same way (`store.js`). A path already under `/api/` is left alone, and
/// `//host/…` looks like a path and is another host, so it is refused.
fn on_origin(host: &str, path: &str) -> Option<String> {
    if !path.starts_with('/') || path.starts_with("//") {
        return None;
    }
    Some(if path.starts_with("/api/") {
        format!("{host}{path}")
    } else {
        format!("{host}/api{path}")
    })
}

/// Resolve one handle — **a wallet, an npub, or a name.**
///
/// The wallet is the account (the operator's *"steamid"*), so passing an
/// address is the ordinary call, and `/who` answers for any wallet. A handle
/// nobody answers to is **not an error**: it comes back [`Identity::bare`].
pub fn resolve(net: &Net, host: &str, handle: &str) -> Fetched<Identity> {
    let handle = handle.trim();
    if handle.is_empty() {
        return Fetched::bad("no handle to resolve");
    }
    let url = format!(
        "{}/api/who?handle={}",
        host.trim_end_matches('/'),
        urlencode(handle)
    );
    let got = net.get_json(&url);
    // ⚠ **404 here means "nobody by that name", not "we could not look".**
    if got.status == Some(404) {
        return Fetched::good(Identity::bare(handle, host));
    }
    match got.value {
        Some(v) => match serde_json::from_value::<WhoReply>(v) {
            Ok(reply) if reply.error.is_some() => Fetched::good(Identity::bare(handle, host)),
            Ok(reply) => Fetched::good(build(&reply, handle, host)),
            Err(e) => Fetched::bad(format!("the directory answered in a shape we do not know: {e}")),
        },
        None if got.reachable => Fetched::bad(got.why),
        None => Fetched::unreachable(got.why),
    }
}

/// Percent-encode everything that is not unreserved. A handle can be a name a
/// person typed, so it can contain anything.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The identity a hive message speaks under, resolved through the same place.
///
/// The room read hands back `who` (a held handle) and `display` (self-declared) per
/// message. Rather than let a chat renderer invent its own precedence, this
/// maps them onto the same [`Identity`] shape the rest of the client uses.
///
/// It does **not** call `/who` per message — a room read is sixty rows and
/// that would be sixty requests. What it does is make the *rule* the same one.
pub fn from_message(m: &crate::Message, host: &str) -> Identity {
    let mut id = Identity::bare(&m.npub, host);
    if m.house {
        id.name = "the house".into();
        id.kind = NameKind::Handle;
        return id;
    }
    if let Some(w) = m.who.as_deref().filter(|s| !s.trim().is_empty()) {
        id.name = w.trim().to_string();
        id.kind = NameKind::Handle;
    } else if let Some(d) = m.display.as_deref().filter(|s| !s.trim().is_empty()) {
        id.name = d.trim().to_string();
        id.kind = NameKind::Declared;
    }
    id
}

/// Cheap check that a string could be a wallet — enough to route a handle,
/// never enough to trust one.
pub fn looks_like_wallet(s: &str) -> bool {
    let s = s.trim();
    s.len() == 42 && s.starts_with("0x") && s[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Refuse a handle that is obviously not one, before it becomes a URL.
pub fn check_handle(s: &str) -> Result<()> {
    let s = s.trim();
    if s.is_empty() {
        return Err(HiveError::new("no handle given"));
    }
    if s.len() > 200 {
        return Err(HiveError::new("that handle is too long to be one"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn who(v: serde_json::Value) -> Identity {
        build(
            &serde_json::from_value(v).unwrap(),
            "0xabc",
            "https://elo.test",
        )
    }

    #[test]
    fn a_held_handle_wins_over_everything() {
        // It is the one name a single wallet holds, and it cannot be set by
        // asserting it.
        let id = who(json!({"identity": {
            "nip05": "magus@elopros.com",
            "agent": "Someone",
            "face": {"display_name": "TOTALLY THE HOUSE"},
        }}));
        assert_eq!(id.name, "magus");
        assert_eq!(id.kind, NameKind::Handle);
        assert_eq!(id.label(), "magus", "a held handle wears no marker");
    }

    #[test]
    fn a_declared_name_is_used_and_marked() {
        let id = who(json!({"identity": {"face": {"display_name": "Anthone"}}}));
        assert_eq!(id.name, "Anthone");
        assert_eq!(id.kind, NameKind::Declared);
        assert_eq!(id.label(), "~Anthone", "nobody checked it, and it says so");
        assert!(!id.kind.checked());
    }

    #[test]
    fn the_precedence_falls_all_the_way_to_the_address() {
        let id = who(json!({"identity": {"agent": "Mara"}}));
        assert_eq!(id.kind, NameKind::Declared, "the server's pick is still self-declared");
        assert_eq!(id.name, "Mara");

        let id = who(json!({"identity": {}}));
        assert_eq!(id.kind, NameKind::Address);
        assert_eq!(id.name, "0xabc");
    }

    #[test]
    fn a_face_is_a_url_the_origin_serves_or_nothing() {
        let bare = who(json!({"identity": {}}));
        assert_eq!(bare.face, "", "no picture is a tile, not an invented url");
        assert_eq!(bare.origin_face("https://elo.test"), None);

        // `avatar` is a route path: it is served under /api.
        let pictured = who(json!({"identity": {"avatar": "/face/0xabc/pic.png"}}));
        assert_eq!(pictured.face, "https://elo.test/api/face/0xabc/pic.png");
        assert_eq!(pictured.origin_face("https://elo.test"), Some(pictured.face.as_str()));

        // A hive picture is kept as the face, and is never one a window may
        // fetch.
        let hive = who(json!({"identity": {"face": {"picture": "https://media.example/p.png"}}}));
        assert_eq!(hive.face, "https://media.example/p.png");
        assert_eq!(hive.origin_face("https://elo.test"), None);

        // `//host/…` looks like a path and is another host.
        let sneaky = who(json!({"identity": {"avatar": "//evil.example/x.svg"}}));
        assert_eq!(sneaky.face, "");
    }

    /// The site's order: the wallet's own picture, the seat it holds, a hive
    /// picture.
    #[test]
    fn the_wallets_own_picture_leads_then_the_seat() {
        let all = json!({"identity": {
            "face": {"picture": "https://media.example/p.png"},
            "seat": {"src": "/seat/41/face.svg"},
            "profile": {"name": "Mara", "picture": "/api/face/0xabc/pic.png?v=1a2b"},
        }});
        let id = who(all.clone());
        assert_eq!(id.face, "https://elo.test/api/face/0xabc/pic.png?v=1a2b");
        assert_eq!(id.name, "Mara");
        assert_eq!(id.kind, NameKind::Declared, "the wallet signed it; nobody checked it");

        let mut no_profile = all;
        no_profile["identity"]["profile"] = json!(null);
        let id = who(no_profile);
        assert_eq!(id.face, "https://elo.test/api/seat/41/face.svg", "a seat path gets /api");
    }

    #[test]
    fn a_flat_reply_resolves_the_same_as_a_nested_one() {
        // The directory serves a flatter row than /who does; both have to land
        // on one identity or two surfaces disagree about a name.
        let nested = who(json!({"identity": {"nip05": "magus@x", "wallet": "0xdead"}}));
        let flat = who(json!({"nip05": "magus@x", "wallet": "0xdead"}));
        assert_eq!(nested.name, flat.name);
        assert_eq!(nested.kind, flat.kind);
        assert_eq!(nested.wallet, flat.wallet);
    }

    #[test]
    fn a_nested_identity_beats_a_flat_field() {
        let id = who(json!({
            "identity": {"nip05": "inner@x"},
            "nip05": "outer@x",
        }));
        assert_eq!(id.name, "inner");
    }

    #[test]
    fn blank_strings_do_not_count_as_names() {
        // A server that answers `""` must not produce a nameless identity that
        // renders as an empty column.
        let id = who(json!({"identity": {
            "nip05": "   ", "agent": "", "face": {"display_name": "  "},
        }}));
        assert_eq!(id.kind, NameKind::Address);
        assert_eq!(id.name, "0xabc");
    }

    #[test]
    fn an_unknown_address_is_bare_and_still_has_a_name() {
        let id = Identity::bare("0x1234567890abcdef1234567890abcdef12345678", "https://elo.test");
        assert!(id.bare);
        assert_eq!(id.kind, NameKind::Address);
        assert_eq!(id.name, "0x1234…5678");
        assert_eq!(id.face, "", "a bare address draws a tile");
        assert_eq!(id.wallet.as_deref(), Some("0x1234567890abcdef1234567890abcdef12345678"));
        assert_eq!(id.npub, None);
        assert_eq!(id.matches, 0);
        assert!(!id.ambiguous());
    }

    #[test]
    fn shortening_is_done_in_one_place() {
        assert_eq!(short("0x1234567890abcdef1234567890abcdef12345678"), "0x1234…5678");
        assert_eq!(
            short("npub1td6cukputruw8emfxhtddy3vtd0htf0m2ae7u00gpygn8jwf782qxrtkt6"),
            "npub1td6cu…tkt6"
        );
        assert_eq!(short("short"), "short", "nothing to shorten");
    }

    #[test]
    fn a_handle_is_encoded_before_it_becomes_a_url() {
        assert_eq!(urlencode("magus"), "magus");
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("../../etc/passwd"), "..%2F..%2Fetc%2Fpasswd");
        assert_eq!(urlencode("龍"), "%E9%BE%8D");
    }

    #[test]
    fn a_hive_message_resolves_by_the_same_rule() {
        let msg = |who: Option<&str>, display: Option<&str>, house: bool| crate::Message {
            id: "x".into(),
            at: 0,
            kind: 1,
            content: String::new(),
            npub: "npub1abcdefghijklmnop".into(),
            who: who.map(str::to_string),
            display: display.map(str::to_string),
            house,
            ..crate::Message::default()
        };
        assert_eq!(from_message(&msg(None, None, true), "h").kind, NameKind::Handle);
        assert_eq!(from_message(&msg(None, None, true), "h").name, "the house");
        assert_eq!(from_message(&msg(Some("magus"), Some("fake"), false), "h").name, "magus");
        assert_eq!(from_message(&msg(Some("magus"), None, false), "h").kind, NameKind::Handle);
        assert_eq!(from_message(&msg(None, Some("Anthone"), false), "h").kind, NameKind::Declared);
        // Blank `who` must not beat a real `display`.
        assert_eq!(from_message(&msg(Some("  "), Some("Anthone"), false), "h").name, "Anthone");
        assert_eq!(from_message(&msg(None, None, false), "h").kind, NameKind::Address);
    }

    #[test]
    fn the_current_who_shape_resolves() {
        // The shape `meter/directory.py` answers today, with the fields an
        // older origin named (vow_id, conduct, reputation) still present and
        // null: they parse and are ignored.
        let id = who(json!({
            "handle": "0x2444",
            "handle_kind": "wallet",
            "matches": 1,
            "authoritative": true,
            "found": {
                "agent": "Mara",
                "kind": "unknown",
                "vow_id": null,
                "wallet": "0x24445EFddB08d4938E3E3627042B2Cf4063d9092",
                "nip05": null,
                "avatar": "/face/0x2444/pic.png",
                "profile": {"name": "Mara", "picture": "/face/0x2444/pic.png"},
                "conduct": null,
                "reputation": null,
                "seat": null,
            },
            "subject": {"kind": "wallet", "wallet": "0x24445EFddB08d4938E3E3627042B2Cf4063d9092",
                        "identities": 1, "reputation": null, "house": null},
        }));
        assert_eq!(id.name, "Mara");
        assert_eq!(id.kind, NameKind::Declared);
        assert_eq!(id.face, "https://elo.test/api/face/0x2444/pic.png");
        assert!(!id.bare);
        assert!(!id.ambiguous());
    }

    #[test]
    fn a_shared_name_says_so_rather_than_picking_quietly() {
        // A self-declared name is not unique; `/who` answers with the count
        // and the surface says it showed one of several.
        let many = who(json!({"matches": 2, "found": {"agent": "Mara", "wallet": "0xa"}}));
        assert_eq!(many.matches, 2);
        assert!(many.ambiguous());
        let one = who(json!({"matches": 1, "found": {"agent": "Solo", "wallet": "0xb"}}));
        assert!(!one.ambiguous());
    }

    #[test]
    fn the_subjects_wallet_outranks_the_rows_copy() {
        // The account is the address; if the two ever disagreed the account
        // is the one that is right.
        let id = who(json!({
            "found": {"wallet": "0xfromtherow", "agent": "x"},
            "subject": {"kind": "wallet", "wallet": "0xtheaccount"},
        }));
        assert_eq!(id.wallet.as_deref(), Some("0xtheaccount"));
    }

    #[test]
    fn a_wallet_is_recognised_only_when_it_is_shaped_like_one() {
        assert!(looks_like_wallet("0x1234567890abcdef1234567890abcdef12345678"));
        assert!(!looks_like_wallet("0x1234"));
        assert!(!looks_like_wallet("npub1abc"));
        assert!(!looks_like_wallet("0xZZZ4567890abcdef1234567890abcdef12345678"));
    }
}
