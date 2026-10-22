//! `elo-shardlist-v1` — the document a title serves so its shards can be
//! found, and the one the Servers window renders.
//!
//! **The shard list is the game's to serve** (`docs/client/LAUNCHER.md` §6). The
//! launcher reads `manifest.servers.url`, fetches it, and renders the rows: it
//! does not invent, cache or rank them, and the broker refuses even to proxy
//! the fetch. This module is the reading half of that rule — a parser and
//! nothing else, with no I/O in it, so the window stays constructible in a
//! test with no network.
//!
//! ```json
//! {"servers": [{"id": "eu-1", "name": "…", "addr": "host:port",
//!               "players": 47, "max_players": 100, "map": "…",
//!               "ping_ms": 31, "status_url": "https://…/status.json"}]}
//! ```
//!
//! ## Two things about the shape are load-bearing
//!
//! **`addr` is `host:port`, not an address.** A shard's certificate is issued
//! for a name and its transport needs that name for SNI, so a row carrying a
//! bare IP would connect and then be unable to say who it is talking to. The
//! address is shape-checked and never resolved here.
//!
//! **No count is ever invented.** A row whose shard cannot report a live
//! player count leaves `players` out, and the window draws `?`. Writing a `0`
//! there would be a busy shard rendering as empty — the "card advertising a
//! reward the rule cannot pay" defect in its smallest form.
//!
//! ## `status_url`, and why the count is polled
//!
//! A row may name where *that shard* answers `GET /status.json`. It is
//! additive and optional; the kind stays `-v1` and a reader that predates it
//! ignores the field.
//!
//! It exists because a count baked into the document is stale the moment the
//! file is written — a title generates this list and serves it for hours or
//! weeks, so a `players` inside it describes whenever someone last ran their
//! generator. **A count that is confidently wrong is worse than the `?` it
//! replaced.** So the row names an endpoint and the window polls it.
//!
//! That is the launcher's own rule one layer down rather than a hole in it:
//! the launcher still does not proxy, cache or rank anything — it asks each
//! shard about itself, exactly as it asks each title's origin about its list.
//! Nothing is intermediated; everyone measures.
//!
//! ## Caps
//!
//! This parses bytes fetched from a url written by a title, so every field is
//! bounded and the policy is always **refuse the document**, never silently
//! truncate it: a list that quietly lost its last eight shards is a list
//! nobody can tell is wrong.

use crate::error::{DepotError, Result};
use serde::Deserialize;

/// The `kind` a manifest declares for this document.
pub const KIND: &str = "elo-shardlist-v1";

/// Most shards one document may list. Sized well above any plausible list, so
/// hitting it means the document is broken or hostile, not popular.
pub const MAX_SHARDS: usize = 64;

/// Longest `id`, `name` and `map` a row may carry, in bytes.
pub const MAX_FIELD_BYTES: usize = 96;

/// Longest `status_url` a row may carry, in bytes. Its own cap because a url
/// is legitimately longer than a name.
pub const MAX_URL_BYTES: usize = 256;

/// Largest document accepted, in bytes — a cap on the READ, so a title whose
/// origin streams forever cannot hold the window open.
pub const MAX_DOC_BYTES: usize = 64 * 1024;

/// Largest `/status.json` accepted from a shard, in bytes. The real document
/// is under 60.
pub const MAX_STATUS_BYTES: usize = 4 * 1024;

/// One shard a player may join.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Shard {
    /// Stable key. The window builds its row key from this, falling back to
    /// `addr`, so changing it splits one shard into two in anyone's list.
    pub id: String,
    pub name: String,
    /// `host:port`. Shape-checked, never resolved.
    pub addr: String,
    #[serde(default)]
    pub players: Option<u32>,
    #[serde(default)]
    pub max_players: Option<u32>,
    #[serde(default)]
    pub map: Option<String>,
    #[serde(default)]
    pub ping_ms: Option<u32>,
    /// Where this shard answers `GET /status.json`, if it does.
    #[serde(default)]
    pub status_url: Option<String>,
}

