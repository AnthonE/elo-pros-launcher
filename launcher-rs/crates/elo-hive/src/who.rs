//! One identity — the wallet is the account, and everything pulls from it.
//!
//! Operator, 2026-08-05: *"all name stuff need to be unifed: we have a wallet
//! for the steamid and then they can set ONE NAME IN ONE PLACE AND EVERYTHING
//! PULLS FROM THAT SAME WITH PFP."*
//!
//! ## The join already existed; nothing used it
//!
//! This was settled on 2026-08-02 — *"the profile is a view, not a table"* —
//! and `GET /api/who?handle=…` is that view. It takes **a wallet, an
//! npub, a vow id or a sworn name** and answers with one identity. The comment
//! in `meter/directory.py` even says it out loud: *"One face across every
//! surface."*
//!
//! So the fix is not a new store. It is that the client was reading names out
//! of whatever surface it happened to be looking at — a hive message's `who`,
//! a kind:0 `display`, a bare npub — instead of asking the one place. This
//! module is that one place, and every surface calls it.
//!
//! ## One name, and the evidence rides ON it
//!
//! There is a real distinction under the names — a handle nobody checked is
//! not a handle the register swore — and collapsing it would let anyone print
//! themselves a sworn name. But that never needed to be *two names*.
//!
//! `ACHIEVEMENTS.md` already solved this shape for badges: **a claim plus a
//! named evidence kind**, rendered carrying its own kind. Applied here: there
//! is exactly one `name`, and [`NameKind`] says who checked it. The thing that
//! varies is not the name — it is the evidence under it.
//!
//! | kind | where it comes from | checked by |
//! |---|---|---|
//! | [`NameKind::Sworn`] | the NIP-05 local part in the locked index | the register |
//! | [`NameKind::Declared`] | `face.display_name` (`POST /api/hive/face`) | **nobody** |
//! | [`NameKind::Derived`] | the vow's agent name | the vow |
//! | [`NameKind::Address`] | the wallet or npub itself | arithmetic |
//!
//! ⚠ A surface may render all four; it may never render `Declared` in the
//! clothes of `Sworn`.

use crate::error::{HiveError, Result};
use elo_net::{Fetched, Net};
use serde::Deserialize;

/// Who checked this name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    /// The NIP-05 local part on the register. Changing it burns SCRY.
    Sworn,
    /// `face.display_name` — wallet-signed, so it is *theirs*, but the string
    /// itself is whatever they typed. Nobody checked that it is true.
    Declared,
    /// The vow's own agent name.
    Derived,
    /// No name at all: the address, shortened.
    Address,
}

impl NameKind {
    /// The one-character marker every surface uses. `~` for a name nobody
    /// checked, blank for one the register swore.
    pub fn mark(&self) -> char {
        match self {
            NameKind::Sworn => ' ',
            NameKind::Declared | NameKind::Derived | NameKind::Address => '~',
        }
    }

    pub fn sworn(&self) -> bool {
        matches!(self, NameKind::Sworn)
    }

    pub fn line(&self) -> &'static str {
        match self {
            NameKind::Sworn => "sworn on the register",
            NameKind::Declared => "self-declared — nobody checked it",
            NameKind::Derived => "from the vow",
            NameKind::Address => "no name set; this is the address",
        }
    }
}

/// The one identity. Built by [`resolve`], read by every surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub wallet: Option<String>,
    pub npub: Option<String>,
    pub vow_id: Option<String>,
    pub nip05: Option<String>,
    /// **The one name.**
    pub name: String,
    /// Who checked it.
    pub kind: NameKind,
    /// **The one picture**, absolute. Never empty — the register's derived
    /// mark is the fallback, so nobody is faceless.
    pub face: String,
    /// True when this identity is only an address with nothing bound to it.
    pub bare: bool,
    /// How many vows this account swore.
    ///
    /// ⚠ **More than one is normal, and this is why "ONE NAME" is not yet
    /// true.** `/api/who` on a wallet answers: *"this wallet is an identifier,
    /// so the vows below are ONE party's, not a collision. Reputation and the
    /// house hang off the wallet."* So the account is the wallet — and the
    /// wallet's `subject` block carries **no name and no face of its own**.
    /// Every name belongs to a vow.
    ///
    /// Until the meter gives the wallet its own name and face (or a designated
    /// primary vow), `name` here is whichever vow `/who` picked, and a surface
    /// showing it for a multi-vow wallet is showing *one of* several. That is
    /// said out loud rather than papered over.
    pub vows: u32,
}

