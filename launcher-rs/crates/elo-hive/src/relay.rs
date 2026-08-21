//! Speaking to the relay — our own socket, authenticated as us.
//!
//! ## Why this exists at all
//!
//! `POST /api/hive/publish` looks like it should be enough, and it is not.
//! buzz binds an event's author to the **NIP-42 authenticated connection**
//! (`meter/hive.py` §hive_publish, measured 2026-07-25): a note forwarded on
//! the house's socket comes back *"event pubkey does not match authenticated
//! identity"* — and on the live relay it came back as **nothing at all**, no
//! ruling, the note simply absent. A silent refusal is the worst shape a
//! failure can take, so the client does what `hive.js` does: opens its own
//! socket, authenticates as itself, and publishes there.
//!
//! ## The flow, which is four messages
//!
//! ```text
//!   relay → ["AUTH", "<challenge>"]
//!   us    → ["AUTH", <kind:22242 event signed over relay + challenge>]
//!   relay → ["OK", "<id>", true, ""]
//!   us    → ["EVENT", <the note>]
//!   relay → ["OK", "<id>", true|false, "<reason>"]
//! ```
//!
//! ⚠ **Every step waits for a ruling and every ruling is reported.** The whole
//! reason this module exists is that the other path failed silently, so
//! "published" here means a relay said `true` and nothing else does.

use crate::error::{HiveError, Result};
use crate::voice::{Event, UnsignedEvent, Voice};
use serde_json::{json, Value};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
use tungstenite::{Message, WebSocket};

/// NIP-42's kind for a client authenticating to a relay.
pub const KIND_AUTH: i64 = 22242;
/// NIP-01 kind:1 with `t=elo` is the town stream; a room is kind:9 `#h`.
pub const KIND_NOTE: i64 = 1;
pub const KIND_ROOM_NOTE: i64 = 9;
/// Presence. Ephemeral, **WS-only** — the HTTP bridge rejects ephemeral kinds
/// outright, which is the one thing about the hive that genuinely needed a
/// socket rather than a POST.
pub const KIND_PRESENCE: i64 = 20001;

/// buzz's documented beat. The relay stores presence with a 90s TTL — three
/// times this — and `meter/hive_presence.py` is where those numbers come from.
pub const HEARTBEAT: Duration = Duration::from_secs(30);
pub const RELAY_TTL: Duration = Duration::from_secs(90);

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const IO_TIMEOUT: Duration = Duration::from_secs(20);
/// A relay that keeps talking without ever ruling is a relay we stop reading.
const MAX_FRAMES_PER_RULING: usize = 64;

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

pub struct Relay {
    socket: WebSocket<Tls>,
    url: String,
}

/// Split a `wss://host[:port]/path` into the three things a socket needs.
fn split_url(url: &str) -> Result<(String, u16, String)> {
    let rest = url
        .strip_prefix("wss://")
        .ok_or_else(|| HiveError::new(format!("a relay must be wss://, got {url:?}")))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.contains(']') => (
            h.to_string(),
            p.parse()
                .map_err(|_| HiveError::new(format!("bad port in {url:?}")))?,
        ),
        _ => (authority.to_string(), 443u16),
    };
    if host.is_empty() {
        return Err(HiveError::new(format!("no host in {url:?}")));
    }
    Ok((host, port, path.to_string()))
}

/// Extra roots named by the environment, on top of the bundled ones.
///
/// `SSL_CERT_FILE` is the de-facto variable curl, Python and OpenSSL all read,
/// and honouring it is not a workaround — a corporate or gateway proxy that
/// terminates TLS with its own CA is an ordinary deployment, and a client that
/// only trusts `webpki-roots` simply does not work on those networks. Found
/// exactly that way: the live-relay test failed `UnknownIssuer` behind a
/// proxy that every other tool on the box trusted.
///
/// ⚠ This only ever ADDS roots. It cannot remove or replace the bundled ones,
/// so a hostile `SSL_CERT_FILE` can make us trust an extra CA — which is the
/// same authority anyone able to set your environment already has — and can
/// never make us stop trusting a real one.
fn extra_roots(store: &mut rustls::RootCertStore) -> usize {
    let Some(path) = std::env::var_os("SSL_CERT_FILE") else {
        return 0;
    };
    let Ok(file) = std::fs::File::open(&path) else {
        return 0;
    };
    let mut reader = std::io::BufReader::new(file);
    let mut added = 0;
    for cert in rustls_pemfile::certs(&mut reader).flatten() {
        if store.add(cert).is_ok() {
            added += 1;
        }
    }
    added
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    extra_roots(&mut roots);
    // The provider is named explicitly rather than taken from the process
    // default: a process default is global mutable state, and a launcher that
    // silently changed which crypto backend verified a relay's certificate
    // depending on load order would be exactly the kind of thing nobody
    // notices until it matters.
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring supports the default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    Arc::new(config)
}

