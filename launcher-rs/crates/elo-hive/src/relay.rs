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
use std::collections::VecDeque;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};
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
/// Frames that arrived while we waited for something else — a room's live
/// messages landing between our note and the relay's ruling on it. Kept so the
/// feed does not lose them; bounded so a relay cannot grow us without limit.
const PENDING_CAP: usize = 512;
/// The same stash, bounded by what it holds: a frame count alone lets a relay
/// park 512 fat frames on us.
const PENDING_BYTES: usize = 2 << 20;
/// The largest message we read off a relay. A note is at most 8000 chars
/// (`chat::MAX_NOTE`, 32 KB of UTF-8); this is far above that and far below
/// tungstenite's default 64 MiB, which a hostile relay could fill.
const MAX_MESSAGE: usize = 256 << 10;
/// The most stored events one REQ may hand back before EOSE. A room's filter
/// asks for sixty; this is the ceiling on a relay that ignores `limit`.
const BACKLOG_CAP: usize = 1000;
/// The most of a relay's own words (a refusal reason, a close reason) one
/// error carries. A sentence, not a payload.
const MAX_REASON: usize = 200;

/// A relay's free text, made fit to sit inside an error.
///
/// ⚠ Every error here ends up on a terminal (`elo: {e}`) or in a status line,
/// and the text is the relay's to choose: an OSC 52 in a refusal reason writes
/// the player's clipboard, `ESC[2J` repaints their screen. Chat lines already
/// go through `render::safe_line`; this is the same door for the words a relay
/// puts in a ruling, a CLOSED or a close frame.
fn relay_said(raw: &str) -> String {
    crate::render::clip(&crate::render::safe_line(raw), MAX_REASON)
}

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

pub struct Relay {
    socket: WebSocket<Tls>,
    url: String,
    /// Frames read while waiting for a ruling, handed to [`Relay::poll`] next,
    /// each with its size on the wire.
    pending: VecDeque<(Vec<Value>, usize)>,
    pending_bytes: usize,
    /// The size of the last frame [`Relay::read_frame`] read.
    last_len: usize,
    /// How long one [`Relay::poll`] may take ([`Relay::set_wait`]).
    wait: Duration,
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

/// The relay's TLS settings, shared with `elo stage`'s Twitch chat reader so
/// the client has one trust policy (bundled roots, `SSL_CERT_FILE` added,
/// distrusted CAs dropped) rather than two.
pub fn tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = elo_net::tls::root_store();
    extra_roots(&mut roots);
    // Again after SSL_CERT_FILE: a system bundle carries the same roots, and
    // adding them back would undo `elo_net::tls::DISTRUSTED`.
    roots.roots.retain(|a| !elo_net::tls::distrusted(&a.subject));
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

