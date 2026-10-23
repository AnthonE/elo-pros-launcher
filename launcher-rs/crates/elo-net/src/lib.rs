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

pub mod chain;
pub mod friends;
pub mod guard;
pub mod orders;
pub mod pairing;
pub mod proxy;
pub mod tls;
pub mod trading;
pub mod wallet;

use elo_depot::{shardlist, DepotError, Fetcher};
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

pub const USER_AGENT: &str = concat!("elo-launcher/", env!("CARGO_PKG_VERSION"));
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// The slowest link a depot file is allowed to arrive over, in bytes a second.
/// A file's deadline is [`TIMEOUT`] plus its size at this rate, because
/// ureq's timeout runs to the last byte of the body: at a flat sixty seconds
/// the 87 MB Gates binary needed about 12 Mbit/s held the whole way, and one
/// slow stretch threw the whole install away.
pub const DEPOT_FLOOR_BYTES_PER_SEC: u64 = 64 * 1024;

/// How long one depot file of `bytes` may take, end to end.
pub fn depot_file_deadline(bytes: i64) -> Duration {
    TIMEOUT + Duration::from_secs(bytes.max(0) as u64 / DEPOT_FLOOR_BYTES_PER_SEC)
}

/// `(scheme, host, port)` of an absolute http(s) url, lowercased with the
/// default port filled in. `None` for anything else — and for a url carrying
/// userinfo, because `origin@elsewhere` reads as one host and connects to the
/// other.
fn origin_of(url: &str) -> Option<(String, String, u16)> {
    let uri: ureq::http::Uri = url.parse().ok()?;
    let auth = uri.authority()?;
    if auth.as_str().contains('@') {
        return None;
    }
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let port = match (auth.port_u16(), scheme.as_str()) {
        (Some(p), _) => p,
        (None, "https") => 443,
        (None, "http") => 80,
        _ => return None,
    };
    Some((scheme, auth.host().to_ascii_lowercase(), port))
}

