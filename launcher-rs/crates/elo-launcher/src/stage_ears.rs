//! The stage's ears and its eyes on the game: what the agent can wait on
//! besides chat.
//!
//! Operator, 2026-10-29: *"lets make our setup best"*, after a demo of a
//! character that "talks with the streamer, reads the chat and decides what
//! to say on its own." Chat was already here; this adds the other two inputs.
//!
//! - **Ears.** The desk listens to the owner's microphone (only while they
//!   switch it on), cuts each thing they say at a pause, and posts it here as
//!   a WAV. It is turned into text by the engine the owner chose: ElevenLabs'
//!   speech-to-text with the key the voice already uses, or any command on
//!   this machine (a local Whisper, say). The text is the owner's words, kept
//!   apart from viewers': the agent reads it as the person it works with.
//!   When the owner starts talking over a line, the line stops (`hush`).
//! - **The game.** The agent's wallet (`agent` in the config) followed on the
//!   island floor (`/api/agents/floor`, public, cached by the origin): on and
//!   off the island, its place on the board, what its body is doing. Any
//!   program on this machine can add its own (`POST /event`, `elo stage
//!   event`): a game, a bot, a script.
//!
//! Like the voice command, the hearing command is set by the CLI alone:
//! nothing that talks to the hub chooses what runs. Audio goes to the engine
//! and nowhere else; the hub keeps none of it.

use crate::stage::{self, Shared};
use elo_net::Net;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The most a posted utterance may be: 16 kHz, 16-bit mono runs 32 KB a
/// second, so this is about two minutes, well past the desk's 15 s cut.
pub const EARS_MAX: usize = 4 * 1024 * 1024;
/// Shorter than this is a cough or a click, not speech.
pub const MIN_SECONDS: f32 = 0.35;
pub const DEFAULT_STT_MODEL: &str = "scribe_v1";
const TEXT_MAX: usize = 400;
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
/// How often the floor is read. The origin caches it 15 s.
const FOLLOW_EVERY: Duration = Duration::from_secs(20);
/// A body's goal changes often; the agent hears of it at most this often.
const GOAL_EVERY: Duration = Duration::from_secs(60);

/// Which engine turns speech into text.
#[derive(Debug, Clone, PartialEq)]
pub enum Engine {
    None,
    Command(Vec<String>),
    ElevenLabs { model: String },
}

/// The engine the config on disk names (`hearing`, set by the CLI).
pub fn engine(cfg: &Value) -> Engine {
    if let Some(argv) = cfg.pointer("/hearing/cmd").and_then(Value::as_array) {
        let argv: Vec<String> = argv.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        if !argv.is_empty() {
            return Engine::Command(argv);
        }
    }
    if let Some(e) = cfg.pointer("/hearing/elevenlabs") {
        let model = e.get("model").and_then(Value::as_str)
            .filter(|m| crate::stage_voice::id_ok(m)).unwrap_or(DEFAULT_STT_MODEL);
        return Engine::ElevenLabs { model: model.into() };
    }
    Engine::None
}

/// What a page may know: which engine, whether the owner switched the ears
/// on, and whether the agent's wallet is followed. Never the command.
pub fn public(state: &Path, cfg: &Value) -> Value {
    let kind = match engine(cfg) {
        Engine::None => "none",
        Engine::Command(_) => "command",
        Engine::ElevenLabs { .. } => "elevenlabs",
    };
    json!({
        "engine": kind,
        "on": cfg.pointer("/ears/on").and_then(Value::as_bool).unwrap_or(false),
        "elevenlabs_key": crate::stage_voice::key(state).is_some(),
        "follow": following(cfg).is_some(),
    })
}

/// The wallet to follow on the floor, if the owner switched it on.
pub fn following(cfg: &Value) -> Option<String> {
    if cfg.pointer("/game/follow").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    cfg.get("agent").and_then(Value::as_str).filter(|a| a.len() == 42).map(str::to_lowercase)
}

/// A WAV's length in seconds, from its header; `None` if it isn't one.
pub fn wav_seconds(b: &[u8]) -> Option<f32> {
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as u32;
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    // Walk the chunks: `fmt ` gives the rate, `data` the length.
    let (mut i, mut rate, mut block) = (12usize, 0u32, 0u32);
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32_at(i + 4) as usize;
        if id == b"fmt " && i + 24 <= b.len() {
            rate = u32_at(i + 12);
            block = u16_at(i + 20);
        } else if id == b"data" {
            if rate == 0 || block == 0 {
                return None;
            }
            let len = len.min(b.len() - (i + 8));
            return Some(len as f32 / (rate * block) as f32);
        }
        i += 8 + len + (len & 1);
    }
    None
}

