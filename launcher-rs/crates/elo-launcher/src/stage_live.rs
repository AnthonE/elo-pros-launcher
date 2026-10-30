//! Go live everywhere: one stream from OBS, sent on to every platform, and a
//! copy kept on disk.
//!
//! Operator, 2026-10-29: *"we might need a restream rip for in house so we can
//! push our vtuber everywhere. even X."*
//!
//! OBS streams to this machine (`rtmp://127.0.0.1:1936/live`, a key of its
//! own), and ffmpeg copies that one stream, untouched, to each destination the
//! owner switched on: Twitch, YouTube, Kick, X, TikTok, Facebook, pump.fun, or any RTMP
//! url. No re-encode, so it costs no CPU, only upload: one copy of the stream
//! per destination. A recording rides along as a Matroska file (it survives a
//! crash mid-stream) for posting later. When the upload can't carry several
//! copies, Elo's relay takes one and sends the rest (`meter/relay.py`): OBS
//! points there instead, and none of this runs.
//!
//! **Stream keys are the platforms' passwords.** They are kept in
//! `stage/live.json` (0600), handed to ffmpeg on its command line (readable by
//! this user's own processes, as everything of theirs is), and never sent to a
//! page: the desk sees the last four characters. ffmpeg's own error lines name
//! the url, key included, so they are never shown; the destination they belong
//! to is found by its number and the reason is kept without the url.
//!
//! **Going live is the owner's act.** The agent's `stage` tool has no verb for
//! it: what goes out, where, and when, is set on the desk or the CLI.

use crate::stage;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const INGEST_PORT: u16 = 1936;
const DEST_MAX: usize = 12;

/// The places a stream can go, with the server url each one gives you, when
/// it is the same for everyone. The rest show theirs in the stream settings.
pub const PLATFORMS: [(&str, &str, &str, &str); 8] = [
    ("twitch", "Twitch", "rtmp://live.twitch.tv/app", "dashboard.twitch.tv → Settings → Stream → Primary Stream key"),
    ("youtube", "YouTube", "rtmp://a.rtmp.youtube.com/live2", "studio.youtube.com → Create → Go live → Stream key"),
    ("kick", "Kick", "", "kick.com → Settings → Stream URL & Key: both"),
    ("x", "X", "", "studio.x.com → Producer → Create source (RTMP): both"),
    ("tiktok", "TikTok", "", "TikTok LIVE Producer → Server URL and Stream key"),
    ("facebook", "Facebook", "rtmps://live-api-s.facebook.com:443/rtmp/", "facebook.com/live/producer → Stream key"),
    ("pumpfun", "pump.fun", "", "your coin's page on pump.fun → Start livestream → RTMP: both (go live there first)"),
    ("custom", "Somewhere else", "", "an rtmp:// or rtmps:// url and its key"),
];

fn path(state: &Path) -> PathBuf {
    stage::dir(state).join("live.json")
}

pub fn recordings_dir(state: &Path) -> PathBuf {
    stage::dir(state).join("recordings")
}

pub fn load(state: &Path) -> Value {
    let mut v = std::fs::read(path(state))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if v["dests"].as_array().is_none() {
        v["dests"] = json!([]);
    }
    if v["ingest_key"].as_str().is_none_or(|k| k.len() != 24) {
        v["ingest_key"] = json!(format!("{:016x}{:08x}", stage::rand_u64(), stage::rand_u64() as u32));
        let _ = save(state, &v);
    }
    if v["record"].as_bool().is_none() {
        v["record"] = json!(true);
    }
    v
}

pub fn save(state: &Path, v: &Value) -> Result<(), String> {
    std::fs::create_dir_all(stage::dir(state)).map_err(|e| e.to_string())?;
    stage::write_private(&path(state), serde_json::to_string_pretty(v).unwrap_or_default().as_bytes())
        .map_err(|e| e.to_string())
}

fn url_ok(u: &str) -> bool {
    (u.starts_with("rtmp://") || u.starts_with("rtmps://"))
        && u.len() <= 300
        && !u.chars().any(|c| c.is_whitespace() || c.is_control() || "|[]'\"\\".contains(c))
}

