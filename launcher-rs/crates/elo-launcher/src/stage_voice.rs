//! The stage's voices: a command on this machine, ElevenLabs, or the page's.
//!
//! Operator, 2026-10-29: *"make sure we make it easy to hook up elevenlabs."*
//!
//! ElevenLabs is built in, so hooking it up is a key and a pick from your own
//! voice list, on the desk or with `elo stage voice elevenlabs`. The call goes
//! from this machine to ElevenLabs and nowhere else. The key is kept here,
//! 0600 (or read from `ELEVENLABS_API_KEY`), sent only to ElevenLabs, and never
//! handed to a page, a driver or the agent: the desk can replace it, never
//! read it. ElevenLabs bills by the character, so every character sent is
//! counted per day, and an optional daily cap falls back to the page's own
//! voice once it is spent.
//!
//! **The mouth.** Operator, 2026-10-29: *"do the lip sync next."* ElevenLabs
//! says when each character of a line is spoken (`with-timestamps`), so a
//! line carries a mouth: a timeline of mouth shapes the stage follows on the
//! audio's own clock ([`mouth`]). A voice that gives no times gets one read
//! from the text, laid out evenly and stretched to the line's length by the
//! page ([`mouth_from_text`]).

use crate::stage;
use elo_net::Net;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const DEFAULT_MODEL: &str = "eleven_flash_v2_5";
/// Offered on the desk, fastest first. Any id ElevenLabs takes is accepted;
/// these are the ones worth a click.
pub const MODELS: [(&str, &str); 5] = [
    ("eleven_flash_v2_5", "Flash v2.5: fastest, cheapest, 32 languages"),
    ("eleven_v4_turbo", "v4 Turbo: real-time and expressive"),
    ("eleven_v4", "v4: most expressive, slower"),
    ("eleven_multilingual_v2", "Multilingual v2: steady on long lines"),
    ("eleven_v3", "v3: dramatic delivery"),
];
const AUDIO_MAX: u64 = 24 * 1024 * 1024;

pub fn api() -> String {
    // Overridable so the suite can stand a fake ElevenLabs on loopback.
    std::env::var("ELEVENLABS_API_BASE")
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "https://api.elevenlabs.io".into())
        .trim_end_matches('/')
        .to_string()
}

/// Which voice speaks a line.
#[derive(Debug, Clone, PartialEq)]
pub enum Voice {
    /// The stage page speaks it with the browser's voice.
    Page,
    Command(Vec<String>),
    ElevenLabs { voice_id: String, model: String, settings: Value, daily_chars: u64 },
}

/// The voice the config on disk names. Read per line, so `elo stage voice`
/// takes effect on a running stage with no restart.
pub fn of(cfg: &Value) -> Voice {
    if let Some(argv) = cfg.pointer("/voice/cmd").and_then(Value::as_array) {
        let argv: Vec<String> = argv.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        if !argv.is_empty() {
            return Voice::Command(argv);
        }
    }
    if let Some(e) = cfg.pointer("/voice/elevenlabs") {
        if let Some(id) = e.get("voice_id").and_then(Value::as_str).filter(|i| id_ok(i)) {
            return Voice::ElevenLabs {
                voice_id: id.to_string(),
                model: e.get("model").and_then(Value::as_str).filter(|m| id_ok(m)).unwrap_or(DEFAULT_MODEL).into(),
                settings: e.get("settings").cloned().unwrap_or_else(|| json!({})),
                daily_chars: e.get("daily_chars").and_then(Value::as_u64).unwrap_or(0),
            };
        }
    }
    Voice::Page
}

/// What a page may know about the voice: which kind, and for ElevenLabs the
/// voice's name and today's count. Never the key, never the command.
pub fn public(state: &Path, cfg: &Value) -> Value {
    let mut v = public_kind(state, cfg);
    v["models"] = json!(MODELS.iter().map(|(id, about)| json!({"id": id, "about": about})).collect::<Vec<_>>());
    v["key"] = json!(key(state).is_some());
    v
}

fn public_kind(state: &Path, cfg: &Value) -> Value {
    match of(cfg) {
        Voice::Page => json!({"kind": "page"}),
        Voice::Command(_) => json!({"kind": "command"}),
        Voice::ElevenLabs { model, daily_chars, settings, voice_id } => json!({
            "kind": "elevenlabs",
            "voice_id": voice_id,
            "voice_name": cfg.pointer("/voice/elevenlabs/voice_name").cloned().unwrap_or(Value::Null),
            "model": model,
            "settings": settings,
            "daily_chars": daily_chars,
            "used_today": used_today(state),
        }),
    }
}

