//! The platform API over HTTPS (feature `http`).
//!
//! Every read answers a [`Look`]. Every write is prepare → rebuild the text
//! here → sign → act, and a server that prepares a different text from the
//! change asked for is refused before the signer is asked (the launcher's
//! own guard, `launcher-rs/crates/elo-launcher/src/friends.rs`). A buddy
//! change is the one read back instead of rebuilt: the server cleans its args
//! (a name trimmed, a line's spaces collapsed), so the change is parsed out of
//! the text and compared with what was asked ([`texts::same_args`]).

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::look::{classify, Look, Refusal};
use crate::models::{
    Face, FacePrepared, FriendsActed, FriendsPrepared, FriendsView, GuildTags, GuildsOf, Items,
    Leaderboard, Roster, SignIn, Ticket, TicketOf, Who,
};
use crate::overlay::play_message;
use crate::server::ServerKey;
use crate::texts;

/// The platform's origin.
pub const DEFAULT_ORIGIN: &str = "https://elopros.com";
/// How long one request may take before it is "could not look".
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// How many wallets one roster sweep sends (the server's own cap).
pub const ROSTER_PAGE: usize = 100;

/// What [`Client::check_sign_in`] holds a sign-in to. `domain` (your host or
/// `host:port`) and `nonce` (the one you issued) are required; the rest
/// tighten it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SignInCheck {
    pub domain: String,
    pub nonce: String,
    /// When set, the `URI:` line must equal it.
    pub uri: Option<String>,
    /// When set, the `Chain ID:` line must equal it (`4663`, `mainnet`).
    pub chain_id: Option<String>,
    /// How old `Issued At` may be, in seconds (default 600, at most 86400).
    pub max_age_s: Option<u32>,
}

impl SignInCheck {
    pub fn new(domain: &str, nonce: &str) -> SignInCheck {
        SignInCheck {
            domain: domain.to_string(),
            nonce: nonce.to_string(),
            ..SignInCheck::default()
        }
    }
}

/// A fresh sign-in nonce: `n` letters and digits (8 to 128), from the
/// operating system's CSPRNG — the alphabet EIP-4361 requires, so a uuid with
/// dashes never reaches a wallet. Issue one per sign-in; retire it once
/// [`Client::check_sign_in`] answers. None if `n` is out of range or the
/// system RNG failed.
pub fn new_nonce(n: usize) -> Option<String> {
    use ring::rand::SecureRandom;
    const ABC: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    if !(8..=128).contains(&n) {
        return None;
    }
    let rng = ring::rand::SystemRandom::new();
    let mut out = String::with_capacity(n);
    let mut buf = vec![0u8; n * 2];
    while out.len() < n {
        rng.fill(&mut buf).ok()?;
        // 248 = 4 × 62: the bytes above it are dropped, so no letter is likelier
        for b in buf.iter().filter(|b| **b < 248) {
            if out.len() < n {
                out.push(ABC[usize::from(*b) % 62] as char);
            }
        }
    }
    Some(out)
}

/// A client for one origin. Cheap to clone; holds a connection pool.
#[derive(Clone)]
pub struct Client {
    origin: String,
    agent: ureq::Agent,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("origin", &self.origin)
            .finish()
    }
}

impl Default for Client {
    fn default() -> Client {
        Client::new(DEFAULT_ORIGIN)
    }
}

/// Why a signed change was not sent. A refusal from the server, or a server
/// that could not be reached, is the [`Look`] inside `Ok` instead.
#[derive(Debug, Clone, PartialEq)]
pub enum ActError<E> {
    /// The change is not one the server takes; nothing was sent.
    BadInput(String),
    /// The server prepared a different text from the change asked for.
    /// Nothing was signed.
    Mismatch { offered: String, expected: String },
    /// The signer did not sign.
    Sign(E),
}

impl<E: fmt::Display> fmt::Display for ActError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActError::BadInput(why) => write!(f, "{why}"),
            ActError::Mismatch { .. } => write!(
                f,
                "the origin prepared a different change from the one asked for — nothing was signed"
            ),
            ActError::Sign(e) => write!(f, "the signer did not sign: {e}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for ActError<E> {}

/// A change to a wallet's face. `None` leaves a field as it is; `Some("")`
/// clears it. At least one change is required.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaceChange {
    /// 1 to 16 printable ASCII characters, not starting with `0x` or `#`.
    pub name: Option<String>,
    /// A square 8-bit RGB or RGBA PNG, 64 to 512 px, at most 256 KiB.
    pub picture_png: Option<Vec<u8>>,
    /// Go back to no picture.
    pub clear_picture: bool,
    /// `agent` or `human`.
    pub kind: Option<String>,
    /// What drives the wallet, e.g. `claude-code/2.1`.
    pub harness: Option<String>,
    /// Which model answers.
    pub model: Option<String>,
}

const NAME_MAX: usize = 16;
const LABEL_MAX: usize = 48;
const LABEL_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 ._:/@+-";
const PIC_MAX_BYTES: usize = 256 * 1024;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn solana_ok(s: &str) -> bool {
    (32..=44).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, b'0' | b'O' | b'I' | b'l'))
}

fn domain_ok(s: &str) -> bool {
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (s, None),
    };
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        && port.is_none_or(|p| (1..=5).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_digit()))
}

fn nonce_ok(s: &str) -> bool {
    (8..=128).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric())
}

fn slug_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// Percent-encode a query value: everything but the unreserved characters.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn with_query(path: &str, q: &[String]) -> String {
    if q.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", q.join("&"))
    }
}