/// Whether `url` is on `origin`: the same scheme, host and port, exactly.
pub fn same_origin(url: &str, origin: &str) -> bool {
    match (origin_of(url), origin_of(origin)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Where a redirect's `location` points, from the url that answered it.
/// Absolute, scheme-relative and path-absolute forms; a bare relative path is
/// taken from the answering url's directory.
fn redirect_target(from: &str, location: &str) -> Option<String> {
    if location.starts_with("https://") || location.starts_with("http://") {
        return Some(location.to_string());
    }
    let uri: ureq::http::Uri = from.parse().ok()?;
    let scheme = uri.scheme_str()?;
    let auth = uri.authority()?.as_str();
    if let Some(rest) = location.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    if location.starts_with('/') {
        return Some(format!("{scheme}://{auth}{location}"));
    }
    let path = uri.path();
    let dir = &path[..path.rfind('/').map(|i| i + 1).unwrap_or(0)];
    Some(format!("{scheme}://{auth}{}{location}", if dir.is_empty() { "/" } else { dir }))
}

/// How many redirects one read follows — ureq's own default.
const MAX_REDIRECTS: u32 = 10;

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

/// Which agents one read goes out on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    /// At [`TIMEOUT`]: documents and depot files.
    Full,
    /// At [`STATUS_TIMEOUT`]: art and the launch-time catalog read.
    Quick,
    /// At [`STATUS_TIMEOUT`], and never to an address on this machine or its
    /// network: a shard's status poll for a list served from the internet.
    Public,
}

/// A resolver that refuses a name if ANY address it resolves to is this
/// machine or its network (`shardlist::local_ip`).
///
/// ⚠ **A status url's host is the title's choice, and a name is not an
/// address.** `169.254.169.254.nip.io`, a name the title points at
/// `192.168.1.1`, or `metadata.google.internal` on a cloud desktop all pass a
/// check that reads the url, and all connect somewhere a player never meant
/// their launcher to send a GET. It is the resolver of the agent that connects
/// (`status_direct`), so the addresses checked are the addresses connected to:
/// a name cannot answer public to a check and local to the connect.
#[derive(Debug, Clone)]
struct PublicOnly(Arc<dyn Resolver>);

impl Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addrs = self.0.resolve(uri, config, timeout)?;
        match addrs.iter().find(|a| shardlist::local_ip(a.ip())) {
            Some(a) => Err(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} resolves to {}, which is this machine or its network — not polled",
                        uri.host().unwrap_or("?"), a.ip()),
            ))),
            None => Ok(addrs),
        }
    }
}

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
    /// `quick_direct` for a status poll on a title's list, resolving through
    /// [`PublicOnly`]. Every such poll goes here, proxy or no proxy.
    ///
    /// ⚠ **Never a check, then a send on another agent.** That agent resolves
    /// the name again (with no proxy, its own resolver; through an HTTP(S)
    /// proxy, the proxy, which may sit on the player's machine or network), so
    /// a name answering public to the check and `127.0.0.1` to the second
    /// lookup got through. A player whose proxy is mandatory sees `?` for a
    /// shard's status; their game traffic never went through it anyway.
    status_direct: ureq::Agent,
    no_proxy: Vec<String>,
    /// A ticket grant (`ELO_GRANT`), sent as `X-Elo-Grant` — but ONLY to
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
                .tls_config(tls::ureq_config())
        };
        let base = || at(TIMEOUT);
        Net {
            proxied: base().build().new_agent(),
            direct: base().proxy(None).build().new_agent(),
            quick_proxied: at(STATUS_TIMEOUT).build().new_agent(),
            quick_direct: at(STATUS_TIMEOUT).proxy(None).build().new_agent(),
            status_direct: ureq::Agent::with_parts(
                at(STATUS_TIMEOUT).proxy(None).build(),
                DefaultConnector::default(),
                PublicOnly(Arc::new(DefaultResolver::default())),
            ),
            no_proxy: proxy::no_proxy_entries(),
            grant: std::env::var("ELO_GRANT").ok().filter(|g| !g.is_empty()),
            grant_origin: std::env::var("ELO_GRANT_ORIGIN")
                .ok()
                .filter(|o| !o.is_empty())
                .unwrap_or_else(|| "https://elopros.com".to_string()),
        }
    }

    /// The grant header for this url, if one applies. A ticketed title's
    /// depot answers 401 without it (`GET /api/ticket/<slug>` has the whole
    /// dance); an unticketed title never needs it, and sending it costs
    /// nothing — the origin ignores a grant on an open door.
    ///
    /// ⚠ **The origin is a tuple, never a prefix.** `starts_with` let
    /// `https://elopros.com.evil.tld/…` and `https://elopros.com@evil.tld/…`
    /// through as the origin; see [`same_origin`].
    fn grant_for(&self, url: &str) -> Option<&str> {
        let g = self.grant.as_deref()?;
        same_origin(url, &self.grant_origin).then_some(g)
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
        let got = self.get_bytes(url, shardlist::MAX_DOC_BYTES as u64 + 1, Lane::Full, MAX_REDIRECTS);
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
    ///
    /// ⚠ **No redirect is followed.** The url is held to the row's own host
    /// (`Shard::status_url_to_poll`), and a `302` to `169.254.169.254` or
    /// the player's router is the way round that. A shard answers for itself.
    ///
    /// ⚠ **Nor to a name that resolves to this machine or its network**
    /// ([`PublicOnly`]) — unless `list_url`, the list the row came from, is
    /// itself served from there: a developer reading their own shards.
    pub fn status(&self, url: &str, list_url: &str) -> Fetched<shardlist::Status> {
        let lane = if shardlist::local_url(list_url) { Lane::Quick } else { Lane::Public };
        let got = self.get_bytes(url, shardlist::MAX_STATUS_BYTES as u64 + 1, lane, 0);
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
    /// decoder is a parser, and `elo_ui::art` is where a failed decode
    /// becomes a placeholder instead of a panic.
    pub fn art(&self, url: &str) -> Fetched<Vec<u8>> {
        let got = self.get_bytes(url, MAX_ART_BYTES + 1, Lane::Quick, MAX_REDIRECTS);
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
    fn get_bytes(&self, url: &str, limit: u64, lane: Lane, hops: u32) -> Fetched<Vec<u8>> {
        let response = match self.get(url, lane, None, hops) {
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
        let got = self.get_bytes(url, MAX_DOC_BYTES, Lane::Quick, MAX_REDIRECTS);
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
        let response = match self.get(url, Lane::Full, None, MAX_REDIRECTS) {
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

    /// One GET, every read's door: redirects followed **by hand**, so the
    /// grant is decided again for each hop.
    ///
    /// ⚠ ureq's own following strips `Authorization` and `Cookie` on a
    /// redirect and keeps every other header, so `x-elo-grant` set for the
    /// origin rode on to wherever the origin, or anything in front of it,
    /// pointed. Here each hop is its own request with its own [`grant_for`],
    /// and its own agent, since a hop can cross the `NO_PROXY` line too.
    ///
    /// [`grant_for`]: Net::grant_for
    fn get(
        &self,
        url: &str,
        lane: Lane,
        deadline: Option<Duration>,
        hops: u32,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let mut at = url.to_string();
        for _ in 0..=hops {
            let agent = match (lane, proxy::should_bypass(&at, &self.no_proxy)) {
                (Lane::Public, _) => &self.status_direct,
                (Lane::Quick, true) => &self.quick_direct,
                (Lane::Quick, false) => &self.quick_proxied,
                (Lane::Full, _) => self.agent_for(&at),
            };
            let mut req = match deadline {
                Some(d) => agent.get(&at).config().max_redirects(0).timeout_global(Some(d)).build(),
                None => agent.get(&at).config().max_redirects(0).build(),
            };
            if let Some(g) = self.grant_for(&at) {
                req = req.header("x-elo-grant", g);
            }
            let response = req.call()?;
            if !response.status().is_redirection() {
                return Ok(response);
            }
            let next = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .and_then(|l| redirect_target(&at, l));
            match next {
                Some(n) => at = n,
                // A 3xx with nowhere to go (a 304, a broken header) is the
                // answer, and the caller reads it as one.
                None => return Ok(response),
            }
        }
        Err(ureq::Error::TooManyRedirects)
    }
}

/// `elo_depot::update::JsonSource` over the real network.
impl elo_depot::JsonSource for Net {
    fn get(&self, url: &str) -> Result<Value, String> {
        let got = self.get_json(url);
        match got.value {
            Some(v) => Ok(v),
            None if got.reachable => Err(got.why),
            None => Err(format!("could not reach it — {}", got.why)),
        }
    }
}

/// `elo_depot::Fetcher` over the real network — streamed to disk, never held
/// in memory, because a depot file is a game asset and not a document.
impl Fetcher for Net {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), DepotError> {
        self.fetch_within(url, dest, TIMEOUT, None)
    }

    fn fetch_sized(&self, url: &str, dest: &Path, bytes: i64) -> Result<(), DepotError> {
        self.fetch_within(url, dest, depot_file_deadline(bytes), Some(bytes.max(0) as u64))
    }
}

impl Net {
    /// `declared` is the depot's size for the file. When it is known the body
    /// is read to exactly that many bytes: one more is a refusal **before** it
    /// lands, because the hash check runs only after the whole stream is on
    /// disk, and a host that keeps sending would otherwise fill the player's
    /// disk until the deadline. Unsized, the ceiling is the largest file a
    /// depot may declare.
    fn fetch_within(
        &self,
        url: &str,
        dest: &Path,
        deadline: Duration,
        declared: Option<u64>,
    ) -> Result<(), DepotError> {
        let response = self
            .get(url, Lane::Full, Some(deadline), MAX_REDIRECTS)
            .map_err(|e| DepotError::new(format!("could not fetch {url}: {e}")))?;
        let cap = declared.unwrap_or(elo_depot::depot::MAX_FILE_BYTES as u64);
        let mut reader = response.into_body().into_reader().take(cap + 1);
        let mut file = std::fs::File::create(dest)
            .map_err(|e| DepotError::new(format!("{}: {e}", dest.display())))?;
        let got = std::io::copy(&mut reader, &mut file)
            .map_err(|e| DepotError::new(format!("could not fetch {url}: {e}")))?;
        if got > cap {
            return Err(DepotError::new(format!(
                "could not fetch {url}: the host sent more than the {cap} bytes declared"
            )));
        }
        if declared.is_some_and(|n| got != n) {
            return Err(DepotError::new(format!(
                "could not fetch {url}: the host sent {got} of the {cap} bytes declared"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The production spellings of the lookalikes `tests/grant.rs` drives over
    /// a socket.
    #[test]
    fn the_origin_is_a_tuple_and_a_lookalike_is_not_it() {
        let o = "https://elopros.com";
        for yes in [
            "https://elopros.com/api/launcher/depot/gates/1",
            "https://ELOPROS.com:443/api/x",
            "HTTPS://elopros.com",
        ] {
            assert!(same_origin(yes, o), "{yes}");
        }
        for no in [
            "https://elopros.com.evil.tld/d.json",
            "https://elopros.com@evil.tld/d.json",
            "https://user@elopros.com/d.json",
            "http://elopros.com/d.json",
            "https://elopros.com:8443/d.json",
            "https://evil.tld/https://elopros.com",
            "elopros.com/d.json",
            "",
        ] {
            assert!(!same_origin(no, o), "{no}");
        }
    }

    /// Resolves every name to one address — a DNS answer the title controls,
    /// such as `169.254.169.254.nip.io`, without asking DNS.
    #[derive(Debug)]
    struct Answers(std::net::SocketAddr);

    impl Resolver for Answers {
        fn resolve(
            &self,
            _: &ureq::http::Uri,
            _: &ureq::config::Config,
            _: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, ureq::Error> {
            let mut v = self.empty();
            v.push(self.0);
            Ok(v)
        }
    }

    /// Did anything connect to `l` within a second?
    fn knocked(l: std::net::TcpListener) -> std::thread::JoinHandle<bool> {
        l.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let until = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < until {
                if l.accept().is_ok() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        })
    }

    const LIST: &str = "https://elopros.com/api/launcher/servers/gates";

    /// l-net-client-ssrf#1: `status_url_to_poll` reads names, and a public
    /// name can resolve to the player's own network. A status poll checks
    /// the addresses it is about to connect to.
    #[test]
    fn a_name_that_resolves_here_is_not_polled() {
        let inside = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = inside.local_addr().unwrap();
        let url = format!("http://shard.example:{}/status.json", at.port());
        let direct = |names: PublicOnly| Net {
            status_direct: ureq::Agent::with_parts(
                ureq::Agent::config_builder().timeout_global(Some(STATUS_TIMEOUT))
                    .proxy(None).build(),
                DefaultConnector::default(),
                names,
            ),
            no_proxy: vec!["shard.example".into()],
            ..Net::new()
        };

        let k = knocked(inside.try_clone().unwrap());
        let got = direct(PublicOnly(Arc::new(Answers(at)))).status(&url, LIST);
        assert!(!k.join().unwrap(), "the poll reached {at} through a name");
        assert!(got.value.is_none());
        assert!(got.why.contains("this machine or its network"), "{}", got.why);

        // The same name, polled for a developer's own list, goes.
        let mut net = direct(PublicOnly(Arc::new(Answers(at))));
        net.quick_direct = ureq::Agent::with_parts(
            ureq::Agent::config_builder().timeout_global(Some(STATUS_TIMEOUT)).proxy(None).build(),
            DefaultConnector::default(),
            Answers(at),
        );
        let k = knocked(inside);
        let _ = net.status(&url, "http://127.0.0.1:3600/api/launcher/servers/gates");
        assert!(k.join().unwrap(), "the stub resolver is what the poll connects through");
    }

    /// l-net-client-ssrf#1, round 4: an HTTP(S) proxy resolves the name a
    /// second time (`CONNECT name:port`), and a proxy on the player's machine
    /// or network then reaches whatever the name answers it. So a status poll
    /// never goes through the proxy: a local answer is refused here, and a
    /// public one is connected to directly.
    #[test]
    fn a_status_poll_never_goes_through_a_proxy() {
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let via = |answer: &str| Net {
            quick_proxied: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_millis(500)))
                .proxy(Some(ureq::Proxy::new(&format!("http://{}", proxy.local_addr().unwrap()))
                    .unwrap()))
                .build()
                .new_agent(),
            status_direct: ureq::Agent::with_parts(
                ureq::Agent::config_builder().timeout_global(Some(Duration::from_millis(500)))
                    .proxy(None).build(),
                DefaultConnector::default(),
                PublicOnly(Arc::new(Answers(answer.parse().unwrap()))),
            ),
            no_proxy: vec![],
            ..Net::new()
        };
        let url = "http://shard.example/status.json";

        let k = knocked(proxy.try_clone().unwrap());
        let got = via("169.254.169.254:80").status(url, LIST);
        assert!(!k.join().unwrap(), "a name resolving to 169.254.169.254 went to the proxy");
        assert!(got.why.contains("this machine or its network"), "{}", got.why);

        let k = knocked(proxy.try_clone().unwrap());
        let _ = via("203.0.113.9:80").status(url, LIST);
        assert!(!k.join().unwrap(), "a public name was handed to the proxy to resolve again");
    }

    /// The same rebinding name as below, for a player with an HTTP proxy on
    /// their own machine: neither the proxy nor the loopback service hears it.
    #[test]
    fn a_proxied_player_cannot_be_rebound_through_their_proxy() {
        let inside = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = inside.local_addr().unwrap().port();
        let dns = Arc::new(Rebinds {
            first: format!("203.0.113.9:{port}").parse().unwrap(),
            then: inside.local_addr().unwrap(),
            asked: Default::default(),
        });
        let quick = || ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(500)));
        let net = Net {
            quick_proxied: quick()
                .proxy(Some(ureq::Proxy::new(&format!("http://{}", proxy.local_addr().unwrap()))
                    .unwrap()))
                .build()
                .new_agent(),
            status_direct: ureq::Agent::with_parts(quick().proxy(None).build(),
                                                   DefaultConnector::default(),
                                                   PublicOnly(dns.clone())),
            no_proxy: vec![],
            ..Net::new()
        };

        let (k_in, k_proxy) = (knocked(inside), knocked(proxy));
        let got = net.status(&format!("http://shard.example:{port}/status.json"), LIST);
        assert!(!k_proxy.join().unwrap(), "the poll was handed to the proxy");
        assert!(!k_in.join().unwrap(), "a rebinding name reached 127.0.0.1:{port}");
        assert!(got.value.is_none());
        assert_eq!(dns.asked.load(std::sync::atomic::Ordering::SeqCst), 1,
                   "the name was resolved again after it was checked");
    }

    /// Answers `first` to the first lookup and `then` to every later one — a
    /// title's DNS rebinding a name between a check and a connect.
    #[derive(Debug)]
    struct Rebinds {
        first: std::net::SocketAddr,
        then: std::net::SocketAddr,
        asked: std::sync::atomic::AtomicUsize,
    }

    impl Resolver for Rebinds {
        fn resolve(
            &self,
            _: &ureq::http::Uri,
            _: &ureq::config::Config,
            _: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, ureq::Error> {
            let n = self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut v = self.empty();
            v.push(if n == 0 { self.first } else { self.then });
            Ok(v)
        }
    }

    /// l-net-client-ssrf#1, round 3: with no proxy configured, the player's
    /// machine is what resolves and connects, so the name is resolved ONCE
    /// and the address checked is the address connected to. A name that
    /// answers public to a check and loopback to a second lookup never
    /// reaches loopback.
    #[test]
    fn an_unproxied_poll_cannot_be_rebound_between_check_and_connect() {
        let inside = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = inside.local_addr().unwrap().port();
        let dns = Arc::new(Rebinds {
            first: format!("203.0.113.9:{port}").parse().unwrap(),
            then: inside.local_addr().unwrap(),
            asked: Default::default(),
        });
        let quick = || ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(500)))
            .proxy(None)
            .build();
        let net = Net {
            // What `Net::new` builds when no proxy is in the environment,
            // resolving through the same (stub) DNS as the check.
            quick_proxied: ureq::Agent::with_parts(quick(), DefaultConnector::default(),
                                                   SharedDns(dns.clone())),
            status_direct: ureq::Agent::with_parts(quick(), DefaultConnector::default(),
                                                   PublicOnly(dns.clone())),
            no_proxy: vec![],
            ..Net::new()
        };

        let k = knocked(inside);
        let got = net.status(&format!("http://shard.example:{port}/status.json"), LIST);
        assert!(!k.join().unwrap(), "a rebinding name reached 127.0.0.1:{port}");
        assert!(got.value.is_none());
        assert_eq!(dns.asked.load(std::sync::atomic::Ordering::SeqCst), 1,
                   "the name was resolved again after it was checked");
    }

    /// An `Arc`'d resolver handed to a second agent, as DNS is shared by
    /// every agent on a real machine.
    #[derive(Debug)]
    struct SharedDns(Arc<Rebinds>);

    impl Resolver for SharedDns {
        fn resolve(
            &self,
            uri: &ureq::http::Uri,
            config: &ureq::config::Config,
            timeout: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, ureq::Error> {
            self.0.resolve(uri, config, timeout)
        }
    }

    #[test]
    fn a_redirect_resolves_against_the_url_that_answered() {
        let at = "https://elopros.com/api/launcher/depot/gates/1";
        assert_eq!(redirect_target(at, "https://cdn.example/x").unwrap(), "https://cdn.example/x");
        assert_eq!(redirect_target(at, "//cdn.example/x").unwrap(), "https://cdn.example/x");
        assert_eq!(redirect_target(at, "/api/y").unwrap(), "https://elopros.com/api/y");
        assert_eq!(redirect_target(at, "2").unwrap(),
                   "https://elopros.com/api/launcher/depot/gates/2");
    }
}