impl Identity {
    /// Does this account answer to more than one name?
    pub fn ambiguous(&self) -> bool {
        self.vows > 1
    }

    /// Everything a surface needs, in one line: `~name` or ` name`.
    pub fn label(&self) -> String {
        format!("{}{}", self.kind.mark(), self.name).trim_start().to_string()
    }

    /// An identity for an address the town knows nothing about.
    ///
    /// A bare wallet still gets a name and a face: *a bare wallet with no vow
    /// still gets a page — no account exists to require.*
    pub fn bare(address: &str, host: &str) -> Identity {
        Identity {
            wallet: address.starts_with("0x").then(|| address.to_string()),
            npub: address.starts_with("npub").then(|| address.to_string()),
            vow_id: None,
            nip05: None,
            name: short(address),
            kind: NameKind::Address,
            vows: 0,
            face: format!("{}/api/mark/{}.svg", host.trim_end_matches('/'), address),
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
    let head = if address.starts_with("npub") { 10 } else { 6 };
    format!("{}…{}", &address[..head], &address[address.len() - 4..])
}

// ── the wire ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
struct Face {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    picture: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct WhoIdentity {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    wallet: Option<String>,
    #[serde(default)]
    npub: Option<String>,
    #[serde(default)]
    nip05: Option<String>,
    #[serde(default)]
    vow_id: Option<String>,
    /// The register's derived mark. An API PATH, not an absolute url.
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    face: Option<Face>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Subject {
    /// `"wallet"` when the handle named one. Parsed but unused: the shape is
    /// documented here so a future reader sees the whole reply, and the
    /// resolver deliberately does not branch on it — a wallet and a vow both
    /// produce one identity.
    #[serde(default)]
    #[allow(dead_code)]
    kind: Option<String>,
    #[serde(default)]
    wallet: Option<String>,
    /// How many vows this wallet swore. **More than one is normal**, and the
    /// server says so: *"this wallet is an identifier, so the vows below are
    /// ONE party's, not a collision."*
    #[serde(default)]
    identities: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct WhoReply {
    /// What `/api/who` actually calls the resolved row. Measured 2026-08-05 —
    /// the first pass looked for `identity` and got nothing, which is why
    /// every real wallet came back bare.
    #[serde(default)]
    found: Option<WhoIdentity>,
    /// The account itself — *a profile is the wallet.*
    #[serde(default)]
    subject: Option<Subject>,
    #[serde(default)]
    matches: Option<u32>,
    #[serde(default)]
    identity: Option<WhoIdentity>,
    /// `/who` answers 200 with an `error` when nothing is sworn for a handle.
    #[serde(default)]
    error: Option<String>,
    // A bare `agent`/`wallet` at the top level, for the flatter shapes the
    // directory serves. Tried second so a nested `identity` always wins.
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    wallet: Option<String>,
    #[serde(default)]
    npub: Option<String>,
    #[serde(default)]
    nip05: Option<String>,
    #[serde(default)]
    vow_id: Option<String>,
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
/// invent its own: **sworn handle, then a declared face name, then the vow's
/// agent, then the address.** Never the other way round.
fn build(reply: &WhoReply, handle: &str, host: &str) -> Identity {
    let id = reply
        .found
        .clone()
        .or_else(|| reply.identity.clone())
        .unwrap_or_default();
    let pick = |a: Option<&String>, b: Option<&String>| non_empty(a).or_else(|| non_empty(b));

    // The wallet is the account, so `subject.wallet` outranks a vow's copy.
    let subject = reply.subject.clone().unwrap_or_default();
    let wallet = non_empty(subject.wallet.as_ref())
        .or_else(|| pick(id.wallet.as_ref(), reply.wallet.as_ref()));
    let npub = pick(id.npub.as_ref(), reply.npub.as_ref());
    let nip05 = pick(id.nip05.as_ref(), reply.nip05.as_ref());
    let vow_id = pick(id.vow_id.as_ref(), reply.vow_id.as_ref());
    let agent = pick(id.agent.as_ref(), reply.agent.as_ref());
    let avatar = pick(id.avatar.as_ref(), reply.avatar.as_ref());
    let face = id.face.clone().or_else(|| reply.face.clone()).unwrap_or_default();
    let declared = non_empty(face.display_name.as_ref());
    let picture = non_empty(face.picture.as_ref());

    // The sworn name is the NIP-05 local part, and it is the only one the
    // register vouches for. It cannot be set by asserting it.
    let sworn = nip05
        .as_deref()
        .and_then(|n| n.split('@').next())
        .map(str::to_string)
        .filter(|n| !n.is_empty());

    let (name, kind) = match (sworn, declared, agent) {
        (Some(n), _, _) => (n, NameKind::Sworn),
        (None, Some(d), _) => (d, NameKind::Declared),
        (None, None, Some(a)) => (a, NameKind::Derived),
        (None, None, None) => (short(handle), NameKind::Address),
    };

    let host = host.trim_end_matches('/');
    let face_url = picture
        // `avatar` is a path on our own origin; `picture` is already absolute
        // and belongs to the hive, so only one of them gets a prefix.
        .or_else(|| avatar.map(|a| format!("{host}{a}")))
        .unwrap_or_else(|| format!("{host}/api/mark/{handle}.svg"));

    Identity {
        bare: wallet.is_none() && npub.is_none() && vow_id.is_none(),
        // How many names this account answers to. One is the ordinary case;
        // more is normal and must not be hidden — see `Identity::vows`.
        vows: reply
            .matches
            .or(subject.identities)
            .unwrap_or(if id.vow_id.is_some() { 1 } else { 0 }),
        wallet,
        npub,
        vow_id,
        nip05,
        name,
        kind,
        face: face_url,
    }
}

/// Resolve one handle — **a wallet, an npub, a vow id, or a sworn name.**
///
/// The wallet is the account (the operator's *"steamid"*), so passing an
/// address is the ordinary call. An address the town knows nothing about is
/// **not an error**: it comes back [`Identity::bare`], because a bare wallet
/// still gets a name and a face and no account exists to require.
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
    // `/api/who` answers 404 for an address with nothing sworn to it, and that
    // is a true statement about a real address: a bare wallet still gets a
    // page, and no account exists to require. Rendering it as an
    // error would make every un-sworn player look like a broken lookup.
    if got.status == Some(404) {
        // …but "not sworn" is not "unknown". The directory carries a row for
        // every vow — an agent name, a wallet, a mark — whether or not a key
        // was ever bound, so it answers for people `/who` will not.
        return match from_directory(net, host, handle) {
            Some(id) => Fetched::good(id),
            None => Fetched::good(Identity::bare(handle, host)),
        };
    }
    match got.value {
        Some(v) => match serde_json::from_value::<WhoReply>(v) {
            // An `error` means nothing is sworn for this handle, which is a
            // real answer about a real address — not a failed lookup.
            Ok(reply) if reply.error.is_some() => Fetched::good(Identity::bare(handle, host)),
            Ok(reply) => Fetched::good(build(&reply, handle, host)),
            Err(e) => Fetched::bad(format!("the register answered in a shape we do not know: {e}")),
        },
        None if got.reachable => Fetched::bad(got.why),
        None => Fetched::unreachable(got.why),
    }
}

/// Second pass: find a handle in the town directory.
///
/// ⚠ **This is a client-side join, and it is here because the server has no
/// route for it.** Measured 2026-08-05:
///
/// * `GET /api/who?handle=…` answers **404** unless the identity is *sworn* —
///   it needs a bound npub or NIP-05, so an ordinary wallet with a vow gets
///   nothing;
/// * `GET /api/directory?q=…` matches on the **agent name only** — a wallet or
///   a vow id returns `count: 0`.
///
/// So neither endpoint resolves *"who is this wallet"*, which is the exact
/// question the client asks most. Until one does, this fetches the directory
/// and scans it. That is honest at 51 entries and will stop being honest;
/// **the fix belongs in the meter** — either `/who` answering for an unsworn
/// wallet, or `/directory?q=` matching wallet and vow id.
fn from_directory(net: &Net, host: &str, handle: &str) -> Option<Identity> {
    let url = format!("{}/api/directory?limit=500", host.trim_end_matches('/'));
    let rows = net.get_json(&url).value?;
    let entries = rows.get("entries")?.as_array()?;
    let needle = handle.trim().to_ascii_lowercase();
    for e in entries {
        let hit = ["wallet", "vow_id", "npub", "nip05", "agent"].iter().any(|k| {
            e.get(*k)
                .and_then(|v| v.as_str())
                .is_some_and(|v| v.to_ascii_lowercase() == needle)
        });
        if !hit {
            continue;
        }
        let reply: WhoReply = serde_json::from_value(e.clone()).ok()?;
        let mut id = build(&reply, handle, host);
        // The directory row proves a vow exists, so this is not a bare address
        // even when no key is bound to it.
        id.bare = false;
        return Some(id);
    }
    None
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
/// The room read hands back `who` (sworn) and `display` (self-declared) per
/// message. Rather than let a chat renderer invent its own precedence, this
/// maps them onto the same [`Identity`] shape the rest of the client uses.
///
/// It does **not** call `/who` per message — a room read is sixty rows and
/// that would be sixty requests. What it does is make the *rule* the same one.
pub fn from_message(m: &crate::Message, host: &str) -> Identity {
    let mut id = Identity::bare(&m.npub, host);
    if m.house {
        id.name = "the house".into();
        id.kind = NameKind::Sworn;
        return id;
    }
    if let Some(w) = m.who.as_deref().filter(|s| !s.trim().is_empty()) {
        id.name = w.trim().to_string();
        id.kind = NameKind::Sworn;
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
    fn the_sworn_handle_wins_over_everything() {
        // It is the only name the register vouches for, and it cannot be set
        // by asserting it.
        let id = who(json!({"identity": {
            "nip05": "magus@elopros.com",
            "agent": "the vow's own name",
            "face": {"display_name": "TOTALLY THE HOUSE"},
        }}));
        assert_eq!(id.name, "magus");
        assert_eq!(id.kind, NameKind::Sworn);
        assert_eq!(id.label(), "magus", "a sworn name wears no marker");
    }

    #[test]
    fn a_declared_name_is_used_and_marked() {
        let id = who(json!({"identity": {
            "agent": "the vow's own name",
            "face": {"display_name": "Anthone"},
        }}));
        assert_eq!(id.name, "Anthone");
        assert_eq!(id.kind, NameKind::Declared);
        assert_eq!(id.label(), "~Anthone", "nobody checked it, and it says so");
        assert!(!id.kind.sworn());
    }

    #[test]
    fn the_precedence_falls_all_the_way_to_the_address() {
        let id = who(json!({"identity": {"agent": "a shade of the floor"}}));
        assert_eq!(id.kind, NameKind::Derived);
        assert_eq!(id.name, "a shade of the floor");

        let id = who(json!({"identity": {}}));
        assert_eq!(id.kind, NameKind::Address);
        assert_eq!(id.name, "0xabc");
    }

    #[test]
    fn nobody_is_faceless() {
        // The register's mark is the fallback, so an identity always has a
        // picture: clearing a field restores the default rather than blanking
        // the profile.
        let bare = who(json!({"identity": {}}));
        assert!(bare.face.starts_with("https://elo.test/"), "{}", bare.face);

        // `avatar` is a path on our origin and gets the prefix…
        let marked = who(json!({"identity": {"avatar": "/vow/77c2/mark.svg"}}));
        assert_eq!(marked.face, "https://elo.test/vow/77c2/mark.svg");

        // …while `picture` is already absolute and must NOT be mangled.
        let faced = who(json!({"identity": {
            "avatar": "/vow/77c2/mark.svg",
            "face": {"picture": "https://media.example/p.png"},
        }}));
        assert_eq!(faced.face, "https://media.example/p.png");
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
    fn an_unsworn_address_is_bare_and_still_has_a_name_and_a_face() {
        let id = Identity::bare("0x1234567890abcdef1234567890abcdef12345678", "https://elo.test");
        assert!(id.bare);
        assert_eq!(id.kind, NameKind::Address);
        assert_eq!(id.name, "0x1234…5678");
        assert!(!id.face.is_empty());
        assert_eq!(id.wallet.as_deref(), Some("0x1234567890abcdef1234567890abcdef12345678"));
        assert_eq!(id.npub, None);
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
        };
        assert_eq!(from_message(&msg(None, None, true), "h").kind, NameKind::Sworn);
        assert_eq!(from_message(&msg(None, None, true), "h").name, "the house");
        assert_eq!(from_message(&msg(Some("magus"), Some("fake"), false), "h").name, "magus");
        assert_eq!(from_message(&msg(Some("magus"), None, false), "h").kind, NameKind::Sworn);
        assert_eq!(from_message(&msg(None, Some("Anthone"), false), "h").kind, NameKind::Declared);
        // Blank `who` must not beat a real `display`.
        assert_eq!(from_message(&msg(Some("  "), Some("Anthone"), false), "h").name, "Anthone");
        assert_eq!(from_message(&msg(None, None, false), "h").kind, NameKind::Address);
    }

    #[test]
    fn the_real_who_shape_resolves() {
        // Measured against the live origin 2026-08-05. The first pass looked
        // for `identity` — the actual key is `found` — so every real wallet
        // came back bare and the bug was invisible until it was driven.
        let id = who(json!({
            "handle": "0x2444",
            "handle_kind": "wallet",
            "matches": 2,
            "found": {
                "agent": "Dummmy",
                "vow_id": "1a434221c5333fa7",
                "wallet": "0x24445EFddB08d4938E3E3627042B2Cf4063d9092",
                "avatar": "/vow/1a434221c5333fa7/mark.svg",
            },
            "subject": {
                "kind": "wallet",
                "wallet": "0x24445EFddB08d4938E3E3627042B2Cf4063d9092",
                "identities": 2,
            },
        }));
        assert_eq!(id.name, "Dummmy");
        assert_eq!(id.kind, NameKind::Derived);
        assert_eq!(id.vow_id.as_deref(), Some("1a434221c5333fa7"));
        assert_eq!(id.face, "https://elo.test/vow/1a434221c5333fa7/mark.svg");
        assert!(!id.bare, "a wallet with a vow is not bare");
    }

    #[test]
    fn a_wallet_with_several_vows_says_so_rather_than_picking_quietly() {
        // "this wallet is an identifier, so the vows below are ONE party's,
        // not a collision" — the account is the wallet, and the wallet has no
        // name of its own yet, so showing one of several must be visible.
        let many = who(json!({
            "matches": 2,
            "found": {"agent": "Dummmy", "vow_id": "a"},
            "subject": {"kind": "wallet", "wallet": "0xdead", "identities": 2},
        }));
        assert_eq!(many.vows, 2);
        assert!(many.ambiguous());

        let one = who(json!({
            "matches": 1,
            "found": {"agent": "Solo", "vow_id": "b"},
            "subject": {"kind": "wallet", "wallet": "0xdead", "identities": 1},
        }));
        assert!(!one.ambiguous());
        assert_eq!(one.vows, 1);
    }

    #[test]
    fn the_subjects_wallet_outranks_a_vows_copy() {
        // The account is the address. A vow carries a copy of it, and if the
        // two ever disagreed the account is the one that is right.
        let id = who(json!({
            "found": {"wallet": "0xfromthevow", "agent": "x"},
            "subject": {"kind": "wallet", "wallet": "0xtheaccount", "identities": 1},
        }));
        assert_eq!(id.wallet.as_deref(), Some("0xtheaccount"));
    }

    #[test]
    fn a_bare_address_has_no_vows() {
        let id = Identity::bare("0xabc", "https://elo.test");
        assert_eq!(id.vows, 0);
        assert!(!id.ambiguous());
    }

    #[test]
    fn a_wallet_is_recognised_only_when_it_is_shaped_like_one() {
        assert!(looks_like_wallet("0x1234567890abcdef1234567890abcdef12345678"));
        assert!(!looks_like_wallet("0x1234"));
        assert!(!looks_like_wallet("npub1abc"));
        assert!(!looks_like_wallet("0xZZZ4567890abcdef1234567890abcdef12345678"));
    }
}
