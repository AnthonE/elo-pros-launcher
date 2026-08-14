//! Reads, and the one distinction that has to survive them.
//!
//! **`reachable` is not `empty`.** The repo's own trap (`CLAUDE.md` §traps): a
//! never-raises reader returns `[]` for both *"nothing there"* and *"we could
//! not look."* This matters more in a desktop client than on a page, because a
//! laptop's wifi drops and a page's does not — a launcher that renders "0
//! games" on a train is lying about the catalog.
//!
//! So every read here returns a [`Fetched`] carrying `ok`, `reachable` and
//! `why`, and no panel may print a count without checking `reachable` first.

pub mod proxy;

use scry_depot::{shardlist, DepotError, Fetcher};
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

pub const USER_AGENT: &str = concat!("scry-launcher/", env!("CARGO_PKG_VERSION"));
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// How long a shard's `/status.json` may take before it is abandoned.
///
/// Far shorter than [`TIMEOUT`], and the difference is the point: a status
/// poll is one number on a row nobody is waiting for, repeated across every
/// shard in a list, so a dead host must cost the *list* a moment rather than a
/// minute. A shard that misses this reads as `?`, never as empty.
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(4);
/// A document, not a build. Depots and manifests are small; a multi-gigabyte
/// "manifest" is a hostile origin, not a big game.
pub const MAX_DOC_BYTES: u64 = 8 * 1024 * 1024;

/// A shelf icon, not an asset. The store draws these at 48px, so anything past
/// this is either the wrong file or an origin spending a player's connection on
/// a thumbnail. Over the cap is a REFUSAL rather than a truncation: half a PNG
/// decodes to nothing, and "the icon did not fit" is a better sentence for a
/// log than "the icon is corrupt".
pub const MAX_ART_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone)]
pub struct Fetched<T> {
    pub value: Option<T>,
    /// Did we get an answer we could parse?
    pub ok: bool,
    /// Did we reach the origin at all? **False means the count below is
    /// meaningless, not zero.**
    pub reachable: bool,
    /// The HTTP status, when there was one.
    ///
    /// Carried rather than folded into `why` because some statuses are real
    /// ANSWERS, not failures: `/api/who` returns **404 for "nobody by that
    /// name"**, which is a true statement about a real address. A caller that
    /// had to string-match `why` to tell that apart would be re-deriving the
    /// reachable/empty distinction one layer up, badly.
    pub status: Option<u16>,
    pub why: String,
}

impl<T> Fetched<T> {
    pub fn good(value: T) -> Self {
        Fetched { value: Some(value), ok: true, reachable: true, status: Some(200),
                  why: String::new() }
    }
    /// We reached the origin and it said something we could not use.
    pub fn bad(why: impl Into<String>) -> Self {
        Fetched { value: None, ok: false, reachable: true, status: None, why: why.into() }
    }
    /// We reached the origin and it answered with a status we did not want.
    pub fn status(code: u16) -> Self {
        Fetched { value: None, ok: false, reachable: true, status: Some(code),
                  why: format!("the origin answered {code}") }
    }
    /// We never got there. The distinction this whole module exists for.
    pub fn unreachable(why: impl Into<String>) -> Self {
        Fetched { value: None, ok: false, reachable: false, status: None, why: why.into() }
    }
}

pub struct Net {
    /// Carries whatever `ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` said.
    proxied: ureq::Agent,
    /// Never proxied, for the hosts `proxy.rs` says must not be.
    direct: ureq::Agent,
    /// The same pair at [`STATUS_TIMEOUT`], for shard status polls only.
    /// Separate agents rather than a per-request override because the timeout
    /// is an agent-level setting in this ureq, and a poll that inherited the
    /// sixty-second one would hang a whole list on one dead shard.
    quick_proxied: ureq::Agent,
    quick_direct: ureq::Agent,
    no_proxy: Vec<String>,
    /// A ticket grant (`SCRY_GRANT`), sent as `X-Scry-Grant` — but ONLY to
    /// urls under `grant_origin`, never to whatever host a manifest names.
    /// A grant is a bearer token for one game's downloads; scoping it to the
    /// origin is what keeps a hostile manifest from harvesting it.
    grant: Option<String>,
    grant_origin: String,
}