/// An id as ElevenLabs writes them: letters, digits, `_`.
pub fn id_ok(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

// ── the key ──────────────────────────────────────────────────────────────────

pub fn key_path(state: &Path) -> PathBuf {
    stage::dir(state).join("elevenlabs.key")
}

/// `ELEVENLABS_API_KEY` wins; else the one saved here.
pub fn key(state: &Path) -> Option<String> {
    std::env::var("ELEVENLABS_API_KEY")
        .ok()
        .or_else(|| std::fs::read_to_string(key_path(state)).ok())
        .map(|k| k.trim().to_string())
        .filter(|k| key_ok(k))
}

pub fn key_ok(k: &str) -> bool {
    (16..=200).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_graphic())
}

pub fn save_key(state: &Path, k: &str) -> Result<(), String> {
    let k = k.trim();
    if !key_ok(k) {
        return Err("that doesn't look like an ElevenLabs API key".into());
    }
    std::fs::create_dir_all(stage::dir(state)).map_err(|e| e.to_string())?;
    stage::write_private(&key_path(state), k.as_bytes()).map_err(|e| e.to_string())
}

// ── the calls ────────────────────────────────────────────────────────────────

pub fn explain(got: &elo_net::Fetched<Vec<u8>>) -> String {
    match got.status {
        Some(401) => "ElevenLabs refused the key (401): check it on elevenlabs.io → API keys".into(),
        Some(402) | Some(429) if got.why.contains("quota") || got.status == Some(402) => {
            "ElevenLabs says the account is out of characters".into()
        }
        Some(429) => "ElevenLabs is rate-limiting this key; lines fall back to the page's voice".into(),
        _ if !got.reachable => format!("couldn't reach ElevenLabs: {}", got.why),
        _ => got.why.chars().take(300).collect(),
    }
}