impl Relay {
    /// Connect, and authenticate as `voice` if the relay asks.
    pub fn connect(url: &str, voice: &Voice) -> Result<Relay> {
        let (host, port, _path) = split_url(url)?;
        let addr = format!("{host}:{port}");
        let tcp = std::net::ToSocketAddrs::to_socket_addrs(&addr)
            .map_err(|e| HiveError::new(format!("cannot resolve {addr}: {e}")))?
            .find_map(|a| TcpStream::connect_timeout(&a, CONNECT_TIMEOUT).ok())
            .ok_or_else(|| HiveError::new(format!("could not reach {addr}")))?;
        tcp.set_read_timeout(Some(IO_TIMEOUT))?;
        tcp.set_write_timeout(Some(IO_TIMEOUT))?;

        let server_name = rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|_| HiveError::new(format!("{host} is not a valid server name")))?;
        let conn = rustls::ClientConnection::new(tls_config(), server_name)
            .map_err(|e| HiveError::new(format!("tls setup failed: {e}")))?;
        let tls = rustls::StreamOwned::new(conn, tcp);

        let (socket, _response) = tungstenite::client(url, tls)
            .map_err(|e| HiveError::new(format!("websocket handshake failed: {e}")))?;
        let mut relay = Relay {
            socket,
            url: url.to_string(),
        };
        relay.authenticate(voice)?;
        Ok(relay)
    }

    /// Read frames until one of them is a JSON array we understand.
    fn next_message(&mut self) -> Result<Vec<Value>> {
        for _ in 0..MAX_FRAMES_PER_RULING {
            let msg = self
                .socket
                .read()
                .map_err(|e| HiveError::new(format!("relay went quiet: {e}")))?;
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                // tungstenite answers pings itself; a close is terminal.
                Message::Close(c) => {
                    return Err(HiveError::new(format!(
                        "relay closed the connection{}",
                        c.map(|f| format!(": {}", f.reason)).unwrap_or_default()
                    )))
                }
                _ => continue,
            };
            if let Ok(Value::Array(a)) = serde_json::from_str::<Value>(&text) {
                return Ok(a);
            }
        }
        Err(HiveError::new("relay never sent a ruling we could read"))
    }

    /// NIP-42. Waits for the relay's challenge, signs it, waits for the OK.
    ///
    /// A relay that never challenges is left alone — that is legal, and it
    /// simply means our events are not bound to this connection.
    fn authenticate(&mut self, voice: &Voice) -> Result<()> {
        let challenge = loop {
            let msg = self.next_message()?;
            match msg.first().and_then(|v| v.as_str()) {
                Some("AUTH") => {
                    break msg
                        .get(1)
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| HiveError::new("relay sent AUTH with no challenge"))?
                        .to_string()
                }
                // Anything else this early is noise we do not need.
                _ => continue,
            }
        };
        let event = voice.sign(UnsignedEvent {
            created_at: crate::clock::now(),
            kind: KIND_AUTH,
            tags: vec![
                vec!["relay".into(), self.url.clone()],
                vec!["challenge".into(), challenge],
            ],
            content: String::new(),
        })?;
        self.send(&json!(["AUTH", event.to_json()]))?;
        let (ok, reason) = self.await_ruling(&event.id)?;
        if !ok {
            return Err(HiveError::new(format!(
                "the relay refused this key: {reason}"
            )));
        }
        Ok(())
    }

    fn send(&mut self, value: &Value) -> Result<()> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(|e| HiveError::new(format!("could not write to the relay: {e}")))?;
        self.socket
            .flush()
            .map_err(|e| HiveError::new(format!("could not flush to the relay: {e}")))
    }

    /// Wait for `["OK", <id>, <bool>, <reason>]` about one event.
    fn await_ruling(&mut self, id: &str) -> Result<(bool, String)> {
        for _ in 0..MAX_FRAMES_PER_RULING {
            let msg = self.next_message()?;
            if msg.first().and_then(|v| v.as_str()) != Some("OK") {
                continue;
            }
            if msg.get(1).and_then(|v| v.as_str()) != Some(id) {
                // A ruling about someone else's event, or an earlier one.
                continue;
            }
            return Ok((
                msg.get(2).and_then(|v| v.as_bool()).unwrap_or(false),
                msg.get(3)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            ));
        }
        // The failure this whole module exists to prevent, named as itself.
        Err(HiveError::new(
            "the relay never ruled on the note — treating that as NOT published",
        ))
    }

    /// Publish one signed event and report what the relay said.
    pub fn publish(&mut self, event: &Event) -> Result<String> {
        // Verify our own output before it leaves. This catches a drift between
        // the id we compute and the bytes we send at the moment it happens,
        // rather than as a relay rejection nobody reads.
        event.verify()?;
        self.send(&json!(["EVENT", event.to_json()]))?;
        let (ok, reason) = self.await_ruling(&event.id)?;
        if !ok {
            return Err(HiveError::new(format!("the relay refused the note: {reason}")));
        }
        Ok(event.id.clone())
    }

    /// Send one presence beat.
    ///
    /// ⚠ **Presence is not a message you send. It is a socket you hold.**
    /// `meter/hive_presence.py` paid for that sentence: buzz derives online
    /// from a *live authenticated connection*, and on socket close it calls
    /// `clear_presence` immediately. A worker that connected, published and
    /// disconnected was offline by construction — a hand-published beat
    /// returned `["OK", …, true, ""]` and left no key at all, because the
    /// publisher's own clean disconnect cleared it on the way out.
    ///
    /// So this is `pub(crate)`-shaped in spirit: call it from [`Presence`],
    /// which holds the socket, and never as a one-shot.
    pub fn beat(&mut self, voice: &Voice, status: Status) -> Result<()> {
        let event = voice.sign(UnsignedEvent {
            created_at: crate::clock::now(),
            kind: KIND_PRESENCE,
            // A bare status string and no tags — buzz's shape, not ours.
            tags: Vec::new(),
            content: status.as_str().to_string(),
        })?;
        self.send(&json!(["EVENT", event.to_json()]))?;
        // Ephemeral kinds may or may not be ruled on. What matters is that the
        // socket stayed up, so a missing ruling is not a failure here — unlike
        // `publish`, where it is the whole point.
        Ok(())
    }

    pub fn close(mut self) {
        let _ = self.socket.close(None);
    }
}