impl Default for Net {
    fn default() -> Self {
        Self::new()
    }
}

impl Net {
    pub fn new() -> Self {
        let at = |t: Duration| {
            ureq::Agent::config_builder()
                .timeout_global(Some(t))
                .user_agent(USER_AGENT)
        };
        let base = || at(TIMEOUT);
        Net {
            proxied: base().build().new_agent(),
            direct: base().proxy(None).build().new_agent(),
            quick_proxied: at(STATUS_TIMEOUT).build().new_agent(),
            quick_direct: at(STATUS_TIMEOUT).proxy(None).build().new_agent(),
            no_proxy: proxy::no_proxy_entries(),
            grant: std::env::var("SCRY_GRANT").ok().filter(|g| !g.is_empty()),
            grant_origin: std::env::var("SCRY_GRANT_ORIGIN")
                .ok()
                .filter(|o| !o.is_empty())
                .unwrap_or_else(|| "https://scry.moreright.xyz".to_string()),
        }
    }

    /// The grant header for this url, if one applies. A ticketed title's
    /// depot answers 401 without it (`GET /api/ticket/<slug>` has the whole
    /// dance); an unticketed title never needs it, and sending it costs
    /// nothing — the origin ignores a grant on an open door.
    fn grant_for(&self, url: &str) -> Option<&str> {
        let g = self.grant.as_deref()?;
        url.starts_with(&self.grant_origin).then_some(g)
    }

    /// The agent for this URL. See `proxy.rs` — ureq does not read `NO_PROXY`,
    /// so a launcher pointed at a local origin would otherwise be tunnelled.
    fn agent_for(&self, url: &str) -> &ureq::Agent {
        if proxy::should_bypass(url, &self.no_proxy) {
            &self.direct
        } else {
            &self.proxied
        }
    }

