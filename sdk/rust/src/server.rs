//! A game server's key, and the text it signs.
//!
//! Your servers sign their writes with an Ed25519 key you named on the dev
//! desk (act `serverkeys`) or in `elo.json` (`server_keys`). We keep the
//! public half only; the private key stays on your server. Steam's Game
//! Server Login Token, without a token anyone could leak from us.
//!
//! ```no_run
//! use elo_sdk::{Client, ServerKey};
//!
//! let pem = std::fs::read_to_string("/etc/my-game/server.key").unwrap();
//! let key = ServerKey::from_pem("my-game", "eu-1", &pem).unwrap();
//! println!("name this with us: {}", key.public_b64());
//! let elo = Client::new("https://elopros.com");
//! let _ = elo.cloud_put(&key, "0x1111111111111111111111111111111111111111", "save", b"...", None, false);
//! ```
//!
//! The text, which every SDK and the meter build the same way
//! (`sdk/testdata/server_texts.json` pins it): `elo server`, the method, the
//! path without `/api` (with `?query`), `<slug>/<key id>`, unix seconds, the
//! nonce, and the body's sha256 hex — one per line, no trailing newline.

use std::time::{SystemTime, UNIX_EPOCH};

use ring::rand::SecureRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};

use crate::http::{b64, sha256_hex};

/// PKCS#8 v1 for an Ed25519 key is this prefix and the 32-byte seed.
const PKCS8_ED25519: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
const NONCE_ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn unb64(s: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut n = 0u32;
    let mut out = Vec::new();
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        bits = bits << 6 | v as u32;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    Some(out)
}

/// The exact text a game server signs for one request.
pub fn server_text(method: &str, path: &str, who: &str, at: &str, body: &[u8], nonce: Option<&str>) -> String {
    let tail = nonce.map(|n| format!("{n}\n")).unwrap_or_default();
    format!(
        "elo server\n{}\n{path}\n{who}\n{at}\n{tail}{}",
        method.to_ascii_uppercase(),
        sha256_hex(body)
    )
}

/// A fresh request nonce: `n` (8 to 64) of letters, digits, `-` and `_`.
pub fn new_server_nonce(n: usize) -> Option<String> {
    if !(8..=64).contains(&n) {
        return None;
    }
    let mut buf = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut buf).ok()?;
    Some(buf.iter().map(|b| NONCE_ABC[(*b & 63) as usize] as char).collect())
}

fn id_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 31
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

fn slug_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// One of a game's server keys.
pub struct ServerKey {
    game: String,
    key_id: String,
    pair: Ed25519KeyPair,
}

impl ServerKey {
    /// From the 32-byte private seed.
    pub fn from_seed(game: &str, key_id: &str, seed: &[u8]) -> Result<ServerKey, String> {
        if !slug_ok(game) {
            return Err("game is the title's slug".into());
        }
        if !id_ok(key_id) {
            return Err("key_id is lowercase letters, digits and '-', up to 31 characters".into());
        }
        let pair = Ed25519KeyPair::from_seed_unchecked(seed)
            .map_err(|_| "an Ed25519 private key is a 32-byte seed".to_string())?;
        Ok(ServerKey { game: game.into(), key_id: key_id.into(), pair })
    }

    /// From a PKCS#8 PEM, the file `openssl genpkey -algorithm ed25519` writes.
    pub fn from_pem(game: &str, key_id: &str, pem: &str) -> Result<ServerKey, String> {
        let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = unb64(&body).ok_or("that PEM is not base64")?;
        if der.len() != 48 || der[..16] != PKCS8_ED25519 {
            return Err("that PEM is not an Ed25519 private key".into());
        }
        ServerKey::from_seed(game, key_id, &der[16..])
    }

    pub fn game(&self) -> &str {
        &self.game
    }

    /// `<slug>/<key id>`, as the request names its signer.
    pub fn who(&self) -> String {
        format!("{}/{}", self.game, self.key_id)
    }

    /// The public key to name on the desk or in `elo.json`.
    pub fn public_b64(&self) -> String {
        b64(self.pair.public_key().as_ref())
    }

    pub fn sign_text(&self, text: &str) -> String {
        b64(self.pair.sign(text.as_bytes()).as_ref())
    }

    /// The four headers for one request; `path` is the route without `/api`.
    pub fn headers(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        at: Option<u64>,
        nonce: Option<&str>,
    ) -> Vec<(&'static str, String)> {
        let at = at.unwrap_or_else(|| {
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
        });
        let nonce = nonce.map(str::to_string).or_else(|| new_server_nonce(22)).unwrap_or_default();
        let at_s = at.to_string();
        let text = server_text(
            method,
            path,
            &self.who(),
            &at_s,
            body,
            (!nonce.is_empty()).then_some(nonce.as_str()),
        );
        vec![
            ("X-Elo-Server", self.who()),
            ("X-Elo-Time", at_s),
            ("X-Elo-Nonce", nonce),
            ("X-Elo-Signature", self.sign_text(&text)),
        ]
    }
}
