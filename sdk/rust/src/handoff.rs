//! Moving a connected player from one of your servers to another.
//!
//! The pass gets a player in; a handoff moves them (`docs/client/HANDOFF.md`).
//! The server a player is on signs a handoff with its server key and hands it
//! to the client with the address to go to; the client joins that server with
//! it; that server checks it offline against your game's public keys and
//! admits the player it names. No call to Elo Pros per hop, and no prompt.
//!
//! ```no_run
//! use elo_sdk::{Client, HandoffDoor, ServerKey};
//!
//! // the server they leave
//! let pem = std::fs::read_to_string("/etc/my-game/server.key").unwrap();
//! let key = ServerKey::from_pem("my-game", "zone-1", &pem).unwrap();
//! let player = "0x1111111111111111111111111111111111111111";
//! let token = key
//!     .handoff(player, "zone1.my-game.example:7000", "zone2.my-game.example:7000", None, 60)
//!     .unwrap();
//!
//! // the server they reach
//! let mut door = HandoffDoor::with_source("my-game", &["zone2.my-game.example:7000"],
//!     Box::new(Client::new("https://elopros.com")));
//! match door.admit_now(&token, b"") {
//!     Ok(got) => println!("welcome, {}", got.player),
//!     Err(r) => println!("{}: let them join the usual way", r.code),
//! }
//! ```
//!
//! `sdk/testdata/handoff.json` pins the text, the token and every refusal.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use ring::rand::SecureRandom;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::Value;

use crate::http::sha256_hex;
use crate::look::Look;

pub const TTL_DEFAULT_S: u64 = 60;
pub const TTL_MAX_S: u64 = 120;
pub const SKEW_S: u64 = 30;
pub const CARRY_MAX: usize = 4096;
const TOKEN_MAX: usize = 1024;
const KEYS_REFRESH_S: u64 = 300;
const KEYS_RETRY_S: u64 = 60;
const SEEN_MAX: usize = 200_000;
const ID_ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// What a handoff says: the player the last server admitted, and the trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub game: String,
    pub key: String,
    pub player: String,
    pub from: String,
    pub to: String,
    pub id: String,
    pub exp: u64,
    /// The sha256 hex of the bytes carried beside it, if any.
    pub carry: Option<String>,
}

/// Why a handoff was not admitted. `code` is one of `malformed`, `wrong_game`,
/// `wrong_destination`, `expired`, `too_long`, `unknown_key`, `bad_signature`,
/// `carry`, `used` or `no_keys`; `why` is for a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffRefused {
    pub code: &'static str,
    pub why: String,
}

impl std::fmt::Display for HandoffRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.why)
    }
}

impl std::error::Error for HandoffRefused {}

fn refused(code: &'static str, why: impl Into<String>) -> HandoffRefused {
    HandoffRefused {
        code,
        why: why.into(),
    }
}

/// The exact text a server signs for one handoff.
#[allow(clippy::too_many_arguments)]
pub fn handoff_text(
    game: &str,
    key_id: &str,
    player: &str,
    from: &str,
    to: &str,
    id: &str,
    exp: u64,
    carry: Option<&[u8]>,
) -> String {
    let c = match carry {
        Some(b) if !b.is_empty() => sha256_hex(b),
        _ => "none".to_string(),
    };
    format!("elo handoff\ngame: {game}\nkey: {key_id}\nplayer: {player}\nfrom: {from}\nto: {to}\nid: {id}\nexp: {exp}\ncarry: {c}")
}

// ── the shapes, by hand: the same rules as the regexes in every other SDK ──

fn lower_digit(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

fn slug_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=63).contains(&b.len())
        && lower_digit(b[0])
        && b.iter().all(|c| lower_digit(*c) || b"_-".contains(c))
}

fn kid_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=31).contains(&b.len())
        && lower_digit(b[0])
        && b.iter().all(|c| lower_digit(*c) || *c == b'-')
}

fn player_ok(s: &str) -> bool {
    let b = s.as_bytes();
    let evm = b.len() == 42
        && s.starts_with("0x")
        && b[2..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c));
    let acct = b.len() == 25
        && s.starts_with("acct_")
        && b[5..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c));
    const B58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let sol = (32..=44).contains(&b.len()) && b.iter().all(|c| B58.contains(c));
    evm || acct || sol
}

