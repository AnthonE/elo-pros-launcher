//! The Chat window's engine — everything the window does, with no window.
//!
//! Operator, 2026-10-20: *"shouldnt we form buzz into our desktop launcher
//! anyways? then we get chat we use anyways?"* — and the command line already
//! had it (`elo chat`). The window did not, and the command line read every
//! room **through the origin's mirror** (`GET /api/hive/room`), so chat in the
//! launcher went dark whenever elopros.com did, with the relay itself fine on
//! another box.
//!
//! ## Where a room is read from, in order
//!
//! 1. **The relay, directly**, over the same NIP-42-authenticated socket the
//!    client already posts on. Live, and the origin is not in the path. Every
//!    event is checked — id recomputed, signature verified — before it is
//!    shown, so the relay carries messages and cannot write them.
//! 2. **The origin's mirror**, when there is no chat key on this machine (the
//!    relay asks a reader to sign in, and a key is the only way to) or the
//!    relay will not answer. Polled. It is also where sworn names come from,
//!    so it is read once per room even when the relay is live.
//! 3. **The last visit's room list**, when the origin cannot be reached at
//!    all, so a client with a key can still find the relay and the rooms.
//!
//! The window reads a [`View`] and sends [`Ask`]s. Nothing here draws.
//!
//! ## One socket, on purpose
//!
//! Posting rides the reading socket rather than a fresh one per note. buzz
//! clears a key's presence the instant ANY of its connections closes
//! (`relay.rs`, [`crate::relay::Presence`]), so a short posting socket would
//! knock the player offline mid-conversation with the reading one still up.

use crate::relay::{self, Polled, Relay, Status};
use crate::voice::{hex_to_npub, Event, UnsignedEvent, Voice};
use crate::who::{self, NameKind};
use crate::{Channel, Channels, Hive, Message};
use elo_net::Net;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How much history one room shows.
pub const HISTORY: usize = 60;
/// How many messages the feed holds before the oldest go.
pub const FEED_CAP: usize = 500;
/// The longest note a room carries — the origin's own cap (`meter/hive.py`).
pub const MAX_NOTE: usize = 8000;
/// The kinds a room's feed shows: the town stream, a room note, and buzz's
/// 40002, which the origin's filter reads beside kind 9.
pub const CHAT_KINDS: [i64; 3] = [1, 9, 40002];

/// How often the mirror is asked again when it is all there is.
const MIRROR_EVERY: Duration = Duration::from_secs(10);
/// How often the room list is re-read once it has been read.
const ROOMS_EVERY: Duration = Duration::from_secs(300);
/// How long a quiet socket is waited on per turn, so a held socket still
/// hears the window quickly.
const POLL_WAIT: Duration = Duration::from_millis(250);
/// The idle turn when nothing is live.
const TICK: Duration = Duration::from_millis(250);
/// The longest wait between attempts on a relay that keeps refusing.
const BACKOFF_MAX: Duration = Duration::from_secs(120);
/// How long a speaker whose lookup could not be made waits before another.
const FACE_RETRY: Duration = Duration::from_secs(300);
/// The most lookups queued at once — a room's worth, not a crowd's.
const FACES_IN_FLIGHT: usize = 24;

// ── where the feed is coming from ───────────────────────────────────────────

/// Where the messages on screen are coming from right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Not read yet.
    Starting,
    /// Straight off the relay, live.
    Relay(String),
    /// The origin's copy, polled.
    Mirror,
    /// Nothing could be reached.
    Offline,
}

/// A speaker as the register names them, with their picture if they have one.
///
/// Resolved through `who::resolve` — the one identity rule every surface in
/// the client uses — so the name and picture here are the ones the site's
/// profile shows: the wallet's own picture, then the Elo Pros seat it holds,
/// then the vow's mark (`who.rs` `build`).
#[derive(Debug, Clone)]
pub struct Person {
    pub name: String,
    pub kind: NameKind,
    /// The wallet this key resolves to — a sworn vow's, or one the wallet
    /// linked (`/api/hive/link`). How the window tells a linked key.
    pub wallet: Option<String>,
    /// Image bytes, fetched from OUR origin only (`Identity::origin_face`) and
    /// checked for an image's magic. The window decodes them.
    pub picture: Option<Arc<Vec<u8>>>,
}

/// What the window shows. A snapshot: the engine owns the real state.
#[derive(Debug, Clone)]
pub struct View {
    pub rooms: Vec<Channel>,
    /// The room list came from this machine's last visit, saved at this unix
    /// time, because the origin could not be reached.
    pub rooms_saved_at: Option<i64>,
    pub room: Option<String>,
    /// Oldest first.
    pub messages: Vec<Message>,
    pub link: Link,
    /// One short sentence for the status line.
    pub status: String,
    /// The detail behind `status` when something failed, for a tooltip.
    pub why: String,
    /// The chat key this machine speaks with.
    pub npub: Option<String>,
    /// The last note's outcome: the event id, or why it did not post.
    pub said: Option<Result<String, String>>,
    /// How many events were thrown away because their signature failed.
    pub dropped: usize,
    /// The speakers looked up so far, by npub.
    pub people: BTreeMap<String, Person>,
    /// Bumped on every change, so the window can skip a redraw.
    pub version: u64,
}