fn key_ok(k: &str) -> bool {
    (1..=300).contains(&k.len()) && !k.chars().any(|c| c.is_whitespace() || c.is_control() || "|[]'\"\\".contains(c))
}

/// Add or change one destination. A missing `key` on an edit keeps the old.
pub fn set_dest(state: &Path, patch: &Value) -> Result<Value, String> {
    let mut v = load(state);
    let platform = patch["platform"].as_str().unwrap_or("custom");
    let preset = PLATFORMS.iter().find(|p| p.0 == platform).ok_or("pick a platform")?;
    let url = patch["url"].as_str().map(str::trim).filter(|u| !u.is_empty()).unwrap_or(preset.2);
    if !url_ok(url) {
        return Err(format!("{}: the server url starts rtmp:// or rtmps:// ({})", preset.1, preset.3));
    }
    let name = stage::clean_text(patch["name"].as_str().unwrap_or(preset.1), 40);
    let id = patch["id"].as_str().filter(|i| i.len() <= 20 && i.bytes().all(|b| b.is_ascii_alphanumeric()));
    let dests = v["dests"].as_array_mut().ok_or("bad live.json")?;
    let at = id.and_then(|i| dests.iter().position(|d| d["id"] == json!(i)));
    let key = match patch["key"].as_str().map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) if key_ok(k) => k.to_string(),
        Some(_) => return Err("that stream key has a character a url can't carry".into()),
        None => match at {
            Some(i) => dests[i]["key"].as_str().unwrap_or("").to_string(),
            None => return Err(format!("{}: paste the stream key ({})", preset.1, preset.3)),
        },
    };
    let on = patch["on"].as_bool().unwrap_or(true);
    let row = json!({"id": id.map(String::from).unwrap_or_else(|| format!("{:08x}", stage::rand_u64() as u32)),
                     "platform": platform, "name": name, "url": url, "key": key, "on": on});
    match at {
        Some(i) => dests[i] = row,
        None if dests.len() >= DEST_MAX => return Err(format!("{DEST_MAX} destinations at most")),
        None => dests.push(row),
    }
    if let Some(r) = patch["record"].as_bool() {
        v["record"] = json!(r);
    }
    save(state, &v)?;
    Ok(public(state, &v, &Live::default()))
}

pub fn remove_dest(state: &Path, id: &str) -> Result<(), String> {
    let mut v = load(state);
    if let Some(d) = v["dests"].as_array_mut() {
        d.retain(|x| x["id"] != json!(id));
    }
    save(state, &v)
}

/// What the desk may see: everything but the keys, which show their end.
pub fn public(state: &Path, v: &Value, live: &Live) -> Value {
    let dests: Vec<Value> = v["dests"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let key = d["key"].as_str().unwrap_or("");
            let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            let id = d["id"].as_str().unwrap_or("");
            json!({"id": id, "platform": d["platform"], "name": d["name"], "url": d["url"],
                   "key_hint": if key.is_empty() { String::new() } else { format!("••••{tail}") },
                   "on": d["on"], "state": live.dest_state.get(id).cloned().unwrap_or(Value::Null)})
        })
        .collect();
    json!({
        "armed": live.armed,
        "state": live.state,
        "since": live.since,
        "bitrate": live.bitrate,
        "dests": dests,
        "record": v["record"],
        "obs": {"server": format!("rtmp://127.0.0.1:{INGEST_PORT}/live"), "key": v["ingest_key"]},
        "ffmpeg": ffmpeg().is_some(),
        "platforms": PLATFORMS.iter().map(|p| json!({"id": p.0, "name": p.1, "url": p.2, "where": p.3})).collect::<Vec<_>>(),
        "recordings": recordings(state),
    })
}

pub fn recordings(state: &Path) -> Vec<Value> {
    let mut out: Vec<(std::time::SystemTime, Value)> = std::fs::read_dir(recordings_dir(state))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let meta = e.metadata().ok()?;
            (name.ends_with(".mkv") && meta.is_file()).then(|| {
                (meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                 json!({"file": name, "bytes": meta.len()}))
            })
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.0));
    out.into_iter().take(30).map(|(_, v)| v).collect()
}