fn addr_ok(s: &str) -> bool {
    let Some((host, port)) = s.rsplit_once(':') else {
        return false;
    };
    let port_ok = (1..=5).contains(&port.len()) && port.bytes().all(|c| c.is_ascii_digit());
    let hb = host.as_bytes();
    let name =
        (1..=100).contains(&hb.len()) && hb.iter().all(|c| lower_digit(*c) || b".-".contains(c));
    let v6 = hb.len() >= 4
        && hb.len() <= 41
        && hb[0] == b'['
        && hb[hb.len() - 1] == b']'
        && hb[1..hb.len() - 1]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c) || *c == b':');
    port_ok && (name || v6)
}

fn id_ok(s: &str) -> bool {
    (16..=64).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

fn b64u(data: &[u8]) -> String {
    crate::http::b64(data)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

fn unb64u(s: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut n = 0u32;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        bits = (bits << 6 | u32::from(v)) & 0xff_ffff;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    Some(out)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    unb64u(&s.trim_end_matches('=').replace('+', "-").replace('/', "_"))
}

fn parts(token: &str) -> Result<(Handoff, Vec<u8>, Vec<u8>), HandoffRefused> {
    let shape = || {
        refused(
            "malformed",
            "a handoff is base64url(text).base64url(signature)",
        )
    };
    if token.len() > TOKEN_MAX || token.matches('.').count() != 1 {
        return Err(shape());
    }
    let (a, b) = token.split_once('.').ok_or_else(shape)?;
    let raw = unb64u(a).ok_or_else(|| refused("malformed", "a half is not base64url"))?;
    let sig = unb64u(b).ok_or_else(|| refused("malformed", "a half is not base64url"))?;
    let lines_bad = || refused("malformed", "the text is not the nine handoff lines");
    let text = std::str::from_utf8(&raw).map_err(|_| lines_bad())?;
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() != 9 || lines[0] != "elo handoff" || sig.len() != 64 {
        return Err(lines_bad());
    }
    let field = |i: usize, name: &str| lines[i].strip_prefix(name).filter(|v| !v.is_empty());
    let (
        Some(game),
        Some(key),
        Some(player),
        Some(from),
        Some(to),
        Some(id),
        Some(exp),
        Some(carry),
    ) = (
        field(1, "game: "),
        field(2, "key: "),
        field(3, "player: "),
        field(4, "from: "),
        field(5, "to: "),
        field(6, "id: "),
        field(7, "exp: "),
        field(8, "carry: "),
    )
    else {
        return Err(lines_bad());
    };
    let exp_ok = (1..=12).contains(&exp.len()) && exp.bytes().all(|c| c.is_ascii_digit());
    let carry_ok = carry == "none"
        || (carry.len() == 64
            && carry
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
    if !(slug_ok(game)
        && kid_ok(key)
        && player_ok(player)
        && addr_ok(from)
        && addr_ok(to)
        && id_ok(id)
        && exp_ok
        && carry_ok)
    {
        return Err(lines_bad());
    }
    let h = Handoff {
        game: game.into(),
        key: key.into(),
        player: player.into(),
        from: from.into(),
        to: to.into(),
        id: id.into(),
        exp: exp.parse().map_err(|_| lines_bad())?,
        carry: (carry != "none").then(|| carry.to_string()),
    };
    Ok((h, raw, sig))
}

/// What a handoff token says, shape only (nothing verified).
pub fn read_handoff(token: &str) -> Result<Handoff, HandoffRefused> {
    parts(token).map(|(h, _, _)| h)
}

/// Every check but `used`, in the posted order. `keys` maps key id to the
/// 32-byte public key; a server answering to several addresses names them all.
pub fn verify_handoff(
    token: &str,
    game: &str,
    names: &[&str],
    keys: &HashMap<String, Vec<u8>>,
    now: u64,
    carry: &[u8],
) -> Result<Handoff, HandoffRefused> {
    let (h, raw, sig) = parts(token)?;
    if h.game != game {
        return Err(refused(
            "wrong_game",
            format!("this handoff is for {}", h.game),
        ));
    }
    if !names.iter().any(|n| n.to_ascii_lowercase() == h.to) {
        return Err(refused(
            "wrong_destination",
            format!("this handoff is for {}", h.to),
        ));
    }
    if h.exp + SKEW_S <= now {
        return Err(refused("expired", "this handoff has run out"));
    }
    if h.exp > now + TTL_MAX_S + SKEW_S {
        return Err(refused(
            "too_long",
            format!("a handoff lives at most {TTL_MAX_S} seconds"),
        ));
    }
    let pub_key = keys
        .get(&h.key)
        .ok_or_else(|| refused("unknown_key", format!("{game} has no server key {}", h.key)))?;
    if UnparsedPublicKey::new(&ED25519, pub_key)
        .verify(&raw, &sig)
        .is_err()
    {
        return Err(refused("bad_signature", "the signature is not that key's"));
    }
    if carry.len() > CARRY_MAX {
        return Err(refused(
            "carry",
            format!("a handoff carries at most {CARRY_MAX} bytes"),
        ));
    }
    let got = (!carry.is_empty()).then(|| sha256_hex(carry));
    if got != h.carry {
        return Err(refused(
            "carry",
            "the carried bytes are not the ones this handoff names",
        ));
    }
    Ok(h)
}

/// A fresh handoff id: `n` (16 to 64) of letters, digits, `-` and `_`.
pub fn new_handoff_id(n: usize) -> Option<String> {
    if !(16..=64).contains(&n) {
        return None;
    }
    let mut buf = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut buf).ok()?;
    Some(
        buf.iter()
            .map(|b| ID_ABC[(*b & 63) as usize] as char)
            .collect(),
    )
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl crate::server::ServerKey {
    /// A handoff moving `player` from this server (`from`, host:port) to
    /// another of this game's servers (`to`), alive `ttl` seconds (1 to 120).
    /// Give the client the token, the address and `carry` if any.
    pub fn handoff(
        &self,
        player: &str,
        from: &str,
        to: &str,
        carry: Option<&[u8]>,
        ttl: u64,
    ) -> Result<String, String> {
        self.handoff_at(player, from, to, carry, ttl, unix_now(), None)
    }

    /// [`Self::handoff`] at a given time and id: for tests and replays.
    #[allow(clippy::too_many_arguments)]
    pub fn handoff_at(
        &self,
        player: &str,
        from: &str,
        to: &str,
        carry: Option<&[u8]>,
        ttl: u64,
        now: u64,
        id: Option<&str>,
    ) -> Result<String, String> {
        if !player_ok(player) {
            return Err(
                "player is a 0x wallet (lowercase), a Solana address or an acct_ id".into(),
            );
        }
        if !addr_ok(from) || !addr_ok(to) {
            return Err("a server address is host:port, lowercase".into());
        }
        if !(1..=TTL_MAX_S).contains(&ttl) {
            return Err(format!("ttl is 1 to {TTL_MAX_S} seconds"));
        }
        if carry.is_some_and(|c| c.len() > CARRY_MAX) {
            return Err(format!("a handoff carries at most {CARRY_MAX} bytes"));
        }
        let id = match id {
            Some(i) => i.to_string(),
            None => new_handoff_id(22).ok_or("no randomness for an id")?,
        };
        if !id_ok(&id) {
            return Err("id is 16 to 64 of A-Z a-z 0-9 - _".into());
        }
        let text = handoff_text(
            self.game(),
            self.key_id(),
            player,
            from,
            to,
            &id,
            now + ttl,
            carry,
        );
        Ok(format!(
            "{}.{}",
            b64u(text.as_bytes()),
            b64u(&self.sign_bytes(text.as_bytes()))
        ))
    }
}

/// Where a door remembers the ids it admitted. Several processes behind one
/// address share one store; implement this over it.
pub trait Seen: Send {
    /// True, and remembered until `until`, if `id` was not seen; else false.
    fn first(&mut self, id: &str, until: u64, now: u64) -> bool;
}

/// The ids one process admitted, until each runs out.
#[derive(Default)]
pub struct MemorySeen {
    until: HashMap<String, u64>,
}

impl Seen for MemorySeen {
    fn first(&mut self, id: &str, until: u64, now: u64) -> bool {
        if self.until.len() >= SEEN_MAX {
            self.until.retain(|_, u| *u > now);
            while self.until.len() >= SEEN_MAX {
                let soonest = self
                    .until
                    .iter()
                    .min_by_key(|(_, u)| **u)
                    .map(|(k, _)| k.clone());
                match soonest {
                    Some(k) => self.until.remove(&k),
                    None => break,
                };
            }
        }
        if self.until.get(id).is_some_and(|u| *u > now) {
            return false;
        }
        self.until.insert(id.to_string(), until);
        true
    }
}

/// Where a door reads the game's public keys: `{keys: [{id, pub}]}`, as
/// `GET /api/store/serverkeys/{game}` answers. [`crate::Client`] is one.
pub trait KeySource: Send {
    fn server_keys(&self, game: &str) -> Look<Value>;
}

impl KeySource for crate::http::Client {
    fn server_keys(&self, game: &str) -> Look<Value> {
        crate::http::Client::server_keys(self, game)
    }
}

/// The receiving side: admits each good handoff once.
///
/// Pinned keys, or a [`KeySource`]: then the door reads the set, again every
/// 300 s so a revoked key stops working, and at once (at most every 60 s)
/// for a key it has not seen, so a rotated key works on its first use.
pub struct HandoffDoor {
    game: String,
    names: Vec<String>,
    keys: HashMap<String, Vec<u8>>,
    source: Option<Box<dyn KeySource>>,
    seen: Box<dyn Seen>,
    read_at: Option<u64>,
    tried_at: Option<u64>,
}

impl HandoffDoor {
    /// A door with the game's key set pinned: `(key id, base64 public key)`.
    pub fn pinned(
        game: &str,
        names: &[&str],
        keys: &[(&str, &str)],
    ) -> Result<HandoffDoor, String> {
        let mut map = HashMap::new();
        for (id, pub_b64) in keys {
            let pk = unb64(pub_b64)
                .filter(|p| p.len() == 32)
                .ok_or_else(|| format!("key {id} is not 32 bytes of base64"))?;
            map.insert((*id).to_string(), pk);
        }
        Ok(HandoffDoor {
            game: game.into(),
            names: names.iter().map(|n| n.to_ascii_lowercase()).collect(),
            keys: map,
            source: None,
            seen: Box::new(MemorySeen::default()),
            read_at: None,
            tried_at: None,
        })
    }

    /// A door that reads the game's key set from `source` (a [`crate::Client`]).
    pub fn with_source(game: &str, names: &[&str], source: Box<dyn KeySource>) -> HandoffDoor {
        HandoffDoor {
            game: game.into(),
            names: names.iter().map(|n| n.to_ascii_lowercase()).collect(),
            keys: HashMap::new(),
            source: Some(source),
            seen: Box::new(MemorySeen::default()),
            read_at: None,
            tried_at: None,
        }
    }

    /// Remember admitted ids somewhere shared instead of in this process.
    pub fn with_seen(mut self, seen: Box<dyn Seen>) -> HandoffDoor {
        self.seen = seen;
        self
    }

    fn refresh(&mut self, now: u64) -> bool {
        self.tried_at = Some(now);
        let Some(src) = &self.source else {
            return false;
        };
        let Look::Answer(v) = src.server_keys(&self.game) else {
            return false;
        };
        let mut fresh = HashMap::new();
        for r in v["keys"].as_array().into_iter().flatten() {
            if let (Some(id), Some(pk)) = (r["id"].as_str(), r["pub"].as_str().and_then(unb64)) {
                if pk.len() == 32 {
                    fresh.insert(id.to_string(), pk);
                }
            }
        }
        self.keys = fresh;
        self.read_at = Some(now);
        true
    }

    /// Admit a handoff once, at `now` (unix seconds).
    pub fn admit(
        &mut self,
        token: &str,
        carry: &[u8],
        now: u64,
    ) -> Result<Handoff, HandoffRefused> {
        if self.source.is_some() {
            let h = read_handoff(token)?;
            let stale = self
                .read_at
                .is_none_or(|t| now.saturating_sub(t) >= KEYS_REFRESH_S);
            let unknown = !self.keys.contains_key(&h.key)
                && self
                    .tried_at
                    .is_none_or(|t| now.saturating_sub(t) >= KEYS_RETRY_S);
            if (stale || unknown) && !self.refresh(now) && self.keys.is_empty() {
                return Err(refused(
                    "no_keys",
                    "could not read the game's server keys; let the player join the usual way",
                ));
            }
        }
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        let got = verify_handoff(token, &self.game, &names, &self.keys, now, carry)?;
        if !self.seen.first(&got.id, got.exp + SKEW_S, now) {
            return Err(refused("used", "this handoff was already admitted"));
        }
        Ok(got)
    }

    /// [`Self::admit`] at the system clock.
    pub fn admit_now(&mut self, token: &str, carry: &[u8]) -> Result<Handoff, HandoffRefused> {
        self.admit(token, carry, unix_now())
    }
}