    pub fn post_json(&self, url: &str, body: &Value) -> Fetched<Value> {
        let payload = body.to_string();
        let response = match self
            .agent_for(url)
            .post(url)
            .header("content-type", "application/json")
            .send(payload.as_str())
        {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) => return Fetched::status(code),
            Err(e) => return Fetched::unreachable(format!("{e}")),
        };
        let mut text = String::new();
        if let Err(e) = response
            .into_body()
            .into_reader()
            .take(MAX_DOC_BYTES)
            .read_to_string(&mut text)
        {
            return Fetched::bad(format!("could not read the reply: {e}"));
        }
        match serde_json::from_str(&text) {
            Ok(v) => Fetched::good(v),
            Err(e) => Fetched::bad(format!("the reply is not JSON: {e}")),
        }
    }

    /// A POST whose REFUSAL survives.
    ///
    /// `post_json` folds any non-2xx into [`Fetched::status`] and drops the
    /// body — right for a depot, wrong for a desk. Every refusal the store desk
    /// makes carries the reason in the body, by design ("a silently dropped
    /// field is an operator believing they published something they did not"),
    /// and a client that printed `the origin answered 422` would be throwing
    /// away the entire point of writing those sentences.
    ///
    /// So this one turns the status off as an error and hands back both: the
    /// parsed body AND the code. `ok` still means 2xx, so a caller cannot
    /// mistake a refusal for a success by forgetting to look.
    pub fn post_json_told(&self, url: &str, body: &Value) -> Fetched<Value> {
        self.send_told(url, "application/json", body.to_string().into_bytes())
    }

    /// The same, for a body that is not JSON — an image, on its way to the
    /// store desk's upload. Query parameters carry the rest; the bytes are the
    /// bytes, because base64 in a JSON envelope would cost a third of them for
    /// nothing.
    pub fn post_bytes_told(&self, url: &str, content_type: &str, body: Vec<u8>) -> Fetched<Value> {
        self.send_told(url, content_type, body)
    }

    fn send_told(&self, url: &str, content_type: &str, body: Vec<u8>) -> Fetched<Value> {
        let response = match self
            .agent_for(url)
            .post(url)
            .config()
            .http_status_as_error(false)
            .build()
            .header("content-type", content_type)
            .send(&body[..])
        {
            Ok(r) => r,
            Err(e) => return Fetched::unreachable(format!("{e}")),
        };
        let code = response.status().as_u16();
        let mut text = String::new();
        if let Err(e) = response
            .into_body()
            .into_reader()
            .take(MAX_DOC_BYTES)
            .read_to_string(&mut text)
        {
            return Fetched {
                value: None,
                ok: false,
                reachable: true,
                status: Some(code),
                why: format!("could not read the reply: {e}"),
            };
        }
        let value: Option<Value> = serde_json::from_str(&text).ok();
        let why = match (&value, (200..300).contains(&code)) {
            (_, true) => String::new(),
            (Some(v), false) => v
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("the origin answered {code}")),
            (None, false) => format!("the origin answered {code}"),
        };
        Fetched {
            ok: (200..300).contains(&code) && value.is_some(),
            reachable: true,
            status: Some(code),
            value,
            why,
        }
    }

    /// The shard list a title serves — `manifest.servers.url`.
    ///
    /// **Fetched from the title's own url and never proxied through us.** The
    /// list is the game's to serve (`docs/client/LAUNCHER.md` §6); the launcher reads
    /// it, and the broker refuses even to relay the request. Bounded at the
    /// parser's own document cap rather than [`MAX_DOC_BYTES`], because a
    /// hostile origin should not be able to spend eight megabytes of a
    /// player's connection on a window that lists sixty-four rows.
    pub fn shards(&self, url: &str) -> Fetched<Vec<shardlist::Shard>> {
        let got = self.get_bytes(url, shardlist::MAX_DOC_BYTES as u64 + 1, false);
        let Some(bytes) = got.value else {
            return Fetched {
                value: None,
                ok: got.ok,
                reachable: got.reachable,
                status: got.status,
                why: got.why,
            };
        };
        match shardlist::parse(&bytes) {
            Ok(rows) => Fetched::good(rows),
            // We reached the origin and it said something we cannot use. NOT
            // `unreachable` — the distinction this whole module exists for.
            Err(e) => Fetched::bad(e.to_string()),
        }
    }

    /// One shard's live player count, from the shard itself.
    ///
    /// The url comes out of a row's `status_url`. At [`STATUS_TIMEOUT`], since
    /// a whole list of these is polled at once and a dead host must not hold
    /// the others up.
    pub fn status(&self, url: &str) -> Fetched<shardlist::Status> {
        let got = self.get_bytes(url, shardlist::MAX_STATUS_BYTES as u64 + 1, true);
        let Some(bytes) = got.value else {
            return Fetched {
                value: None,
                ok: got.ok,
                reachable: got.reachable,
                status: got.status,
                why: got.why,
            };
        };
        match shardlist::parse_status(&bytes) {
            Ok(s) => Fetched::good(s),
            Err(e) => Fetched::bad(e.to_string()),
        }
    }

    /// A shelf icon's bytes — a capsule or a card the origin serves for a
    /// title.
    ///
    /// **On the quick agent on purpose.** Art is decoration on a row whose
    /// words are already correct, so a slow or dead image host must cost the
    /// Store four seconds and not sixty. A row whose art did not arrive draws
    /// the placeholder and says nothing false; that is the only failure mode
    /// this read has.
    ///
    /// Over [`MAX_ART_BYTES`] is refused rather than truncated — see the
    /// constant. The bytes are still UNTRUSTED after this returns: an image
    /// decoder is a parser, and `scry_ui::art` is where a failed decode
    /// becomes a placeholder instead of a panic.
    pub fn art(&self, url: &str) -> Fetched<Vec<u8>> {
        let got = self.get_bytes(url, MAX_ART_BYTES + 1, true);
        match got.value {
            Some(b) if b.len() as u64 > MAX_ART_BYTES => Fetched::bad(format!(
                "the art is over {} KB, so it is not a shelf icon",
                MAX_ART_BYTES / 1024
            )),
            Some(b) => Fetched::good(b),
            None => Fetched {
                value: None,
                ok: got.ok,
                reachable: got.reachable,
                status: got.status,
                why: got.why,
            },
        }
    }

    /// One GET, capped, as bytes. The shared body of the two reads above —
    /// they differ only in their cap, their agent and what they parse.
    fn get_bytes(&self, url: &str, limit: u64, quick: bool) -> Fetched<Vec<u8>> {
        let agent = if quick {
            if proxy::should_bypass(url, &self.no_proxy) {
                &self.quick_direct
            } else {
                &self.quick_proxied
            }
        } else {
            self.agent_for(url)
        };
        let mut req = agent.get(url);
        if let Some(g) = self.grant_for(url) {
            req = req.header("x-scry-grant", g);
        }
        let response = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) => return Fetched::status(code),
            Err(e) => return Fetched::unreachable(format!("{e}")),
        };
        let mut body = Vec::new();
        if let Err(e) = response
            .into_body()
            .into_reader()
            .take(limit)
            .read_to_end(&mut body)
        {
            return Fetched::bad(format!("could not read the reply: {e}"));
        }
        Fetched::good(body)
    }

    /// A document read on the **short** deadline, for a caller who is not
    /// waiting on the answer.
    ///
    /// [`STATUS_TIMEOUT`], not [`TIMEOUT`], and the difference is the whole
    /// point — the same reason a shard poll has its own agent. The Play button
    /// reads a manifest to fill `{servers}`, on the UI thread, and the player
    /// pressing it is waiting for **the game**, not for a shard list. At sixty
    /// seconds an unreachable origin would freeze the launcher for a minute
    /// before starting a game that was installed and ready the whole time,
    /// which is worse than the empty list this read exists to prevent. A miss
    /// launches without the list; the game asks the door for it afterwards.
    pub fn get_json_quick(&self, url: &str) -> Fetched<Value> {
        let got = self.get_bytes(url, MAX_DOC_BYTES, true);
        let Some(bytes) = got.value else {
            return Fetched { value: None, ok: got.ok, reachable: got.reachable,
                             status: got.status, why: got.why };
        };
        match serde_json::from_slice(&bytes) {
            Ok(v) => Fetched::good(v),
            Err(e) => Fetched::bad(format!("the reply is not JSON: {e}")),
        }
    }

    pub fn get_json(&self, url: &str) -> Fetched<Value> {
        let mut req = self.agent_for(url).get(url);
        if let Some(g) = self.grant_for(url) {
            req = req.header("x-scry-grant", g);
        }
        let response = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) => return Fetched::status(code),
            Err(e) => return Fetched::unreachable(format!("{e}")),
        };
        let mut body = String::new();
        if let Err(e) = response
            .into_body()
            .into_reader()
            .take(MAX_DOC_BYTES)
            .read_to_string(&mut body)
        {
            return Fetched::bad(format!("could not read the reply: {e}"));
        }
        match serde_json::from_str(&body) {
            Ok(v) => Fetched::good(v),
            Err(e) => Fetched::bad(format!("the reply is not JSON: {e}")),
        }
    }
}

/// `scry_depot::update::JsonSource` over the real network.
impl scry_depot::JsonSource for Net {
    fn get(&self, url: &str) -> Result<Value, String> {
        let got = self.get_json(url);
        match got.value {
            Some(v) => Ok(v),
            None if got.reachable => Err(got.why),
            None => Err(format!("could not reach it — {}", got.why)),
        }
    }
}

/// `scry_depot::Fetcher` over the real network — streamed to disk, never held
/// in memory, because a depot file is a game asset and not a document.
impl Fetcher for Net {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), DepotError> {
        let mut req = self.agent_for(url).get(url);
        if let Some(g) = self.grant_for(url) {
            req = req.header("x-scry-grant", g);
        }
        let response = req
            .call()
            .map_err(|e| DepotError::new(format!("could not fetch {url}: {e}")))?;
        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(dest)
            .map_err(|e| DepotError::new(format!("{}: {e}", dest.display())))?;
        std::io::copy(&mut reader, &mut file)
            .map_err(|e| DepotError::new(format!("could not fetch {url}: {e}")))?;
        Ok(())
    }
}