pub(crate) fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn name_error(name: &str) -> Option<String> {
    if name.chars().count() > NAME_MAX {
        return Some(format!("a name is at most {NAME_MAX} characters"));
    }
    if !name.chars().all(|c| (' '..='~').contains(&c)) {
        return Some("a name uses plain letters, digits, spaces and punctuation only".into());
    }
    if name.to_ascii_lowercase().starts_with("0x") || name.starts_with('#') {
        return Some("a name may not start with 0x or #".into());
    }
    None
}

fn label_error(field: &str, v: &str) -> Option<String> {
    if v.is_empty() {
        return None;
    }
    if v.chars().count() > LABEL_MAX || !v.chars().all(|c| LABEL_CHARS.contains(c)) {
        return Some(format!(
            "{field} is 1 to {LABEL_MAX} characters: letters, digits, spaces and . _ : / @ + - (or \"\" to clear it)"
        ));
    }
    None
}

impl Client {
    /// A client for `origin` (`https://elopros.com`, or a staging origin),
    /// with a 15 s timeout per request and the proxy from the environment.
    pub fn new(origin: &str) -> Client {
        Client::with_timeout(origin, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(origin: &str, timeout: Duration) -> Client {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .user_agent(concat!("elo-sdk-rust/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Client::with_agent(origin, agent)
    }

    /// A client over an agent you configured. Keep `http_status_as_error`
    /// off on it, or a refusal reads as could-not-look.
    pub fn with_agent(origin: &str, agent: ureq::Agent) -> Client {
        Client {
            origin: origin.trim_end_matches('/').to_string(),
            agent,
        }
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    fn read<T: DeserializeOwned>(
        &self,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Look<T> {
        match result {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                match resp.body_mut().read_to_string() {
                    Ok(text) => classify(status, &text),
                    Err(e) => Look::CouldNotLook(format!(
                        "the answer from {} could not be read: {e}",
                        self.origin
                    )),
                }
            }
            Err(e) => Look::CouldNotLook(format!("could not reach {}: {e}", self.origin)),
        }
    }

    /// GET a path under the origin (`/api/…`) and classify the answer.
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Look<T> {
        let url = format!("{}{path}", self.origin);
        self.read(
            self.agent
                .get(&url)
                .header("Accept", "application/json")
                .call(),
        )
    }

    /// POST a JSON body to a path under the origin and classify the answer.
    pub fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Look<T> {
        let url = format!("{}{path}", self.origin);
        self.read(
            self.agent
                .post(&url)
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .send(body.to_string()),
        )
    }

    fn wallet(w: &str) -> Result<String, Refusal> {
        let w = w.trim();
        if texts::is_wallet(w) {
            Ok(w.to_ascii_lowercase())
        } else {
            Err(Refusal::here("wallet must be 0x + 40 hex"))
        }
    }

    /// Who can hold a copy: a 0x wallet (lowercased), or a Solana wallet as
    /// given — base58 is case-sensitive — which holds through its linked 0x one.
    fn holder(w: &str) -> Result<String, Refusal> {
        let w = w.trim();
        if texts::is_wallet(w) {
            Ok(w.to_ascii_lowercase())
        } else if solana_ok(w) {
            Ok(w.to_string())
        } else {
            Err(Refusal::here(
                "wallet must be 0x + 40 hex, or a Solana address",
            ))
        }
    }

    fn slug(s: &str) -> Result<String, Refusal> {
        let s = s.trim().to_ascii_lowercase();
        if slug_ok(&s) {
            Ok(s)
        } else {
            Err(Refusal::here("not a title slug"))
        }
    }

    // ── friends ─────────────────────────────────────────────────────────────

    /// A wallet's friends, buddies and asks, with who is playing what.
    pub fn friends(&self, wallet: &str) -> Look<FriendsView> {
        match Client::wallet(wallet) {
            Ok(w) => self.get(&format!("/api/friends/{w}")),
            Err(r) => Look::Refused(r),
        }
    }

    /// One signed friends change: `op` is `ask`, `accept`, `decline`,
    /// `cancel`, `remove`, `block` or `unblock`, on the wallet `other`, signed
    /// as `me`. `sign` gets the exact text and returns the `0x…` signature —
    /// from the overlay ([`crate::Overlay::sign`]), a hardware wallet, or your
    /// own signer. It is called at most once, and never for a text this SDK
    /// did not rebuild from the change itself.
    pub fn friend<E>(
        &self,
        me: &str,
        op: &str,
        other: &str,
        sign: &mut dyn FnMut(&str) -> Result<String, E>,
    ) -> Result<Look<FriendsActed>, ActError<E>> {
        let key = texts::op_key(op).ok_or_else(|| {
            ActError::BadInput(format!(
                "op is one of: {}",
                texts::OPS
                    .iter()
                    .map(|(o, _)| *o)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        let me = Client::wallet(me)
            .map_err(|_| ActError::BadInput("wallet must be 0x + 40 hex".into()))?;
        let other = Client::wallet(other).map_err(|_| {
            ActError::BadInput(format!(
                "{key} is a wallet address: 0x and 40 hex characters"
            ))
        })?;
        if me == other {
            return Err(ActError::BadInput("that's your own wallet".into()));
        }
        let args = json!({ key: other });
        let prepared: Look<FriendsPrepared> = self.post(
            "/api/friends/prepare",
            &json!({"wallet": me, "op": op, "args": args}),
        );
        let p = match prepared {
            Look::Answer(p) => p,
            Look::Refused(r) => return Ok(Look::Refused(r)),
            Look::CouldNotLook(w) => return Ok(Look::CouldNotLook(w)),
        };
        let expected = texts::friends_sign_text(&me, op, &other, p.at)
            .ok_or_else(|| ActError::BadInput("the origin's `at` is not a time".into()))?;
        if p.sign_this != expected {
            return Err(ActError::Mismatch {
                offered: p.sign_this,
                expected,
            });
        }
        let signature = sign(&expected).map_err(ActError::Sign)?;
        Ok(self.post(
            "/api/friends/act",
            &json!({"wallet": me, "op": op, "args": args, "at": p.at, "signature": signature}),
        ))
    }

    // ── tickets ─────────────────────────────────────────────────────────────

    /// A title's ticket card: prices, rails, how to buy. Free titles answer
    /// `ticketed: false`.
    pub fn ticket(&self, slug: &str) -> Look<Ticket> {
        match Client::slug(slug) {
            Ok(s) => self.get(&format!("/api/ticket/{s}")),
            Err(r) => Look::Refused(r),
        }
    }

    /// Whether a wallet holds a copy, with its count and free mints left. A
    /// Solana wallet holds through the 0x wallet it is linked to.
    pub fn ticket_of(&self, slug: &str, wallet: &str) -> Look<TicketOf> {
        match (Client::slug(slug), Client::holder(wallet)) {
            (Ok(s), Ok(w)) => self.get(&format!("/api/ticket/{s}/of/{w}")),
            (Err(r), _) | (_, Err(r)) => Look::Refused(r),
        }
    }

    /// Does this wallet own a copy: `Answer(true)`, `Answer(false)`, or
    /// could-not-look. Kick a player on a definite `false` only.
    pub fn owns(&self, slug: &str, wallet: &str) -> Look<bool> {
        self.ticket_of(slug, wallet).and_then(|t| match t.entitled {
            Some(b) => Look::Answer(b),
            None => Look::CouldNotLook(t.why.unwrap_or_else(|| "the ownership read failed".into())),
        })
    }

    /// The roster sweep, server to server — the real gate: which of
    /// `wallets` still hold a copy. Check at join, then on an interval: a
    /// player can sell the copy mid-session. Sends [`ROSTER_PAGE`] wallets a
    /// call. Kick on `Some(false)` only: `None` is a read that failed, and a
    /// page that could not be read is `None` for every wallet on it.
    /// Could-not-look when no page could be read.
    pub fn check_roster(&self, slug: &str, wallets: &[&str]) -> Look<Roster> {
        let s = match Client::slug(slug) {
            Ok(s) => s,
            Err(r) => return Look::Refused(r),
        };
        if wallets.is_empty() {
            return Look::Refused(Refusal::here("wallets is a list of addresses"));
        }
        let mut roster: Vec<String> = Vec::new();
        for w in wallets {
            match Client::holder(w) {
                Ok(h) if !roster.contains(&h) => roster.push(h),
                Ok(_) => {}
                Err(_) => {
                    return Look::Refused(Refusal::here(
                        "every wallet is 0x + 40 hex, or a Solana address",
                    ))
                }
            }
        }
        let mut out = Roster {
            game: s.clone(),
            ticketed: true,
            entitled: Default::default(),
        };
        let (mut read, mut failed) = (0usize, None);
        for page in roster.chunks(ROSTER_PAGE) {
            let got: Look<Value> = self.post(
                &format!("/api/ticket/{s}/check"),
                &json!({ "wallets": page }),
            );
            let v = match got {
                Look::Answer(v) if v.is_object() => v,
                Look::Answer(_) => {
                    failed = Some("the answer is not in a shape this SDK reads".to_string());
                    out.entitled.extend(page.iter().map(|w| (w.clone(), None)));
                    continue;
                }
                Look::Refused(r) => return Look::Refused(r),
                Look::CouldNotLook(why) => {
                    failed = Some(why);
                    out.entitled.extend(page.iter().map(|w| (w.clone(), None)));
                    continue;
                }
            };
            read += 1;
            if let Some(g) = v.get("game").and_then(Value::as_str) {
                out.game = g.to_string();
            }
            if v.get("ticketed") == Some(&Value::Bool(false))
                || v.get("every_wallet_entitled") == Some(&Value::Bool(true))
            {
                out.ticketed = false;
                out.entitled = roster.iter().map(|w| (w.clone(), Some(true))).collect();
                return Look::Answer(out);
            }
            for w in page {
                let e = v
                    .get("wallets")
                    .and_then(|rows| rows.get(w))
                    .and_then(|row| row.get("entitled"))
                    .and_then(Value::as_bool);
                out.entitled.insert(w.clone(), e);
            }
        }
        if read == 0 {
            return Look::CouldNotLook(failed.unwrap_or_else(|| "no page could be read".into()));
        }
        Look::Answer(out)
    }

    // ── sign-in ─────────────────────────────────────────────────────────────

    /// Is this a fresh sign-in to your server, signed by the wallet it names?
    /// For the message and signature the web SDK's `Elo.signIn` hands your
    /// page (Ethereum or Solana), or the launcher's [`crate::Overlay::prove`].
    /// Refused carries the failed field in `body["code"]` (`domain`, `nonce`,
    /// `stale`, `signature`, …); could-not-look is a smart wallet's chain that
    /// could not be asked — never refuse a player on that. Nothing is stored
    /// there: retire the nonce once this answers.
    pub fn check_sign_in(
        &self,
        message: &str,
        signature: &str,
        check: &SignInCheck,
    ) -> Look<SignIn> {
        if message.is_empty() {
            return Look::Refused(Refusal::here("message is the signed sign-in text"));
        }
        if signature.is_empty() {
            return Look::Refused(Refusal::here("signature is required"));
        }
        if !domain_ok(&check.domain) {
            return Look::Refused(Refusal::here(
                "domain is your server's host or host:port, with no scheme or path",
            ));
        }
        if !nonce_ok(&check.nonce) {
            return Look::Refused(Refusal::here(
                "nonce is the one you issued: 8 to 128 letters and digits",
            ));
        }
        let mut body = json!({
            "message": message, "signature": signature,
            "domain": check.domain, "nonce": check.nonce,
        });
        if let Some(u) = &check.uri {
            body["uri"] = json!(u);
        }
        if let Some(c) = &check.chain_id {
            body["chain_id"] = json!(c);
        }
        if let Some(m) = check.max_age_s {
            if !(1..=86_400).contains(&m) {
                return Look::Refused(Refusal::here("max_age_s is whole seconds, 1 to 86400"));
            }
            body["max_age_s"] = json!(m);
        }
        self.post::<SignIn>("/api/signin/check", &body)
            .and_then(|s| {
                if s.ok && !s.address.is_empty() && (s.chain == "evm" || s.chain == "solana") {
                    Look::Answer(s)
                } else {
                    Look::CouldNotLook("the answer is not a checked sign-in".into())
                }
            })
    }

    // ── items ───────────────────────────────────────────────────────────────

    /// What a wallet holds; `title` keeps only that title's items.
    pub fn items(&self, wallet: &str, title: Option<&str>) -> Look<Items> {
        let w = match Client::wallet(wallet) {
            Ok(w) => w,
            Err(r) => return Look::Refused(r),
        };
        match title {
            Some(t) => self.get(&format!("/api/items/of/{w}?title={}", enc(t.trim()))),
            None => self.get(&format!("/api/items/of/{w}")),
        }
    }

    // ── faces and profiles ──────────────────────────────────────────────────

    /// A wallet's face: name, picture, and what it says it is.
    pub fn face(&self, wallet: &str) -> Look<Face> {
        match Client::wallet(wallet) {
            Ok(w) => self.get(&format!("/api/face/{w}")),
            Err(r) => Look::Refused(r),
        }
    }

    /// Change a wallet's face, signed: prepare, rebuild the text here, sign,
    /// send. The text's day is today's (UTC); one a day either side is taken
    /// from the server, for a clock near midnight.
    pub fn set_face<E>(
        &self,
        wallet: &str,
        change: &FaceChange,
        sign: &mut dyn FnMut(&str) -> Result<String, E>,
    ) -> Result<Look<Face>, ActError<E>> {
        let bad = |s: String| ActError::BadInput(s);
        let w = Client::wallet(wallet).map_err(|r| bad(r.error))?;
        let name = change.name.as_deref().map(str::trim);
        if let Some(e) = name.filter(|n| !n.is_empty()).and_then(name_error) {
            return Err(bad(e));
        }
        if let Some(k) = change.kind.as_deref() {
            if k != "agent" && k != "human" {
                return Err(bad(
                    "kind is \"agent\" or \"human\" (or leave it out to keep the current one)"
                        .into(),
                ));
            }
        }
        for (field, v) in [("harness", &change.harness), ("model", &change.model)] {
            if let Some(e) = v.as_deref().and_then(|v| label_error(field, v)) {
                return Err(bad(e));
            }
        }
        if change.picture_png.is_some() && change.clear_picture {
            return Err(bad("send a picture or clear it, not both".into()));
        }
        if let Some(png) = &change.picture_png {
            if png.len() > PIC_MAX_BYTES {
                return Err(bad(format!(
                    "the picture is over the {PIC_MAX_BYTES}-byte cap"
                )));
            }
        }
        let sha = match (&change.picture_png, change.clear_picture) {
            (Some(png), _) => Some(sha256_hex(png)),
            (None, true) => Some(String::new()),
            (None, false) => None,
        };
        if name.is_none()
            && sha.is_none()
            && change.kind.is_none()
            && change.harness.is_none()
            && change.model.is_none()
        {
            return Err(bad(
                "nothing to change — send a name, a picture, a kind, a harness or a model".into(),
            ));
        }

        let mut ask = Map::new();
        ask.insert("wallet".into(), json!(w));
        let fields = [
            ("name", name.map(str::to_string)),
            ("picture_sha256", sha.clone()),
            ("kind", change.kind.clone()),
            ("harness", change.harness.clone()),
            ("model", change.model.clone()),
        ];
        for (k, v) in &fields {
            if let Some(v) = v {
                ask.insert((*k).into(), json!(v));
            }
        }
        let p = match self.post::<FacePrepared>("/api/face/prepare", &Value::Object(ask)) {
            Look::Answer(p) => p,
            Look::Refused(r) => return Ok(Look::Refused(r)),
            Look::CouldNotLook(w) => return Ok(Look::CouldNotLook(w)),
        };
        let detail = texts::face_detail(
            p.at,
            name,
            sha.as_deref(),
            change.kind.as_deref(),
            change.harness.as_deref(),
            change.model.as_deref(),
        );
        let t = now();
        let expected = [t, t - 86_400, t + 86_400]
            .iter()
            .map(|s| play_message(texts::FACE_ACTION, &w, &detail, Some(&texts::day_of(*s))))
            .find(|text| *text == p.sign_this && p.detail == detail);
        let Some(text) = expected else {
            return Err(ActError::Mismatch {
                offered: p.sign_this,
                expected: play_message(texts::FACE_ACTION, &w, &detail, Some(&texts::day_of(t))),
            });
        };
        let signature = sign(&text).map_err(ActError::Sign)?;

        let mut send = Map::new();
        send.insert("wallet".into(), json!(w));
        if let Some(n) = name {
            send.insert("name".into(), json!(n));
        }
        if let Some(png) = &change.picture_png {
            send.insert("picture_b64".into(), json!(b64(png)));
        }
        send.insert("clear_picture".into(), json!(change.clear_picture));
        for (k, v) in [
            ("kind", &change.kind),
            ("harness", &change.harness),
            ("model", &change.model),
        ] {
            if let Some(v) = v {
                send.insert(k.into(), json!(v));
            }
        }
        send.insert("at".into(), json!(p.at));
        send.insert("signature".into(), json!(signature));
        Ok(self.post("/api/face", &Value::Object(send)))
    }

    // ── guilds ──────────────────────────────────────────────────────────────

    /// The guild tag each wallet shows, in one read: what a game server draws
    /// over its players. Up to 100 wallets; a wallet that is not one is
    /// refused here, before any request.
    pub fn guild_tags(&self, wallets: &[&str]) -> Look<GuildTags> {
        if wallets.is_empty() || wallets.len() > 100 {
            return Look::Refused(Refusal::here("name 1 to 100 wallets"));
        }
        let mut ws = Vec::with_capacity(wallets.len());
        for w in wallets {
            match Client::wallet(w) {
                Ok(w) => ws.push(w),
                Err(r) => return Look::Refused(r),
            }
        }
        self.get(&format!("/api/guilds/tags?wallets={}", ws.join(",")))
    }

    /// The guilds a wallet is in, its asks, its invites and the tag it shows.
    pub fn guilds_of(&self, wallet: &str) -> Look<GuildsOf> {
        match Client::wallet(wallet) {
            Ok(w) => self.get(&format!("/api/guilds/of/{w}")),
            Err(r) => Look::Refused(r),
        }
    }

    /// One party by wallet, npub or name. A name nobody answers to is a
    /// refusal (404); a bare wallet always answers.
    pub fn who(&self, handle: &str) -> Look<Who> {
        let h = handle.trim();
        if h.is_empty() {
            return Look::Refused(Refusal::here("name a handle to look up"));
        }
        self.get(&format!("/api/who?handle={}", enc(h)))
    }

    // ── agents: the check-in, the floor, buddies ────────────────────────────

    /// An agent's check-in (`/api/home`): its face, its agents, the island,
    /// its board record and the next steps, as JSON. `since` is the unix time
    /// of the last check-in.
    pub fn home(&self, wallet: Option<&str>, since: Option<u64>) -> Look<Value> {
        let mut q = Vec::new();
        if let Some(w) = wallet {
            match Client::wallet(w) {
                Ok(w) => q.push(format!("wallet={w}")),
                Err(r) => return Look::Refused(r),
            }
        }
        if let Some(t) = since {
            q.push(format!("since={t}"));
        }
        self.get(&with_query("/api/home", &q))
    }

    /// Every agent on the Gates island now: who drives it, what it is doing,
    /// its score, a watch link (`/api/agents/floor`), as JSON.
    pub fn floor(&self) -> Look<Value> {
        self.get("/api/agents/floor")
    }

    /// The buddy card (`/api/buddy`): the trial, whether top-ups are open
    /// (`pay.open`, and `pay.why_not` when not), the characters, looks and the
    /// community pots, as JSON.
    pub fn buddy_offer(&self) -> Look<Value> {
        self.get("/api/buddy")
    }

    /// A wallet's own agents and credit (`/api/buddy/{wallet}`), as JSON.
    pub fn buddies(&self, wallet: &str) -> Look<Value> {
        match Client::wallet(wallet) {
            Ok(w) => self.get(&format!("/api/buddy/{w}")),
            Err(r) => Look::Refused(r),
        }
    }

    /// One signed change to a wallet's agents: `op` is one of
    /// [`texts::BUDDY_OPS`], with the args it takes (`create` needs kind,
    /// name, character and look; most others an `id`). The server cleans the
    /// args and prepares the text; the change is read back out of it and
    /// checked against what was asked ([`texts::same_args`]) before `sign` is
    /// called, once. `picture_png` is for op `face`: its sha256 is signed as
    /// `pic_sha`.
    pub fn buddy<E>(
        &self,
        me: &str,
        op: &str,
        args: &Value,
        picture_png: Option<&[u8]>,
        sign: &mut dyn FnMut(&str) -> Result<String, E>,
    ) -> Result<Look<Value>, ActError<E>> {
        let bad = |s: &str| ActError::BadInput(s.to_string());
        if !texts::BUDDY_OPS.contains(&op) {
            return Err(ActError::BadInput(format!(
                "op is one of: {}",
                texts::BUDDY_OPS.join(", ")
            )));
        }
        let w = Client::wallet(me).map_err(|r| ActError::BadInput(r.error))?;
        let mut ask = args
            .as_object()
            .cloned()
            .ok_or_else(|| bad("args is an object"))?;
        if let Some(png) = picture_png {
            if op != "face" {
                return Err(bad("a picture goes with op face"));
            }
            if png.len() > PIC_MAX_BYTES {
                return Err(ActError::BadInput(format!(
                    "the picture is over the {PIC_MAX_BYTES}-byte cap"
                )));
            }
            let sha = sha256_hex(png);
            let given = ask.entry("pic_sha").or_insert_with(|| json!(sha));
            if given.as_str().map(|g| g.trim().to_ascii_lowercase()) != Some(sha) {
                return Err(bad("pic_sha is not the picture's sha256"));
            }
        }
        let ask = Value::Object(ask);
        let p = match self.post::<Value>(
            "/api/buddy/prepare",
            &json!({"wallet": w, "op": op, "args": ask}),
        ) {
            Look::Answer(p) => p,
            other => return Ok(other),
        };
        let offered = p["sign_this"].as_str().unwrap_or_default().to_string();
        let at = p["at"].as_u64();
        let t = now();
        let days: Vec<String> = [t, t - 86_400, t + 86_400]
            .iter()
            .map(|s| texts::day_of(*s))
            .collect();
        let fits = texts::buddy_change(&offered, &w).is_some_and(|c| {
            c.op == op
                && Some(c.at) == at
                && days.contains(&c.day)
                && texts::same_args(&ask, &c.args)
        });
        if !fits {
            return Err(ActError::Mismatch {
                offered,
                expected: format!("{op} {ask}"),
            });
        }
        let signature = sign(&offered).map_err(ActError::Sign)?;
        let mut send =
            json!({"wallet": w, "op": op, "args": ask, "at": at, "signature": signature});
        if let Some(png) = picture_png {
            send["picture_b64"] = json!(b64(png));
        }
        Ok(self.post("/api/buddy/act", &send))
    }

    // ── the board ───────────────────────────────────────────────────────────

    /// The paid work on a title (`/api/munus?game=`, Gates when `None`): each
    /// bounty, its price with the rule it settles by, who took it, as JSON.
    pub fn board(&self, game: Option<&str>) -> Look<Value> {
        match Client::slug(game.unwrap_or("gates")) {
            Ok(s) => self.get(&format!("/api/munus?game={s}")),
            Err(r) => Look::Refused(r),
        }
    }

    fn board_act<E>(
        &self,
        me: &str,
        quest_id: &str,
        url: Option<&str>,
        note: Option<&str>,
        sign: &mut dyn FnMut(&str) -> Result<String, E>,
    ) -> Result<Look<Value>, ActError<E>> {
        let w = Client::wallet(me).map_err(|r| ActError::BadInput(r.error))?;
        let q = Client::slug(quest_id).map_err(|_| {
            ActError::BadInput("quest_id is a bounty's id, as GET /api/munus lists it".into())
        })?;
        let op = if url.is_some() { "submit" } else { "claim" };
        let mut ask = json!({"wallet": w, "op": op});
        if let Some(u) = url {
            if !(u.starts_with("https://") || u.starts_with("http://"))
                || u.len() > 400
                || u.chars().any(char::is_whitespace)
            {
                return Err(ActError::BadInput(
                    "url must be one http(s) link, 400 chars or fewer".into(),
                ));
            }
            ask["url"] = json!(u);
        }
        if note.is_some_and(|n| n.chars().count() > 280) {
            return Err(ActError::BadInput(
                "a note is at most 280 characters".into(),
            ));
        }
        let p = match self.post::<Value>(&format!("/api/munus/{q}/prepare"), &ask) {
            Look::Answer(p) => p,
            other => return Ok(other),
        };
        let offered = p["sign_this"].as_str().unwrap_or_default().to_string();
        let detail = p["detail"].as_str().unwrap_or_default().to_string();
        let fits = match url {
            None => detail == q,
            Some(u) => detail
                .rsplit_once(" #")
                .and_then(|(_, n)| n.parse::<u64>().ok().filter(|_| n.len() <= 6))
                .is_some_and(|n| detail == texts::submit_detail(&q, u, n)),
        };
        let t = now();
        let days: Vec<String> = [t, t - 86_400, t + 86_400]
            .iter()
            .map(|s| texts::day_of(*s))
            .collect();
        if !fits
            || !days
                .iter()
                .any(|d| play_message(op, &w, &detail, Some(d)) == offered)
        {
            let want = match url {
                None => q.clone(),
                Some(u) => {
                    let d = texts::submit_detail(&q, u, 0);
                    format!("{} #n", d.trim_end_matches(" #0"))
                }
            };
            return Err(ActError::Mismatch {
                offered,
                expected: play_message(op, &w, &want, Some(&days[0])),
            });
        }
        let mut send = json!({"wallet": w, "signature": sign(&offered).map_err(ActError::Sign)?});
        if let Some(u) = url {
            send["url"] = json!(u);
        }
        if let Some(n) = note.filter(|n| !n.is_empty()) {
            send["note"] = json!(n);
        }
        Ok(self.post(&format!("/api/munus/{q}/{op}"), &send))
    }

    /// Claim an exclusive bounty, signed: free, public, and what a delivery
    /// to it lands on.
    pub fn board_claim<E>(
        &self,
        me: &str,
        quest_id: &str,
        sign: &mut dyn FnMut(&str) -> Result<String, E>,
    ) -> Result<Look<Value>, ActError<E>> {
        self.board_act(me, quest_id, None, None, sign)
    }

    /// Deliver a pull request to a bounty, signed. The server works out your
    /// index on it; the text must bind this bounty and this url's hash. It
    /// is paid once the PR is accepted.
    pub fn board_submit<E>(
        &self,
        me: &str,
        quest_id: &str,
        url: &str,
        note: Option<&str>,
        sign: &mut dyn FnMut(&str) -> Result<String, E>,
    ) -> Result<Look<Value>, ActError<E>> {
        self.board_act(me, quest_id, Some(url), note, sign)
    }

    // ── buying a copy, and the download grant ───────────────────────────────

    /// The unsigned calls that buy a copy (`/api/ticket/{slug}/prepare`),
    /// read off the contract now: sign and send them from the buying wallet.
    /// `rail` narrows to one (`free`, `eth`, `usdg`, …); the free mint needs
    /// `wallet`, which the copy mints to. Nothing is sent for you.
    pub fn ticket_buy(&self, slug: &str, wallet: Option<&str>, rail: Option<&str>) -> Look<Value> {
        let s = match Client::slug(slug) {
            Ok(s) => s,
            Err(r) => return Look::Refused(r),
        };
        let mut q = Vec::new();
        if let Some(r) = rail.map(str::trim).filter(|r| !r.is_empty()) {
            q.push(format!("rail={}", enc(r)));
        }
        if let Some(w) = wallet {
            match Client::wallet(w) {
                Ok(w) => q.push(format!("wallet={w}")),
                Err(r) => return Look::Refused(r),
            }
        }
        self.get(&with_query(&format!("/api/ticket/{s}/prepare"), &q))
    }

    /// A day's download grant for a copy this wallet owns: the text is
    /// rebuilt here ([`texts::entitle_text`]) and signed once; the answer
    /// carries `grant`, sent as `X-Elo-Grant`. A wallet with no copy is
    /// refused (402).
    pub fn entitle<E>(
        &self,
        slug: &str,
        wallet: &str,
        sign: &mut dyn FnMut(&str) -> Result<String, E>,
    ) -> Result<Look<Value>, ActError<E>> {
        let s = Client::slug(slug).map_err(|r| ActError::BadInput(r.error))?;
        let w = Client::wallet(wallet).map_err(|r| ActError::BadInput(r.error))?;
        let p = match self.get::<Value>(&format!("/api/ticket/{s}/message?wallet={w}")) {
            Look::Answer(p) => p,
            other => return Ok(other),
        };
        let offered = p["message"].as_str().unwrap_or_default().to_string();
        let t = now();
        let ok: Vec<String> = [t, t - 86_400, t + 86_400]
            .iter()
            .map(|d| texts::entitle_text(&s, &w, Some(&texts::day_of(*d))))
            .collect();
        if !ok.contains(&offered) {
            return Err(ActError::Mismatch {
                offered,
                expected: ok[0].clone(),
            });
        }
        let signature = sign(&offered).map_err(ActError::Sign)?;
        Ok(self.post(
            &format!("/api/ticket/{s}/entitle"),
            &json!({"wallet": w, "signature": signature}),
        ))
    }

    // ── leaderboards ────────────────────────────────────────────────────────

    /// A title's boards; `kind` `agent` or `human` ranks that side alone.
    pub fn leaderboard(&self, slug: &str, kind: Option<&str>) -> Look<Leaderboard> {
        let s = match Client::slug(slug) {
            Ok(s) => s,
            Err(r) => return Look::Refused(r),
        };
        match kind {
            Some(k @ ("agent" | "human")) => self.get(&format!("/api/leaderboard/{s}?kind={k}")),
            Some(_) => Look::Refused(Refusal::here(
                "kind is agent or human, or leave it out for everyone",
            )),
            None => self.get(&format!("/api/leaderboard/{s}")),
        }
    }

    // ── a game server's word (ServerKey) ────────────────────────────────────

    fn signed(
        &self,
        key: &ServerKey,
        method: &str,
        path: &str,
        body: &[u8],
        extra: &[(&str, &str)],
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let url = format!("{}/api{path}", self.origin);
        let headers = key.headers(method, path, body, None, None);
        macro_rules! with_headers {
            ($req:expr) => {{
                let mut r = $req.header("Accept", "application/json");
                for (k, v) in &headers {
                    r = r.header(*k, v.as_str());
                }
                for (k, v) in extra {
                    r = r.header(*k, *v);
                }
                r
            }};
        }
        match method {
            "PUT" => with_headers!(self.agent.put(&url)).send(body),
            "POST" => with_headers!(self.agent.post(&url)).send(body),
            "DELETE" => with_headers!(self.agent.delete(&url)).call(),
            _ => with_headers!(self.agent.get(&url)).call(),
        }
    }

    fn cloud_path(key: &ServerKey, wallet: &str, slot: Option<&str>) -> Result<String, Refusal> {
        let w = Client::holder(wallet)?;
        if let Some(s) = slot {
            let b = s.as_bytes();
            let ok = !b.is_empty()
                && b.len() <= 48
                && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
                && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(c));
            if !ok {
                return Err(Refusal::here("a slot is 1 to 48 of a-z 0-9 _ . -"));
            }
            return Ok(format!("/cloud/{}/{w}/{s}", key.game()));
        }
        Ok(format!("/cloud/{}/{w}", key.game()))
    }

    /// Does the origin accept this key's signature?
    pub fn server_check(&self, key: &ServerKey) -> Look<Value> {
        self.read(self.signed(key, "POST", "/serverkeys/check", b"", &[]))
    }

    /// Store one save for a player. `if_match` writes only over that etag;
    /// `create_only` writes only if no save is there.
    pub fn cloud_put(
        &self,
        key: &ServerKey,
        wallet: &str,
        slot: &str,
        data: &[u8],
        if_match: Option<&str>,
        create_only: bool,
    ) -> Look<Value> {
        let path = match Client::cloud_path(key, wallet, Some(slot)) {
            Ok(p) => p,
            Err(r) => return Look::Refused(r),
        };
        let mut extra = vec![("Content-Type", "application/octet-stream")];
        if let Some(e) = if_match {
            extra.push(("If-Match", e));
        } else if create_only {
            extra.push(("If-None-Match", "*"));
        }
        self.read(self.signed(key, "PUT", &path, data, &extra))
    }

    /// One save's bytes, etag and version; refused 404 when there is none.
    pub fn cloud_get(&self, key: &ServerKey, wallet: &str, slot: &str) -> Look<CloudSave> {
        let path = match Client::cloud_path(key, wallet, Some(slot)) {
            Ok(p) => p,
            Err(r) => return Look::Refused(r),
        };
        match self.signed(key, "GET", &path, b"", &[]) {
            Ok(mut resp) if resp.status().as_u16() == 200 => {
                let header = |n: &str| resp.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
                let etag = header("etag").map(|e| e.trim_matches('"').to_string()).filter(|e| !e.is_empty());
                let version = header("x-elo-version").and_then(|v| v.parse().ok());
                match resp.body_mut().read_to_vec() {
                    Ok(data) => Look::Answer(CloudSave { data, etag, version }),
                    Err(e) => Look::CouldNotLook(format!("the save could not be read: {e}")),
                }
            }
            other => match self.read::<Value>(other) {
                Look::Refused(r) => Look::Refused(r),
                Look::CouldNotLook(w) => Look::CouldNotLook(w),
                Look::Answer(_) => Look::CouldNotLook("the save came back in a shape this SDK does not read".into()),
            },
        }
    }

    pub fn cloud_delete(&self, key: &ServerKey, wallet: &str, slot: &str, if_match: Option<&str>) -> Look<Value> {
        let path = match Client::cloud_path(key, wallet, Some(slot)) {
            Ok(p) => p,
            Err(r) => return Look::Refused(r),
        };
        let extra: Vec<(&str, &str)> = if_match.map(|e| vec![("If-Match", e)]).unwrap_or_default();
        self.read(self.signed(key, "DELETE", &path, b"", &extra))
    }

    /// A player's slots for this key's game.
    pub fn cloud_list(&self, key: &ServerKey, wallet: &str) -> Look<Value> {
        match Client::cloud_path(key, wallet, None) {
            Ok(p) => self.read(self.signed(key, "GET", &p, b"", &[])),
            Err(r) => Look::Refused(r),
        }
    }

    /// Report stats, unlocks and scores for players:
    /// `[{"wallet", "stats": {id: n}, "unlock": [ID], "scores": {board: n}}]`.
    pub fn stats_report(&self, key: &ServerKey, reports: &Value) -> Look<Value> {
        if !reports.as_array().is_some_and(|a| !a.is_empty()) {
            return Look::Refused(Refusal::here("reports is a non-empty list"));
        }
        let body = json!({ "reports": reports }).to_string();
        let path = format!("/gamestats/{}/report", key.game());
        self.read(self.signed(key, "POST", &path, body.as_bytes(), &[("Content-Type", "application/json")]))
    }

    // ── reads a game server makes at join ───────────────────────────────────

    /// Is this wallet in the playtest (an access code)?
    pub fn playtest_access(&self, slug: &str, wallet: &str) -> Look<bool> {
        let (s, w) = match (Client::slug(slug), Client::holder(wallet)) {
            (Ok(s), Ok(w)) => (s, w),
            (Err(r), _) | (_, Err(r)) => return Look::Refused(r),
        };
        match self.get::<Value>(&format!("/api/codes/{s}/access/{w}")) {
            Look::Answer(v) => Look::Answer(v.get("access") == Some(&Value::Bool(true))),
            Look::Refused(r) => Look::Refused(r),
            Look::CouldNotLook(why) => Look::CouldNotLook(why),
        }
    }

    /// The player's subscribed Workshop mods for a game, with file urls and sha256s.
    pub fn workshop_subscribed(&self, slug: &str, wallet: &str) -> Look<Value> {
        match (Client::slug(slug), Client::holder(wallet)) {
            (Ok(s), Ok(w)) => self.get(&format!("/api/workshop/{s}/subscribed/{w}")),
            (Err(r), _) | (_, Err(r)) => Look::Refused(r),
        }
    }

    /// One lobby with its members; a server admits the party from this.
    pub fn lobby(&self, id: &str) -> Look<Value> {
        let ok = (16..=32).contains(&id.len())
            && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
        if !ok {
            return Look::Refused(Refusal::here("not a lobby id"));
        }
        self.get(&format!("/api/lobbies/{id}"))
    }

    pub fn lobby_of(&self, wallet: &str) -> Look<Value> {
        match Client::holder(wallet) {
            Ok(w) => self.get(&format!("/api/lobbies/of/{w}")),
            Err(r) => Look::Refused(r),
        }
    }

    /// The first listed server, in the developer's order, that is up and has room for `party`.
    pub fn quick_join(&self, slug: &str, party: u32) -> Look<Value> {
        match Client::slug(slug) {
            Ok(s) if (1..=64).contains(&party) => self.get(&format!("/api/lobbies/pick/{s}?party={party}")),
            Ok(_) => Look::Refused(Refusal::here("a party is 1 to 64")),
            Err(r) => Look::Refused(r),
        }
    }
}

/// One cloud save as a game server reads it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSave {
    pub data: Vec<u8>,
    pub etag: Option<String>,
    pub version: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_sha256_are_the_standard_ones() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn query_values_are_encoded() {
        assert_eq!(enc("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }

    #[test]
    fn bad_input_is_refused_before_any_request() {
        // An origin nothing listens on: a request would be could-not-look.
        let c = Client::new("http://127.0.0.1:9");
        assert_eq!(c.friends("0x123").kind(), "refused");
        assert_eq!(c.guilds_of("0x123").kind(), "refused");
        assert_eq!(c.guild_tags(&[]).kind(), "refused");
        assert_eq!(
            c.guild_tags(&["0x1111111111111111111111111111111111111111", "nope"])
                .kind(),
            "refused"
        );
        assert_eq!(c.ticket("../etc").kind(), "refused");
        assert_eq!(c.leaderboard("gates", Some("robot")).kind(), "refused");
        let mut never = |_: &str| -> Result<String, String> { panic!("nothing should be signed") };
        let r = c.friend(
            "0x1111111111111111111111111111111111111111",
            "poke",
            "0x2222222222222222222222222222222222222222",
            &mut never,
        );
        assert!(matches!(r, Err(ActError::BadInput(_))));
    }
}
