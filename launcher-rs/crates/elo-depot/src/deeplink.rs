//! Join links — `elo://join/<slug>/<host:port>`.
//!
//! **What this buys: a friend pastes a link and the game comes up on the shard
//! they are standing on.** The launcher is the right owner of that scheme and
//! the game is not, for four reasons that are all the same reason — the
//! launcher is the piece that is *installed*:
//!
//! - it already starts a title with `{server}` substituted into the launch
//!   args (`launch.rs`'s `ARG_VARS`), so joining is a value it already knows
//!   how to pass;
//! - it already holds the account the shard will ask to sign, so a join can be
//!   an authenticated one without the link carrying anything sensitive;
//! - one scheme serves every title, rather than each game registering its own;
//! - **a link for a title the player has not installed is still useful**,
//!   because the launcher can offer to install it. A game-owned scheme cannot
//!   do that: the game is the thing that is missing.
//!
//! ## What a link may say, and what it may not
//!
//! Exactly two facts: which title, and which address. That is the whole
//! grammar, and the narrowness is the security property rather than an
//! unfinished feature — **a join link is untrusted input from a stranger by
//! construction**, since being pasted into a chat window is its entire
//! purpose. A link that could name an identity would be a phishing primitive;
//! one that could name a path, a grant, a host override or an env var would be
//! a way to reconfigure someone's launcher from a chat window. So anything
//! past the address is **refused rather than ignored** — ignored is how an
//! unnoticed segment becomes a supported one.
//!
//! The address is then shape-checked ([`check_addr`]) before it can reach a
//! launch argv: no scheme, no slash, no whitespace, no port zero. The slug is
//! checked against the same character class a manifest slug already obeys, so
//! a link cannot name `../../etc` and have it reach a path join.
//!
//! **Resolving a link is not the same as acting on one.** This module returns
//! a [`Join`]; whether to install a missing title, and whether to start
//! anything at all, belongs to the caller — which is where the player is
//! standing and where a prompt can be shown.

use crate::error::{DepotError, Result};

/// The scheme the platform owns.
pub const SCHEME: &str = "elo";

/// The verb inside it. One today; named as a constant because `elo://` is the
/// whole platform's namespace and the next verb will want to be told apart
/// from this one rather than guessed at.
pub const VERB_JOIN: &str = "join";

/// Longest link accepted, in bytes. A `host:port` and a slug; anything of this
/// length is a probe, not an invitation.
pub const MAX_LINK_BYTES: usize = 256;

/// Longest slug accepted, in bytes.
pub const MAX_SLUG_BYTES: usize = 64;

/// A resolved join link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Join {
    /// Which title. Lowercased, and validated as a slug — never a path.
    pub slug: String,
    /// `host:port`, shape-checked. Goes to the game as `{server}`.
    pub addr: String,
}

impl Join {
    /// The canonical spelling of this join — what a "copy invite" button puts
    /// on the clipboard.
    ///
    /// One writer for the shared form, so what a window shows is by
    /// construction what [`parse`] accepts.
    pub fn link(&self) -> String {
        format!("{SCHEME}://{VERB_JOIN}/{}/{}", self.slug, self.addr)
    }
}

/// Is this argument a link at all?
///
/// Cheap, total, and deliberately not a validation: the desktop hands a
/// handler its url as `argv[1]`, where it lands in the verb slot, so the CLI
/// needs to *recognise* one before it can decide that a malformed one is a bad
/// link rather than an unknown command.
pub fn is_link(s: &str) -> bool {
    let t = s.trim();
    t.len() > SCHEME.len() + 3 && t[..SCHEME.len() + 3].eq_ignore_ascii_case(&format!("{SCHEME}://"))
}

/// Build the canonical link for a title and an address, validating both.
///
/// The writer's door — `Join::link` is the same string once a link has been
/// parsed. Fails rather than emitting something [`parse`] would refuse, which
/// is what stops a window from displaying an invite nobody can use.
pub fn link_for(slug: &str, addr: &str) -> Result<String> {
    Ok(Join {
        slug: check_slug(slug)?,
        addr: {
            check_addr(addr)?;
            addr.trim().to_string()
        },
    }
    .link())
}