impl Shard {
    /// The key a window should use for this row.
    pub fn key(&self) -> &str {
        if self.id.trim().is_empty() {
            &self.addr
        } else {
            &self.id
        }
    }

    /// `47/100`, `47`, `?/100`, or `?` — whatever the row actually states.
    /// Never fills a missing count with a zero.
    pub fn population(&self) -> String {
        match (self.players, self.max_players) {
            (Some(p), Some(m)) => format!("{p}/{m}"),
            (Some(p), None) => format!("{p}"),
            (None, Some(m)) => format!("?/{m}"),
            (None, None) => "?".to_string(),
        }
    }

    /// Is this shard full? `false` when the count is unknown — a row must
    /// never *look* closed because nobody could measure it.
    pub fn full(&self) -> bool {
        matches!((self.players, self.max_players), (Some(p), Some(m)) if p >= m)
    }

    /// Fold a freshly polled status in. **The poll wins over the file**: both
    /// describe the same shard and one of them was measured seconds ago.
    ///
    /// A *failed* poll never reaches here — a row keeps whatever count it had
    /// rather than being zeroed by a timeout, which would turn a network blip
    /// into "everyone left".
    pub fn apply_status(&mut self, s: &Status) {
        self.players = Some(s.players);
        self.max_players = Some(s.max_players);
    }

    /// The `status_url` this machine may poll, for a list read from
    /// `list_url` — or `None`, and the row draws `?`.
    ///
    /// ⚠ **The list is the title's, so its urls are the title's choice of
    /// where this machine sends a GET.** Polled as written, a row could aim
    /// the player's own launcher at `169.254.169.254`, a router on
    /// `192.168.1.1` or a port on loopback. So the poll goes to the row's own
    /// `addr` host and nowhere else — the allowlist `meter/launcher.py` holds
    /// for its own poll — and never to this machine or its network unless the
    /// list itself was served from there, which is a developer reading their
    /// own shards. This reads names; a public name that RESOLVES here
    /// (`169.254.169.254.nip.io`) is refused by `elo_net::Net::status`,
    /// which checks the addresses it connects to.
    pub fn status_url_to_poll(&self, list_url: &str) -> Option<&str> {
        let u = self.status_url.as_deref()?;
        let host = url_host(u)?;
        if !host.eq_ignore_ascii_case(addr_host(&self.addr)) {
            return None;
        }
        if local_host(&host) && !local_url(list_url) {
            return None;
        }
        Some(u)
    }
}

/// The host of an `http(s)` url, lowercased, inside the brackets for IPv6.
/// `None` for a url carrying userinfo: `https://a@b/` connects to `b`.
fn url_host(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://")?.1;
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    if auth.contains('@') || auth.is_empty() {
        return None;
    }
    let host = match auth.strip_prefix('[') {
        Some(v6) => v6.split_once(']')?.0,
        None => auth.rsplit_once(':').map(|(h, _)| h).unwrap_or(auth),
    };
    Some(host.to_ascii_lowercase())
}

/// Does `url` name this machine or its network (see [`local_ip`])? The test
/// for "a developer is reading their own shards": a list served from there
/// may poll shards there.
pub fn local_url(url: &str) -> bool {
    url_host(url).is_some_and(|h| local_host(&h))
}

/// Is `ip` this machine or a network only it can see — loopback, private,
/// link-local, CGNAT, multicast, reserved, unspecified — in any spelling,
/// including a v4 address inside v6 (mapped, compatible, NAT64)?
/// `elo_net` holds every address a status poll's name resolves to here.
pub fn local_ip(ip: std::net::IpAddr) -> bool {
    use std::net::{IpAddr, Ipv4Addr};
    let v4 = |ip: Ipv4Addr| {
        let [a, b, ..] = ip.octets();
        ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
            || ip.is_multicast() || a == 0 || a >= 240 || (a == 100 && (64..128).contains(&b))
            || (a == 198 && (b & 0xfe) == 18)
    };
    match ip {
        IpAddr::V4(ip) => v4(ip),
        IpAddr::V6(ip) => {
            let s = ip.segments();
            let tail = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
            ip.is_loopback() || ip.is_unspecified() || ip.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] & 0xffc0) == 0xfec0 // site-local
                || ip.to_ipv4_mapped().is_some_and(v4)
                || (s[..6] == [0; 6] && v4(tail)) // ::a.b.c.d
                || (s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] && v4(tail)) // NAT64
                || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1) // local-use NAT64
        }
    }
}

