//! YouTube and Kick chat for the stage, each through the platform's official
//! door.
//!
//! Operator, 2026-10-29: *"do youtube and kick chat natively too. also yea if
//! we need to have some serverside stuff for this thats fine."*
//!
//! **YouTube** is read from this machine with the YouTube Data API and the
//! streamer's own API key (Google Cloud → APIs → YouTube Data API v3 → a key):
//! free, and the quota is theirs. Finding the live stream costs one unit: the
//! channel's public feed lists its recent videos and one `videos.list` asks
//! which is live. Reading costs five units a poll, so polls are at least ten
//! seconds apart and a daily budget stops before Google does; the desk says
//! what is left.
//!
//! **Kick** is read through our server. Kick's official chat is webhooks to a
//! registered app, which a desktop cannot receive, so the origin holds one Elo
//! Pros Kick app, subscribes to the channel, checks Kick's signature on every
//! message, and this machine long-polls `/api/stage/kick/<channel>` for them
//! (`meter/stagechat.py`). No account and no key on this side.

use crate::stage::{self, Shared};
use elo_net::Net;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Overridable so the suite can stand a fake YouTube on loopback.
fn yt_api() -> String {
    std::env::var("YOUTUBE_API_BASE").ok().filter(|b| !b.is_empty())
        .unwrap_or_else(|| "https://www.googleapis.com/youtube/v3".into())
}
/// Five units a poll; ten seconds apart is about 1,800 units an hour.
const YT_POLL_FLOOR: Duration = Duration::from_secs(10);
/// Google's free quota is 10,000 units a day; stop a little before it.
pub const YT_DEFAULT_BUDGET: u64 = 9_500;

/// `YOUTUBE_POLL_FLOOR_MS` lowers it for the suite; nothing else should.
fn poll_floor() -> Duration {
    std::env::var("YOUTUBE_POLL_FLOOR_MS").ok().and_then(|v| v.parse().ok())
        .map(Duration::from_millis).unwrap_or(YT_POLL_FLOOR)
}

fn alive(current: &AtomicU64, gen: u64) -> bool {
    current.load(Ordering::SeqCst) == gen
}

