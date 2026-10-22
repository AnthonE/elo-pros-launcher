//! The platform API over HTTPS (feature `http`).
//!
//! Every read answers a [`Look`]. Every write is prepare → rebuild the text
//! here → sign → act, and a server that prepares a different text from the
//! change asked for is refused before the signer is asked (the launcher's
//! own guard, `launcher-rs/crates/elo-launcher/src/friends.rs`).

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::look::{classify, Look, Refusal};
use crate::models::{
    Face, FacePrepared, FriendsActed, FriendsPrepared, FriendsView, Items, Leaderboard, Ticket,
    TicketOf, Who,
};
use crate::overlay::play_message;
use crate::texts;

/// The platform's origin.
pub const DEFAULT_ORIGIN: &str = "https://elopros.com";
/// How long one request may take before it is "could not look".
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

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

fn b64(data: &[u8]) -> String {
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

fn sha256_hex(data: &[u8]) -> String {
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

    /// Whether a wallet holds a copy, with its count and free mints left.
    pub fn ticket_of(&self, slug: &str, wallet: &str) -> Look<TicketOf> {
        match (Client::slug(slug), Client::wallet(wallet)) {
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

    /// One party by sworn name, npub, vow id or wallet. A handle nobody has
    /// sworn is a refusal (404); a bare wallet always answers.
    pub fn who(&self, handle: &str) -> Look<Who> {
        let h = handle.trim();
        if h.is_empty() {
            return Look::Refused(Refusal::here("name a handle to look up"));
        }
        self.get(&format!("/api/who?handle={}", enc(h)))
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