/// Parse a join link.
///
/// Every refusal names what was wrong. This string reaches a player who was
/// handed a link by a friend and has no idea what a shard is.
pub fn parse(link: &str) -> Result<Join> {
    let raw = link.trim();
    if raw.len() > MAX_LINK_BYTES {
        return Err(DepotError::new(format!(
            "join link is {} bytes, over the {MAX_LINK_BYTES}-byte cap",
            raw.len()
        )));
    }
    let (scheme, rest) = raw.split_once("://").ok_or_else(|| {
        DepotError::new(format!(
            "{raw:?} is not a join link — expected {SCHEME}://{VERB_JOIN}/<title>/host:port"
        ))
    })?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return Err(DepotError::new(format!(
            "{scheme:?} is not this launcher's scheme — expected {SCHEME}://"
        )));
    }

    // A query or fragment is dropped, not refused: a link that survived a chat
    // client often arrives with tracking junk stapled on, and that is not the
    // player's fault. Nothing past the address is ever READ — see the module
    // docs on what a link may not say.
    let rest = rest
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();

    let Some(verb) = parts.first() else {
        return Err(DepotError::new(format!(
            "{raw:?} says nothing — expected {SCHEME}://{VERB_JOIN}/<title>/host:port"
        )));
    };
    if !verb.eq_ignore_ascii_case(VERB_JOIN) {
        return Err(DepotError::new(format!(
            "{SCHEME}://{verb}/… is not a join link — \
             expected {SCHEME}://{VERB_JOIN}/<title>/host:port"
        )));
    }
    // Exactly three segments. A fourth is refused rather than dropped: this is
    // the check that keeps a link from ever growing a field nobody decided on.
    let [_, slug, addr] = parts.as_slice() else {
        return Err(DepotError::new(format!(
            "a join link names a title and an address and nothing else — \
             expected {SCHEME}://{VERB_JOIN}/<title>/host:port"
        )));
    };

    let slug = check_slug(slug)?;
    let addr = percent_decode(addr)?;
    check_addr(&addr)?;
    Ok(Join { slug, addr })
}

/// Validate a title slug and lowercase it.
///
/// The same character class a manifest slug obeys. **This is a path-safety
/// check as much as a naming one**: the slug reaches a manifest url and an
/// install directory, so `..`, a slash and a NUL all have to die here.
pub fn check_slug(slug: &str) -> Result<String> {
    let s = slug.trim();
    if s.is_empty() {
        return Err(DepotError::new("a join link names no title"));
    }
    if s.len() > MAX_SLUG_BYTES {
        return Err(DepotError::new(format!(
            "the title in this link is over the {MAX_SLUG_BYTES}-byte cap"
        )));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(DepotError::new(format!(
            "{s:?} is not a title slug — letters, digits, - and _ only"
        )));
    }
    Ok(s.to_ascii_lowercase())
}