/// Is `host` this machine or a network only it can see ([`local_ip`]), a name
/// only that network answers for (`localhost`, `.local`, `.internal`, `.lan`,
/// `.home.arpa`), or a number a resolver would read as an address (`127.1`,
/// `2130706433`, `0x7f.0.0.1`)? A DNS name's last label is never numeric, so
/// one that is is an address in disguise and counts as local.
///
/// Names only. A public name that resolves to a local address is caught where
/// it is resolved — `elo_net::Net::status` checks every address it connects to.
fn local_host(host: &str) -> bool {
    use std::net::IpAddr;
    let h = host.trim_end_matches('.');
    if h == "localhost"
        || [".localhost", ".local", ".internal", ".lan", ".home.arpa", ".localdomain"]
            .iter()
            .any(|s| h.ends_with(s))
    {
        return true;
    }
    match h.parse::<IpAddr>() {
        Ok(ip) => local_ip(ip),
        Err(_) => {
            let last = h.rsplit('.').next().unwrap_or("");
            last.is_empty() || last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x")
                || h.split('.').next().is_some_and(|f| f.starts_with("0x"))
        }
    }
}

/// A shard's `GET /status.json`. Integers only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Status {
    pub players: u32,
    pub max_players: u32,
    /// Liveness: a number that stops moving is a shard that stopped ticking.
    /// Defaulted, so a shard that serves only the two counts still parses.
    #[serde(default)]
    pub tick: u64,
}

#[derive(Debug, Deserialize)]
struct Doc {
    #[serde(default)]
    servers: Vec<Shard>,
}

/// Parse a `elo-shardlist-v1` document.
///
/// Every refusal names what was wrong, because the window draws this string —
/// "the shard list did not parse" tells a player nothing, and a dark panel
/// that cannot say why is the one thing this client's discipline forbids.
pub fn parse(bytes: &[u8]) -> Result<Vec<Shard>> {
    if bytes.len() > MAX_DOC_BYTES {
        return Err(DepotError::new(format!(
            "the shard list is {} bytes, over the {MAX_DOC_BYTES}-byte cap",
            bytes.len()
        )));
    }
    // The top level must be an OBJECT, checked before the struct is built:
    // serde will deserialize a struct from a JSON array positionally, so a
    // bare `[]` would read as `{"servers": []}` — indistinguishable from a
    // healthy list with nothing up. A wrong document must never be able to
    // present as an honest empty one.
    let v: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| DepotError::new(format!("the shard list did not parse: {e}")))?;
    if !v.is_object() {
        return Err(DepotError::new(
            "the shard list is not an object with a \"servers\" key",
        ));
    }
    let doc: Doc = serde_json::from_value(v)
        .map_err(|e| DepotError::new(format!("the shard list did not parse: {e}")))?;

    if doc.servers.len() > MAX_SHARDS {
        return Err(DepotError::new(format!(
            "the shard list has {} rows, over the {MAX_SHARDS} cap",
            doc.servers.len()
        )));
    }
    for (n, s) in doc.servers.iter().enumerate() {
        let field = |what: &str, v: &str| -> Result<()> {
            if v.trim().is_empty() {
                return Err(DepotError::new(format!("shard row {n}: {what} is empty")));
            }
            if v.len() > MAX_FIELD_BYTES {
                return Err(DepotError::new(format!(
                    "shard row {n}: {what} is over the {MAX_FIELD_BYTES}-byte cap"
                )));
            }
            Ok(())
        };
        field("id", &s.id)?;
        field("name", &s.name)?;
        if let Some(m) = &s.map {
            field("map", m)?;
        }
        crate::deeplink::check_addr(&s.addr)
            .map_err(|e| DepotError::new(format!("shard row {n} ({}): {e}", s.id)))?;
        if let Some(u) = &s.status_url {
            check_status_url(u)
                .map_err(|e| DepotError::new(format!("shard row {n} ({}): {e}", s.id)))?;
        }
    }
    Ok(doc.servers)
}