/// Your voices, as `[{voice_id, name, category, about}]`: the ones on your
/// ElevenLabs account, premade, cloned and from the library.
pub fn voices(net: &Net, key: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut token = String::new();
    for _ in 0..5 {
        let mut url = format!("{}/v2/voices?page_size=100&include_total_count=false", api());
        if !token.is_empty() {
            url.push_str(&format!("&next_page_token={token}"));
        }
        let got = net.request_bytes("GET", &url, &[("xi-api-key", key), ("accept", "application/json")], None, 8 << 20);
        let bytes = got.value.clone().ok_or_else(|| explain(&got))?;
        let doc: Value = serde_json::from_slice(&bytes).map_err(|e| format!("ElevenLabs sent something odd: {e}"))?;
        for v in doc.get("voices").and_then(Value::as_array).cloned().unwrap_or_default() {
            let Some(id) = v.get("voice_id").and_then(Value::as_str).filter(|i| id_ok(i)) else { continue };
            let labels = v.get("labels").and_then(Value::as_object);
            let about = labels
                .map(|l| l.values().filter_map(Value::as_str).take(4).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            out.push(json!({
                "voice_id": id,
                "name": stage::clean_text(v.get("name").and_then(Value::as_str).unwrap_or(id), 60),
                "category": v.get("category").and_then(Value::as_str).unwrap_or(""),
                "about": stage::clean_text(&about, 80),
            }));
        }
        match doc.get("next_page_token").and_then(Value::as_str).filter(|t| doc["has_more"] == true && !t.is_empty()) {
            Some(t) if t.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_=".contains(&b)) => token = t.into(),
            _ => break,
        }
    }
    Ok(out)
}

/// One line, spoken, with when each character is said: MP3 and the mouth.
/// `Err` with the status when ElevenLabs refused, so the caller can fall
/// back to [`speak`] for a model that gives no times.
pub fn speak_timed(net: &Net, key: &str, voice_id: &str, model: &str, settings: &Value, text: &str)
                   -> Result<(Vec<u8>, Value), (Option<u16>, String)> {
    let url = format!("{}/v1/text-to-speech/{voice_id}/with-timestamps?output_format=mp3_44100_128", api());
    let got = net.request_bytes(
        "POST",
        &url,
        &[("xi-api-key", key), ("accept", "application/json")],
        Some(("application/json", tts_body(model, settings, text).to_string().into_bytes())),
        AUDIO_MAX * 2,
    );
    let bytes = got.value.clone().ok_or_else(|| (got.status, explain(&got)))?;
    let doc: Value = serde_json::from_slice(&bytes).map_err(|e| (None, format!("ElevenLabs sent something odd: {e}")))?;
    let audio = doc["audio_base64"].as_str().and_then(base64).ok_or((None, "ElevenLabs sent no audio".to_string()))?;
    if audio.len() < 64 {
        return Err((None, "ElevenLabs sent no audio".into()));
    }
    // `alignment` is the text as sent; `normalized_alignment` as spoken
    // (numbers read out), which is the one the mouth should follow.
    let a = if doc["normalized_alignment"]["characters"].is_array() { &doc["normalized_alignment"] } else { &doc["alignment"] };
    let chars: Vec<String> = a["characters"].as_array().into_iter().flatten()
        .map(|c| c.as_str().unwrap_or(" ").to_string()).collect();
    let times = |k: &str| -> Vec<f64> { a[k].as_array().into_iter().flatten().map(|v| v.as_f64().unwrap_or(0.0)).collect() };
    Ok((audio, mouth(&chars, &times("character_start_times_seconds"), &times("character_end_times_seconds"))))
}

fn tts_body(model: &str, settings: &Value, text: &str) -> Value {
    let mut body = json!({"text": text, "model_id": model});
    let pick = |k: &str, lo: f64, hi: f64| settings.get(k).and_then(Value::as_f64).map(|v| v.clamp(lo, hi));
    let mut vs = serde_json::Map::new();
    for (k, lo, hi) in [("stability", 0.0, 1.0), ("similarity_boost", 0.0, 1.0), ("style", 0.0, 1.0), ("speed", 0.7, 1.2)] {
        if let Some(v) = pick(k, lo, hi) {
            vs.insert(k.into(), json!(v));
        }
    }
    if !vs.is_empty() {
        body["voice_settings"] = Value::Object(vs);
    }
    body
}

/// Standard base64, padding and whitespace ignored; `None` on a stray byte.
pub fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in s.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ── the mouth ────────────────────────────────────────────────────────────────

/// The most cues a line's mouth carries; a 600-character line needs fewer.
const MOUTH_MAX: usize = 1200;

/// The mouth shape a character makes, and how open: `aa ih ou ee oh` are the
/// five a VRM carries, `mm` is lips closed (m, b, p), and "" is at rest.
pub fn shape(c: char) -> (&'static str, f64) {
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'a' | 'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'æ' => ("aa", 1.0),
        'e' | 'é' | 'è' | 'ê' | 'ë' => ("ee", 0.8),
        'i' | 'í' | 'ì' | 'î' | 'ï' | 'y' => ("ih", 0.7),
        'o' | 'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => ("oh", 0.9),
        'u' | 'ú' | 'ù' | 'û' | 'ü' | 'w' | 'q' => ("ou", 0.8),
        'm' | 'b' | 'p' => ("mm", 0.0),
        'f' | 'v' => ("ih", 0.2),
        'r' | 'l' => ("oh", 0.3),
        c if c.is_alphabetic() => ("ee", 0.3),
        c if c.is_numeric() => ("aa", 0.5),
        _ => ("", 0.0),
    }
}

/// A line's mouth from when each character is said: `{"cues": [[start, end,
/// shape, open], …]}` in seconds, a run of one shape merged into one cue.
pub fn mouth(chars: &[String], starts: &[f64], ends: &[f64]) -> Value {
    let n = chars.len().min(starts.len()).min(ends.len());
    let mut cues: Vec<(f64, f64, &str, f64)> = Vec::new();
    for i in 0..n {
        let Some(c) = chars[i].chars().next() else { continue };
        let (s, e) = (starts[i].max(0.0), ends[i].max(starts[i]));
        if !s.is_finite() || !e.is_finite() {
            continue;
        }
        let (sh, w) = shape(c);
        match cues.last_mut() {
            Some(last) if last.2 == sh && (last.3 - w).abs() < 1e-9 && s - last.1 < 0.05 => last.1 = e,
            _ => cues.push((s, e, sh, w)),
        }
    }
    cues.truncate(MOUTH_MAX);
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    json!({"cues": cues.iter().map(|(s, e, sh, w)| json!([r(*s), r(*e), sh, w])).collect::<Vec<_>>()})
}