        let limits = tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let (socket, _response) = tungstenite::client::client_with_config(url, tls, Some(limits))
            .map_err(|e| HiveError::new(format!("websocket handshake failed: {e}")))?;
        let mut relay = Relay {
            socket,
            url: url.to_string(),
            pending: VecDeque::new(),
            pending_bytes: 0,
            last_len: 0,
            wait: IO_TIMEOUT,
        };
        relay.authenticate(voice)?;
        Ok(relay)
    }

    /// One frame off the socket. `None` means the read timeout passed with
    /// nothing to read — not an error, just a quiet relay — and an empty array
    /// is a frame that was not a JSON array (a ping, a stray string).
    fn read_frame(&mut self) -> Result<Option<Vec<Value>>> {
        let msg = match self.socket.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
            {
                return Ok(None)
            }
            Err(e) => return Err(HiveError::new(format!("relay went quiet: {e}"))),
        };
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
            // tungstenite answers pings itself; a close is terminal.
            Message::Close(c) => {
                return Err(HiveError::new(format!(
                    "relay closed the connection{}",
                    c.map(|f| format!(": {}", relay_said(&f.reason))).unwrap_or_default()
                )))
            }
            _ => return Ok(Some(Vec::new())),
        };
        self.last_len = text.len();
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(a)) => Ok(Some(a)),
            _ => Ok(Some(Vec::new())),
        }
    }

    /// Read frames until one of them is a JSON array we understand.
    ///
    /// The deadline is the caller's, for a wait made of many messages: a relay
    /// that keeps talking would restart a per-message deadline forever, and a
    /// live feed polls on a short socket timeout, so a ruling that takes a
    /// second must not read as a relay that went away.
    fn next_message_before(&mut self, deadline: Instant) -> Result<Vec<Value>> {
        let mut frames = 0;
        loop {
            match self.read_frame()? {
                Some(a) if !a.is_empty() => return Ok(a),
                Some(_) => {
                    frames += 1;
                    if frames >= MAX_FRAMES_PER_RULING {
                        return Err(HiveError::new("relay never sent a ruling we could read"));
                    }
                }
                None if Instant::now() >= deadline => {
                    return Err(HiveError::new("relay went quiet"))
                }
                None => {}
            }
        }
    }

    /// Keep a frame for [`Relay::poll`] rather than drop it on the floor.
    /// Always the frame just read, so [`Relay::last_len`] is its size.
    fn stash(&mut self, frame: Vec<Value>) {
        let len = self.last_len;
        if self.pending.len() < PENDING_CAP && self.pending_bytes + len <= PENDING_BYTES {
            self.pending_bytes += len;
            self.pending.push_back((frame, len));
        }
    }

    fn unstash(&mut self) -> Option<Vec<Value>> {
        let (frame, len) = self.pending.pop_front()?;
        self.pending_bytes -= len;
        Some(frame)
    }

    /// NIP-42. Waits for the relay's challenge, signs it, waits for the OK.
    ///
    /// A relay that never challenges is an error after [`IO_TIMEOUT`]: buzz
    /// derives presence from an authenticated socket, so an unbound one is
    /// not a connection we can use.
    ///
    /// The challenge gets one deadline and a frame budget for the whole wait:
    /// a relay that talks without challenging does not hold us forever.
    fn authenticate(&mut self, voice: &Voice) -> Result<()> {
        let deadline = Instant::now() + IO_TIMEOUT;
        let mut frames = 0;
        let challenge = loop {
            let msg = self.next_message_before(deadline)?;
            frames += 1;
            if frames > MAX_FRAMES_PER_RULING || Instant::now() >= deadline {
                return Err(HiveError::new("relay never sent the AUTH challenge"));
            }
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
                "the relay refused this key: {}",
                relay_said(&reason)
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
    ///
    /// Anything else that arrives meanwhile is stashed for [`Relay::poll`]: on
    /// a socket that also holds a room open, the room's messages keep coming
    /// while we wait, and dropping them would punch holes in the feed.
    fn await_ruling(&mut self, id: &str) -> Result<(bool, String)> {
        let deadline = Instant::now() + IO_TIMEOUT;
        for _ in 0..MAX_FRAMES_PER_RULING + PENDING_CAP {
            if Instant::now() >= deadline {
                break;
            }
            let msg = self.next_message_before(deadline)?;
            if msg.first().and_then(|v| v.as_str()) != Some("OK") {
                self.stash(msg);
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
            return Err(HiveError::new(format!("the relay refused the note: {}", relay_said(&reason))));
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

    /// Open a subscription and read its stored events up to `EOSE`.
    ///
    /// **Every event is checked before it is kept** ([`checked`]): the id is
    /// recomputed and the signature verified, so a relay — ours or anyone's —
    /// cannot put words in a key's mouth. What fails is counted, not shown.
    ///
    /// A relay that sends more than the filter's `limit` is read to `EOSE`
    /// and the excess is not kept.
    pub fn subscribe(&mut self, sub: &str, filter: &Value) -> Result<Backlog> {
        self.send(&json!(["REQ", sub, filter]))?;
        let keep = filter
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(BACKLOG_CAP, |n| (n as usize).min(BACKLOG_CAP));
        let mut backlog = Backlog::default();
        // One deadline for the whole history, as for a ruling: a relay that
        // talks without sending EOSE does not hold the room's thread.
        let deadline = Instant::now() + IO_TIMEOUT;
        for _ in 0..BACKLOG_CAP + MAX_FRAMES_PER_RULING {
            if Instant::now() >= deadline {
                break;
            }
            let msg = self.next_message_before(deadline)?;
            match classify(&msg) {
                Frame::Event { sub: s, .. } if s == sub && backlog.events.len() >= keep => {}
                Frame::Event { sub: s, event } if s == sub => match checked(&event) {
                    Ok(e) => backlog.events.push(e),
                    Err(_) => backlog.dropped += 1,
                },
                Frame::Eose(s) if s == sub => return Ok(backlog),
                Frame::Closed { sub: s, why } if s == sub => {
                    return Err(HiveError::new(if why.is_empty() {
                        "the relay closed the room".to_string()
                    } else {
                        format!("the relay closed the room: {}", relay_said(&why))
                    }))
                }
                _ => self.stash(msg),
            }
        }
        Err(HiveError::new("the relay never finished sending the room's history"))
    }

    /// The next live event on `sub`, or `None` when nothing arrived within the
    /// socket's read timeout ([`Relay::set_wait`]).
    ///
    /// Stashed frames are drained first, so a message that landed while we
    /// were waiting on a ruling still reaches the feed, in order.
    ///
    /// One call takes about the wait, however much else the relay says: past
    /// it, or past [`MAX_FRAMES_PER_RULING`] frames that are not this sub's,
    /// it returns `None` so the thread holding it can hear the window.
    pub fn poll(&mut self, sub: &str) -> Result<Option<Polled>> {
        let started = Instant::now();
        let mut read = 0;
        loop {
            let frame = match self.unstash() {
                Some(f) => f,
                None => {
                    if read >= MAX_FRAMES_PER_RULING || started.elapsed() >= self.wait {
                        return Ok(None);
                    }
                    read += 1;
                    match self.read_frame()? {
                        Some(f) => f,
                        None => return Ok(None),
                    }
                }
            };
            match classify(&frame) {
                Frame::Event { sub: s, event } if s == sub => {
                    return Ok(Some(match checked(&event) {
                        Ok(e) => Polled::Event(e),
                        Err(_) => Polled::Dropped,
                    }))
                }
                Frame::Closed { sub: s, why } if s == sub => {
                    return Err(HiveError::new(format!("the relay closed the room: {}", relay_said(&why))))
                }
                // Rulings on beats, notices, other subscriptions: not the feed.
                _ => continue,
            }
        }
    }

    /// Stop a subscription. The relay may still deliver a frame or two for it;
    /// [`Relay::poll`] ignores anything not addressed to the sub it was asked.
    pub fn unsubscribe(&mut self, sub: &str) -> Result<()> {
        self.send(&json!(["CLOSE", sub]))
    }

    /// How long one [`Relay::poll`] waits on a quiet socket. A live feed wants
    /// this short so the thread holding it can also hear the window.
    pub fn set_wait(&mut self, wait: Duration) -> Result<()> {
        self.socket.get_mut().sock.set_read_timeout(Some(wait))?;
        self.wait = wait;
        Ok(())
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn close(mut self) {
        let _ = self.socket.close(None);
    }
}

/// A room's stored history, as one REQ returned it.
#[derive(Debug, Default)]
pub struct Backlog {
    /// Events whose id and signature checked out.
    pub events: Vec<Event>,
    /// Events that did not. Counted so the window can say so, never shown.
    pub dropped: usize,
}

/// One thing a live subscription produced.
#[derive(Debug)]
pub enum Polled {
    Event(Event),
    /// An event whose signature did not check out — never shown.
    Dropped,
}

/// What one relay frame is, by its NIP-01 label.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Event { sub: String, event: Value },
    Eose(String),
    Closed { sub: String, why: String },
    Ok { id: String, ok: bool, why: String },
    Auth(String),
    Notice(String),
    Other,
}

/// Name a frame. A frame missing the fields its label needs is `Other` —
/// a relay that sends half a message gets no benefit of the doubt.
pub fn classify(frame: &[Value]) -> Frame {
    let s = |i: usize| frame.get(i).and_then(|v| v.as_str()).map(str::to_string);
    match frame.first().and_then(|v| v.as_str()) {
        Some("EVENT") => match (s(1), frame.get(2)) {
            (Some(sub), Some(event)) if event.is_object() => {
                Frame::Event { sub, event: event.clone() }
            }
            _ => Frame::Other,
        },
        Some("EOSE") => s(1).map(Frame::Eose).unwrap_or(Frame::Other),
        Some("CLOSED") => match s(1) {
            Some(sub) => Frame::Closed { sub, why: s(2).unwrap_or_default() },
            None => Frame::Other,
        },
        Some("OK") => match (s(1), frame.get(2).and_then(|v| v.as_bool())) {
            (Some(id), Some(ok)) => Frame::Ok { id, ok, why: s(3).unwrap_or_default() },
            _ => Frame::Other,
        },
        Some("AUTH") => s(1).map(Frame::Auth).unwrap_or(Frame::Other),
        Some("NOTICE") => s(1).map(Frame::Notice).unwrap_or(Frame::Other),
        _ => Frame::Other,
    }
}

/// Parse one event off the wire and prove it: the id is recomputed from its
/// own contents and the signature checked against its own pubkey. This is what
/// makes a relay a carrier rather than an authority — it can drop a message,
/// and it cannot forge one.
pub fn checked(raw: &Value) -> Result<Event> {
    let event: Event = serde_json::from_value(raw.clone())
        .map_err(|e| HiveError::new(format!("not an event: {e}")))?;
    event.verify()?;
    Ok(event)
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

    fn frame(v: Value) -> Vec<Value> {
        v.as_array().unwrap().clone()
    }

    #[test]
    fn every_frame_a_room_read_needs_is_named() {
        assert_eq!(
            classify(&frame(json!(["EVENT", "elo-1", {"id": "x"}]))),
            Frame::Event { sub: "elo-1".into(), event: json!({"id": "x"}) }
        );
        assert_eq!(classify(&frame(json!(["EOSE", "elo-1"]))), Frame::Eose("elo-1".into()));
        assert_eq!(
            classify(&frame(json!(["CLOSED", "elo-1", "restricted: members only"]))),
            Frame::Closed { sub: "elo-1".into(), why: "restricted: members only".into() }
        );
        assert_eq!(
            classify(&frame(json!(["OK", "abc", false, "blocked"]))),
            Frame::Ok { id: "abc".into(), ok: false, why: "blocked".into() }
        );
        assert_eq!(classify(&frame(json!(["AUTH", "chal"]))), Frame::Auth("chal".into()));
        assert_eq!(classify(&frame(json!(["NOTICE", "hi"]))), Frame::Notice("hi".into()));
    }

    #[test]
    fn half_a_frame_is_not_given_the_benefit_of_the_doubt() {
        for bad in [
            json!(["EVENT", "elo-1"]),
            json!(["EVENT", "elo-1", "not an object"]),
            json!(["EVENT", 7, {"id": "x"}]),
            json!(["OK", "abc"]),
            json!(["OK", "abc", "true"]),
            json!(["EOSE"]),
            json!(["WHAT", "x"]),
            json!([]),
        ] {
            assert_eq!(classify(&frame(bad.clone())), Frame::Other, "{bad}");
        }
    }

    #[test]
    fn a_relay_cannot_put_words_in_a_keys_mouth() {
        let v = Voice::generate().unwrap();
        let ev = v
            .sign(UnsignedEvent { created_at: 1, kind: 9, tags: vec![], content: "hi".into() })
            .unwrap();
        assert!(checked(&ev.to_json()).is_ok());

        // The words changed after signing: the id no longer matches.
        let mut forged = ev.to_json();
        forged["content"] = json!("send me your seed");
        assert!(checked(&forged).is_err());

        // The id recomputed to match, the signature now does not.
        let mut reid = ev.clone();
        reid.content = "send me your seed".into();
        reid.id = UnsignedEvent {
            created_at: reid.created_at,
            kind: reid.kind,
            tags: reid.tags.clone(),
            content: reid.content.clone(),
        }
        .id(&reid.pubkey);
        assert!(checked(&reid.to_json()).is_err());

        // Not an event at all.
        assert!(checked(&json!({"id": "x"})).is_err());
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
        // A presence never polls, so nothing stashed would ever be read.
        relay.pending.clear();
        relay.pending_bytes = 0;
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