/// Validate a `host:port` for shape, without resolving it.
///
/// Deliberately not `SocketAddr::from_str`, which refuses every hostname — and
/// a name is the normal case, because a game server's certificate is issued
/// for one and its transport needs that name for SNI. Mirrors the game's own
/// `shardlist::check_addr`; the two must agree, since this validates what that
/// one will be handed.
pub fn check_addr(addr: &str) -> Result<()> {
    let addr = addr.trim();
    let bad = |why: String| Err(DepotError::new(why));
    if addr.is_empty() {
        return bad("the address is empty".into());
    }
    if addr.len() > MAX_LINK_BYTES {
        return bad("the address is absurdly long".into());
    }
    if addr.contains("://") || addr.contains('/') {
        return bad(format!("{addr:?} is a url, not a host:port"));
    }
    if addr.chars().any(char::is_whitespace) {
        return bad(format!("{addr:?} contains whitespace"));
    }
    if addr.chars().any(char::is_control) {
        return bad(format!("{addr:?} contains a control character"));
    }

    // An IPv6 literal is bracketed; anything else splits on the LAST colon, so
    // a bare `::1` is refused rather than read as host `:` port `1`.
    let (host, port) = if let Some(rest) = addr.strip_prefix('[') {
        let (h, r) = rest
            .split_once(']')
            .ok_or_else(|| DepotError::new(format!("{addr:?} opens a bracket it never closes")))?;
        let p = r
            .strip_prefix(':')
            .ok_or_else(|| DepotError::new(format!("{addr:?} has no port")))?;
        (h, p)
    } else {
        let (h, p) = addr
            .rsplit_once(':')
            .ok_or_else(|| DepotError::new(format!("{addr:?} has no port — expected host:port")))?;
        if h.contains(':') {
            return bad(format!(
                "{addr:?} looks like a bare IPv6 address; bracket it as [{h}]:{p}"
            ));
        }
        (h, p)
    };
    if host.is_empty() {
        return bad(format!("{addr:?} has no host"));
    }
    match port.parse::<u16>() {
        Ok(0) => bad(format!("{addr:?} has port 0")),
        Ok(_) => Ok(()),
        Err(_) => bad(format!("{addr:?} has a bad port {port:?}")),
    }
}