/// Parse a shard's `/status.json`.
///
/// Refuse-don't-guess, for the same reason as above: a shard that answers junk
/// must leave the row's count *absent*, never zero.
pub fn parse_status(bytes: &[u8]) -> Result<Status> {
    if bytes.len() > MAX_STATUS_BYTES {
        return Err(DepotError::new(format!(
            "status is {} bytes, over the {MAX_STATUS_BYTES}-byte cap",
            bytes.len()
        )));
    }
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| DepotError::new(format!("status: {e}")))?;
    if !v.is_object() {
        return Err(DepotError::new("status: top level is not an object"));
    }
    let s: Status =
        serde_json::from_value(v).map_err(|e| DepotError::new(format!("status: {e}")))?;
    if s.max_players == 0 {
        return Err(DepotError::new("status: max_players is 0"));
    }
    // A shard reporting more players than it can hold is broken or hostile,
    // and `412/100` is nonsense to draw either way.
    if s.players > s.max_players {
        return Err(DepotError::new(format!(
            "status: {} players over a cap of {}",
            s.players, s.max_players
        )));
    }
    Ok(s)
}

/// Validate a `status_url` for shape. `http(s)` only, capped.
pub fn check_status_url(url: &str) -> Result<()> {
    let u = url.trim();
    if u.is_empty() {
        return Err(DepotError::new("status_url is empty"));
    }
    if u.len() > MAX_URL_BYTES {
        return Err(DepotError::new(format!(
            "status_url is over the {MAX_URL_BYTES}-byte cap"
        )));
    }
    if u.chars().any(char::is_whitespace) {
        return Err(DepotError::new(format!(
            "status_url {u:?} contains whitespace"
        )));
    }
    if !u.starts_with("https://") && !u.starts_with("http://") {
        return Err(DepotError::new(format!(
            "status_url {u:?} is not an http(s) url"
        )));
    }
    let rest = u.split_once("://").map(|(_, r)| r).unwrap_or("");
    if rest.is_empty() || rest.starts_with('/') {
        return Err(DepotError::new(format!("status_url {u:?} has no host")));
    }
    Ok(())
}

/// The host half of an `addr` — inside the brackets for an IPv6 literal.
fn addr_host(addr: &str) -> &str {
    match addr.strip_prefix('[') {
        Some(rest) => rest.split_once(']').map(|(h, _)| h).unwrap_or(rest),
        None => addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr),
    }
}

