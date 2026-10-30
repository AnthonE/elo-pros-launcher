//! Viewers talking to the agent on stage: where chat comes from, and what
//! reaches the agent.
//!
//! Operator, 2026-10-29: *"do the chat piece next so viewers can talk to her."*
//!
//! Chat arrives two ways. **Twitch is read here**, anonymously over Twitch's own
//! chat socket, straight from this machine: a channel name and nothing else, no
//! account, no key. **Everything else is a POST** to the hub (`/chat`), so any
//! tool that already relays YouTube, Kick or TikTok chat can hand it over, and
//! the desk has a box to type as a viewer.
//!
//! Every message goes through one [`judge`] before the agent can see it: the
//! owner's blocked words and viewers, links, a trigger word, a cooldown per
//! viewer, chat bots and their commands. A message the owner holds for
//! approval waits on the desk. What passes is still a stranger's words; the
//! agent's tool says so with every read, and a viewer's text is never shown
//! on stage except quoted beside the agent's own reply to it.

use crate::stage::{self, Shared};
use serde_json::{json, Value};
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::Message;

pub const CHAT_TEXT_MAX: usize = 300;
pub const MODES: [&str; 3] = ["open", "approve", "off"];
/// Chat bots whose lines are never a viewer talking. Matched on the login.
pub const BOTS: [&str; 8] = [
    "nightbot", "streamelements", "fossabot", "moobot", "streamlabs", "wizebot", "sery_bot", "botrixoficial",
];
const TWITCH_URL: &str = "wss://irc-ws.chat.twitch.tv:443";

/// What the owner set on the desk, read out of the stage config's `chat`.
#[derive(Debug, Clone)]
pub struct Filter {
    pub mode: String,
    pub trigger: String,
    pub cooldown: u64,
    pub block_words: Vec<String>,
    pub block_users: Vec<String>,
    pub links: bool,
    pub bots: bool,
}

impl Filter {
    pub fn from_cfg(cfg: &Value) -> Filter {
        let c = cfg.get("chat").cloned().unwrap_or_else(|| json!({}));
        let list = |k: &str| -> Vec<String> {
            c.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty()).collect())
                .unwrap_or_default()
        };
        Filter {
            mode: c.get("mode").and_then(Value::as_str).filter(|m| MODES.contains(m)).unwrap_or("open").into(),
            trigger: c.get("trigger").and_then(Value::as_str).unwrap_or("").trim().to_lowercase(),
            cooldown: c.get("cooldown").and_then(Value::as_u64).unwrap_or(15),
            block_words: list("block_words"),
            block_users: list("block_users"),
            links: c.get("links").and_then(Value::as_bool).unwrap_or(false),
            bots: c.get("bots").and_then(Value::as_bool).unwrap_or(true),
        }
    }

    /// A blocked word in `text`, if any. Words match inside other words on
    /// purpose: a list of slurs should catch them with a suffix on.
    pub fn blocked_word(&self, text: &str) -> Option<&str> {
        // `1` and `|` stand in for both i and l, so the text is read both ways.
        let (as_i, as_l) = (fold(text, 'i'), fold(text, 'l'));
        self.block_words
            .iter()
            .find(|w| {
                let w = fold(w, 'i');
                !w.is_empty() && (as_i.contains(&w) || as_l.contains(&w))
            })
            .map(String::as_str)
    }
}

/// Lower case, and the commonest stand-ins (0 for o, $ for s, 1 for `one`…)
/// put back, so a blocked word is not one keystroke from getting through.
fn fold(s: &str, one: char) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            '0' => 'o', '1' | '|' => one, '!' => 'i', '3' => 'e', '4' | '@' => 'a', '5' | '$' => 's', '7' => 't',
            c => c,
        })
        .filter(|c| c.is_alphanumeric())
        .collect()
}