/// Undo percent-encoding in the address segment.
///
/// Worth the lines because a link's one special character is the colon in
/// `host:port`, and a chat client that encodes it turns a working link into a
/// refusal nobody can diagnose. **Safe only because `check_addr` runs after
/// it** — a `%2F` that becomes a slash is caught by the same rule that catches
/// a typed one. Decode, then validate; never the reverse.
fn percent_decode(s: &str) -> Result<String> {
    if !s.contains('%') {
        return Ok(s.to_string());
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = b
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .ok_or_else(|| DepotError::new(format!("{s:?} ends in a half-written % escape")))?;
            let byte = u8::from_str_radix(hex, 16)
                .map_err(|_| DepotError::new(format!("{s:?} has a bad % escape %{hex}")))?;
            out.push(byte);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| DepotError::new(format!("{s:?} decodes to invalid utf-8")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_canonical_form_round_trips() {
        let j = parse("elo://join/gates/game.elopros.com:61234").expect("canonical");
        assert_eq!(j.slug, "gates");
        assert_eq!(j.addr, "game.elopros.com:61234");
        assert_eq!(j.link(), "elo://join/gates/game.elopros.com:61234");
        // The writer's door produces exactly what the reader accepts. This is
        // the property that keeps a displayed invite usable.
        let written = link_for("gates", "game.elopros.com:61234").unwrap();
        assert_eq!(written, j.link());
        assert_eq!(parse(&written).unwrap(), j);
    }

    #[test]
    fn a_link_names_a_title_and_an_address_and_nothing_else() {
        // The grammar IS the security boundary. Every one of these is a field
        // a stranger must not be able to set from a chat window, and each is
        // refused rather than dropped.
        for bad in [
            "elo://join/gates/h:1/0xdeadbeef",       // an identity
            "elo://join/gates/h:1/--wallet/0x1",     // a flag
            "elo://join/gates/h:1/extra",            // anything at all
            "elo://join/gates",                      // no address
            "elo://join/h:1",                        // no title
            "elo://join",                            // neither
            "elo://install/gates/h:1",               // another verb
            "elo://gates/h:1",                       // no verb
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_slug_cannot_be_a_path() {
        // It reaches a manifest url and an install directory.
        for bad in [
            "elo://join/../../etc/h:1",
            "elo://join/..%2F..%2Fetc/h:1",
            "elo://join/a b/h:1",
            "elo://join/a.b/h:1",
            "elo://join//h:1",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(check_slug("gates").is_ok());
        assert!(check_slug("some-game_2").is_ok());
        assert!(check_slug("GATES").map(|s| s == "gates").unwrap_or(false));
        assert!(check_slug("../etc").is_err());
        assert!(check_slug("a/b").is_err());
        assert!(check_slug("").is_err());
        assert!(check_slug(&"x".repeat(MAX_SLUG_BYTES + 1)).is_err());
    }

    #[test]
    fn an_address_is_a_shape_and_a_name_is_normal() {
        // A hostname is the NORMAL case: a shard's certificate is issued for
        // one, and its transport needs that name for SNI.
        for good in [
            "game.elopros.com:61234",
            "127.0.0.1:4433",
            "[::1]:4433",
            "[2001:db8::1]:443",
        ] {
            check_addr(good).unwrap_or_else(|e| panic!("{good:?}: {e}"));
        }
        for bad in [
            "", "host", "host:", "host:0", "host:99999", "host:port", ":4433",
            "https://host:4433", "host:4433/join", "host :4433", "::1:4433", "[::1:4433",
        ] {
            assert!(check_addr(bad).is_err(), "{bad:?} should be refused");
        }
        // A bare v6 names its own fix rather than only saying no.
        let why = check_addr("::1:4433").expect_err("bare v6").0;
        assert!(why.contains("[::1]:4433"), "{why}");
    }

    #[test]
    fn an_encoded_colon_still_joins_and_cannot_smuggle() {
        assert_eq!(parse("elo://join/gates/h%3A1").unwrap().addr, "h:1");
        // Decoding runs BEFORE validation, which is the only reason it is safe.
        for bad in [
            "elo://join/gates/h%2Fx:1",  // a slash
            "elo://join/gates/h%20x:1",  // a space
            "elo://join/gates/h:1%00",   // a NUL
            "elo://join/gates/h:%",      // half an escape
            "elo://join/gates/h:%zz",    // not hex
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn chat_client_junk_is_dropped_rather_than_refused() {
        for ok in [
            "elo://join/gates/h:1?utm_source=discord",
            "elo://join/gates/h:1#anchor",
            "elo://join/gates/h:1/",
            "  elo://join/gates/h:1  ",
        ] {
            assert_eq!(parse(ok).expect(ok).addr, "h:1", "{ok:?}");
        }
    }

    #[test]
    fn recognising_a_link_is_separate_from_accepting_one() {
        // The desktop hands a handler its url in the VERB slot, so the CLI has
        // to tell "a malformed link" from "an unknown command".
        assert!(is_link("elo://join/gates/h:1"));
        assert!(is_link("ELO://join/gates/h:1"));
        assert!(is_link("elo://nonsense")); // malformed, but still a link
        assert!(!is_link("play"));
        assert!(!is_link("gates"));
        assert!(!is_link(""));
        assert!(!is_link("https://elopros.com/x"));
    }

    #[test]
    fn a_scheme_and_verb_are_case_insensitive_because_a_url_is() {
        let j = parse("ELO://JOIN/Gates/game.elopros.com:61234").unwrap();
        assert_eq!(j.slug, "gates");
        // The ADDRESS keeps its case: it is a DNS name whose spelling belongs
        // to whoever wrote it, and lowercasing it here would be this parser
        // editing an address on its way to a transport.
        assert_eq!(j.addr, "game.elopros.com:61234");
    }

    #[test]
    fn junk_is_refused_without_panicking() {
        for junk in [
            "", "://", "elo://", "elo:/join/gates/h:1", "not a link",
            "http://elopros.com/join/gates/h:1", "javascript:alert(1)",
            "file:///etc/passwd", "elo://join/gates/",
        ] {
            assert!(parse(junk).is_err(), "{junk:?} should be refused");
        }
        let long = format!("elo://join/gates/{}:1", "h".repeat(MAX_LINK_BYTES));
        assert!(parse(&long).is_err());
        // A writer that would emit something the reader refuses must fail at
        // the writer instead.
        assert!(link_for("../etc", "h:1").is_err());
        assert!(link_for("gates", "not-an-address").is_err());
    }
}