/// Turn one utterance into text.
pub fn transcribe(state: &Path, engine: &Engine, wav: &[u8]) -> Result<String, String> {
    let secs = wav_seconds(wav).ok_or("that isn't a WAV")?;
    if secs < MIN_SECONDS {
        return Err("too short to be speech".into());
    }
    let raw = match engine {
        Engine::None => return Err("no hearing engine: `elo stage ears elevenlabs` or `elo stage ears -- <command…>`".into()),
        Engine::Command(argv) => run_command(argv, wav)?,
        Engine::ElevenLabs { model } => {
            let key = crate::stage_voice::key(state)
                .ok_or("no ElevenLabs key: add one on the desk or with `elo stage voice elevenlabs`")?;
            elevenlabs(&Net::new(), &key, model, wav)?
        }
    };
    Ok(stage::clean_text(&raw, TEXT_MAX))
}

/// ElevenLabs speech-to-text: one multipart POST, the text back.
pub fn elevenlabs(net: &Net, key: &str, model: &str, wav: &[u8]) -> Result<String, String> {
    let boundary = format!("elo-stage-{:016x}", stage::rand_u64());
    let mut body = Vec::with_capacity(wav.len() + 512);
    let field = |body: &mut Vec<u8>, name: &str, value: &str| {
        body.extend_from_slice(format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ).as_bytes());
    };
    field(&mut body, "model_id", model);
    field(&mut body, "tag_audio_events", "false");
    body.extend_from_slice(format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"said.wav\"\r\n\
         Content-Type: audio/wav\r\n\r\n"
    ).as_bytes());
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let url = format!("{}/v1/speech-to-text", crate::stage_voice::api());
    let ctype = format!("multipart/form-data; boundary={boundary}");
    let got = net.request_bytes("POST", &url, &[("xi-api-key", key), ("accept", "application/json")],
                                Some((&ctype, body)), 1 << 20);
    let bytes = got.value.clone().ok_or_else(|| crate::stage_voice::explain(&got))?;
    let doc: Value = serde_json::from_slice(&bytes).map_err(|e| format!("ElevenLabs sent something odd: {e}"))?;
    doc.get("text").and_then(Value::as_str).map(String::from).ok_or_else(|| "ElevenLabs sent no text".into())
}

/// The owner's command: the WAV at `{in}` if an argument names it, else on
/// stdin; the text on stdout. No shell.
pub fn run_command(argv: &[String], wav: &[u8]) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("elo-ears-{}-{:016x}.wav", std::process::id(), stage::rand_u64()));
    let to_file = argv.iter().any(|a| a.contains("{in}"));
    if to_file {
        stage::write_private(&path, wav).map_err(|e| format!("couldn't write the utterance: {e}"))?;
    }
    let args: Vec<String> = argv.iter().map(|a| a.replace("{in}", &path.to_string_lossy())).collect();
    let started = Instant::now();
    let result = (|| {
        let mut child = std::process::Command::new(&args[0])
            .args(&args[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("the hearing command didn't start ({}): {e}", args[0]))?;
        let stdin = child.stdin.take();
        let feed = if to_file { Vec::new() } else { wav.to_vec() };
        let writer = std::thread::spawn(move || {
            if let Some(mut s) = stdin {
                let _ = s.write_all(&feed);
            }
        });
        let mut stdout = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(o) = stdout.as_mut() {
                let _ = o.take(64 * 1024).read_to_end(&mut buf);
            }
            buf
        });
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() > RUN_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("the hearing command took over 30 s".to_string());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(e.to_string()),
            }
        }
        let _ = writer.join();
        Ok(String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned())
    })();
    if to_file {
        let _ = std::fs::remove_file(&path);
    }
    result
}

/// Hear one utterance and hand it to the agent. Blocking; the caller is a
/// connection's own thread, never the hub's lock.
pub fn hear(hub: &Shared, wav: &[u8]) -> Value {
    let (state, cfg) = match hub.lock() {
        Ok(h) => (h.state_dir(), h.config()),
        Err(_) => return json!({"t": "error", "why": "the stage is down"}),
    };
    if cfg.pointer("/ears/on").and_then(Value::as_bool) != Some(true) {
        return json!({"t": "error", "why": "the ears are off: switch them on at the desk"});
    }
    match transcribe(&state, &engine(&cfg), wav) {
        Ok(text) if text.is_empty() => json!({"t": "ack", "text": ""}),
        Ok(text) => {
            let mut reply = stage::feed_in(hub, "owner", "", &text);
            reply["text"] = json!(text);
            reply
        }
        Err(why) => json!({"t": "error", "why": why}),
    }
}

// ── the game, followed on the floor ──────────────────────────────────────────