pub fn has_link(text: &str) -> bool {
    let t = text.to_lowercase();
    if t.contains("http://") || t.contains("https://") || t.contains("www.") {
        return true;
    }
    // A bare domain: a word with a dot and a known-shaped ending.
    t.split_whitespace().any(|w| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        match w.rsplit_once('.') {
            Some((head, tld)) => {
                !head.is_empty() && (2..=6).contains(&tld.len()) && tld.chars().all(|c| c.is_ascii_lowercase())
                    && head.chars().any(|c| c.is_ascii_alphabetic())
            }
            None => false,
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Through, with the text the agent sees and the state it starts in.
    Keep { text: String, state: &'static str },
    Drop(&'static str),
}

/// One message, judged. `last` is when this viewer last got a message
/// through; mods and the broadcaster skip the cooldown and the trigger.
pub fn judge(f: &Filter, from: &str, badge: &str, text: &str, last: Option<u64>, now: u64) -> Verdict {
    if f.mode == "off" {
        return Verdict::Drop("chat is off");
    }
    let text = stage::clean_text(text, CHAT_TEXT_MAX);
    if text.is_empty() {
        return Verdict::Drop("empty");
    }
    let login = from.trim().to_lowercase();
    if f.block_users.contains(&login) {
        return Verdict::Drop("a blocked viewer");
    }
    if f.bots && BOTS.contains(&login.as_str()) {
        return Verdict::Drop("a chat bot");
    }
    if f.blocked_word(&text).is_some() || f.blocked_word(from).is_some() {
        return Verdict::Drop("a blocked word");
    }
    if !f.links && has_link(&text) {
        return Verdict::Drop("a link");
    }
    let staff = badge == "broadcaster" || badge == "mod";
    let mut text = text;
    if !f.trigger.is_empty() {
        // Found on the text itself, never on a lower-cased copy: lower-casing
        // can change a string's byte length, and a stranger chooses the text.
        match find_ci(&text, &f.trigger) {
            Some(at) => {
                // Take the trigger out, so "!ask what's the plan" reads as the question.
                let before = text[..at].trim_end();
                let after = text[at + f.trigger.len()..].trim_start_matches([',', ':', ' ']);
                text = stage::clean_text(&format!("{before} {after}"), CHAT_TEXT_MAX);
                if text.is_empty() {
                    return Verdict::Drop("only the trigger");
                }
            }
            None if staff => {}
            None => return Verdict::Drop("no trigger"),
        }
    } else if text.starts_with('!') {
        return Verdict::Drop("a bot command");
    }
    if !staff {
        if let Some(t) = last {
            if now.saturating_sub(t) < f.cooldown {
                return Verdict::Drop("cooldown");
            }
        }
    }
    Verdict::Keep { text, state: if f.mode == "approve" { "held" } else { "new" } }
}

/// Where `needle` (already lower case) starts in `hay`, ignoring case.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    hay.char_indices().map(|(i, _)| i).find(|&i| {
        let end = i + needle.len();
        end <= hay.len() && hay.is_char_boundary(end) && hay[i..end].to_lowercase() == needle
    })
}

/// A Twitch channel name as Twitch allows them.
pub fn twitch_channel(s: &str) -> Option<String> {
    let s = s.trim().trim_start_matches('#').trim_start_matches("https://www.twitch.tv/")
        .trim_start_matches("https://twitch.tv/").trim_end_matches('/').to_lowercase();
    let ok = (3..=25).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    ok.then_some(s)
}

// ── Twitch, read anonymously ─────────────────────────────────────────────────

/// One IRC line: `@tags :prefix COMMAND params :trailing`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Irc {
    pub tags: Vec<(String, String)>,
    pub prefix: String,
    pub command: String,
    pub params: Vec<String>,
}