impl Default for View {
    fn default() -> Self {
        View {
            rooms: Vec::new(),
            rooms_saved_at: None,
            room: None,
            messages: Vec::new(),
            link: Link::Starting,
            status: "connecting…".into(),
            why: String::new(),
            npub: None,
            said: None,
            dropped: 0,
            people: BTreeMap::new(),
            version: 0,
        }
    }
}

/// What the window can ask for.
#[derive(Debug, Clone)]
pub enum Ask {
    /// Show this room (by id).
    Open(String),
    /// Post this text to the open room.
    Say(String),
    /// Read everything again now.
    Refresh,
    /// Make a chat key on this machine, if there is none.
    MakeVoice,
    /// The key on disk changed (it was linked to a wallet): load it, drop the
    /// socket the old key held, and look the new one up.
    ReloadVoice,
    Stop,
}

/// The window's handle on the engine. Dropping it stops the engine, which
/// closes the socket, which is what takes the player offline.
pub struct Chat {
    tx: Sender<Ask>,
    view: Arc<Mutex<View>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Chat {
    /// Start reading. `voice_dir` is where the chat key lives and `cache_dir`
    /// is where the last good room list and names are kept.
    pub fn start(host: &str, voice_dir: PathBuf, cache_dir: PathBuf) -> Chat {
        let (tx, rx) = mpsc::channel();
        let view = Arc::new(Mutex::new(View::default()));
        let host = host.trim_end_matches('/').to_string();
        // Faces on their own thread: a lookup is up to two origin reads and a
        // picture, and none of that may hold up the socket the room is on.
        let (ask_face, faces_in) = mpsc::channel::<String>();
        let (faces_out, got_face) = mpsc::channel::<(String, Option<Person>)>();
        {
            let host = host.clone();
            std::thread::spawn(move || faces(&host, faces_in, faces_out));
        }
        let worker = {
            let view = Arc::clone(&view);
            std::thread::spawn(move || {
                let mut w = Worker::new(host, voice_dir, cache_dir, view, rx);
                w.faces = Some((ask_face, got_face));
                w.run()
            })
        };
        Chat { tx, view, worker: Some(worker) }
    }

    pub fn ask(&self, ask: Ask) {
        let _ = self.tx.send(ask);
    }

    /// The current snapshot.
    pub fn view(&self) -> View {
        lock(&self.view).clone()
    }

    /// Cheap: has anything changed since `seen`?
    pub fn version(&self) -> u64 {
        lock(&self.view).version
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        let _ = self.tx.send(Ask::Stop);
        // Not joined: a worker mid-handshake can take its IO timeout to
        // notice, and the window closing must not wait on a socket.
        let _ = self.worker.take();
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The cache directory: `chat/` under the client's state root.
pub fn default_cache_dir() -> PathBuf {
    elo_depot::dirs::state_root().join("chat")
}

/// The faces thread: one npub in, one person out — or `None` when the
/// register could not be reached, so the engine asks again later rather than
/// remembering a stranger as faceless.
fn faces(host: &str, asks: Receiver<String>, out: Sender<(String, Option<Person>)>) {
    let net = Net::new();
    while let Ok(npub) = asks.recv() {
        let got = who::resolve(&net, host, &npub);
        let person = got.value.map(|id| Person {
            picture: id
                .origin_face(host)
                .and_then(|u| net.art(u).value)
                .filter(|b| looks_like_image(b))
                .map(Arc::new),
            name: id.name,
            kind: id.kind,
            wallet: id.wallet.map(|w| w.to_lowercase()),
        });
        if out.send((npub, person)).is_err() {
            return;
        }
    }
}

/// A first look at picture bytes: PNG, JPEG or SVG by their opening bytes.
/// The window sniffs again before anything reaches a decoder (`art::Icon`);
/// this only keeps an error page from being carried that far.
pub fn looks_like_image(b: &[u8]) -> bool {
    b.starts_with(b"\x89PNG\r\n\x1a\n")
        || b.starts_with(&[0xff, 0xd8, 0xff])
        || String::from_utf8_lossy(&b[..b.len().min(256)]).trim_start().starts_with("<svg")
        || String::from_utf8_lossy(&b[..b.len().min(256)]).trim_start().starts_with("<?xml")
}

// ── names ───────────────────────────────────────────────────────────────────

/// What the origin said a key is called, kept so a message read straight off
/// the relay — which carries only a pubkey — wears the same name the mirror
/// gave it. Sworn and self-declared stay separate, as they do everywhere.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Name {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub house: bool,
}

/// How many keys' names are remembered. A cache, so it may forget.
const NAMES_CAP: usize = 4000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Names(pub BTreeMap<String, Name>);

impl Names {
    /// Remember what the mirror said about each speaker. A message with no
    /// name teaches nothing, and never erases a name already known.
    pub fn learn(&mut self, messages: &[Message]) -> bool {
        let mut changed = false;
        for m in messages {
            let n = Name {
                who: m.who.clone().filter(|s| !s.trim().is_empty()),
                display: m.display.clone().filter(|s| !s.trim().is_empty()),
                house: m.house,
            };
            if m.npub.is_empty() || n == Name::default() {
                continue;
            }
            if self.0.get(&m.npub) != Some(&n) {
                if self.0.len() >= NAMES_CAP && !self.0.contains_key(&m.npub) {
                    // Bounded the cheap way: forget everything but this room's.
                    self.0.clear();
                }
                self.0.insert(m.npub.clone(), n);
                changed = true;
            }
        }
        changed
    }

    /// Put the remembered name on a message that arrived without one.
    pub fn dress(&self, m: &mut Message) {
        if let Some(n) = self.0.get(&m.npub) {
            if m.who.is_none() {
                m.who = n.who.clone();
            }
            if m.display.is_none() {
                m.display = n.display.clone();
            }
            m.house |= n.house;
        }
    }

    pub fn knows(&self, npub: &str) -> bool {
        self.0.contains_key(npub)
    }
}

// ── the feed ────────────────────────────────────────────────────────────────

/// One room's messages: deduplicated by id, oldest first, bounded.
#[derive(Debug, Default)]
pub struct Feed {
    messages: Vec<Message>,
    ids: HashSet<String>,
}

impl Feed {
    /// Add what is new; return whether anything was. The mirror and the relay
    /// both deliver the same events, so a merge that did not dedupe by id
    /// would show every message twice.
    pub fn merge<I: IntoIterator<Item = Message>>(&mut self, incoming: I) -> bool {
        let mut added = false;
        for m in incoming {
            if m.id.is_empty() || self.ids.contains(&m.id) {
                continue;
            }
            self.ids.insert(m.id.clone());
            self.messages.push(m);
            added = true;
        }
        if added {
            self.messages.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
            if self.messages.len() > FEED_CAP {
                let cut = self.messages.len() - FEED_CAP;
                for m in self.messages.drain(..cut) {
                    self.ids.remove(&m.id);
                }
            }
        }
        added
    }

    /// Fill in names the cache learned after these messages arrived.
    pub fn redress(&mut self, names: &Names) {
        for m in self.messages.iter_mut() {
            names.dress(m);
        }
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}

/// A relay event as a feed message, if it belongs in `room`'s feed.
///
/// ⚠ **The room is checked, not assumed.** A relay answering our REQ chooses
/// what to send under it; an event for another room, or of a kind that is not
/// chat, is not this room's message whatever subscription it arrived on.
pub fn message_of(ev: &Event, room: &str, names: &Names) -> Option<Message> {
    if !CHAT_KINDS.contains(&ev.kind) {
        return None;
    }
    // Longer than any note we would send (`check_note`): the feed keeps
    // FEED_CAP messages, and a relay does not get to make each one fat.
    if ev.content.len() > MAX_NOTE && ev.content.chars().count() > MAX_NOTE {
        return None;
    }
    let in_room = ev.tags.iter().any(|t| t.len() >= 2 && t[0] == "h" && t[1] == room);
    if !room.is_empty() && !in_room {
        return None;
    }
    let mut m = Message {
        id: ev.id.clone(),
        at: ev.created_at,
        kind: ev.kind,
        content: ev.content.clone(),
        npub: hex_to_npub(&ev.pubkey).unwrap_or_else(|_| ev.pubkey.clone()),
        ..Message::default()
    };
    names.dress(&mut m);
    Some(m)
}

/// The note a room takes, from its posted `post` shape when it has one. Read
/// and write must agree or a note lands where nobody is reading, so this
/// follows the origin's shape and falls back to `relay::note_for`'s.
pub fn note_to(room: &Channel, text: &str, at: i64) -> UnsignedEvent {
    let shaped = room.post.as_ref().and_then(|p| {
        let kind = p.get("kind")?.as_i64()?;
        let tags = p
            .get("tags")?
            .as_array()?
            .iter()
            .map(|t| {
                t.as_array().map(|a| {
                    a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>()
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(UnsignedEvent { created_at: at, kind, tags, content: text.to_string() })
    });
    shaped.unwrap_or_else(|| relay::note_for(Some(&room.id), text, at))
}

/// Refuse a note before it is signed, with the reason a player reads.
pub fn check_note(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("nothing to send".into());
    }
    if t.chars().count() > MAX_NOTE {
        return Err(format!("too long — a note is at most {MAX_NOTE} characters"));
    }
    Ok(t.to_string())
}

// ── the cache on disk ───────────────────────────────────────────────────────

const CACHE_NAME: &str = "chat.json";

/// The last room list that was read and the names learned since. Nothing in
/// here is a secret, and none of it is a record — it is how a client finds
/// the relay again when the origin cannot tell it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    pub saved_at: i64,
    #[serde(default)]
    pub channels: Option<Channels>,
    #[serde(default)]
    pub names: Names,
}

impl Cache {
    /// A missing or unreadable cache is an empty one: it is a convenience,
    /// and losing it costs one read of the origin.
    pub fn load(dir: &Path) -> Cache {
        std::fs::read(dir.join(CACHE_NAME))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Written whole to a temporary file and renamed into place, so a killed
    /// client never leaves half a file for the next start to choke on.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{CACHE_NAME}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec(self).unwrap_or_default())?;
        std::fs::rename(&tmp, dir.join(CACHE_NAME))
    }
}

// ── the worker ──────────────────────────────────────────────────────────────

/// The two ends the worker holds on the faces thread: npubs out, people back
/// (`None` when the register could not be reached).
type FacesLink = (Sender<String>, Receiver<(String, Option<Person>)>);

struct Live {
    relay: Relay,
    sub: String,
    room: String,
}

struct Worker {
    host: String,
    voice_dir: PathBuf,
    cache_dir: PathBuf,
    net: Net,
    view: Arc<Mutex<View>>,
    rx: Receiver<Ask>,
    cache: Cache,
    channels: Option<Channels>,
    rooms_saved_at: Option<i64>,
    room: Option<String>,
    feed: Feed,
    voice: Option<Voice>,
    live: Option<Live>,
    link: Link,
    why: String,
    said: Option<Result<String, String>>,
    dropped: usize,
    sub_n: u64,
    next_rooms: Instant,
    next_mirror: Instant,
    next_connect: Instant,
    next_names: Instant,
    backoff: Duration,
    last_beat: Instant,
    names_dirty: bool,
    dirty: bool,
    /// The faces thread: where to ask, and where answers come back.
    faces: Option<FacesLink>,
    people: BTreeMap<String, Person>,
    /// Asked and not yet answered, or answered `None` at this instant.
    asked: HashMap<String, Option<Instant>>,
}

impl Worker {
    fn new(host: String, voice_dir: PathBuf, cache_dir: PathBuf, view: Arc<Mutex<View>>,
           rx: Receiver<Ask>) -> Worker {
        let now = Instant::now();
        let cache = Cache::load(&cache_dir);
        Worker {
            host,
            voice: Voice::load(&voice_dir).ok(),
            voice_dir,
            cache_dir,
            net: Net::new(),
            view,
            rx,
            cache,
            channels: None,
            rooms_saved_at: None,
            room: None,
            feed: Feed::default(),
            live: None,
            link: Link::Starting,
            why: String::new(),
            said: None,
            dropped: 0,
            sub_n: 0,
            next_rooms: now,
            next_mirror: now,
            next_connect: now,
            next_names: now,
            backoff: Duration::from_secs(5),
            last_beat: now,
            names_dirty: false,
            dirty: true,
            faces: None,
            people: BTreeMap::new(),
            asked: HashMap::new(),
        }
    }

    fn run(mut self) {
        loop {
            let wait = if self.live.is_some() { Duration::ZERO } else { TICK };
            match self.rx.recv_timeout(wait) {
                Ok(Ask::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(ask) => self.handle(ask),
                Err(RecvTimeoutError::Timeout) => {}
            }
            let mut stop = false;
            while let Ok(ask) = self.rx.try_recv() {
                if matches!(ask, Ask::Stop) {
                    stop = true;
                    break;
                }
                self.handle(ask);
            }
            if stop {
                break;
            }
            self.turn();
            self.publish();
        }
        self.shutdown();
    }

    fn handle(&mut self, ask: Ask) {
        match ask {
            Ask::Open(id) => self.open(id),
            Ask::Say(text) => self.say(&text),
            Ask::Refresh => {
                let now = Instant::now();
                self.next_rooms = now;
                self.next_mirror = now;
                self.next_connect = now;
                self.backoff = Duration::from_secs(5);
            }
            Ask::MakeVoice => self.make_voice(),
            Ask::ReloadVoice => self.reload_voice(),
            Ask::Stop => {}
        }
        self.dirty = true;
    }

    /// One pass: the room list if it is due, then the open room's feed.
    fn turn(&mut self) {
        let now = Instant::now();
        if now >= self.next_rooms {
            self.read_rooms();
        }
        if self.room.is_none() {
            if let Some(c) = &self.channels {
                if let Ok(Some(id)) = c.resolve(None) {
                    self.room = Some(id);
                    self.dirty = true;
                }
            }
        }
        if self.room.is_none() {
            return;
        }
        if self.voice.is_some() && self.live.is_none() && now >= self.next_connect {
            // Names first: the mirror knows who is sworn, the relay does not.
            if self.feed.messages().is_empty() {
                self.read_mirror();
            }
            self.go_live();
        }
        if self.live.is_some() {
            self.pump();
        } else if now >= self.next_mirror {
            self.read_mirror();
        }
        if self.names_dirty && now >= self.next_names {
            let _ = self.cache.save(&self.cache_dir);
            self.names_dirty = false;
        }
        self.trade_faces();
    }

    /// Ask for the faces of speakers not yet looked up, and take in answers.
    fn trade_faces(&mut self) {
        let Some((ask, got)) = self.faces.as_ref() else { return };
        while let Ok((npub, person)) = got.try_recv() {
            match person {
                Some(p) => {
                    self.asked.remove(&npub);
                    self.people.insert(npub, p);
                    self.dirty = true;
                }
                None => {
                    self.asked.insert(npub, Some(Instant::now()));
                }
            }
        }
        let in_flight = self.asked.values().filter(|v| v.is_none()).count();
        let mut room = FACES_IN_FLIGHT.saturating_sub(in_flight);
        let mine = self.voice.as_ref().map(Voice::npub);
        let speakers = self
            .feed
            .messages()
            .iter()
            .filter(|m| !m.house)
            .map(|m| m.npub.clone())
            .chain(mine);
        for npub in speakers {
            if room == 0 {
                break;
            }
            if npub.is_empty() || self.people.contains_key(&npub) {
                continue;
            }
            match self.asked.get(&npub) {
                Some(None) => continue,
                Some(Some(at)) if at.elapsed() < FACE_RETRY => continue,
                _ => {}
            }
            if ask.send(npub.clone()).is_err() {
                return;
            }
            self.asked.insert(npub, None);
            room -= 1;
        }
    }

    fn read_rooms(&mut self) {
        let got = Hive::new(&self.net, &self.host).channels();
        let now = Instant::now();
        match got.value {
            Some(c) if c.posted => {
                self.channels = Some(c.clone());
                self.rooms_saved_at = None;
                self.cache.channels = Some(c);
                self.cache.saved_at = crate::clock::now();
                let _ = self.cache.save(&self.cache_dir);
                self.next_rooms = now + ROOMS_EVERY;
            }
            Some(c) => {
                // A real answer: the house has posted no relay yet.
                self.channels = Some(c);
                self.why = "no chat server is posted yet".into();
                self.next_rooms = now + ROOMS_EVERY;
            }
            None => {
                if self.channels.is_none() {
                    if let Some(c) = self.cache.channels.clone() {
                        self.channels = Some(c);
                        self.rooms_saved_at = Some(self.cache.saved_at);
                    }
                }
                self.why = got.why;
                self.next_rooms = now + Duration::from_secs(30);
            }
        }
        self.dirty = true;
    }

    fn read_mirror(&mut self) {
        let Some(room) = self.room.clone() else { return };
        let got = Hive::new(&self.net, &self.host).room(Some(&room), HISTORY);
        let now = Instant::now();
        match got.value {
            Some(r) => {
                if self.cache.names.learn(&r.messages) {
                    self.names_dirty = true;
                    self.feed.redress(&self.cache.names);
                }
                let mut msgs = r.messages;
                // Kind 0 here is a message whose `kind` the origin left out
                // (it deserialises to the default), not a profile: the
                // mirror's own filter asks for chat kinds only.
                msgs.retain(|m| CHAT_KINDS.contains(&m.kind) || m.kind == 0);
                self.feed.merge(msgs);
                if self.live.is_none() {
                    self.link = Link::Mirror;
                }
                self.next_mirror = now + MIRROR_EVERY;
            }
            None => {
                if self.live.is_none() {
                    self.link = Link::Offline;
                    self.why = got.why;
                }
                self.next_mirror = now + Duration::from_secs(15);
            }
        }
        self.next_names = now + Duration::from_secs(60);
        self.dirty = true;
    }

    fn channel(&self, id: &str) -> Option<Channel> {
        self.channels.as_ref()?.by_id(id).cloned()
    }

    fn go_live(&mut self) {
        let (Some(room), Some(channels)) = (self.room.clone(), self.channels.clone()) else {
            return;
        };
        let Some(ch) = self.channel(&room) else { return };
        let Some(voice) = self.voice.as_ref() else { return };
        let mut last = "no relay is posted".to_string();
        for url in channels.relays() {
            match Relay::connect(&url, voice) {
                Ok(mut relay) => {
                    self.sub_n += 1;
                    let sub = format!("elo-{}", self.sub_n);
                    let opened = relay
                        .set_wait(POLL_WAIT)
                        .and_then(|_| relay.subscribe(&sub, &ch.filter(HISTORY)));
                    match opened {
                        Ok(backlog) => {
                            self.dropped += backlog.dropped;
                            let names = &self.cache.names;
                            let msgs: Vec<Message> = backlog
                                .events
                                .iter()
                                .filter_map(|e| message_of(e, &room, names))
                                .collect();
                            self.feed.merge(msgs);
                            let _ = relay.beat(voice, Status::Online);
                            self.last_beat = Instant::now();
                            self.link = Link::Relay(url.clone());
                            self.why.clear();
                            self.backoff = Duration::from_secs(5);
                            self.live = Some(Live { relay, sub, room });
                            self.dirty = true;
                            return;
                        }
                        Err(e) => {
                            relay.close();
                            last = e.to_string();
                        }
                    }
                }
                Err(e) => last = e.to_string(),
            }
        }
        self.why = last;
        self.next_connect = Instant::now() + self.backoff;
        self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
        self.next_mirror = Instant::now();
        self.dirty = true;
    }

    /// Read what has arrived on the held socket, and keep it alive.
    fn pump(&mut self) {
        let Some(live) = self.live.as_mut() else { return };
        let mut failed = None;
        let mut fresh = Vec::new();
        for _ in 0..64 {
            match live.relay.poll(&live.sub) {
                Ok(Some(Polled::Event(e))) => fresh.push(e),
                Ok(Some(Polled::Dropped)) => self.dropped += 1,
                Ok(None) => break,
                Err(e) => {
                    failed = Some(e.to_string());
                    break;
                }
            }
        }
        if failed.is_none() && self.last_beat.elapsed() >= relay::HEARTBEAT {
            if let Some(v) = self.voice.as_ref() {
                if let Err(e) = live.relay.beat(v, Status::Online) {
                    failed = Some(e.to_string());
                }
            }
            self.last_beat = Instant::now();
        }
        let room = live.room.clone();
        if !fresh.is_empty() {
            let names = &self.cache.names;
            let msgs: Vec<Message> =
                fresh.iter().filter_map(|e| message_of(e, &room, names)).collect();
            let strangers = msgs.iter().any(|m| !names.knows(&m.npub));
            if self.feed.merge(msgs) {
                self.dirty = true;
            }
            // Someone the cache cannot name: ask the mirror, at most once a
            // minute, rather than show an npub where a sworn name exists.
            if strangers && Instant::now() >= self.next_names {
                self.read_mirror();
            }
        }
        if let Some(why) = failed {
            if let Some(l) = self.live.take() {
                l.relay.close();
            }
            self.link = Link::Mirror;
            self.why = why;
            self.next_connect = Instant::now() + self.backoff;
            self.next_mirror = Instant::now();
            self.dirty = true;
        }
    }

    fn open(&mut self, id: String) {
        if self.room.as_deref() == Some(id.as_str()) {
            return;
        }
        self.room = Some(id.clone());
        self.feed = Feed::default();
        self.said = None;
        self.next_mirror = Instant::now();
        let Some(ch) = self.channel(&id) else { return };
        if let Some(live) = self.live.as_mut() {
            let _ = live.relay.unsubscribe(&live.sub);
            self.sub_n += 1;
            let sub = format!("elo-{}", self.sub_n);
            match live.relay.subscribe(&sub, &ch.filter(HISTORY)) {
                Ok(backlog) => {
                    self.dropped += backlog.dropped;
                    let names = &self.cache.names;
                    let msgs: Vec<Message> =
                        backlog.events.iter().filter_map(|e| message_of(e, &id, names)).collect();
                    self.feed.merge(msgs);
                    live.sub = sub;
                    live.room = id;
                }
                Err(e) => {
                    // This room will not open on this socket — read it from
                    // the mirror and let the next connect try again.
                    if let Some(l) = self.live.take() {
                        l.relay.close();
                    }
                    self.link = Link::Mirror;
                    self.why = e.to_string();
                    self.next_connect = Instant::now() + self.backoff;
                }
            }
        }
        self.read_mirror();
    }

    fn say(&mut self, text: &str) {
        let text = match check_note(text) {
            Ok(t) => t,
            Err(e) => {
                self.said = Some(Err(e));
                return;
            }
        };
        let Some(voice) = self.voice.as_ref() else {
            self.said = Some(Err("make a chat key first".into()));
            return;
        };
        let Some(room) = self.room.clone().and_then(|r| self.channel(&r)) else {
            self.said = Some(Err("no room is open".into()));
            return;
        };
        let event = match voice.sign(note_to(&room, &text, crate::clock::now())) {
            Ok(e) => e,
            Err(e) => {
                self.said = Some(Err(e.to_string()));
                return;
            }
        };
        let result = match self.live.as_mut() {
            Some(live) => live.relay.publish(&event).map_err(|e| e.to_string()),
            // Nothing held: one socket for this note, on each relay in turn.
            None => {
                let relays = self.channels.as_ref().map(Channels::relays).unwrap_or_default();
                let mut out = Err("no relay is posted".to_string());
                for url in relays {
                    match Relay::connect(&url, voice) {
                        Ok(mut r) => {
                            out = r.publish(&event).map_err(|e| e.to_string());
                            r.close();
                            if out.is_ok() {
                                break;
                            }
                        }
                        Err(e) => out = Err(e.to_string()),
                    }
                }
                out
            }
        };
        if result.is_ok() {
            if let Some(m) = message_of(&event, &room.id, &self.cache.names) {
                self.feed.merge([m]);
            }
        }
        self.said = Some(result);
    }

    fn make_voice(&mut self) {
        if self.voice.is_some() || Voice::exists(&self.voice_dir) {
            self.voice = Voice::load(&self.voice_dir).ok();
            return;
        }
        match Voice::generate().and_then(|v| v.save(&self.voice_dir).map(|_| v)) {
            Ok(v) => {
                self.voice = Some(v);
                self.next_connect = Instant::now();
                self.backoff = Duration::from_secs(5);
            }
            Err(e) => self.said = Some(Err(format!("could not make a chat key: {e}"))),
        }
    }

    fn reload_voice(&mut self) {
        if let Some(old) = self.voice.as_ref().map(Voice::npub) {
            self.people.remove(&old);
            self.asked.remove(&old);
        }
        if let Some(l) = self.live.take() {
            l.relay.close();
        }
        self.voice = Voice::load(&self.voice_dir).ok();
        if let Some(new) = self.voice.as_ref().map(Voice::npub) {
            self.people.remove(&new);
            self.asked.remove(&new);
        }
        self.link = Link::Mirror;
        self.next_connect = Instant::now();
        self.backoff = Duration::from_secs(5);
    }

    fn shutdown(mut self) {
        if let (Some(mut live), Some(v)) = (self.live.take(), self.voice.as_ref()) {
            let _ = live.relay.beat(v, Status::Offline);
            live.relay.close();
        }
        if self.names_dirty {
            let _ = self.cache.save(&self.cache_dir);
        }
    }

    fn publish(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut v = lock(&self.view);
        v.rooms = self.channels.as_ref().map(|c| c.channels.clone()).unwrap_or_default();
        v.rooms_saved_at = self.rooms_saved_at;
        v.room = self.room.clone();
        v.messages = self.feed.messages().to_vec();
        v.link = self.link.clone();
        v.status = status_line(&self.link, self.voice.is_some(), self.rooms_saved_at.is_some(),
                               self.channels.is_some());
        v.why = self.why.clone();
        v.npub = self.voice.as_ref().map(Voice::npub);
        v.said = self.said.clone();
        v.dropped = self.dropped;
        v.people = self.people.clone();
        v.version += 1;
    }
}

/// The status line, from where the feed is coming from. Plain and short: what
/// is happening, and the one thing that would change it.
pub fn status_line(link: &Link, has_voice: bool, rooms_cached: bool, has_rooms: bool) -> String {
    match link {
        Link::Starting => "connecting…".into(),
        Link::Relay(url) => format!("live · {}", url.trim_start_matches("wss://")),
        Link::Mirror if !has_voice => "updates every few seconds · make a chat key to talk".into(),
        Link::Mirror => "can't reach the chat server directly — showing elopros.com's copy".into(),
        Link::Offline if rooms_cached => "offline — rooms from your last visit, retrying".into(),
        Link::Offline if has_rooms => "can't load this room right now — retrying".into(),
        Link::Offline => "can't reach chat right now — retrying".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(id: &str, at: i64, npub: &str) -> Message {
        Message { id: id.into(), at, kind: 9, content: format!("m{id}"), npub: npub.into(),
                  ..Message::default() }
    }

    #[test]
    fn the_feed_is_deduped_by_id_and_sorted_oldest_first() {
        let mut f = Feed::default();
        assert!(f.merge([msg("b", 20, "n"), msg("a", 10, "n")]));
        // The mirror and the relay deliver the same events.
        assert!(!f.merge([msg("a", 10, "n"), msg("b", 20, "n")]));
        assert!(f.merge([msg("c", 15, "n")]));
        let ids: Vec<&str> = f.messages().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["a", "c", "b"]);
    }

    #[test]
    fn the_feed_is_bounded_and_forgets_the_oldest() {
        let mut f = Feed::default();
        f.merge((0..FEED_CAP as i64 + 10).map(|i| msg(&format!("{i:05}"), i, "n")));
        assert_eq!(f.messages().len(), FEED_CAP);
        assert_eq!(f.messages()[0].id, "00010");
        // A forgotten id can come back, because it is no longer held.
        assert!(f.merge([msg("00000", 0, "n")]));
    }

    #[test]
    fn an_empty_id_never_enters_the_feed() {
        let mut f = Feed::default();
        assert!(!f.merge([msg("", 1, "n")]));
    }

    #[test]
    fn names_learned_from_the_mirror_dress_a_relay_message() {
        let mut names = Names::default();
        let mut sworn = msg("a", 1, "npub1alice");
        sworn.who = Some("alice".into());
        let mut guest = msg("b", 2, "npub1guest");
        guest.display = Some("totally-the-house".into());
        assert!(names.learn(&[sworn, guest, msg("c", 3, "npub1nobody")]));

        let mut m = msg("d", 4, "npub1alice");
        names.dress(&mut m);
        assert_eq!(m.who.as_deref(), Some("alice"));

        // A self-declared name stays self-declared: it never becomes `who`.
        let mut g = msg("e", 5, "npub1guest");
        names.dress(&mut g);
        assert_eq!(g.who, None);
        assert_eq!(g.display.as_deref(), Some("totally-the-house"));

        // A key with no name taught nothing.
        assert!(!names.knows("npub1nobody"));
    }

    #[test]
    fn a_nameless_message_never_erases_a_known_name() {
        let mut names = Names::default();
        let mut sworn = msg("a", 1, "npub1alice");
        sworn.who = Some("alice".into());
        names.learn(&[sworn]);
        assert!(!names.learn(&[msg("b", 2, "npub1alice")]));
        assert!(names.knows("npub1alice"));
    }

    fn event(kind: i64, tags: Vec<Vec<String>>) -> Event {
        let v = Voice::generate().unwrap();
        v.sign(UnsignedEvent { created_at: 100, kind, tags, content: "hi".into() }).unwrap()
    }

    #[test]
    fn a_relay_event_only_counts_for_its_own_room_and_chat_kinds() {
        let names = Names::default();
        let here = event(9, vec![vec!["h".into(), "room-1".into()]]);
        let m = message_of(&here, "room-1", &names).expect("this room's note");
        assert!(m.npub.starts_with("npub1"), "{}", m.npub);
        assert_eq!(m.content, "hi");

        let elsewhere = event(9, vec![vec!["h".into(), "room-2".into()]]);
        assert!(message_of(&elsewhere, "room-1", &names).is_none());

        let presence = event(20001, vec![vec!["h".into(), "room-1".into()]]);
        assert!(message_of(&presence, "room-1", &names).is_none());
    }

    #[test]
    fn a_note_longer_than_any_we_send_is_not_kept() {
        // The feed holds FEED_CAP of these; a relay's fat note must not be one.
        let names = Names::default();
        let mut fat = event(9, vec![vec!["h".into(), "room-1".into()]]);
        fat.content = "é".repeat(MAX_NOTE);
        assert!(message_of(&fat, "room-1", &names).is_some(), "the cap counts characters");
        fat.content.push('x');
        assert!(message_of(&fat, "room-1", &names).is_none());
    }

    #[test]
    fn a_note_takes_the_shape_the_room_posted() {
        let room = Channel {
            id: "room-1".into(),
            name: "general".into(),
            about: String::new(),
            sub: None,
            post: Some(json!({"kind": 9, "tags": [["h", "room-1"]]})),
        };
        let n = note_to(&room, "hello", 7);
        assert_eq!(n.kind, 9);
        assert_eq!(n.tags, vec![vec!["h".to_string(), "room-1".to_string()]]);
        assert_eq!(n.created_at, 7);

        // No posted shape (a room from an old cache): the client's own.
        let bare = Channel { post: None, ..room.clone() };
        let n = note_to(&bare, "hello", 7);
        assert_eq!(n.kind, relay::KIND_ROOM_NOTE);
        assert_eq!(n.tags, vec![vec!["h".to_string(), "room-1".to_string()]]);

        // A malformed shape is not half-obeyed.
        let odd = Channel { post: Some(json!({"kind": "nine"})), ..room };
        assert_eq!(note_to(&odd, "hello", 7).kind, relay::KIND_ROOM_NOTE);
    }

    #[test]
    fn a_note_is_checked_before_it_is_signed() {
        assert_eq!(check_note("  hi  "), Ok("hi".into()));
        assert!(check_note("   ").is_err());
        assert!(check_note(&"x".repeat(MAX_NOTE + 1)).is_err());
        assert!(check_note(&"é".repeat(MAX_NOTE)).is_ok(), "the cap counts characters, not bytes");
    }

    #[test]
    fn a_room_filter_uses_the_origins_and_sets_the_limit() {
        let mut room = Channel {
            id: "room-1".into(),
            name: "general".into(),
            about: String::new(),
            sub: Some(json!({"kinds": [9, 40002], "#h": ["room-1"], "limit": 3})),
            post: None,
        };
        assert_eq!(room.filter(60), json!({"kinds": [9, 40002], "#h": ["room-1"], "limit": 60}));
        room.sub = None;
        assert_eq!(room.filter(60), json!({"kinds": [9, 40002], "#h": ["room-1"], "limit": 60}));
    }

    #[test]
    fn the_cache_round_trips_and_a_bad_one_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache {
            saved_at: 42,
            channels: Some(Channels {
                posted: true,
                relay: Some("wss://hive.example".into()),
                ..Channels::default()
            }),
            ..Cache::default()
        };
        c.save(dir.path()).unwrap();
        let back = Cache::load(dir.path());
        assert_eq!(back.saved_at, 42);
        assert_eq!(back.channels.unwrap().relay.as_deref(), Some("wss://hive.example"));

        std::fs::write(dir.path().join(CACHE_NAME), b"{ not json").unwrap();
        assert!(Cache::load(dir.path()).channels.is_none());
    }

    #[test]
    fn faces_are_asked_once_per_speaker_and_never_for_the_house() {
        let keys = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let (_tx, rx) = mpsc::channel();
        let mut w = Worker::new("https://elo.test".into(), keys.path().into(),
                                cache.path().into(), Arc::new(Mutex::new(View::default())), rx);
        let (ask, asked) = mpsc::channel();
        let (answer, got) = mpsc::channel();
        w.faces = Some((ask, got));
        let mut house = msg("h", 1, "npub1house");
        house.house = true;
        w.feed.merge([msg("a", 2, "npub1alice"), msg("b", 3, "npub1alice"), house,
                      msg("c", 4, "npub1bob")]);
        w.trade_faces();
        let sent: Vec<String> = asked.try_iter().collect();
        assert_eq!(sent, ["npub1alice", "npub1bob"], "one ask per speaker, none for the house");

        // Asked and unanswered: not asked again.
        w.trade_faces();
        assert_eq!(asked.try_iter().count(), 0);

        // An answer lands in the view; an unreachable lookup waits FACE_RETRY.
        answer.send(("npub1alice".to_string(), Some(Person {
            name: "alice".into(), kind: NameKind::Sworn, wallet: None, picture: None,
        }))).unwrap();
        answer.send(("npub1bob".to_string(), None)).unwrap();
        w.trade_faces();
        assert!(w.people.contains_key("npub1alice"));
        assert!(!w.people.contains_key("npub1bob"));
        assert_eq!(asked.try_iter().count(), 0, "bob waits before another try");
    }

    #[test]
    fn only_image_bytes_are_carried_to_the_window() {
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\n...."));
        assert!(looks_like_image(b"\xff\xd8\xff\xe0"));
        assert!(looks_like_image(b"  <svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        assert!(!looks_like_image(b"<!DOCTYPE html><title>404</title>"));
        assert!(!looks_like_image(b"{\"error\": \"no picture\"}"));
        assert!(!looks_like_image(b""));
    }

    #[test]
    fn the_status_line_says_which_of_the_three_states_it_is() {
        let live = status_line(&Link::Relay("wss://hive.elopros.com".into()), true, false, true);
        assert_eq!(live, "live · hive.elopros.com");
        let keyless = status_line(&Link::Mirror, false, false, true);
        let fallen = status_line(&Link::Mirror, true, false, true);
        let cached = status_line(&Link::Offline, true, true, true);
        let dark = status_line(&Link::Offline, true, false, false);
        let all = [&live, &keyless, &fallen, &cached, &dark];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two states read the same");
            }
        }
    }
}