/// Build the note a room expects: kind:9 with `#h` for a room, kind:1 with
/// `t=elo` for the town stream.
///
/// The shapes come from `GET /api/hive/channels`' own `post` field, and are
/// mirrored here rather than hardcoded blind — `meter/hive.py:room_post` is
/// the source, and read and write have to match or a note lands somewhere
/// nobody is reading.
pub fn note_for(room: Option<&str>, content: &str, at: i64) -> UnsignedEvent {
    match room.filter(|r| !r.is_empty()) {
        Some(room) => UnsignedEvent {
            created_at: at,
            kind: KIND_ROOM_NOTE,
            tags: vec![vec!["h".into(), room.to_string()]],
            content: content.to_string(),
        },
        None => UnsignedEvent {
            created_at: at,
            kind: KIND_NOTE,
            tags: vec![vec!["t".into(), "elo".into()]],
            content: content.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relay_url_splits_into_host_port_and_path() {
        assert_eq!(
            split_url("wss://hive.elopros.com").unwrap(),
            ("hive.elopros.com".into(), 443, "/".into())
        );
        assert_eq!(
            split_url("wss://example.com:7777/relay").unwrap(),
            ("example.com".into(), 7777, "/relay".into())
        );
    }

    #[test]
    fn a_plaintext_relay_is_refused() {
        // A relay carries a signed identity; downgrading it to ws:// is not a
        // convenience, it is handing the connection to whoever is in the way.
        for bad in ["ws://hive.example", "http://hive.example", "hive.example", ""] {
            assert!(split_url(bad).is_err(), "should refuse {bad:?}");
        }
    }

    #[test]
    fn the_note_shape_matches_what_the_room_reads() {
        let town = note_for(None, "hello", 100);
        assert_eq!(town.kind, KIND_NOTE);
        assert_eq!(town.tags, vec![vec!["t", "elo"]]);

        let room = note_for(Some("70e0e2b7-3d6d-4e44-8459-a433be1df670"), "hi", 100);
        assert_eq!(room.kind, KIND_ROOM_NOTE);
        assert_eq!(room.tags, vec![vec!["h", "70e0e2b7-3d6d-4e44-8459-a433be1df670"]]);

        // An empty room string is the town stream, not a room named "".
        assert_eq!(note_for(Some(""), "hi", 100).kind, KIND_NOTE);
    }
}


/// What a beat says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Online,
    Away,
    /// Sent on the way out. Mostly a courtesy — closing the socket is what
    /// actually clears presence, and the relay does it immediately.
    Offline,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Online => "online",
            Status::Away => "away",
            Status::Offline => "offline",
        }
    }
}

/// A held connection that keeps a voice online.
///
/// Owns the socket for its whole life and beats on a timer. Dropping it closes
/// the socket, which is what takes the voice offline — there is no "go
/// offline" call that works without that, because the relay reads the
/// connection and not the message.
pub struct Presence {
    relay: Relay,
    voice: Voice,
    status: Status,
    beats: u64,
}

impl Presence {
    /// Connect, authenticate, and send the first beat.
    pub fn hold(url: &str, voice: Voice, status: Status) -> Result<Presence> {
        let mut relay = Relay::connect(url, &voice)?;
        relay.beat(&voice, status)?;
        Ok(Presence { relay, voice, status, beats: 1 })
    }

    pub fn beats(&self) -> u64 {
        self.beats
    }

    pub fn npub(&self) -> String {
        self.voice.npub()
    }

    /// Beat again. Call at most every [`HEARTBEAT`]; the relay's TTL is three
    /// times that, so one missed beat is survivable and three are not.
    pub fn tick(&mut self) -> Result<()> {
        self.relay.beat(&self.voice, self.status)?;
        self.beats += 1;
        Ok(())
    }

    pub fn set_status(&mut self, status: Status) -> Result<()> {
        self.status = status;
        self.tick()
    }

    /// Say goodbye, then close. The close is what actually clears it.
    pub fn release(mut self) {
        let _ = self.relay.beat(&self.voice, Status::Offline);
        self.relay.close();
    }
}
