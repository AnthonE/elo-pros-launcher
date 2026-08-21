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
}