impl Irc {
    pub fn tag(&self, k: &str) -> Option<&str> {
        self.tags.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
    /// The login, from `:login!login@login.tmi.twitch.tv`.
    pub fn login(&self) -> &str {
        self.prefix.split('!').next().unwrap_or("")
    }
}

pub fn parse_irc(line: &str) -> Option<Irc> {
    let mut rest = line.trim_end_matches(['\r', '\n']);
    if rest.is_empty() {
        return None;
    }
    let mut irc = Irc::default();
    if let Some(r) = rest.strip_prefix('@') {
        let (tags, r) = r.split_once(' ')?;
        irc.tags = tags
            .split(';')
            .map(|t| {
                let (k, v) = t.split_once('=').unwrap_or((t, ""));
                (k.to_string(), unescape_tag(v))
            })
            .collect();
        rest = r;
    }
    if let Some(r) = rest.strip_prefix(':') {
        let (p, r) = r.split_once(' ').unwrap_or((r, ""));
        irc.prefix = p.to_string();
        rest = r;
    }
    let (head, trailing) = match rest.split_once(" :") {
        Some((h, t)) => (h, Some(t)),
        None => (rest, None),
    };
    let mut words = head.split(' ').filter(|w| !w.is_empty());
    irc.command = words.next()?.to_string();
    irc.params = words.map(String::from).collect();
    if let Some(t) = trailing {
        irc.params.push(t.to_string());
    }
    Some(irc)
}

/// IRCv3 tag values escape `;`, space, `\`, CR and LF.
fn unescape_tag(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// The badge that matters for chat: broadcaster, mod, vip, sub, or none.
pub fn badge_of(irc: &Irc) -> &'static str {
    let badges = irc.tag("badges").unwrap_or("");
    let has = |b: &str| badges.split(',').any(|x| x.split('/').next() == Some(b));
    if has("broadcaster") {
        "broadcaster"
    } else if has("moderator") || irc.tag("mod") == Some("1") {
        "mod"
    } else if has("vip") {
        "vip"
    } else if has("subscriber") || has("founder") {
        "sub"
    } else {
        ""
    }
}

/// Read one channel until `current` moves past `gen` (the channel changed or
/// chat was turned off). Reconnects with a backoff; every change of state is
/// reported to the desk.
pub fn run_twitch(hub: Shared, channel: String, gen: u64, current: Arc<AtomicU64>) {
    let mut wait = Duration::from_secs(2);
    while current.load(Ordering::SeqCst) == gen {
        stage::chat_status(&hub, "twitch", json!({"channel": channel, "state": "connecting"}));
        let started = Instant::now();
        let why = match read_twitch(&hub, &channel, gen, &current) {
            Ok(()) => return,
            Err(why) => why,
        };
        if current.load(Ordering::SeqCst) != gen {
            return;
        }
        if started.elapsed() > Duration::from_secs(60) {
            wait = Duration::from_secs(2);
        }
        stage::chat_status(&hub, "twitch", json!({"channel": channel, "state": "retrying", "why": why}));
        let until = Instant::now() + wait;
        while Instant::now() < until && current.load(Ordering::SeqCst) == gen {
            std::thread::sleep(Duration::from_millis(200));
        }
        wait = (wait * 2).min(Duration::from_secs(60));
    }
}

fn read_twitch(hub: &Shared, channel: &str, gen: u64, current: &AtomicU64) -> Result<(), String> {
    let tcp = std::net::ToSocketAddrs::to_socket_addrs(&"irc-ws.chat.twitch.tv:443")
        .map_err(|e| format!("can't find Twitch's chat: {e}"))?
        .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(8)).ok())
        .ok_or("can't reach Twitch's chat")?;
    tcp.set_read_timeout(Some(Duration::from_secs(1))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let name = rustls::pki_types::ServerName::try_from("irc-ws.chat.twitch.tv").map_err(|e| e.to_string())?;
    let conn = rustls::ClientConnection::new(elo_hive::relay::tls_config(), name).map_err(|e| e.to_string())?;
    let tls = rustls::StreamOwned::new(conn, tcp);
    let limits = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(64 * 1024))
        .max_frame_size(Some(64 * 1024));
    let (mut ws, _) = tungstenite::client::client_with_config(TWITCH_URL, tls, Some(limits))
        .map_err(|e| format!("Twitch's chat refused the connection: {e}"))?;
    // Anonymous read: the `justinfan` nick is Twitch's own convention for a
    // reader with no account, and it can read any public channel.
    let nick = format!("justinfan{}", 10000 + stage::rand_u64() % 80000);
    for line in [
        "CAP REQ :twitch.tv/tags twitch.tv/commands".to_string(),
        "PASS SCHMOOPIIE".to_string(),
        format!("NICK {nick}"),
        format!("JOIN #{channel}"),
    ] {
        ws.send(Message::text(line)).map_err(|e| e.to_string())?;
    }
    let mut joined = false;
    let mut last_heard = Instant::now();
    while current.load(Ordering::SeqCst) == gen {
        match ws.read() {
            Ok(Message::Text(t)) => {
                last_heard = Instant::now();
                for line in t.as_str().split("\r\n") {
                    let Some(irc) = parse_irc(line) else { continue };
                    match irc.command.as_str() {
                        "PING" => {
                            let arg = irc.params.last().cloned().unwrap_or_default();
                            ws.send(Message::text(format!("PONG :{arg}"))).map_err(|e| e.to_string())?;
                        }
                        "JOIN" | "366" | "ROOMSTATE" if !joined => {
                            joined = true;
                            stage::chat_status(hub, "twitch", json!({"channel": channel, "state": "reading"}));
                        }
                        "PRIVMSG" => {
                            let from = irc.tag("display-name").filter(|n| !n.is_empty())
                                .unwrap_or_else(|| irc.login()).to_string();
                            let text = irc.params.last().cloned().unwrap_or_default();
                            // `/me` actions arrive wrapped in CTCP.
                            let text = text.strip_prefix("\u{1}ACTION ").map(|t| t.trim_end_matches('\u{1}'))
                                .unwrap_or(&text).to_string();
                            stage::chat_in(hub, "twitch", &from, irc.login(), badge_of(&irc), &text,
                                           irc.tag("id"));
                        }
                        // A mod deleted one message, or timed out or banned a
                        // viewer: the agent must not answer what Twitch took down.
                        "CLEARMSG" => stage::chat_forget(hub, irc.tag("target-msg-id"), None),
                        "CLEARCHAT" => {
                            let who = irc.params.get(1).map(String::as_str);
                            stage::chat_forget(hub, None, who.or(Some("*")));
                        }
                        "RECONNECT" => return Err("Twitch asked for a reconnect".into()),
                        "NOTICE" => {
                            let why = irc.params.last().cloned().unwrap_or_default();
                            stage::chat_status(hub, "twitch", json!({"channel": channel, "state": "notice", "why": why}));
                        }
                        _ => {}
                    }
                }
            }
            Ok(Message::Close(_)) => return Err("Twitch closed the connection".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                // Twitch pings about every five minutes; silence well past that is a dead line.
                if last_heard.elapsed() > Duration::from_secs(400) {
                    return Err("Twitch went quiet".into());
                }
            }
            Err(e) => return Err(format!("the chat connection dropped: {e}")),
        }
    }
    let _ = ws.close(None);
    Ok(())
}