/// Sleep up to `d`, waking early if the reader was replaced.
fn nap(d: Duration, current: &AtomicU64, gen: u64) {
    let end = std::time::Instant::now() + d;
    while std::time::Instant::now() < end && alive(current, gen) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ── YouTube ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YtTarget {
    Video(String),
    Channel(String),
    Handle(String),
}

fn video_id_ok(s: &str) -> bool {
    s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// What the streamer typed: a video or live url, an 11-character video id, a
/// channel url (`/channel/UC…`), or a handle (`@name`, or its url).
pub fn youtube_target(raw: &str) -> Option<YtTarget> {
    let s = raw.trim();
    let path = s
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    if let Some(rest) = path.strip_prefix("youtu.be/") {
        let id = rest.split(['?', '/', '&']).next().unwrap_or("");
        return video_id_ok(id).then(|| YtTarget::Video(id.into()));
    }
    if let Some(rest) = path.strip_prefix("youtube.com/") {
        if let Some(q) = rest.strip_prefix("watch?") {
            let id = q.split('&').find_map(|p| p.strip_prefix("v=")).unwrap_or("");
            return video_id_ok(id).then(|| YtTarget::Video(id.into()));
        }
        for pre in ["live/", "shorts/"] {
            if let Some(r) = rest.strip_prefix(pre) {
                let id = r.split(['?', '/']).next().unwrap_or("");
                return video_id_ok(id).then(|| YtTarget::Video(id.into()));
            }
        }
        if let Some(r) = rest.strip_prefix("channel/") {
            let id = r.split(['?', '/']).next().unwrap_or("");
            let ok = id.len() == 24 && id.starts_with("UC")
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            return ok.then(|| YtTarget::Channel(id.into()));
        }
        if let Some(r) = rest.strip_prefix('@') {
            return handle(r.split(['?', '/']).next().unwrap_or(""));
        }
        return None;
    }
    if let Some(h) = s.strip_prefix('@') {
        return handle(h);
    }
    if s.len() == 24 && s.starts_with("UC") {
        return youtube_target(&format!("youtube.com/channel/{s}"));
    }
    if video_id_ok(s) {
        return Some(YtTarget::Video(s.into()));
    }
    handle(s)
}

fn handle(h: &str) -> Option<YtTarget> {
    let ok = (3..=30).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    ok.then(|| YtTarget::Handle(h.to_string()))
}

pub fn youtube_key_path(state: &Path) -> PathBuf {
    stage::dir(state).join("youtube.key")
}

/// `YOUTUBE_API_KEY` wins; else the one saved here.
pub fn youtube_key(state: &Path) -> Option<String> {
    std::env::var("YOUTUBE_API_KEY")
        .ok()
        .or_else(|| std::fs::read_to_string(youtube_key_path(state)).ok())
        .map(|k| k.trim().to_string())
        .filter(|k| youtube_key_ok(k))
}

pub fn youtube_key_ok(k: &str) -> bool {
    (20..=100).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn save_youtube_key(state: &Path, k: &str) -> Result<(), String> {
    let k = k.trim();
    if !youtube_key_ok(k) {
        return Err("that doesn't look like a Google API key".into());
    }
    std::fs::create_dir_all(stage::dir(state)).map_err(|e| e.to_string())?;
    stage::write_private(&youtube_key_path(state), k.as_bytes()).map_err(|e| e.to_string())
}

fn day() -> u64 {
    // Google's quota day turns at midnight Pacific; UTC−8 is close enough to
    // never count a day twice.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().saturating_sub(8 * 3600) / 86_400)
        .unwrap_or(0)
}

fn units_path(state: &Path) -> PathBuf {
    stage::dir(state).join("youtube-units.json")
}

pub fn units_today(state: &Path) -> u64 {
    std::fs::read(units_path(state))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(|v| v["day"].as_u64() == Some(day()))
        .and_then(|v| v["units"].as_u64())
        .unwrap_or(0)
}

fn spend(state: &Path, units: u64) {
    let total = units_today(state) + units;
    let _ = std::fs::write(units_path(state), json!({"day": day(), "units": total}).to_string());
}

fn yt_get(net: &Net, url: &str) -> Result<Value, (Option<u16>, String)> {
    let got = net.request_bytes("GET", url, &[("accept", "application/json")], None, 4 << 20);
    match got.value {
        Some(b) => serde_json::from_slice(&b).map_err(|e| (got.status, e.to_string())),
        None => {
            // Google's error body names the reason; the key in the url is not repeated.
            let reason = got.why.split("\"reason\": \"").nth(1).and_then(|r| r.split('"').next())
                .or_else(|| got.why.split("\"reason\":\"").nth(1).and_then(|r| r.split('"').next()))
                .unwrap_or("")
                .to_string();
            let why = match (got.status, reason.as_str()) {
                (_, "quotaExceeded") | (_, "dailyLimitExceeded") => "Google's quota for today is spent".into(),
                (_, "keyInvalid") | (Some(400), "badRequest") if got.why.contains("API key") => {
                    "Google refused the API key".into()
                }
                (_, "accessNotConfigured") => "turn on \"YouTube Data API v3\" for that key's project".into(),
                (_, "liveChatEnded") => "the stream ended".into(),
                (_, "liveChatNotFound") => "that stream has no live chat".into(),
                (_, "liveChatDisabled") => "chat is off on that stream".into(),
                (None, _) => format!("couldn't reach YouTube: {}", got.why),
                (Some(c), r) => format!("YouTube answered {c}{}", if r.is_empty() { String::new() } else { format!(" ({r})") }),
            };
            Err((got.status, why))
        }
    }
}

/// The live chat to read, as `(video id, live chat id)`, or `None` when the
/// channel is not live right now.
pub fn resolve_youtube(net: &Net, state: &Path, key: &str, target: &YtTarget) -> Result<Option<(String, String)>, String> {
    let ids: Vec<String> = match target {
        YtTarget::Video(id) => vec![id.clone()],
        YtTarget::Channel(c) => recent_videos(net, c)?,
        YtTarget::Handle(h) => {
            let url = format!("{}/channels?part=id&forHandle=%40{h}&key={key}", yt_api());
            let v = yt_get(net, &url).map_err(|(_, w)| w)?;
            spend(state, 1);
            let c = v["items"][0]["id"].as_str().ok_or_else(|| format!("no YouTube channel @{h}"))?;
            recent_videos(net, c)?
        }
    };
    if ids.is_empty() {
        return Ok(None);
    }
    let url = format!("{}/videos?part=liveStreamingDetails&id={}&key={key}", yt_api(), ids.join(","));
    let v = yt_get(net, &url).map_err(|(_, w)| w)?;
    spend(state, 1);
    Ok(v["items"].as_array().into_iter().flatten().find_map(|it| {
        let d = &it["liveStreamingDetails"];
        let chat = d["activeLiveChatId"].as_str()?;
        d["actualEndTime"].is_null().then(|| (it["id"].as_str().unwrap_or("").to_string(), chat.to_string()))
    }))
}

/// The channel's public feed: its newest videos, a live one among them.
fn recent_videos(net: &Net, channel: &str) -> Result<Vec<String>, String> {
    let url = format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel}");
    let got = net.request_bytes("GET", &url, &[], None, 2 << 20);
    let body = got.value.ok_or_else(|| format!("couldn't read the channel's feed: {}", got.why))?;
    let text = String::from_utf8_lossy(&body);
    Ok(text
        .split("<yt:videoId>")
        .skip(1)
        .filter_map(|s| s.split('<').next())
        .filter(|id| video_id_ok(id))
        .take(15)
        .map(String::from)
        .collect())
}

fn yt_badge(a: &Value) -> &'static str {
    if a["isChatOwner"] == true {
        "broadcaster"
    } else if a["isChatModerator"] == true {
        "mod"
    } else if a["isChatSponsor"] == true {
        "sub"
    } else {
        ""
    }
}

/// One page of YouTube chat into the hub. Returns the ids it took in.
pub fn take_youtube(hub: &Shared, items: &[Value]) -> usize {
    let mut n = 0;
    for it in items {
        let s = &it["snippet"];
        let a = &it["authorDetails"];
        let from = a["displayName"].as_str().unwrap_or("");
        let login = a["channelId"].as_str().unwrap_or(from);
        match s["type"].as_str() {
            Some("textMessageEvent") | Some("superChatEvent") | Some("superStickerEvent") => {
                let text = s["textMessageDetails"]["messageText"].as_str()
                    .or_else(|| s["superChatDetails"]["userComment"].as_str())
                    .or_else(|| s["displayMessage"].as_str())
                    .unwrap_or("");
                stage::chat_in(hub, "youtube", from, login, yt_badge(a), text, it["id"].as_str());
                n += 1;
            }
            Some("messageDeletedEvent") => {
                stage::chat_forget(hub, s["messageDeletedDetails"]["deletedMessageId"].as_str(), None)
            }
            Some("userBannedEvent") => {
                if let Some(c) = s["userBannedDetails"]["bannedUserDetails"]["channelId"].as_str() {
                    stage::chat_forget(hub, None, Some(c));
                }
            }
            _ => {}
        }
    }
    n
}

pub fn run_youtube(hub: Shared, state: PathBuf, raw: String, budget: u64, gen: u64, current: Arc<AtomicU64>) {
    let status = |s: &str, why: Option<String>| {
        stage::chat_status(&hub, "youtube", json!({"channel": raw, "state": s, "why": why,
                                                   "units": units_today(&state), "budget": budget}));
    };
    let Some(target) = youtube_target(&raw) else {
        return status("stopped", Some("that isn't a YouTube channel, handle or video".into()));
    };
    let net = Net::new();
    while alive(&current, gen) {
        let Some(key) = youtube_key(&state) else {
            status("stopped", Some("add a YouTube API key on the desk".into()));
            return;
        };
        if units_today(&state) + 10 > budget {
            status("waiting", Some("today's YouTube quota budget is spent; it resets at midnight Pacific".into()));
            nap(Duration::from_secs(600), &current, gen);
            continue;
        }
        status("finding the live stream", None);
        let (video, chat) = match resolve_youtube(&net, &state, &key, &target) {
            Ok(Some(found)) => found,
            Ok(None) => {
                status("waiting", Some("not live yet; looking again every two minutes".into()));
                nap(Duration::from_secs(120), &current, gen);
                continue;
            }
            Err(why) => {
                status("retrying", Some(why));
                nap(Duration::from_secs(120), &current, gen);
                continue;
            }
        };
        let mut page: Option<String> = None;
        let mut first = true;
        loop {
            if !alive(&current, gen) {
                return;
            }
            if units_today(&state) + 5 > budget {
                status("waiting", Some("today's YouTube quota budget is spent; it resets at midnight Pacific".into()));
                break;
            }
            let mut url = format!(
                "{}/liveChat/messages?liveChatId={chat}&part=snippet,authorDetails&maxResults=200&key={key}",
                yt_api()
            );
            if let Some(p) = &page {
                url.push_str(&format!("&pageToken={p}"));
            }
            match yt_get(&net, &url) {
                Ok(v) => {
                    spend(&state, 5);
                    // The first page is the backlog: messages from before we
                    // looked are not for the agent to answer now.
                    if !first {
                        take_youtube(&hub, v["items"].as_array().map(Vec::as_slice).unwrap_or(&[]));
                    }
                    first = false;
                    status("reading", Some(format!("youtube.com/watch?v={video}")));
                    if v["offlineAt"].is_string() {
                        status("waiting", Some("the stream ended".into()));
                        break;
                    }
                    page = v["nextPageToken"].as_str().map(String::from);
                    let ms = v["pollingIntervalMillis"].as_u64().unwrap_or(10_000);
                    nap(Duration::from_millis(ms).max(poll_floor()), &current, gen);
                }
                Err((_, why)) => {
                    status("retrying", Some(why));
                    nap(Duration::from_secs(60), &current, gen);
                    break;
                }
            }
        }
    }
}

// ── Kick, through the origin ─────────────────────────────────────────────────

/// A Kick channel's slug as Kick writes them.
pub fn kick_slug(raw: &str) -> Option<String> {
    let s = raw.trim().trim_start_matches("https://").trim_start_matches("www.")
        .trim_start_matches("kick.com/").trim_end_matches('/').to_lowercase();
    let ok = (3..=25).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    ok.then_some(s)
}

pub fn run_kick(hub: Shared, slug: String, gen: u64, current: Arc<AtomicU64>) {
    let net = Net::new();
    let origin = stage::origin();
    let mut after = String::new();
    let mut wait = Duration::from_secs(2);
    stage::chat_status(&hub, "kick", json!({"channel": slug, "state": "connecting"}));
    while alive(&current, gen) {
        let url = format!("{origin}/api/stage/kick/{slug}?after={after}&wait=25");
        let got = net.get_json_told(&url);
        let v = match got.value {
            Some(v) if got.ok => v,
            Some(v) => {
                let why = v["error"].as_str().unwrap_or(&got.why).to_string();
                stage::chat_status(&hub, "kick", json!({"channel": slug, "state": "retrying", "why": why}));
                nap(wait, &current, gen);
                wait = (wait * 2).min(Duration::from_secs(120));
                continue;
            }
            None => {
                stage::chat_status(&hub, "kick", json!({"channel": slug, "state": "retrying",
                                                        "why": format!("couldn't reach {origin}: {}", got.why)}));
                nap(wait, &current, gen);
                wait = (wait * 2).min(Duration::from_secs(120));
                continue;
            }
        };
        wait = Duration::from_secs(2);
        if !alive(&current, gen) {
            return;
        }
        let first = after.is_empty();
        if let Some(c) = v["cursor"].as_str() {
            after = c.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect();
        }
        // The first answer only sets the cursor: no backlog for the agent.
        if !first {
            for m in v["messages"].as_array().into_iter().flatten() {
                stage::chat_in(&hub, "kick", m["from"].as_str().unwrap_or(""), m["login"].as_str().unwrap_or(""),
                               m["badge"].as_str().unwrap_or(""), m["text"].as_str().unwrap_or(""),
                               m["id"].as_str());
            }
        }
        for id in v["removed"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            stage::chat_forget(&hub, Some(id), None);
        }
        for who in v["banned"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            stage::chat_forget(&hub, None, Some(who));
        }
        stage::chat_status(&hub, "kick", json!({"channel": slug, "state": v["state"].as_str().unwrap_or("reading"),
                                                "why": v["why"]}));
    }
}