/// ffmpeg on PATH, or `ELO_FFMPEG`.
pub fn ffmpeg() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ELO_FFMPEG").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    std::env::var_os("PATH")
        .and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(exe)).find(|p| p.is_file()))
}

/// The ffmpeg argv: listen for OBS on loopback, copy to each destination
/// that is on, and to the recording. Returns the argv and, in tee order,
/// which destination id each output is ("rec" for the recording).
pub fn argv(state: &Path, v: &Value, recording: Option<&Path>) -> Result<(Vec<String>, Vec<String>), String> {
    let mut outs = Vec::new();
    let mut order = Vec::new();
    for d in v["dests"].as_array().into_iter().flatten().filter(|d| d["on"] == true) {
        let (url, key) = (d["url"].as_str().unwrap_or(""), d["key"].as_str().unwrap_or(""));
        if !url_ok(url) || !key_ok(key) {
            continue;
        }
        outs.push(format!("[f=flv:onfail=ignore]{}/{}", url.trim_end_matches('/'), key));
        order.push(d["id"].as_str().unwrap_or("").to_string());
    }
    if let Some(rec) = recording {
        let p = rec.to_string_lossy();
        if p.chars().any(|c| "|[]".contains(c)) {
            return Err("the recordings folder's path has a character ffmpeg's tee can't take".into());
        }
        outs.push(format!("[f=matroska:onfail=ignore]{p}"));
        order.push("rec".into());
    }
    if outs.is_empty() {
        return Err("switch on a destination, or the recording".into());
    }
    let _ = state;
    let ingest = format!("rtmp://127.0.0.1:{INGEST_PORT}/live/{}", v["ingest_key"].as_str().unwrap_or("live"));
    let args = vec![
        "-hide_banner", "-nostdin", "-loglevel", "warning", "-stats_period", "2", "-progress", "pipe:1",
        "-listen", "1", "-i", &ingest, "-map", "0", "-c", "copy", "-f", "tee", &outs.join("|"),
    ]
    .into_iter()
    .map(String::from)
    .collect();
    Ok((args, order))
}

/// One ffmpeg error line, as the desk may see it: which output and why, with
/// the url (and so the key) cut out.
pub fn failure(line: &str) -> Option<(usize, String)> {
    // "[tee @ 0x..] Slave muxer #2 failed: Connection refused, continuing with 3/4 slaves."
    let at = line.find("Slave muxer #")?;
    let rest = &line[at + "Slave muxer #".len()..];
    let (n, rest) = rest.split_once(' ')?;
    let n: usize = n.parse().ok()?;
    let why = rest.strip_prefix("failed: ").unwrap_or(rest);
    let why = why.split(", continuing").next().unwrap_or(why).trim();
    Some((n, stage::clean_text(why, 120)))
}

#[derive(Default)]
pub struct Live {
    pub armed: bool,
    pub state: String,
    pub since: u64,
    pub bitrate: String,
    pub dest_state: std::collections::HashMap<String, Value>,
    pub child: Option<Child>,
}

