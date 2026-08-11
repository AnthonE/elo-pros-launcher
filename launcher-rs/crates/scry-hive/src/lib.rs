//! The hive, read — the town's talk annex, ripped out of `watchtower/hive.html`.
//!
//! Operator, 2026-08-05: *"can we also check out the hive page we already have
//! and like rip the basics out into the app or what not?"*
//!
//! ## What turned out to be cheap, and it is not what the last session said
//!
//! Reading the hive needs **no key, no WebSocket and no Nostr implementation**.
//! The meter already mirrors the relay server-side:
//!
//! | | |
//! |---|---|
//! | `GET /api/hive/channels` | the relay's own open rooms |
//! | `GET /api/hive/room` | recent messages, town stream or one room |
//! | `GET /api/hive/voice` | the beekeeper's outbox — the house's own notes |
//!
//! All three are plain JSON over HTTP, need **no key**, and are the basics.
//!
//! ## Speaking is a different matter, and the reason is measured
//!
//! ⚠ **Correcting this file's own earlier claim.** It said posting was HTTP
//! too, via `POST /api/hive/publish`. That endpoint exists, but
//! `meter/hive.py` records what happened when it was tried against the real
//! relay on 2026-07-25: **buzz binds an event's author to the NIP-42
//! authenticated connection**, so a note carried on the house's socket is
//! refused — *"event pubkey does not match authenticated identity"* — and
//! refused **silently**, with no ruling returned and the note simply never
//! appearing. `hive.js` therefore publishes on its own socket and treats the
//! HTTP route as a fallback for other relay software.
//!
//! So speaking needs three things, and this crate now has all of them:
//! a **schnorr (BIP-340)** key (`voice.rs`), our **own authenticated
//! WebSocket** (`relay.rs`), and NIP-42 AUTH on it. The transport was never
//! the hard part; the author-binding rule was.
//!
//! ## What a browser gave us that a terminal does not
//!
//! `hive.html` renders into a DOM, where the browser escapes text and a CSP
//! bounds what it can do. A terminal has neither. Hive content is **written by
//! strangers** — the town stream carries anything anyone signed — and a raw
//! `\x1b[` sequence in a chat line can clear the screen, move the cursor, or
//! paint fake output above itself. So every string that came off the relay
//! goes through [`safe_line`] before it is printed. See `render.rs`.

pub mod clock;
pub mod error;
pub mod relay;
pub mod render;
pub mod voice;
pub mod who;

pub use error::{HiveError, Result};
pub use voice::{Event, UnsignedEvent, Voice};
pub use who::{Identity, NameKind};

use scry_net::{Fetched, Net};
use serde::Deserialize;

/// One of the relay's open rooms.
#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub about: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Channels {
    /// Has the house posted a relay at all? False is a real state, not a fault.
    #[serde(default)]
    pub posted: bool,
    #[serde(default)]
    pub relay: Option<String>,
    #[serde(default)]
    pub channels: Vec<Channel>,
}

/// One message in a room.
///
/// `who` is the register's sworn name; `display` is the key's own kind:0
/// profile name — self-declared, never a record. The API says so in its own
/// `names` field and the distinction is kept here rather than flattened,
/// because collapsing them would let anyone print themselves a sworn handle.
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: String,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub kind: i64,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub npub: String,
    /// The sworn name from the register, or `None`.
    #[serde(default)]
    pub who: Option<String>,
    /// The key's self-declared profile name. **Not evidence of anything.**
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default)]
    pub house: bool,
}

// ⚠ `Message::speaker()` USED to live here and is deliberately gone. It was a
// second place that decided what a name is, and two such places is exactly the
// fragmentation the operator called out on 2026-08-05. `who::from_message` is
// the one rule now, and it produces the same `Identity` every other surface
// uses.

#[derive(Debug, Clone, Deserialize)]
pub struct Room {
    #[serde(default)]
    pub posted: bool,
    #[serde(default)]
    pub relay: Option<String>,
    #[serde(default)]
    pub room: Option<String>,
    /// "the public town stream", or the room's scope.
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub messages: Vec<Message>,
}

/// One note from the beekeeper's outbox.
#[derive(Debug, Clone, Deserialize)]
pub struct Note {
    pub id: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub who: Option<String>,
    /// Did a relay acknowledge it? Otherwise the note waits for the room.
    #[serde(default)]
    pub carried: bool,
}

/// The beekeeper's outbox — the house's own signed notes.
///
/// Named for what the API calls it. It is **not** [`voice::Voice`], which is a
/// secret key; the two were briefly both called `Voice` and that is exactly
/// the kind of collision that ends with a key in a log line.
#[derive(Debug, Clone, Deserialize)]
pub struct Outbox {
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub composed: usize,
    #[serde(default)]
    pub queued: usize,
    #[serde(default)]
    pub relay: Option<String>,
}

// ⚠ `short_npub()` USED to live here and is deliberately gone. `who::short`
// is the one shortener — two of them meant two surfaces could disagree about
// how a name looks, which is the same fragmentation in miniature that
// `who.rs` exists to end. A test that expected the wrong one is what found it.

pub struct Hive<'a> {
    net: &'a Net,
    host: String,
}

impl<'a> Hive<'a> {
    pub fn new(net: &'a Net, host: &str) -> Self {
        Hive {
            net,
            host: host.trim_end_matches('/').to_string(),
        }
    }

    /// The meter is mounted under `/api`. The prefix lives in exactly one
    /// place here for the same reason `net.py` gives: moving it cost this repo
    /// a day when a `--root-path` made paid calls fall through unpaid.
    fn api(&self, path: &str) -> String {
        format!("{}/api/{}", self.host, path.trim_start_matches('/'))
    }

    pub fn channels(&self) -> Fetched<Channels> {
        decode(self.net.get_json(&self.api("hive/channels")))
    }

    pub fn room(&self, room: Option<&str>, limit: usize) -> Fetched<Room> {
        let mut url = format!("{}?limit={}", self.api("hive/room"), limit.clamp(1, 200));
        if let Some(r) = room.filter(|s| !s.is_empty()) {
            url.push_str("&room=");
            url.push_str(r);
        }
        decode(self.net.get_json(&url))
    }

    /// The beekeeper's outbox. Named `outbox` rather than `voice` so it can
    /// never be confused with the key that speaks.
    pub fn outbox(&self, limit: usize) -> Fetched<Outbox> {
        decode(self.net.get_json(&format!(
            "{}?limit={}",
            self.api("hive/voice"),
            limit.clamp(1, 200)
        )))
    }
}

/// Carry `reachable` through the decode.
///
/// The whole point of `Fetched` is that *we could not look* and *there is
/// nothing there* are different sentences. A decode that dropped `reachable`
/// on the floor would rebuild the trap one layer up, so it is threaded rather
/// than reconstructed.
fn decode<T: for<'de> Deserialize<'de>>(got: Fetched<serde_json::Value>) -> Fetched<T> {
    match got.value {
        Some(v) => match serde_json::from_value::<T>(v) {
            Ok(parsed) => Fetched::good(parsed),
            Err(e) => Fetched::bad(format!("the hive answered in a shape we do not know: {e}")),
        },
        None if got.reachable => Fetched::bad(got.why),
        None => Fetched::unreachable(got.why),
    }
}