/// What changed between two reads of one agent's floor row, as lines the
/// agent reads. `was` is the last row (or null on the first read), `now` this
/// one (null when the floor doesn't list the wallet), `board` the first
/// board's name.
pub fn floor_changes(was: &Value, now: &Value, board: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let on = |r: &Value| r["on_island"].as_bool() == Some(true);
    if was.is_null() {
        if now.is_null() || !on(now) {
            lines.push("Your body isn't on the island right now.".into());
        } else {
            let place = now["place"].as_u64().map(|p| format!(", #{p} on {board}")).unwrap_or_default();
            lines.push(format!("Your body is on the island{place}."));
        }
        return lines;
    }
    if on(now) && !on(was) {
        lines.push("Your body is back on the island.".into());
    } else if !on(now) && on(was) {
        lines.push("Your body left the island.".into());
    }
    match (was["place"].as_u64(), now["place"].as_u64()) {
        (Some(a), Some(b)) if b < a => lines.push(format!("You moved up to #{b} on {board} (score {}).", now["score"])),
        (Some(a), Some(b)) if b > a => lines.push(format!("You slipped to #{b} on {board} (score {}).", now["score"])),
        (None, Some(b)) => lines.push(format!("You made {board} at #{b}.")),
        (Some(_), None) if on(now) => lines.push(format!("You dropped off {board}.")),
        _ => {}
    }
    lines
}

/// What the body is doing, if it isn't what the agent was last told:
/// (the goal, the line).
pub fn goal_line(now: &Value, told: Option<&str>) -> Option<(String, String)> {
    let g = now.pointer("/now/goal").and_then(Value::as_str)?;
    if told == Some(g) {
        return None;
    }
    let why = now.pointer("/now/reason").and_then(Value::as_str)
        .map(|r| format!(" ({})", stage::clean_text(r, 120))).unwrap_or_default();
    Some((g.to_string(), format!("Your body is now {}{why}.", goal_words(g))))
}

/// A goal as words: `gather_wood` → chopping wood, `craft:axe` → crafting axe.
pub fn goal_words(goal: &str) -> String {
    if let Some((verb, what)) = goal.split_once(':') {
        let lead = match verb {
            "craft" => "crafting",
            "go" => "heading to",
            "research" => "researching",
            "build" => "building",
            "loot" => "looting",
            "buy" => "buying",
            "sell" => "selling",
            "give" => "giving",
            v => return format!("{} {what}", v.replace('_', " ")),
        };
        return format!("{lead} {what}");
    }
    match goal {
        "gather_ore" => "gathering ore",
        "gather_wood" => "chopping wood",
        "gather_stone" => "mining stone",
        "gather_hemp" => "gathering hemp",
        "explore" => "exploring",
        "loot" => "looting",
        "go_home" => "heading home",
        "fight" => "fighting",
        "flee" => "running",
        "hunt" => "hunting",
        "build" => "building",
        "raid" => "raiding",
        "eat" => "eating",
        "drink" => "drinking",
        "heal" => "healing",
        "follow" => "following a friend",
        "warm" => "warming up by a fire",
        "cook" => "cooking",
        "defend" => "defending the base",
        g => return g.replace('_', " "),
    }
    .into()
}

/// Follow one wallet on the floor until the settings change.
pub fn run_follow(hub: Shared, wallet: String, gen: u64, current: Arc<AtomicU64>) {
    let net = Net::new();
    let url = format!("{}/api/agents/floor", stage::origin());
    let alive = || current.load(Ordering::SeqCst) == gen;
    let mut was = Value::Null;
    let mut told: Option<String> = None;
    let mut told_at: Option<Instant> = None;
    stage::game_status(&hub, json!({"wallet": wallet, "state": "connecting"}));
    while alive() {
        let got = net.get_json_quick(&url);
        match got.value {
            Some(floor) if floor["agents"].is_array() => {
                let row = floor["agents"].as_array().into_iter().flatten()
                    .find(|r| r["wallet"].as_str() == Some(wallet.as_str())).cloned().unwrap_or(Value::Null);
                let board = floor.pointer("/standings/board").and_then(Value::as_str).unwrap_or("the board").to_string();
                if !alive() {
                    return;
                }
                for l in floor_changes(&was, &row, &board) {
                    stage::feed_in(&hub, "game", "the island", &l);
                }
                // A goal that changes while paced is told on a later read.
                if told_at.is_none_or(|t| t.elapsed() >= GOAL_EVERY) {
                    if let Some((g, line)) = goal_line(&row, told.as_deref()) {
                        stage::feed_in(&hub, "game", "the island", &line);
                        told = Some(g);
                        told_at = Some(Instant::now());
                    }
                }
                stage::game_status(&hub, json!({"wallet": wallet, "state": "following",
                                                "on_island": row["on_island"]}));
                was = row;
            }
            Some(floor) => {
                let why = floor.pointer("/roster/why").and_then(Value::as_str).unwrap_or("the floor can't say who is on");
                stage::game_status(&hub, json!({"wallet": wallet, "state": "retrying", "why": why}));
            }
            None => {
                stage::game_status(&hub, json!({"wallet": wallet, "state": "retrying",
                                                "why": format!("couldn't read the floor: {}", got.why)}));
            }
        }
        let until = Instant::now() + FOLLOW_EVERY;
        while alive() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}