pub type Shared = Arc<Mutex<Live>>;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Arm: ffmpeg listens for OBS, and listens again after each stream ends,
/// until `stop`. `tell` is called with the public state on every change.
pub fn start(state: PathBuf, live: Shared, tell: Arc<dyn Fn(Value) + Send + Sync>) -> Result<(), String> {
    let exe = ffmpeg().ok_or("ffmpeg isn't installed: winget install ffmpeg · brew install ffmpeg · apt install ffmpeg")?;
    {
        let mut l = live.lock().unwrap_or_else(|p| p.into_inner());
        if l.armed {
            return Ok(());
        }
        // Check the settings before arming, so a mistake is said at once.
        let v = load(&state);
        let rec = (v["record"] == true).then(|| PathBuf::from("check.mkv"));
        argv(&state, &v, rec.as_deref())?;
        l.armed = true;
        l.state = "waiting for OBS".into();
    }
    std::thread::spawn(move || {
        while live.lock().map(|l| l.armed).unwrap_or(false) {
            let v = load(&state);
            let rec = (v["record"] == true).then(|| {
                let _ = std::fs::create_dir_all(recordings_dir(&state));
                recordings_dir(&state).join(format!("stage-{}.mkv", stamp()))
            });
            let (args, order) = match argv(&state, &v, rec.as_deref()) {
                Ok(a) => a,
                Err(why) => {
                    let mut l = live.lock().unwrap_or_else(|p| p.into_inner());
                    l.armed = false;
                    l.state = why;
                    let snap = public(&state, &v, &l);
                    drop(l);
                    tell(snap);
                    return;
                }
            };
            let child = Command::new(&exe).args(&args).stdin(Stdio::null()).stdout(Stdio::piped())
                .stderr(Stdio::piped()).spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    let mut l = live.lock().unwrap_or_else(|p| p.into_inner());
                    l.armed = false;
                    l.state = format!("ffmpeg didn't start: {e}");
                    let snap = public(&state, &v, &l);
                    drop(l);
                    tell(snap);
                    return;
                }
            };
            let (out, err) = (child.stdout.take(), child.stderr.take());
            {
                let mut l = live.lock().unwrap_or_else(|p| p.into_inner());
                l.child = Some(child);
                l.state = "waiting for OBS".into();
                l.dest_state.clear();
                l.bitrate.clear();
                let snap = public(&state, &v, &l);
                drop(l);
                tell(snap);
            }
            // stderr: per-output failures, mapped to their destination.
            let (live2, state2, tell2, order2) = (Arc::clone(&live), state.clone(), Arc::clone(&tell), order.clone());
            let errs = std::thread::spawn(move || {
                for line in BufReader::new(err.unwrap_or_else(|| unreachable!())).lines().map_while(Result::ok) {
                    if let Some((n, why)) = failure(&line) {
                        let mut l = live2.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(id) = order2.get(n) {
                            l.dest_state.insert(id.clone(), json!({"ok": false, "why": why}));
                        }
                        let snap = public(&state2, &load(&state2), &l);
                        drop(l);
                        tell2(snap);
                    }
                }
            });
            // stdout: `-progress`, which starts once OBS is sending.
            let mut told_live = false;
            let mut size: u64 = 0;
            for line in BufReader::new(out.unwrap_or_else(|| unreachable!())).lines().map_while(Result::ok) {
                if let Some(b) = line.strip_prefix("total_size=") {
                    size = b.trim().parse().unwrap_or(size);
                }
                // A copy reports no bitrate of its own: bytes over time it is.
                if let Some(t) = line.strip_prefix("out_time_us=") {
                    let us: u64 = t.trim().parse().unwrap_or(0);
                    let mut l = live.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(kbps) = (size * 8 * 1000).checked_div(us) {
                        l.bitrate = format!("{kbps} kbps");
                    }
                    if !told_live {
                        told_live = true;
                        l.state = "live".into();
                        l.since = now();
                        for id in &order {
                            l.dest_state.entry(id.clone()).or_insert(json!({"ok": true}));
                        }
                        let snap = public(&state, &v, &l);
                        drop(l);
                        tell(snap);
                    }
                }
            }
            let _ = errs.join();
            let mut l = live.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(mut c) = l.child.take() {
                let _ = c.wait();
            }
            l.state = if l.armed { "waiting for OBS".into() } else { "off".into() };
            l.bitrate.clear();
            let snap = public(&state, &load(&state), &l);
            drop(l);
            tell(snap);
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    Ok(())
}

pub fn stop(live: &Shared) {
    let mut l = live.lock().unwrap_or_else(|p| p.into_inner());
    l.armed = false;
    l.state = "off".into();
    if let Some(c) = l.child.as_mut() {
        let _ = c.kill();
    }
}

fn stamp() -> String {
    // UTC, sortable, and safe in a file name.
    let s = now();
    let (d, rem) = (s / 86_400, s % 86_400);
    let (y, m, dd) = civil(d as i64);
    format!("{y:04}-{m:02}-{dd:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 to (year, month, day), Howard Hinnant's algorithm.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