/// A mouth read from the text alone, for a voice that says nothing of its
/// timing: each character an even step, a pause a longer one, and `scale`
/// set so the page stretches it to however long the line turns out to be.
pub fn mouth_from_text(text: &str) -> Value {
    let (mut chars, mut starts, mut ends) = (Vec::new(), Vec::new(), Vec::new());
    let mut t = 0.0;
    for c in text.chars() {
        let step = match c {
            ',' | ';' | ':' => 0.25,
            '.' | '!' | '?' | '…' => 0.4,
            ' ' => 0.06,
            _ => 0.066,
        };
        chars.push(c.to_string());
        starts.push(t);
        t += step;
        ends.push(t);
    }
    let mut m = mouth(&chars, &starts, &ends);
    m["scale"] = json!(true);
    m["length"] = json!((t * 1000.0).round() / 1000.0);
    m
}

/// One line, spoken. MP3 back.
pub fn speak(net: &Net, key: &str, voice_id: &str, model: &str, settings: &Value, text: &str) -> Result<Vec<u8>, String> {
    let url = format!("{}/v1/text-to-speech/{voice_id}?output_format=mp3_44100_128", api());
    let body = tts_body(model, settings, text);
    let got = net.request_bytes(
        "POST",
        &url,
        &[("xi-api-key", key), ("accept", "audio/mpeg")],
        Some(("application/json", body.to_string().into_bytes())),
        AUDIO_MAX,
    );
    let bytes = got.value.clone().ok_or_else(|| explain(&got))?;
    if bytes.len() < 64 {
        return Err("ElevenLabs sent no audio".into());
    }
    Ok(bytes)
}

// ── counting characters ──────────────────────────────────────────────────────

fn today() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

fn usage_path(state: &Path) -> PathBuf {
    stage::dir(state).join("elevenlabs-usage.json")
}

/// Characters sent to ElevenLabs today (UTC).
pub fn used_today(state: &Path) -> u64 {
    std::fs::read(usage_path(state))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(|v| v["day"].as_u64() == Some(today()))
        .and_then(|v| v["chars"].as_u64())
        .unwrap_or(0)
}

pub fn add_usage(state: &Path, chars: u64) -> u64 {
    let total = used_today(state) + chars;
    let _ = std::fs::write(usage_path(state), json!({"day": today(), "chars": total}).to_string());
    total
}

/// Speak `text` with `voice`: audio bytes and their media type, or why not
/// (and the page speaks it instead).
pub fn render(state: &Path, voice: &Voice, text: &str) -> Result<(Vec<u8>, &'static str), String> {
    render_timed(state, voice, text).map(|(b, t, _)| (b, t))
}

/// [`render`], and the line's mouth: ElevenLabs' own timing, else one read
/// from the text for the page to stretch.
pub fn render_timed(state: &Path, voice: &Voice, text: &str) -> Result<(Vec<u8>, &'static str, Value), String> {
    match voice {
        Voice::Page => Err("the page speaks it".into()),
        Voice::Command(argv) => stage::run_voice(argv, text).map(|b| {
            let t = sniff(&b);
            (b, t, mouth_from_text(text))
        }),
        Voice::ElevenLabs { voice_id, model, settings, daily_chars } => {
            let k = key(state).ok_or("no ElevenLabs key: add one on the desk or with `elo stage voice elevenlabs`")?;
            let n = text.chars().count() as u64;
            if *daily_chars > 0 && used_today(state) + n > *daily_chars {
                return Err(format!("today's ElevenLabs cap ({daily_chars} characters) is spent"));
            }
            let net = Net::new();
            let (bytes, m) = match speak_timed(&net, &k, voice_id, model, settings, text) {
                Ok(got) => got,
                // A refusal of the timed call that isn't about the key or the
                // quota (a model without times, say): the plain call, no times.
                Err((Some(code), _)) if !matches!(code, 401 | 402 | 429) => {
                    (speak(&net, &k, voice_id, model, settings, text)?, mouth_from_text(text))
                }
                Err((_, why)) => return Err(why),
            };
            add_usage(state, n);
            Ok((bytes, "audio/mpeg", m))
        }
    }
}

/// The media type of audio bytes, from their first bytes.
pub fn sniff(b: &[u8]) -> &'static str {
    if b.starts_with(b"RIFF") {
        "audio/wav"
    } else if b.starts_with(b"ID3") || (b.len() > 1 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0) {
        "audio/mpeg"
    } else if b.starts_with(b"OggS") {
        "audio/ogg"
    } else if b.starts_with(b"fLaC") {
        "audio/flac"
    } else {
        "application/octet-stream"
    }
}