/// Is `site` (a sign-in domain, port dropped) one of the title's own: a host
/// its shard list names, or the host that serves the list?
///
/// This is what lets `prove` sign with no prompt (`elo_broker::Host::serves`).
/// Exact names only — `evil-shard-3.gates.example` is not
/// `shard-3.gates.example`, and a parent domain is not every name under it.
///
/// ⚠ For a house title the list host is the store itself
/// (`/api/launcher/servers/{slug}` on the origin), so a game that says hello
/// with a published list proves for the store's domain unasked. That proof
/// does not sign anyone into the website: the site's sign-in
/// (`meter/signin.py`) takes a different statement ("Sign in to … in a web
/// browser"), never `protocol::statement`'s "Prove your identity to play …".
/// Keep the two sentences apart, or drop the list host here.
pub fn serves(rows: &[Shard], list_url: &str, site: &str) -> bool {
    if site.is_empty() {
        return false;
    }
    let origin = crate::depot::origin_of(list_url);
    let list_host = origin.split_once("://").map(|(_, h)| h).unwrap_or("");
    let list_host = list_host.rsplit_once(':').map(|(h, _)| h).unwrap_or(list_host);
    list_host.eq_ignore_ascii_case(site)
        || rows.iter().any(|r| addr_host(&r.addr).eq_ignore_ascii_case(site))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape as `docs/client/LAUNCHER.md` §6 writes it. If the document changes,
    /// this is the test that should go red first.
    const SPEC: &str = r#"{"servers": [{"id": "eu-1", "name": "Gates EU 1",
        "addr": "game.elopros.com:61234", "players": 47, "max_players": 100,
        "map": "island", "ping_ms": 31}]}"#;

    /// The document `ci/shardlist.py` in `AnthonE/Gates` actually writes,
    /// verbatim, for a two-shard `shards.toml` — one shard publishing a status
    /// endpoint and one not.
    ///
    /// **This is the seam, and nothing else covers it.** Gates' generator
    /// self-test checks its caps against Gates' OWN Rust parser; this file is a
    /// second parser in a second repo, and the two have never been compared by
    /// anything. A field renamed on one side would leave both suites green and
    /// the Servers window empty. Regenerate with:
    ///
    /// ```text
    /// ./ci/shardlist.py --shards shards.toml --out -
    /// ```
    const GATES_OUTPUT: &str = r#"{
  "servers": [
    {
      "addr": "game.elopros.com:61234",
      "id": "eu-1",
      "map": "island 20260731",
      "max_players": 100,
      "name": "Gates EU 1",
      "status_url": "https://game.elopros.com:8080/status.json"
    },
    {
      "addr": "127.0.0.1:4433",
      "id": "dev",
      "max_players": 100,
      "name": "Dev shard"
    }
  ]
}"#;

    #[test]
    fn what_gates_actually_writes_is_what_this_reads() {
        let rows = parse(GATES_OUTPUT.as_bytes()).expect("Gates' own output must parse here");
        assert_eq!(rows.len(), 2);

        // The shard that publishes an endpoint: no count in the file, and a
        // url to go and get one from. This pair is the whole design — the
        // generator states where to look and never what it would have found.
        assert_eq!(rows[0].key(), "eu-1");
        assert_eq!(rows[0].addr, "game.elopros.com:61234");
        assert_eq!(rows[0].players, None, "the generator must not bake a count");
        assert_eq!(rows[0].population(), "?/100");
        assert_eq!(
            rows[0].status_url.as_deref(),
            Some("https://game.elopros.com:8080/status.json")
        );

        // The shard that does not: no url, and it stays `?` forever. Correct.
        assert_eq!(rows[1].status_url, None);
        assert_eq!(rows[1].population(), "?/100");

        // And every row is invitable, which is what the window will offer.
        for r in &rows {
            let link = crate::deeplink::link_for("gates", &r.addr).expect("invitable");
            assert_eq!(crate::deeplink::parse(&link).unwrap().addr, r.addr);
        }
    }

    #[test]
    fn the_documented_shape_parses() {
        let rows = parse(SPEC.as_bytes()).expect("spec shape");
        assert_eq!(rows.len(), 1);
        let s = &rows[0];
        assert_eq!(s.key(), "eu-1");
        assert_eq!(s.addr, "game.elopros.com:61234");
        assert_eq!(s.population(), "47/100");
        assert!(!s.full());
    }

    #[test]
    fn an_empty_list_is_a_list_and_not_an_error() {
        // "no shards up" is a real answer and the window renders it. It is not
        // a parse failure and must not be reported as one.
        assert_eq!(parse(br#"{"servers":[]}"#).unwrap().len(), 0);
        assert_eq!(parse(br#"{}"#).unwrap().len(), 0);
    }

    #[test]
    fn a_countless_row_stays_countless() {
        let rows = parse(br#"{"servers":[{"id":"a","name":"A","addr":"h:1"}]}"#).unwrap();
        assert_eq!(rows[0].players, None);
        assert_eq!(rows[0].population(), "?");
        // An unmeasured shard must never LOOK closed.
        assert!(!rows[0].full());
    }

    #[test]
    fn a_polled_status_is_what_lights_the_count() {
        let mut s = parse(br#"{"servers":[{"id":"a","name":"A","addr":"h:1",
                              "status_url":"https://h:8080/status.json"}]}"#)
        .unwrap()[0]
            .clone();
        assert_eq!(s.population(), "?");
        assert_eq!(s.status_url.as_deref(), Some("https://h:8080/status.json"));
        s.apply_status(&parse_status(br#"{"players":3,"max_players":100,"tick":9}"#).unwrap());
        assert_eq!(s.population(), "3/100");
        // A measured zero is a measurement. Only an UNMEASURED count is `?`.
        s.apply_status(&parse_status(br#"{"players":0,"max_players":100}"#).unwrap());
        assert_eq!(s.population(), "0/100");
        // And full is full.
        s.apply_status(&parse_status(br#"{"players":100,"max_players":100}"#).unwrap());
        assert!(s.full());
    }

    #[test]
    fn a_poll_beats_a_baked_count() {
        // One of them was measured a second ago. Pinned because the opposite
        // order reads as reasonable and would make the field pointless.
        let mut s = parse(
            br#"{"servers":[{"id":"a","name":"A","addr":"h:1","players":99,"max_players":100}]}"#,
        )
        .unwrap()[0]
            .clone();
        assert_eq!(s.population(), "99/100");
        s.apply_status(&parse_status(br#"{"players":4,"max_players":100}"#).unwrap());
        assert_eq!(s.population(), "4/100");
    }

    #[test]
    fn a_shard_answering_junk_leaves_the_row_alone() {
        for junk in [
            &b""[..],
            b"not json",
            b"[]",
            b"null",
            b"{}",
            br#"{"players":1}"#,
            br#"{"players":-1,"max_players":100}"#,
            br#"{"players":"3","max_players":"100"}"#,
            br#"{"players":5,"max_players":0}"#,
            br#"{"players":412,"max_players":100}"#,
            b"\xff\xfe\x00",
        ] {
            assert!(parse_status(junk).is_err(), "{junk:?} should be refused");
        }
        assert!(parse_status(&vec![b' '; MAX_STATUS_BYTES + 1]).is_err());
    }

    #[test]
    fn the_caps_refuse_rather_than_truncate() {
        let rows: Vec<String> = (0..MAX_SHARDS + 1)
            .map(|i| format!(r#"{{"id":"s{i}","name":"n","addr":"h:1"}}"#))
            .collect();
        let why = parse(format!(r#"{{"servers":[{}]}}"#, rows.join(",")).as_bytes())
            .expect_err("over the row cap")
            .0;
        assert!(why.contains("over the"), "{why}");
        // The failure must NOT be a silent short list.
        assert!(why.contains(&(MAX_SHARDS + 1).to_string()), "{why}");

        let long = "x".repeat(MAX_FIELD_BYTES + 1);
        assert!(parse(
            format!(r#"{{"servers":[{{"id":"a","name":"{long}","addr":"h:1"}}]}}"#).as_bytes()
        )
        .is_err());
        assert!(parse(&vec![b' '; MAX_DOC_BYTES + 1]).is_err());
    }

    #[test]
    fn a_bad_row_is_refused_and_names_itself() {
        let why = parse(br#"{"servers":[{"id":"eu-1","name":"n","addr":"nonsense"}]}"#)
            .expect_err("no port")
            .0;
        assert!(why.contains("eu-1"), "{why}");
        assert!(why.contains("host:port"), "{why}");

        for bad in ["h:8080/status.json", "ftp://h/s.json", "https://", "https:///s.json"] {
            let doc = format!(
                r#"{{"servers":[{{"id":"eu-1","name":"n","addr":"h:1","status_url":"{bad}"}}]}}"#
            );
            let why = parse(doc.as_bytes()).expect_err(bad).0;
            assert!(why.contains("eu-1"), "{bad}: {why}");
        }
    }

    #[test]
    fn junk_is_refused_without_panicking() {
        // A bare `[]` is the one that matters: it must not read as an honest
        // empty list.
        for junk in [
            &b""[..],
            b"not json",
            b"[]",
            b"null",
            br#"{"servers":{}}"#,
            br#"{"servers":[{"id":"a"}]}"#,
            br#"{"servers":[{"id":"","name":"n","addr":"h:1"}]}"#,
            b"\xff\xfe\x00",
        ] {
            assert!(parse(junk).is_err(), "{junk:?} should be refused");
        }
    }

    /// l-net-client-ssrf#1: a list a title serves names the hosts this
    /// machine will be sent to. A status poll goes to the row's own shard and
    /// nowhere else, and never to a host that NAMES this machine or its
    /// network for a list served from the internet — the allowlist
    /// `meter/launcher.py` applies to its own poll. A name that resolves there
    /// is `elo-net`'s to refuse (`a_name_that_resolves_here_is_not_polled`).
    #[test]
    fn a_status_url_is_polled_only_on_the_rows_own_public_host() {
        const LIST: &str = "https://elopros.com/api/launcher/servers/gates";
        let row = |addr: &str, status: &str| Shard {
            id: "a".into(), name: "A".into(), addr: addr.into(), players: None,
            max_players: None, map: None, ping_ms: None, status_url: Some(status.into()),
        };
        let gates = parse(GATES_OUTPUT.as_bytes()).unwrap();
        assert_eq!(gates[0].status_url_to_poll(LIST),
                   Some("https://game.elopros.com:8080/status.json"));
        assert_eq!(gates[1].status_url_to_poll(LIST), None, "no url, no poll");

        for (addr, status) in [
            // Another host than the shard's own.
            ("game.example:1", "http://169.254.169.254/latest/meta-data/"),
            ("game.example:1", "http://192.168.1.1/reboot"),
            ("game.example:1", "https://evil.example/status.json"),
            ("game.example:1", "https://game.example@127.0.0.1/status.json"),
            // The shard's own host, but it is this machine or its network.
            ("127.0.0.1:1", "http://127.0.0.1:8080/status.json"),
            ("localhost:1", "http://localhost:6379/"),
            ("10.0.0.7:1", "http://10.0.0.7/status.json"),
            ("169.254.169.254:80", "http://169.254.169.254/latest/meta-data/"),
            ("[::1]:1", "http://[::1]:8080/status.json"),
            ("[fd00::1]:1", "http://[fd00::1]/status.json"),
            ("[::ffff:127.0.0.1]:1", "http://[::ffff:127.0.0.1]/status.json"),
            // Numbers a resolver reads as an address.
            ("2130706433:1", "http://2130706433/status.json"),
            ("127.1:1", "http://127.1/status.json"),
            ("0x7f.0.0.1:1", "http://0x7f.0.0.1/status.json"),
            // IPv6 spellings of a local v4 address, and site-local.
            ("[64:ff9b::a9fe:a9fe]:1", "http://[64:ff9b::a9fe:a9fe]/latest/meta-data/"),
            ("[::7f00:1]:1", "http://[::7f00:1]/status.json"),
            ("[fec0::1]:1", "http://[fec0::1]/status.json"),
            // Names only this machine's network answers for. A public name
            // that resolves here is `Net::status`'s to refuse: it resolves.
            ("router.local:80", "http://router.local/reboot"),
            ("metadata.google.internal:80", "http://metadata.google.internal/computeMetadata/v1/"),
            ("nas.lan:80", "http://nas.lan/"),
            ("printer.home.arpa:80", "http://printer.home.arpa/"),
        ] {
            assert_eq!(row(addr, status).status_url_to_poll(LIST), None, "{addr} {status}");
        }

        // A developer's own list on their own machine polls their own shard.
        assert_eq!(
            row("127.0.0.1:4433", "http://127.0.0.1:8080/status.json")
                .status_url_to_poll("http://127.0.0.1:3600/api/launcher/servers/gates"),
            Some("http://127.0.0.1:8080/status.json")
        );
        // Names compare without case, and a public IP literal is a host.
        assert!(row("Game.Example:1", "https://game.example/s.json").status_url_to_poll(LIST).is_some());
        assert!(row("203.0.113.9:1", "https://203.0.113.9/s.json").status_url_to_poll(LIST).is_some());
    }
}
