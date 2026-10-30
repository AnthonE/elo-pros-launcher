//! `elo stage` — an agent's body on your screen, driven from this machine.
//!
//! Operator, 2026-10-28: *"i just wish hermes had a tcp connection and could
//! change what i see on the screen"* and *"the less load on our backend the
//! better and the more decentralized we can get."*
//!
//! So the stage is a hub on `127.0.0.1` and nothing else. It serves two pages
//! out of this binary: the **stage** (a transparent page OBS loads as one
//! browser source: the avatar, its captions, a card) and the **desk** (where a
//! person imports a model, picks a voice, and pokes the avatar by hand). An
//! agent drives it through `elo stage mcp`; anything else drives it through
//! the same websocket, a plain `POST /op`, or `elo stage say`. Models, voices
//! and audio stay on this machine. The origin sees none of it.
//!
//! ## What guards it, and what does not
//!
//! * **Bound to loopback only.** Nobody on the network reaches it.
//! * **A token in every url that does anything** (`stage/token`, 0600). The
//!   page code is public and needs none; state, models, audio and the socket
//!   do. The pages send `Referrer-Policy: no-referrer` so the token stays put.
//! * **Host and Origin are checked.** A web page you happen to have open
//!   cannot reach `127.0.0.1` through DNS rebinding (Host) or puppet the
//!   avatar from its own origin (Origin), even holding the token.
//! * **The voice command is set on the CLI only.** Nothing that speaks to the
//!   hub can choose what it runs; a driver supplies text, and text goes to the
//!   command on stdin, never into an argument or a shell.
//!
//! Besides chat, an agent can wait on two more inputs (`stage_ears.rs`): the
//! owner's own voice, heard through the desk's microphone while they switch
//! it on, and the game, its body followed on the island floor or reported by
//! any program here (`POST /event`). `stage` what=listen hands over whichever
//! comes first, the owner first.
//!
//! What it is NOT: a boundary against a process running as you. Anything with
//! your uid can read the token file, as it can read everything else you own.
//! What a driver can do is bounded by the op list below: pixels and sound on
//! your own stage. It signs nothing, posts nothing and spends nothing.

use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::protocol::{Role, WebSocketConfig};
use tungstenite::{Message, WebSocket};

pub const DEFAULT_PORT: u16 = 7357;

/// What a driver may ask for. The list is the whole of a driver's authority,
/// so it is a constant and a test pins it.
pub const OPS: [&str; 16] = ["hello", "say", "expression", "look", "layout", "card", "clear",
                              "spoken", "ready", "chat", "chat_read", "chat_mod",
                              "hush", "event", "feed_read", "move"];
pub const LAYOUTS: [&str; 5] = ["corner", "center", "full", "hidden", "brb"];
pub const LOOKS: [&str; 6] = ["viewer", "left", "right", "up", "down", "around"];
/// The six expressions every VRM 1.0 model carries. A PNG pack names its own.
pub const VRM_EXPRESSIONS: [&str; 6] = ["neutral", "happy", "angry", "sad", "relaxed", "surprised"];

pub const SAY_MAX: usize = 600;
const CARD_MAX: usize = 300;
const NAME_MAX: usize = 40;
const HEAD_MAX: usize = 16 * 1024;
const BODY_MAX: usize = 64 * 1024;
const MODEL_MAX: usize = 96 * 1024 * 1024;
const AUDIO_MAX: usize = 24 * 1024 * 1024;
const AUDIO_KEPT: usize = 12;
const CONN_MAX: usize = 64;
const VOICE_TIMEOUT: Duration = Duration::from_secs(30);

// ── the page, compiled in ────────────────────────────────────────────────────
//
// Served from the binary so the stage works offline and no file on disk can
// be swapped under it. `vendor.js` is three.js and three-vrm, bundled once
// from npm tarballs checked against the registry's own integrity hash
// (`stage/vendor/build.sh`); `tests/stage.rs` pins its sha256.

const ASSETS: [(&str, &str, &[u8]); 8] = [
    ("/", "text/html; charset=utf-8", include_bytes!("../stage/desk.html")),
    ("/stage", "text/html; charset=utf-8", include_bytes!("../stage/stage.html")),
    ("/stage.css", "text/css; charset=utf-8", include_bytes!("../stage/stage.css")),
    ("/stage.js", "text/javascript; charset=utf-8", include_bytes!("../stage/stage.js")),
    ("/avatar.js", "text/javascript; charset=utf-8", include_bytes!("../stage/avatar.js")),
    ("/desk.js", "text/javascript; charset=utf-8", include_bytes!("../stage/desk.js")),
    ("/link.js", "text/javascript; charset=utf-8", include_bytes!("../stage/link.js")),
    ("/vendor.js", "text/javascript; charset=utf-8", include_bytes!("../stage/vendor.js")),
];

pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    let path = if path == "/desk" { "/" } else { path };
    ASSETS.iter().find(|(p, _, _)| *p == path).map(|(_, t, b)| (*t, *b))
}

// ── where things live ────────────────────────────────────────────────────────

pub fn dir(state: &Path) -> PathBuf {
    state.join("stage")
}

/// The token every url that does anything carries. Made once, kept 0600, so
/// the url pasted into OBS keeps working across restarts.
pub fn token(state: &Path) -> std::io::Result<String> {
    let d = dir(state);
    std::fs::create_dir_all(&d)?;
    let path = d.join("token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim().to_string();
        if t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(t);
        }
    }
    let mut raw = [0u8; 16];
    getrandom::fill(&mut raw).map_err(|e| std::io::Error::other(e.to_string()))?;
    let t: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    write_private(&path, t.as_bytes())?;
    Ok(t)
}

pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(bytes)?;
    }
    std::fs::rename(&tmp, path)
}

pub fn load_config(state: &Path) -> Value {
    std::fs::read(dir(state).join("stage.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

pub fn save_config(state: &Path, cfg: &Value) -> std::io::Result<()> {
    std::fs::create_dir_all(dir(state))?;
    write_private(
        &dir(state).join("stage.json"),
        serde_json::to_string_pretty(cfg).unwrap_or_default().as_bytes(),
    )
}

/// A file name the hub will store and serve: plain, no path, no dotfile.
pub fn safe_file(name: &str) -> Option<String> {
    let n = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let ok = !n.is_empty()
        && n.len() <= 80
        && !n.starts_with('.')
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    ok.then(|| n.to_string())
}

fn model_ext(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    [".vrm", ".glb", ".vrma", ".png", ".webp", ".gif", ".jpg", ".jpeg"]
        .into_iter()
        .find(|e| lower.ends_with(e))
}

/// An expression's name as a pack or a driver writes it.
pub fn expr_name(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    let ok = !s.is_empty()
        && s.len() <= 24
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    ok.then_some(s)
}

/// Text from a driver, made fit to show: control characters out, length
/// capped on a character boundary. It is shown with `textContent`, never as
/// HTML, and it is somebody's words, never an instruction to the hub.
pub fn clean_text(s: &str, max: usize) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max {
        out = out.chars().take(max - 1).collect::<String>() + "…";
    }
    out
}

/// What the public half of the config looks like: everything a page needs and
/// nothing it should not see. The voice command is not in it.
pub fn public_config(cfg: &Value) -> Value {
    let mut out = Map::new();
    for k in ["name", "label", "agent", "browser_voice", "model", "accent", "chat"] {
        if let Some(v) = cfg.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    out.insert("voice".into(), json!(cfg.get("voice").is_some()));
    Value::Object(out)
}

// ── a model, imported ────────────────────────────────────────────────────────

/// Copy a model into the stage and make it the current one: a `.vrm`, a
/// rigged `.glb`, a folder with a `stage-pack.json`, or PNGs named for their
/// expressions (`neutral.png`, `happy.png`, …; the first is the default).
pub fn import(state: &Path, paths: &[PathBuf]) -> Result<Value, String> {
    let models = dir(state).join("models");
    std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    let mut cfg = load_config(state);
    if paths.len() == 1 && paths[0].is_dir() {
        let pack_dir = &paths[0];
        let raw = std::fs::read(pack_dir.join("stage-pack.json"))
            .map_err(|_| format!("{} has no stage-pack.json", pack_dir.display()))?;
        let pack: Value = serde_json::from_slice(&raw).map_err(|e| format!("stage-pack.json: {e}"))?;
        let mut model = pack_model(&pack, |f| {
            let src = pack_dir.join(f);
            copy_in(&src, &models)
        })?;
        bake_moves(&models, &mut model)?;
        for k in ["name", "label", "accent", "browser_voice"] {
            if let Some(v) = pack.get(k) {
                cfg[k] = v.clone();
            }
        }
        cfg["model"] = model;
    } else if paths.len() == 1 && model_ext(&paths[0].to_string_lossy()).is_some_and(|e| e == ".vrm" || e == ".glb") {
        let kind = if model_ext(&paths[0].to_string_lossy()) == Some(".glb") { "glb" } else { "vrm" };
        let f = copy_in(&paths[0], &models)?;
        cfg["model"] = json!({"kind": kind, "file": f});
    } else {
        let mut files = Map::new();
        let mut first = None;
        for p in paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if !matches!(model_ext(&name), Some(".png" | ".webp" | ".gif" | ".jpg" | ".jpeg")) {
                return Err(format!("{name}: a model is one .vrm, a pack folder, or pictures"));
            }
            let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
            let expr = expr_name(stem).ok_or_else(|| format!("{name}: name the file for its expression, like happy.png"))?;
            let f = copy_in(p, &models)?;
            first.get_or_insert_with(|| expr.clone());
            files.insert(expr, json!(f));
        }
        let default = first.ok_or("which files? — a .vrm, a pack folder, or pictures")?;
        cfg["model"] = json!({"kind": "png", "default": default, "files": files});
    }
    save_config(state, &cfg).map_err(|e| e.to_string())?;
    Ok(cfg["model"].clone())
}

/// A pack's model, its files fetched through `fetch` (which copies one and
/// returns the stored name). Shared by the CLI's import and the desk's.
pub fn pack_model(pack: &Value, mut fetch: impl FnMut(&str) -> Result<String, String>) -> Result<Value, String> {
    match pack.get("kind").and_then(Value::as_str) {
        Some("vrm") => {
            let f = pack.get("file").and_then(Value::as_str).ok_or("a vrm pack names its `file`")?;
            Ok(json!({"kind": "vrm", "file": fetch(f)?}))
        }
        // A rigged glTF that is no VRM (a Meshy or Mixamo export, say): the
        // page poses it, and `face` places the eyelids and mouth it draws over
        // a painted face, since such a model carries no face shapes of its own.
        Some("glb") => {
            let f = pack.get("file").and_then(Value::as_str).ok_or("a glb pack names its `file`")?;
            let mut out = json!({"kind": "glb", "file": fetch(f)?});
            if let Some(face) = pack.get("face") {
                out["face"] = glb_face(face)?;
            }
            for k in ["bones", "clips"] {
                if let Some(m) = pack.get(k) {
                    out[k] = glb_names(m).map_err(|why| format!("`{k}`: {why}"))?;
                }
            }
            // Clips from another file (a .glb or .vrma of moves), put on this
            // body by `bake_moves` once both files are in.
            if let Some(m) = pack.get("moves") {
                let f = m.as_str().filter(|f| matches!(model_ext(f), Some(".glb" | ".vrma")))
                    .ok_or("`moves` names a .glb or .vrma of clips")?;
                out["moves_from"] = json!(fetch(f)?);
            }
            Ok(out)
        }
        Some("png") => {
            let files = pack.get("files").and_then(Value::as_object).ok_or("a png pack has `files`")?;
            let mut out = Map::new();
            for (k, v) in files {
                let expr = expr_name(k).ok_or_else(|| format!("{k:?} isn't an expression name"))?;
                let f = v.as_str().ok_or("each file is a name")?;
                out.insert(expr, json!(fetch(f)?));
            }
            let default = pack
                .get("default")
                .and_then(Value::as_str)
                .and_then(expr_name)
                .filter(|d| out.contains_key(d))
                .or_else(|| out.keys().next().cloned())
                .ok_or("a png pack needs at least one picture")?;
            Ok(json!({"kind": "png", "default": default, "files": out}))
        }
        _ => Err("stage-pack.json: `kind` is vrm, glb or png".into()),
    }
}

/// A glb pack's moves, made its own: the clips its `clips` names from the
/// `moves` file are retargeted onto the model's bare skeleton (`elo_rig`), in
/// place, and kept beside it as `<model>.moves.glb`, which the page plays on
/// the model by node name. Done once, at import, on this machine. A name in
/// neither the model nor the moves file is refused with the names there are.
pub fn bake_moves(models: &Path, model: &mut Value) -> Result<(), String> {
    let Some(from) = model.get("moves_from").and_then(Value::as_str).map(str::to_string) else {
        return Ok(());
    };
    let file = model["file"].as_str().unwrap_or("").to_string();
    let read = |n: &str| std::fs::read(models.join(n)).map_err(|e| format!("{n}: {e}"));
    let (body, moves) = (read(&file)?, read(&from)?);
    let clips_of = |b: &[u8], n: &str| -> Result<Vec<String>, String> {
        let info = elo_rig::inspect(b).map_err(|why| format!("{n}: {why}"))?;
        Ok(info["clips"].as_array().into_iter().flatten()
            .filter_map(|c| c["name"].as_str().map(String::from)).collect())
    };
    let (own, there) = (clips_of(&body, &file)?, clips_of(&moves, &from)?);
    let mut wanted: Vec<String> = Vec::new();
    for (k, n) in model.get("clips").and_then(Value::as_object).into_iter().flatten() {
        let n = n.as_str().unwrap_or("");
        if own.iter().any(|c| c == n) {
            continue;
        }
        if !there.iter().any(|c| c == n) {
            return Err(format!("clips.{k}: {n:?} is in neither {file} nor {from}, which has {}", there.join(", ")));
        }
        if !wanted.iter().any(|w| w == n) {
            wanted.push(n.to_string());
        }
    }
    if wanted.is_empty() {
        return Ok(());
    }
    let skeleton = elo_rig::skeleton(&body)?;
    let (baked, _) = elo_rig::retarget(&skeleton, &moves, &wanted, true).map_err(|why| format!("{from}: {why}"))?;
    let stem: String = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(&file).chars().take(60).collect();
    let name = format!("{stem}.moves.glb");
    std::fs::write(models.join(&name), baked).map_err(|e| format!("{name}: {e}"))?;
    model["moves"] = json!(name);
    Ok(())
}

/// A glb pack's face: `skin` and `mouth_color` (#rrggbb), `eyes` (one or two)
/// and `mouth`, each `{at, normal, size}` in the model's own metres at rest.
pub fn glb_face(v: &Value) -> Result<Value, String> {
    let colour = |k: &str| -> Result<Value, String> {
        match v.get(k) {
            None => Ok(Value::Null),
            Some(c) => c.as_str()
                .filter(|c| c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit()))
                .map(|c| json!(c)).ok_or_else(|| format!("face.{k} is a colour like #f2eef0")),
        }
    };
    let nums = |p: &Value, k: &str, n: usize, lo: f64, hi: f64| -> Result<Value, String> {
        let a = p.get(k).and_then(Value::as_array).filter(|a| a.len() == n)
            .ok_or_else(|| format!("{k} is {n} numbers"))?;
        let xs: Vec<f64> = a.iter().filter_map(Value::as_f64).filter(|x| x.is_finite() && (lo..=hi).contains(x)).collect();
        if xs.len() != n {
            return Err(format!("{k}: numbers from {lo} to {hi}"));
        }
        Ok(json!(xs))
    };
    let spot = |p: &Value| -> Result<Value, String> {
        Ok(json!({"at": nums(p, "at", 3, -10.0, 10.0)?, "normal": nums(p, "normal", 3, -1.0, 1.0)?,
                  "size": nums(p, "size", 2, 0.001, 1.0)?}))
    };
    let eyes = match v.get("eyes") {
        None => vec![],
        Some(e) => {
            let a = e.as_array().filter(|a| (1..=2).contains(&a.len())).ok_or("face.eyes is one or two spots")?;
            a.iter().map(|p| spot(p).map_err(|w| format!("face.eyes: {w}"))).collect::<Result<_, _>>()?
        }
    };
    let mouth = match v.get("mouth") {
        None => Value::Null,
        Some(m) => spot(m).map_err(|w| format!("face.mouth: {w}"))?,
    };
    Ok(json!({"skin": colour("skin")?, "mouth_color": colour("mouth_color")?, "eyes": eyes, "mouth": mouth}))
}

/// A glb pack's `bones` (a part to the node that moves it: head, neck, chest,
/// spine, upper_arm_l, …) or `clips` (a use to an animation: dance, wave).
pub fn glb_names(v: &Value) -> Result<Value, String> {
    let o = v.as_object().filter(|o| o.len() <= 24).ok_or("an object of at most 24 names")?;
    let mut out = Map::new();
    for (k, n) in o {
        let ok_key = expr_name(k).is_some_and(|e| e == *k);
        let n = n.as_str().filter(|n| !n.is_empty() && n.len() <= 64 && n.chars().all(|c| !c.is_control()))
            .ok_or_else(|| format!("{k}: a name of at most 64 characters"))?;
        if !ok_key {
            return Err(format!("{k:?} isn't a plain key"));
        }
        out.insert(k.clone(), json!(n));
    }
    Ok(Value::Object(out))
}

fn copy_in(src: &Path, models: &Path) -> Result<String, String> {
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let name = safe_file(&name).ok_or_else(|| format!("{name:?}: use a plain file name"))?;
    model_ext(&name).ok_or_else(|| format!("{name}: not a model or a picture"))?;
    let meta = std::fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if meta.len() as usize > MODEL_MAX {
        return Err(format!("{name} is over {} MB", MODEL_MAX >> 20));
    }
    std::fs::copy(src, models.join(&name)).map_err(|e| format!("{name}: {e}"))?;
    Ok(name)
}

// ── the hub ──────────────────────────────────────────────────────────────────

struct Client {
    id: u64,
    role: String,
    name: String,
    tx: Sender<String>,
}

pub struct Hub {
    state: PathBuf,
    token: String,
    port: u16,
    cfg: Value,
    live: Value,
    clients: Vec<Client>,
    audio: VecDeque<(u64, Vec<u8>, &'static str)>,
    next: u64,
    spoken: Vec<u64>,
    /// Viewers' messages, newest last, each `{id, platform, from, login,
    /// badge, text, at, state, pid}`; `state` is new, held, read or dropped.
    chat: VecDeque<Value>,
    /// When each viewer (by platform and login) last got a message through.
    chat_last: std::collections::HashMap<String, u64>,
    chat_status: Value,
    /// Bumped to stop the chat readers when their settings change.
    reader_gen: Arc<std::sync::atomic::AtomicU64>,
    /// Going live everywhere (`stage_live.rs`).
    golive: crate::stage_live::Shared,
    /// A YouTube sign-in waiting for Google's answer: (state, verifier, when).
    yt_pending: Option<(String, String, Instant)>,
    /// Recordings on their way to YouTube, by file name.
    uploads: Value,
    /// What the agent hears besides chat, newest last, each `{id, kind, from,
    /// text, at, state}`: `kind` is owner (the owner, out loud) or game.
    feed: VecDeque<Value>,
    /// Where following the agent on the island stands, for the desk.
    game_status: Value,
}

pub type Shared = Arc<Mutex<Hub>>;

impl Hub {
    pub fn new(state: &Path, token: String, port: u16) -> Hub {
        Hub {
            state: state.to_path_buf(),
            token,
            port,
            cfg: load_config(state),
            live: json!({"expression": null, "look": "viewer", "layout": "corner",
                          "card": null, "caption": null, "expressions": []}),
            clients: Vec::new(),
            audio: VecDeque::new(),
            next: 1,
            spoken: Vec::new(),
            chat: VecDeque::new(),
            chat_last: std::collections::HashMap::new(),
            chat_status: json!({}),
            reader_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            golive: Arc::new(Mutex::new(crate::stage_live::Live::default())),
            yt_pending: None,
            uploads: json!({}),
            feed: VecDeque::new(),
            game_status: Value::Null,
        }
    }

    pub fn state_dir(&self) -> PathBuf {
        self.state.clone()
    }

    pub fn config(&self) -> Value {
        // The hearing command lives on disk alone, like the voice's.
        let mut cfg = self.cfg.clone();
        if let Some(h) = load_config(&self.state).get("hearing") {
            cfg["hearing"] = h.clone();
        }
        cfg
    }

    /// For the suite: swap the config without a disk round trip.
    #[allow(dead_code)]
    pub fn cfg_for_test(&mut self, cfg: Value) {
        self.cfg = cfg;
    }

    fn snapshot(&self) -> Value {
        let mut live = self.live.clone();
        // A picture model's faces are its file names; `<face>-talk` is that
        // face's open mouth, a frame and not a face of its own.
        if let Some(files) = self.cfg.pointer("/model/files").and_then(Value::as_object) {
            live["expressions"] = json!(files.keys().filter(|k| !k.ends_with("-talk")).collect::<Vec<_>>());
        }
        let mut config = public_config(&self.cfg);
        config["voice"] = crate::stage_voice::public(&self.state, &load_config(&self.state));
        live["chat_status"] = self.chat_status.clone();
        config["youtube_key"] = json!(crate::stage_sources::youtube_key(&self.state).is_some());
        config["youtube_units"] = json!(crate::stage_sources::units_today(&self.state));
        config["ears"] = crate::stage_ears::public(&self.state, &self.config());
        live["game_status"] = self.game_status.clone();
        live["moves"] = json!(self.moves());
        json!({
            "t": "state",
            "config": config,
            "live": live,
            "chat": self.chat.iter().rev().take(40).rev().collect::<Vec<_>>(),
            "feed": self.feed.iter().rev().take(20).rev().collect::<Vec<_>>(),
            "clients": self.clients.iter()
                .map(|c| json!({"role": c.role, "name": c.name}))
                .collect::<Vec<_>>(),
        })
    }

    fn broadcast(&mut self, msg: &Value) {
        let text = msg.to_string();
        self.clients.retain(|c| c.tx.send(text.clone()).is_ok());
    }

    /// The uses a glb pack names (`idle`, `wave`, …): what `move` plays.
    fn moves(&self) -> Vec<String> {
        self.cfg.pointer("/model/clips").and_then(Value::as_object)
            .filter(|_| self.cfg.pointer("/model/kind").and_then(Value::as_str) == Some("glb"))
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn expressions(&self) -> Vec<String> {
        self.snapshot()["live"]["expressions"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }
}

/// Apply one op. Returns the reply for whoever sent it; broadcasting is done
/// here, so a websocket, `POST /op` and the CLI all go through one door.
pub fn apply(hub: &Shared, op: &Value, from: Option<u64>) -> Value {
    let kind = op.get("op").and_then(Value::as_str).unwrap_or("");
    if !OPS.contains(&kind) {
        return json!({"t": "error", "why": format!("no op {kind:?} — one of {}", OPS.join(", "))});
    }
    let s = |k: &str| op.get(k).and_then(Value::as_str);
    let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
    match kind {
        "hello" => {
            let role = s("role").filter(|r| ["view", "drive", "desk"].contains(r)).unwrap_or("drive");
            let name = clean_text(s("name").unwrap_or(""), NAME_MAX);
            if let Some(c) = from.and_then(|id| h.clients.iter_mut().find(|c| c.id == id)) {
                c.role = role.into();
                c.name = name;
            }
            let snap = h.snapshot();
            h.broadcast(&json!({"t": "clients", "clients": snap["clients"]}));
            snap
        }
        "say" => {
            let text = clean_text(s("text").unwrap_or(""), SAY_MAX);
            if text.is_empty() {
                return json!({"t": "error", "why": "say needs `text`"});
            }
            // The owner's blocked words hold for the agent's own lines too:
            // a "repeat after me" never gets a blocked word on stream.
            if crate::stage_chat::Filter::from_cfg(&h.cfg).blocked_word(&text).is_some() {
                return json!({"t": "error", "why": "that line has a word the owner blocked; say it another way"});
            }
            // A reply quotes the viewer beside it, and only a message that
            // reached the agent: never one held, dropped or taken down.
            let quote = match op.get("reply_to").and_then(Value::as_u64) {
                None => Value::Null,
                Some(rid) => match h.chat.iter().find(|m| m["id"] == json!(rid)) {
                    Some(m) if m["state"] == "read" || m["state"] == "new" => {
                        json!({"from": m["from"], "text": m["text"], "platform": m["platform"]})
                    }
                    _ => return json!({"t": "error", "why": format!("no chat message #{rid} to reply to")}),
                },
            };
            let expression = match s("expression").map(|e| check_expr(&h, e)) {
                Some(Err(why)) => return json!({"t": "error", "why": why}),
                Some(Ok(e)) => Some(e),
                None => None,
            };
            let id = h.next;
            h.next += 1;
            // A line's expression lasts the line; the standing one is
            // `expression`'s, so a reconnecting page shows that.
            h.live["caption"] = json!(text);
            let voice = crate::stage_voice::of(&load_config(&h.state));
            if voice == crate::stage_voice::Voice::Page {
                let ev = json!({"t": "say", "id": id, "text": text, "expression": expression,
                                "audio": null, "quote": quote,
                                "mouth": crate::stage_voice::mouth_from_text(&text)});
                h.broadcast(&ev);
            } else {
                // The voice runs off the lock; the line goes out when its
                // audio is ready, or without it (the page's voice) if it failed.
                let hub = Arc::clone(hub);
                let state = h.state.clone();
                std::thread::spawn(move || {
                    let got = crate::stage_voice::render_timed(&state, &voice, &text);
                    let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
                    let (audio, why, mouth) = match got {
                        Ok((bytes, ctype, mouth)) => {
                            h.audio.push_back((id, bytes, ctype));
                            while h.audio.len() > AUDIO_KEPT {
                                h.audio.pop_front();
                            }
                            (json!(format!("/audio/{id}")), Value::Null, mouth)
                        }
                        Err(why) => (Value::Null, json!(why), crate::stage_voice::mouth_from_text(&text)),
                    };
                    let ev = json!({"t": "say", "id": id, "text": text, "expression": expression,
                                    "audio": audio, "voice_error": why, "quote": quote, "mouth": mouth});
                    h.broadcast(&ev);
                });
            }
            let views = h.clients.iter().filter(|c| c.role == "view").count();
            json!({"t": "ack", "op": "say", "id": id, "views": views})
        }
        "expression" => match check_expr(&h, s("name").unwrap_or("")) {
            Ok(e) => {
                h.live["expression"] = json!(e);
                h.broadcast(&json!({"t": "expression", "name": e}));
                json!({"t": "ack", "op": "expression", "name": e})
            }
            Err(why) => json!({"t": "error", "why": why}),
        },
        // One of the body's moves, played once (`idle`, `talk` and `dance`
        // also play on their own: at rest, while a line plays, on brb).
        "move" => {
            let known = h.moves();
            match s("name").and_then(expr_name).filter(|n| known.contains(n)) {
                Some(n) => {
                    h.broadcast(&json!({"t": "move", "name": n}));
                    json!({"t": "ack", "op": "move", "name": n})
                }
                None if known.is_empty() => json!({"t": "error", "why": "this body has no moves: a glb pack's `clips` names them"}),
                None => json!({"t": "error", "why": format!("this body's moves are {}", known.join(", "))}),
            }
        }
        "look" => {
            let at = s("at").unwrap_or("viewer");
            if !LOOKS.contains(&at) {
                return json!({"t": "error", "why": format!("look at one of {}", LOOKS.join(", "))});
            }
            h.live["look"] = json!(at);
            h.broadcast(&json!({"t": "look", "at": at}));
            json!({"t": "ack", "op": "look", "at": at})
        }
        "layout" => {
            let name = s("name").unwrap_or("");
            if !LAYOUTS.contains(&name) {
                return json!({"t": "error", "why": format!("layout is one of {}", LAYOUTS.join(", "))});
            }
            h.live["layout"] = json!(name);
            h.broadcast(&json!({"t": "layout", "name": name}));
            json!({"t": "ack", "op": "layout", "name": name})
        }
        "card" => {
            let title = clean_text(s("title").unwrap_or(""), 60);
            let body = clean_text(s("body").unwrap_or(""), CARD_MAX);
            if title.is_empty() && body.is_empty() {
                return json!({"t": "error", "why": "a card needs a `title` or a `body`"});
            }
            let seconds = op.get("seconds").and_then(Value::as_u64).unwrap_or(10).clamp(2, 600);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let card = json!({"title": title, "body": body, "seconds": seconds, "until": now + seconds});
            h.live["card"] = card.clone();
            h.broadcast(&json!({"t": "card", "card": card}));
            json!({"t": "ack", "op": "card"})
        }
        "clear" => {
            h.live["card"] = Value::Null;
            h.live["caption"] = Value::Null;
            h.broadcast(&json!({"t": "clear"}));
            json!({"t": "ack", "op": "clear"})
        }
        // From a stage page: a line finished playing. The first report wins,
        // so a second open stage cannot double-count it.
        "spoken" => {
            let id = op.get("id").and_then(Value::as_u64).unwrap_or(0);
            if id == 0 || id >= h.next || h.spoken.contains(&id) {
                return json!({"t": "ack", "op": "spoken"});
            }
            h.spoken.push(id);
            if h.spoken.len() > 64 {
                h.spoken.remove(0);
            }
            // `cut`: the line stopped because the owner started talking.
            let cut = op.get("cut").and_then(Value::as_bool).unwrap_or(false);
            h.broadcast(&json!({"t": "spoken", "id": id, "cut": cut}));
            json!({"t": "ack", "op": "spoken"})
        }
        // The owner started talking: every stage stops the line it is saying
        // and drops the ones queued behind it.
        "hush" => {
            h.live["caption"] = Value::Null;
            h.broadcast(&json!({"t": "hush"}));
            json!({"t": "ack", "op": "hush"})
        }
        // Something happened in a game, from a program on this machine.
        "event" => {
            let from = clean_text(s("from").unwrap_or("a game"), NAME_MAX);
            drop(h);
            feed_in(hub, "game", &from, s("text").unwrap_or(""))
        }
        // The agent reads what it has heard besides chat. Every read marks
        // what it returned, so each is handed over once.
        "feed_read" => {
            let max = op.get("max").and_then(Value::as_u64).unwrap_or(20).clamp(1, 40) as usize;
            let mut out = Vec::new();
            for m in h.feed.iter_mut() {
                if out.len() >= max {
                    break;
                }
                if m["state"] == "new" {
                    m["state"] = json!("read");
                    out.push(m.clone());
                }
            }
            let left = h.feed.iter().filter(|m| m["state"] == "new").count();
            json!({"t": "feed", "items": out, "left": left, "game_status": h.game_status})
        }
        // From a stage page: the expressions the loaded model carries (a VRM
        // can carry custom ones the hub cannot know until it is loaded).
        "ready" => {
            let list: Vec<String> = op
                .get("expressions")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().and_then(expr_name)).take(48).collect())
                .unwrap_or_default();
            h.live["expressions"] = json!(list);
            json!({"t": "ack", "op": "ready"})
        }
        // A viewer's message, from a program or the desk's test box. Twitch
        // comes in through `chat_in` directly.
        "chat" => {
            let from = s("from").unwrap_or("");
            let platform = s("platform").filter(|p| expr_name(p).is_some()).unwrap_or("chat");
            let badge = s("badge").filter(|b| ["broadcaster", "mod", "vip", "sub"].contains(b)).unwrap_or("");
            drop(h);
            chat_in(hub, platform, from, from, badge, s("text").unwrap_or(""), s("pid"))
        }
        // The agent reads what is new. Every read marks what it returned, so a
        // message is handed over once.
        "chat_read" => {
            let max = op.get("max").and_then(Value::as_u64).unwrap_or(10).clamp(1, 30) as usize;
            let mut out = Vec::new();
            for m in h.chat.iter_mut() {
                if out.len() >= max {
                    break;
                }
                if m["state"] == "new" {
                    m["state"] = json!("read");
                    out.push(m.clone());
                }
            }
            let left = h.chat.iter().filter(|m| m["state"] == "new").count();
            let held = h.chat.iter().filter(|m| m["state"] == "held").count();
            if !out.is_empty() {
                let ids: Vec<Value> = out.iter().map(|m| m["id"].clone()).collect();
                h.broadcast(&json!({"t": "chat_state", "ids": ids, "state": "read"}));
            }
            json!({"t": "chat", "msgs": out, "left": left, "held": held,
                   "mode": crate::stage_chat::Filter::from_cfg(&h.cfg).mode,
                   "status": h.chat_status})
        }
        // The owner's hand: approve or drop one, or clear the lot. Only the
        // desk may: an agent approving its own held chat would undo the hold.
        "chat_mod" => {
            let role = from.and_then(|id| h.clients.iter().find(|c| c.id == id)).map(|c| c.role.clone());
            if role.as_deref() != Some("desk") {
                return json!({"t": "error", "why": "only the desk moderates chat"});
            }
            match s("action") {
                Some("clear") => {
                    for m in h.chat.iter_mut() {
                        if m["state"] == "new" || m["state"] == "held" {
                            m["state"] = json!("dropped");
                        }
                    }
                    let snap = h.snapshot();
                    h.broadcast(&snap);
                    json!({"t": "ack", "op": "chat_mod"})
                }
                Some(a @ ("approve" | "drop")) => {
                    let id = op.get("id").and_then(Value::as_u64).unwrap_or(0);
                    let want = if a == "approve" { "new" } else { "dropped" };
                    match h.chat.iter_mut().find(|m| m["id"] == json!(id)) {
                        Some(m) if m["state"] == "held" || (a == "drop" && m["state"] == "new") => {
                            m["state"] = json!(want);
                            h.broadcast(&json!({"t": "chat_state", "ids": [id], "state": want}));
                            json!({"t": "ack", "op": "chat_mod"})
                        }
                        _ => json!({"t": "error", "why": format!("#{id} isn't waiting")}),
                    }
                }
                _ => json!({"t": "error", "why": "action is approve, drop or clear"}),
            }
        }
        _ => unreachable!("OPS is checked above"),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

const CHAT_KEPT: usize = 120;
const FEED_KEPT: usize = 60;
const FEED_TEXT_MAX: usize = 400;

/// Something for the agent besides chat: the owner's words (`owner`) or a
/// game's (`game`). Kept, shown on the desk, and woken on by `listen`.
pub fn feed_in(hub: &Shared, kind: &str, from: &str, text: &str) -> Value {
    let text = clean_text(text, FEED_TEXT_MAX);
    if text.is_empty() {
        return json!({"t": "error", "why": "an event needs `text`"});
    }
    let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
    // The owner's blocked words hold here too: a game reporting a player's
    // name is a stranger's words reaching the agent.
    if kind != "owner" && crate::stage_chat::Filter::from_cfg(&h.cfg).blocked_word(&text).is_some() {
        return json!({"t": "dropped", "why": "a blocked word"});
    }
    let id = h.next;
    h.next += 1;
    let item = json!({"id": id, "kind": kind, "from": clean_text(from, NAME_MAX), "text": text,
                      "at": now_secs(), "state": "new"});
    h.feed.push_back(item.clone());
    while h.feed.len() > FEED_KEPT {
        h.feed.pop_front();
    }
    h.broadcast(&json!({"t": "feed", "item": item}));
    json!({"t": "ack", "op": "event", "id": id})
}

/// Where following the agent on the island stands.
pub fn game_status(hub: &Shared, status: Value) {
    let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
    if h.game_status == status {
        return;
    }
    h.game_status = status.clone();
    h.broadcast(&json!({"t": "game_status", "status": status}));
}

/// A viewer's message in: judged by the owner's filter, kept, and shown on
/// the desk. Returns what happened to it.
pub fn chat_in(hub: &Shared, platform: &str, from: &str, login: &str, badge: &str, text: &str,
               pid: Option<&str>) -> Value {
    let from = clean_text(from, NAME_MAX);
    if from.is_empty() {
        return json!({"t": "error", "why": "a message needs who it is `from`"});
    }
    let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
    let filter = crate::stage_chat::Filter::from_cfg(&h.cfg);
    let who = format!("{platform}:{}", login.to_lowercase());
    let now = now_secs();
    let verdict = crate::stage_chat::judge(&filter, login, badge, text, h.chat_last.get(&who).copied(), now);
    let (text, state) = match verdict {
        crate::stage_chat::Verdict::Keep { text, state } => (text, state),
        crate::stage_chat::Verdict::Drop(why) => {
            // Dropped lines are not kept: a blocked word never sits in memory
            // or on the desk. The desk sees that one was dropped, and why.
            h.broadcast(&json!({"t": "chat_dropped", "from": from, "why": why}));
            return json!({"t": "dropped", "why": why});
        }
    };
    h.chat_last.insert(who, now);
    if h.chat_last.len() > 5000 {
        h.chat_last.retain(|_, t| now.saturating_sub(*t) < 3600);
    }
    let id = h.next;
    h.next += 1;
    let msg = json!({"id": id, "platform": platform, "from": from, "login": clean_text(login, NAME_MAX),
                     "badge": badge, "text": text, "at": now, "state": state,
                     "pid": pid.map(|p| clean_text(p, 64))});
    h.chat.push_back(msg.clone());
    while h.chat.len() > CHAT_KEPT {
        h.chat.pop_front();
    }
    h.broadcast(&json!({"t": "chat", "msg": msg}));
    json!({"t": "ack", "op": "chat", "id": id, "state": state})
}

/// A platform took messages down: one by its own id, or everything one viewer
/// said (`"*"`: the whole chat was cleared). The agent never answers them.
pub fn chat_forget(hub: &Shared, pid: Option<&str>, login: Option<&str>) {
    let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
    let mut ids = Vec::new();
    for m in h.chat.iter_mut() {
        let hit = match (pid, login) {
            (Some(p), _) => m["pid"] == json!(p),
            (None, Some("*")) => true,
            (None, Some(l)) => m["login"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(l)),
            (None, None) => false,
        };
        if hit && m["state"] != "dropped" {
            m["state"] = json!("dropped");
            ids.push(m["id"].clone());
        }
    }
    if !ids.is_empty() {
        h.broadcast(&json!({"t": "chat_state", "ids": ids, "state": "dropped"}));
    }
}

/// Where one chat reader (`twitch`, `youtube`, `kick`) stands, for the desk.
pub fn chat_status(hub: &Shared, source: &str, status: Value) {
    let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
    if !h.chat_status.is_object() {
        h.chat_status = json!({});
    }
    h.chat_status[source] = status;
    let all = h.chat_status.clone();
    h.broadcast(&json!({"t": "chat_status", "status": all}));
}

/// The settings that decide which readers run; a change to any restarts them.
pub fn reader_settings(cfg: &Value) -> Value {
    let c = cfg.get("chat").cloned().unwrap_or_else(|| json!({}));
    json!([c["twitch"], c["youtube"], c["kick"], c["youtube_budget"], crate::stage_ears::following(cfg)])
}

/// Start (or stop, or move) the chat readers to match the config: Twitch
/// and YouTube from this machine, Kick through the origin.
pub fn restart_readers(hub: &Shared) {
    use crate::{stage_chat, stage_sources};
    let (cfg, state, gen, current) = {
        let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
        let gen = h.reader_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        h.chat_status = json!({});
        h.game_status = Value::Null;
        (h.cfg.clone(), h.state.clone(), gen, Arc::clone(&h.reader_gen))
    };
    let s = |k: &str| cfg.pointer(&format!("/chat/{k}")).and_then(Value::as_str).map(str::trim)
        .filter(|v| !v.is_empty()).map(String::from);
    if let Some(ch) = s("twitch").and_then(|c| stage_chat::twitch_channel(&c)) {
        let (hub, current) = (Arc::clone(hub), Arc::clone(&current));
        std::thread::spawn(move || stage_chat::run_twitch(hub, ch, gen, current));
    }
    if let Some(yt) = s("youtube") {
        let budget = cfg.pointer("/chat/youtube_budget").and_then(Value::as_u64)
            .unwrap_or(stage_sources::YT_DEFAULT_BUDGET);
        let (hub, current) = (Arc::clone(hub), Arc::clone(&current));
        std::thread::spawn(move || stage_sources::run_youtube(hub, state, yt, budget, gen, current));
    }
    if let Some(slug) = s("kick").and_then(|k| stage_sources::kick_slug(&k)) {
        let (hub, current) = (Arc::clone(hub), Arc::clone(&current));
        std::thread::spawn(move || stage_sources::run_kick(hub, slug, gen, current));
    }
    if let Some(wallet) = crate::stage_ears::following(&cfg) {
        let (hub, current) = (Arc::clone(hub), Arc::clone(&current));
        std::thread::spawn(move || crate::stage_ears::run_follow(hub, wallet, gen, current));
    }
    let all = hub.lock().map(|h| h.chat_status.clone()).unwrap_or_default();
    if let Ok(mut h) = hub.lock() {
        h.broadcast(&json!({"t": "chat_status", "status": all}));
    }
}

static ORIGIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The origin the Kick reader asks (`--host`, default elopros.com).
pub fn set_origin(host: &str) {
    let _ = ORIGIN.set(host.trim_end_matches('/').to_string());
}

pub fn origin() -> String {
    ORIGIN.get().cloned().unwrap_or_else(|| "https://elopros.com".into())
}

/// Write the hub's config, keeping the voice and hearing commands that are on
/// disk: those are set by the CLI alone and the hub never holds them.
fn persist(h: &Hub) {
    let mut cfg = h.cfg.clone();
    let disk = load_config(&h.state);
    for k in ["voice", "hearing"] {
        match disk.get(k) {
            Some(v) => cfg[k] = v.clone(),
            None => {
                cfg.as_object_mut().map(|o| o.remove(k));
            }
        }
    }
    let _ = save_config(&h.state, &cfg);
}

fn check_expr(h: &Hub, e: &str) -> Result<String, String> {
    let e = expr_name(e).ok_or_else(|| format!("{e:?} isn't an expression name"))?;
    let known = h.expressions();
    let known = if known.is_empty() && h.cfg.pointer("/model/kind").and_then(Value::as_str) != Some("png") {
        VRM_EXPRESSIONS.iter().map(|s| s.to_string()).collect()
    } else {
        known
    };
    if known.is_empty() || known.contains(&e) {
        Ok(e)
    } else {
        Err(format!("this model's expressions are {}", known.join(", ")))
    }
}

/// Run the voice command: the text on stdin, `{out}` (if any argument has it)
/// replaced by a temp file, else the audio read from stdout. No shell.
pub fn run_voice(argv: &[String], text: &str) -> Result<Vec<u8>, String> {
    let out = std::env::temp_dir().join(format!("elo-stage-{}-{:016x}.wav", std::process::id(), rand_u64()));
    let to_file = argv.iter().any(|a| a.contains("{out}"));
    let args: Vec<String> = argv.iter().map(|a| a.replace("{out}", &out.to_string_lossy())).collect();
    let mut child = std::process::Command::new(&args[0])
        .args(&args[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("the voice command didn't start ({}): {e}", args[0]))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(o) = stdout.as_mut() {
            let _ = o.take(AUDIO_MAX as u64 + 1).read_to_end(&mut buf);
        }
        buf
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > VOICE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the voice command took over 30 s".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let piped = reader.join().unwrap_or_default();
    let bytes = if to_file {
        let b = std::fs::read(&out).map_err(|e| format!("the voice wrote no file: {e}"));
        let _ = std::fs::remove_file(&out);
        b?
    } else {
        piped
    };
    if bytes.len() < 44 || bytes.len() > AUDIO_MAX {
        return Err("the voice command made no usable audio".into());
    }
    Ok(bytes)
}

pub fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    u64::from_le_bytes(b)
}

// ── http, by hand ────────────────────────────────────────────────────────────

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    pub fn q(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub fn parse_head(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?;
    let (path, qs) = target.split_once('?').unwrap_or((target, ""));
    let query = qs
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (unescape(k), unescape(v))
        })
        .collect();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Some(Request { method, path: path.to_string(), query, headers })
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Is this request for this hub, from this hub's own pages or from no page?
///
/// Host: only the loopback names on our port — a DNS-rebound name is refused.
/// Origin: none (a program, OBS's first load), or our own. A browser always
/// sends Origin on a websocket and a cross-site POST, so a page elsewhere is
/// refused even if it learned the token.
pub fn local_ok(req: &Request, port: u16) -> bool {
    let ours = |v: &str| {
        let v = v.trim_start_matches("http://");
        [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
            .iter()
            .any(|h| h == v)
    };
    let host_ok = req.header("host").is_some_and(ours);
    let origin_ok = match req.header("origin") {
        None => true,
        Some(o) => o.starts_with("http://") && ours(o),
    };
    host_ok && origin_ok
}

/// Constant-time, so the token cannot be guessed a byte at a time by timing.
pub fn token_ok(given: Option<&str>, token: &str) -> bool {
    let Some(g) = given else { return false };
    if g.len() != token.len() {
        return false;
    }
    g.bytes().zip(token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn respond(s: &mut TcpStream, code: &str, ctype: &str, body: &[u8], port: u16) {
    let csp = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' blob: data:; media-src 'self' blob:; \
         connect-src 'self' blob: ws://127.0.0.1:{port} ws://localhost:{port}; \
         frame-src 'self'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'"
    );
    let head = format!(
        "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n\
         X-Content-Type-Options: nosniff\r\nContent-Security-Policy: {csp}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
    let _ = s.flush();
}

fn respond_json(s: &mut TcpStream, code: &str, v: &Value, port: u16) {
    respond(s, code, "application/json", v.to_string().as_bytes(), port);
}

fn media_type(name: &str) -> &'static str {
    match model_ext(name) {
        Some(".png") => "image/png",
        Some(".webp") => "image/webp",
        Some(".gif") => "image/gif",
        Some(".jpg" | ".jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

fn read_head(r: &mut BufReader<TcpStream>) -> Option<String> {
    let mut head = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = r.by_ref().take(HEAD_MAX as u64).read_until(b'\n', &mut line).ok()?;
        if n == 0 {
            return None;
        }
        head.extend_from_slice(&line);
        if head.len() > HEAD_MAX {
            return None;
        }
        if line == b"\r\n" || line == b"\n" {
            break;
        }
    }
    String::from_utf8(head).ok()
}

fn read_body(r: &mut BufReader<TcpStream>, req: &Request, max: usize) -> Result<Vec<u8>, &'static str> {
    let len: usize = req
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .ok_or("411 Length Required")?;
    if len > max {
        return Err("413 Payload Too Large");
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|_| "400 Bad Request")?;
    Ok(body)
}

fn handle(stream: TcpStream, hub: Shared) {
    let port = hub.lock().map(|h| h.port).unwrap_or(DEFAULT_PORT);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(write_half) = stream.try_clone() else { return };
    let mut r = BufReader::new(stream);
    let Some(head) = read_head(&mut r) else { return };
    let Some(req) = parse_head(&head) else { return };
    let mut w = write_half;
    if !local_ok(&req, port) {
        return respond(&mut w, "403 Forbidden", "text/plain", b"this stage answers its own pages only\n", port);
    }
    if req.method == "GET" {
        if let Some((ctype, body)) = asset(&req.path) {
            return respond(&mut w, "200 OK", ctype, body, port);
        }
    }
    // Google's answer to a YouTube sign-in carries no key of ours (it went by
    // way of Google); it carries the one-time state this hub made instead.
    if req.method == "GET" && req.path == "/youtube/oauth" {
        return youtube_oauth(&mut w, &req, &hub, port);
    }
    let token = hub.lock().map(|h| h.token.clone()).unwrap_or_default();
    if !token_ok(req.q("k"), &token) {
        return respond(&mut w, "401 Unauthorized", "text/plain",
                       b"open the url `elo stage url` prints: it carries this stage's key\n", port);
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/ws") => {
            if !req.header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket")) {
                return respond(&mut w, "400 Bad Request", "text/plain", b"a websocket\n", port);
            }
            let Some(key) = req.header("sec-websocket-key") else {
                return respond(&mut w, "400 Bad Request", "text/plain", b"no key\n", port);
            };
            let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
            let head = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                 Sec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            if w.write_all(head.as_bytes()).is_err() {
                return;
            }
            let leftover = r.buffer().to_vec();
            let role = req.q("role").unwrap_or("drive").to_string();
            let name = req.q("name").unwrap_or("").to_string();
            socket(w, leftover, hub, &role, &name);
        }
        ("GET", "/state") => {
            let snap = hub.lock().map(|h| h.snapshot()).unwrap_or_default();
            respond_json(&mut w, "200 OK", &snap, port)
        }
        ("POST", "/op") => match read_body(&mut r, &req, BODY_MAX) {
            Ok(body) => match serde_json::from_slice::<Value>(&body) {
                Ok(op) => {
                    let reply = apply(&hub, &op, None);
                    let code = if reply["t"] == "error" { "422 Unprocessable Entity" } else { "200 OK" };
                    respond_json(&mut w, code, &reply, port)
                }
                Err(e) => respond_json(&mut w, "400 Bad Request", &json!({"t": "error", "why": e.to_string()}), port),
            },
            Err(code) => respond(&mut w, code, "text/plain", b"", port),
        },
        // The desk's import: one file per request, then `/model/use`.
        ("POST", "/model/file") => {
            let Some(name) = req.q("file").and_then(safe_file).filter(|n| model_ext(n).is_some()) else {
                return respond_json(&mut w, "422 Unprocessable Entity",
                                    &json!({"t": "error", "why": "a .vrm, .glb, .png, .webp, .gif or .jpg"}), port);
            };
            match read_body(&mut r, &req, MODEL_MAX) {
                Ok(body) => {
                    let state = hub.lock().map(|h| h.state.clone()).unwrap_or_default();
                    let models = dir(&state).join("models");
                    let ok = std::fs::create_dir_all(&models).and_then(|_| std::fs::write(models.join(&name), &body));
                    match ok {
                        Ok(()) => respond_json(&mut w, "200 OK", &json!({"t": "ack", "file": name}), port),
                        Err(e) => respond_json(&mut w, "500 Internal Server Error",
                                               &json!({"t": "error", "why": e.to_string()}), port),
                    }
                }
                Err(code) => respond(&mut w, code, "text/plain", b"", port),
            }
        }
        // The desk's settings: the model and the safe half of the config. The
        // voice COMMAND is not settable here (the module note says why).
        ("POST", "/config") => match read_body(&mut r, &req, BODY_MAX)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            Some(patch) => {
                let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
                let models = dir(&h.state).join("models");
                let readers_were = reader_settings(&h.cfg);
                let reply = set_config(&mut h.cfg, &patch, &models);
                let readers_moved = reader_settings(&h.cfg) != readers_were;
                if reply["t"] != "error" {
                    persist(&h);
                    if patch.get("model").is_some() {
                        h.live["expressions"] = json!([]);
                    }
                    let snap = h.snapshot();
                    h.broadcast(&snap);
                }
                drop(h);
                if readers_moved {
                    restart_readers(&hub);
                }
                respond_json(&mut w, "200 OK", &reply, port)
            }
            None => respond_json(&mut w, "400 Bad Request", &json!({"t": "error", "why": "a JSON object"}), port),
        },
        ("GET", p) if p.starts_with("/model/") => {
            let name = &p["/model/".len()..];
            let state = hub.lock().map(|h| h.state.clone()).unwrap_or_default();
            match safe_file(name).filter(|n| n == name && model_ext(n).is_some()) {
                Some(n) => match std::fs::read(dir(&state).join("models").join(&n)) {
                    Ok(b) => respond(&mut w, "200 OK", media_type(&n), &b, port),
                    Err(_) => respond(&mut w, "404 Not Found", "text/plain", b"no such model file\n", port),
                },
                None => respond(&mut w, "404 Not Found", "text/plain", b"", port),
            }
        }
        ("GET", p) if p.starts_with("/audio/") => {
            let id: u64 = p["/audio/".len()..].split('.').next().unwrap_or("").parse().unwrap_or(0);
            let got = hub
                .lock()
                .ok()
                .and_then(|h| h.audio.iter().find(|(i, _, _)| *i == id).map(|(_, b, t)| (b.clone(), *t)));
            match got {
                Some((b, t)) => respond(&mut w, "200 OK", t, &b, port),
                None => respond(&mut w, "404 Not Found", "text/plain", b"", port),
            }
        }
        // A viewer's message from any program: `{"from", "text", "platform"}`.
        ("POST", "/chat") => match read_body(&mut r, &req, BODY_MAX)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            Some(mut v) => {
                v["op"] = json!("chat");
                let reply = apply(&hub, &v, None);
                respond_json(&mut w, if reply["t"] == "error" { "422 Unprocessable Entity" } else { "200 OK" },
                             &reply, port)
            }
            None => respond_json(&mut w, "400 Bad Request", &json!({"t": "error", "why": "a JSON object"}), port),
        },
        // Something that happened in a game, from any program: `{"text", "from"}`.
        ("POST", "/event") => match read_body(&mut r, &req, BODY_MAX)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            Some(mut v) => {
                v["op"] = json!("event");
                let reply = apply(&hub, &v, None);
                respond_json(&mut w, if reply["t"] == "error" { "422 Unprocessable Entity" } else { "200 OK" },
                             &reply, port)
            }
            None => respond_json(&mut w, "400 Bad Request", &json!({"t": "error", "why": "a JSON object"}), port),
        },
        // One thing the owner said, as a WAV from the desk's microphone.
        ("POST", "/ears") => match read_body(&mut r, &req, crate::stage_ears::EARS_MAX) {
            Ok(wav) => {
                let reply = crate::stage_ears::hear(&hub, &wav);
                respond_json(&mut w, "200 OK", &reply, port)
            }
            Err(code) => respond(&mut w, code, "text/plain", b"", port),
        },
        // The desk may pick ElevenLabs or nothing; only the CLI sets a command.
        ("POST", "/ears/engine") => {
            let want = read_body(&mut r, &req, BODY_MAX).ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v["engine"].as_str().map(String::from))
                .unwrap_or_default();
            let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
            let mut disk = load_config(&h.state);
            let reply = match want.as_str() {
                "elevenlabs" if crate::stage_voice::key(&h.state).is_none() => {
                    json!({"t": "error", "why": "add your ElevenLabs key first (the voice uses the same one)"})
                }
                "elevenlabs" => {
                    disk["hearing"] = json!({"elevenlabs": {"model": crate::stage_ears::DEFAULT_STT_MODEL}});
                    json!({"t": "ack"})
                }
                "none" => {
                    disk.as_object_mut().map(|o| o.remove("hearing"));
                    json!({"t": "ack"})
                }
                _ => json!({"t": "error", "why": "engine is elevenlabs or none; `elo stage ears -- <command…>` sets a local one"}),
            };
            if reply["t"] == "ack" {
                let _ = save_config(&h.state, &disk);
                let snap = h.snapshot();
                h.broadcast(&snap);
            }
            respond_json(&mut w, "200 OK", &reply, port)
        }
        // ElevenLabs, from the desk: the key in (never out), your voices, the pick.
        ("POST", "/voice/elevenlabs/key") => {
            let key = read_body(&mut r, &req, BODY_MAX).ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v["key"].as_str().map(|k| k.trim().to_string()))
                .unwrap_or_default();
            let state = hub.lock().map(|h| h.state.clone()).unwrap_or_default();
            let reply = if !crate::stage_voice::key_ok(&key) {
                json!({"t": "error", "why": "that doesn't look like an ElevenLabs API key"})
            } else {
                match crate::stage_voice::voices(&elo_net::Net::new(), &key) {
                    Err(why) => json!({"t": "error", "why": why}),
                    Ok(list) => match crate::stage_voice::save_key(&state, &key) {
                        Ok(()) => json!({"t": "ack", "voices": list}),
                        Err(why) => json!({"t": "error", "why": why}),
                    },
                }
            };
            respond_json(&mut w, "200 OK", &reply, port)
        }
        // Going live everywhere: the owner's act, from the desk or the CLI.
        ("GET", "/live") => {
            let (state, live) = hub.lock().map(|h| (h.state.clone(), Arc::clone(&h.golive))).unwrap_or_default();
            let snap = live_snapshot(&state, &live);
            respond_json(&mut w, "200 OK", &snap, port)
        }
        ("POST", p) if p.starts_with("/live/") => {
            let body = read_body(&mut r, &req, BODY_MAX).ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or_else(|| json!({}));
            let (state, live) = hub.lock().map(|h| (h.state.clone(), Arc::clone(&h.golive))).unwrap_or_default();
            let done: Result<(), String> = match p {
                "/live/dest" => crate::stage_live::set_dest(&state, &body).map(|_| ()),
                "/live/dest/remove" => crate::stage_live::remove_dest(&state, body["id"].as_str().unwrap_or("")),
                "/live/record" => {
                    let mut v = crate::stage_live::load(&state);
                    v["record"] = json!(body["on"].as_bool().unwrap_or(true));
                    crate::stage_live::save(&state, &v)
                }
                "/live/start" => {
                    let h2 = Arc::clone(&hub);
                    let tell: Arc<dyn Fn(Value) + Send + Sync> = Arc::new(move |snap: Value| {
                        if let Ok(mut h) = h2.lock() {
                            h.broadcast(&json!({"t": "live", "live": snap}));
                        }
                    });
                    crate::stage_live::start(state.clone(), Arc::clone(&live), tell)
                }
                "/live/stop" => {
                    crate::stage_live::stop(&live);
                    Ok(())
                }
                _ => Err("no such live action".into()),
            };
            let snap = live_snapshot(&state, &live);
            if let Ok(mut h) = hub.lock() {
                h.broadcast(&json!({"t": "live", "live": snap.clone()}));
            }
            let reply = match done {
                Ok(()) => json!({"t": "ack", "live": snap}),
                Err(why) => json!({"t": "error", "why": why, "live": snap}),
            };
            respond_json(&mut w, "200 OK", &reply, port)
        }
        // Posting recordings to YouTube (`stage_youtube.rs`).
        ("GET", "/youtube") => {
            let (state, uploads) = hub.lock().map(|h| (h.state.clone(), h.uploads.clone())).unwrap_or_default();
            let mut v = crate::stage_youtube::public(&state);
            v["recordings"] = json!(crate::stage_live::recordings(&state));
            v["uploads"] = uploads;
            respond_json(&mut w, "200 OK", &v, port)
        }
        ("POST", p) if p.starts_with("/youtube/") && p != "/youtube/key" => {
            let body = read_body(&mut r, &req, BODY_MAX).ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or_else(|| json!({}));
            let state = hub.lock().map(|h| h.state.clone()).unwrap_or_default();
            let s = |k: &str| body[k].as_str().unwrap_or("").to_string();
            let reply = match p {
                "/youtube/client" => crate::stage_youtube::set_client(&state, &s("id"), &s("secret"))
                    .map(|_| json!({"t": "ack"})),
                "/youtube/connect" => crate::stage_youtube::begin(&state, port).map(|(url, st, verifier)| {
                    if let Ok(mut h) = hub.lock() {
                        h.yt_pending = Some((st, verifier, Instant::now()));
                    }
                    json!({"t": "ack", "url": url})
                }),
                "/youtube/forget" => crate::stage_youtube::forget(&state).map(|_| json!({"t": "ack"})),
                "/youtube/upload" => start_upload(&hub, &state, &body),
                _ => Err("no such YouTube action".into()),
            };
            let reply = reply.unwrap_or_else(|why| json!({"t": "error", "why": why}));
            respond_json(&mut w, "200 OK", &reply, port)
        }
        // The YouTube Data API key: in, never out.
        ("POST", "/youtube/key") => {
            let key = read_body(&mut r, &req, BODY_MAX).ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v["key"].as_str().map(|k| k.trim().to_string()))
                .unwrap_or_default();
            let state = hub.lock().map(|h| h.state.clone()).unwrap_or_default();
            let reply = match crate::stage_sources::save_youtube_key(&state, &key) {
                Ok(()) => json!({"t": "ack"}),
                Err(why) => json!({"t": "error", "why": why}),
            };
            if reply["t"] == "ack" {
                restart_readers(&hub);
            }
            respond_json(&mut w, "200 OK", &reply, port)
        }
        ("GET", "/voice/elevenlabs/voices") => {
            let state = hub.lock().map(|h| h.state.clone()).unwrap_or_default();
            let reply = match crate::stage_voice::key(&state) {
                None => json!({"t": "error", "why": "add your ElevenLabs key first"}),
                Some(k) => match crate::stage_voice::voices(&elo_net::Net::new(), &k) {
                    Ok(list) => json!({"t": "ack", "voices": list}),
                    Err(why) => json!({"t": "error", "why": why}),
                },
            };
            respond_json(&mut w, "200 OK", &reply, port)
        }
        ("POST", "/voice/elevenlabs") | ("POST", "/voice/page") => {
            let body = read_body(&mut r, &req, BODY_MAX).ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or_else(|| json!({}));
            let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
            let mut disk = load_config(&h.state);
            let reply = if req.path == "/voice/page" {
                disk.as_object_mut().map(|o| o.remove("voice"));
                json!({"t": "ack"})
            } else {
                match elevenlabs_setting(&body) {
                    Ok(v) => {
                        disk["voice"] = json!({"elevenlabs": v});
                        json!({"t": "ack"})
                    }
                    Err(why) => json!({"t": "error", "why": why}),
                }
            };
            if reply["t"] == "ack" {
                let _ = save_config(&h.state, &disk);
                let snap = h.snapshot();
                h.broadcast(&snap);
            }
            respond_json(&mut w, "200 OK", &reply, port)
        }
        _ => respond(&mut w, "404 Not Found", "text/plain", b"", port),
    }
}

fn youtube_oauth(w: &mut TcpStream, req: &Request, hub: &Shared, port: u16) {
    let pending = hub.lock().ok().and_then(|mut h| h.yt_pending.take());
    let page = |title: &str, line: &str| {
        format!("<!doctype html><meta charset=utf-8><title>{title}</title>\
                 <body style=\"font:16px system-ui;background:#0a0c0a;color:#eef3ee;padding:48px\">\
                 <h1>{title}</h1><p>{line}</p></body>")
    };
    let result = match (pending, req.q("state"), req.q("code")) {
        (_, _, None) => Err(req.q("error").map(|e| clean_text(e, 80)).unwrap_or_else(|| "no answer from Google".into())),
        (Some((st, verifier, at)), Some(given), Some(code))
            if token_ok(Some(given), &st) && at.elapsed() < Duration::from_secs(600) => {
            let state = hub.lock().map(|h| h.state.clone()).unwrap_or_default();
            crate::stage_youtube::finish(&state, port, code, &verifier)
        }
        _ => Err("that sign-in is stale: press Connect on the desk again".into()),
    };
    let html = match &result {
        Ok(()) => page("YouTube connected", "You can close this tab and post recordings from the desk."),
        Err(why) => page("YouTube didn't connect", &clean_text(why, 200).replace('<', "&lt;")),
    };
    if result.is_ok() {
        if let Ok(mut h) = hub.lock() {
            let state = h.state.clone();
            h.broadcast(&json!({"t": "youtube", "youtube": crate::stage_youtube::public(&state)}));
        }
    }
    respond(w, "200 OK", "text/html; charset=utf-8", html.as_bytes(), port);
}

/// Send one recording to YouTube on a thread, telling the desk as it goes.
fn start_upload(hub: &Shared, state: &Path, body: &Value) -> Result<Value, String> {
    let file = body["file"].as_str().and_then(safe_file).filter(|f| f.ends_with(".mkv"))
        .ok_or("which recording?")?;
    let path = crate::stage_live::recordings_dir(state).join(&file);
    if !path.is_file() {
        return Err(format!("{file} isn't in the recordings"));
    }
    if crate::stage_youtube::public(state)["connected"] != true {
        return Err("connect YouTube first".into());
    }
    let title = clean_text(body["title"].as_str().unwrap_or(""), 100);
    let title = if title.is_empty() { file.trim_end_matches(".mkv").to_string() } else { title };
    let desc = body["description"].as_str().unwrap_or("").to_string();
    let privacy = body["privacy"].as_str().unwrap_or("private").to_string();
    {
        let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
        if h.uploads[&file]["state"] == "uploading" {
            return Err("that one is already on its way".into());
        }
        h.uploads[&file] = json!({"state": "uploading", "sent": 0, "total": 0});
    }
    let (hub, state) = (Arc::clone(hub), state.to_path_buf());
    std::thread::spawn(move || {
        let tell = |v: Value| {
            if let Ok(mut h) = hub.lock() {
                h.uploads[&file] = v.clone();
                h.broadcast(&json!({"t": "upload", "file": file, "upload": v}));
            }
        };
        let mut last = Instant::now() - Duration::from_secs(5);
        let got = crate::stage_youtube::upload(&state, &path, &title, &desc, &privacy, |sent, total| {
            if last.elapsed() > Duration::from_millis(700) || sent == total {
                last = Instant::now();
                tell(json!({"state": "uploading", "sent": sent, "total": total}));
            }
        });
        match got {
            Ok(id) => tell(json!({"state": "done", "video": id, "url": format!("https://youtu.be/{id}")})),
            Err(why) => tell(json!({"state": "failed", "why": why})),
        }
    });
    Ok(json!({"t": "ack"}))
}

fn live_snapshot(state: &Path, live: &crate::stage_live::Shared) -> Value {
    let v = crate::stage_live::load(state);
    let l = live.lock().unwrap_or_else(|p| p.into_inner());
    crate::stage_live::public(state, &v, &l)
}

/// The chat settings, merged onto what is there. Unknown keys are refused.
pub fn chat_setting(was: Option<&Value>, patch: &Value) -> Result<Value, String> {
    let mut c = was.cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    let p = patch.as_object().ok_or("chat is an object")?;
    let words = |v: &Value, what: &str| -> Result<Value, String> {
        let list: Vec<String> = match v {
            Value::Array(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
            Value::String(s) => s.split([',', '\n']).map(String::from).collect(),
            _ => return Err(format!("{what} is a list")),
        };
        let list: Vec<String> = list.iter().map(|w| clean_text(w, 40).to_lowercase())
            .filter(|w| !w.is_empty()).take(500).collect();
        Ok(json!(list))
    };
    for (k, v) in p {
        match k.as_str() {
            "twitch" => {
                let raw = v.as_str().unwrap_or("").trim();
                c[k] = if raw.is_empty() {
                    json!("")
                } else {
                    json!(crate::stage_chat::twitch_channel(raw).ok_or("a Twitch channel is its name, like tarshish")?)
                };
            }
            "youtube" => {
                let raw = clean_text(v.as_str().unwrap_or(""), 120);
                if !raw.is_empty() && crate::stage_sources::youtube_target(&raw).is_none() {
                    return Err("YouTube: a channel's @handle or url, or a live video's url".into());
                }
                c[k] = json!(raw);
            }
            "kick" => {
                let raw = v.as_str().unwrap_or("").trim();
                c[k] = if raw.is_empty() {
                    json!("")
                } else {
                    json!(crate::stage_sources::kick_slug(raw).ok_or("a Kick channel is its name, like tarshish")?)
                };
            }
            "youtube_budget" => c[k] = json!(v.as_u64().ok_or("youtube_budget is quota units a day")?.min(1_000_000)),
            "mode" => {
                let m = v.as_str().unwrap_or("");
                if !crate::stage_chat::MODES.contains(&m) {
                    return Err("chat mode is open, approve or off".into());
                }
                c[k] = json!(m);
            }
            "trigger" => c[k] = json!(clean_text(v.as_str().unwrap_or(""), 24).to_lowercase()),
            "cooldown" => c[k] = json!(v.as_u64().ok_or("cooldown is seconds")?.min(3600)),
            "block_words" | "block_users" => c[k] = words(v, k)?,
            "links" | "bots" => c[k] = json!(v.as_bool().ok_or_else(|| format!("{k} is true or false"))?),
            other => return Err(format!("{other:?} isn't a chat setting")),
        }
    }
    Ok(c)
}

/// The ElevenLabs pick, as the desk sends it.
pub fn elevenlabs_setting(v: &Value) -> Result<Value, String> {
    let id = v["voice_id"].as_str().filter(|i| crate::stage_voice::id_ok(i)).ok_or("pick a voice")?;
    let model = v["model"].as_str().filter(|m| crate::stage_voice::id_ok(m))
        .unwrap_or(crate::stage_voice::DEFAULT_MODEL);
    let mut settings = json!({});
    for (k, lo, hi) in [("stability", 0.0, 1.0), ("similarity_boost", 0.0, 1.0), ("style", 0.0, 1.0), ("speed", 0.7, 1.2)] {
        if let Some(x) = v.pointer(&format!("/settings/{k}")).and_then(Value::as_f64) {
            settings[k] = json!(x.clamp(lo, hi));
        }
    }
    Ok(json!({
        "voice_id": id,
        "voice_name": clean_text(v["voice_name"].as_str().unwrap_or(""), 60),
        "model": model,
        "settings": settings,
        "daily_chars": v["daily_chars"].as_u64().unwrap_or(0).min(10_000_000),
    }))
}

/// The settable half of the config. Unknown keys are refused, not ignored.
pub fn set_config(cfg: &mut Value, patch: &Value, models: &Path) -> Value {
    let Some(p) = patch.as_object() else {
        return json!({"t": "error", "why": "a JSON object"});
    };
    for (k, v) in p {
        match k.as_str() {
            "name" => cfg[k] = json!(clean_text(v.as_str().unwrap_or(""), NAME_MAX)),
            // The pill beside the name. Blank is the default, never nothing:
            // an agent on stage is shown as one (its own SOUL says the same).
            "label" => {
                let l = clean_text(v.as_str().unwrap_or(""), NAME_MAX);
                cfg[k] = json!(if l.is_empty() { "AI agent".to_string() } else { l });
            }
            "accent" => {
                let ok = v.as_str().is_some_and(|c| {
                    c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
                });
                if !ok {
                    return json!({"t": "error", "why": "accent is a colour like #6ee7b7"});
                }
                cfg[k] = v.clone();
            }
            "agent" => {
                let a = v.as_str().unwrap_or("");
                let ok = a.is_empty() || (a.len() == 42 && a.starts_with("0x") && a[2..].bytes().all(|b| b.is_ascii_hexdigit()));
                if !ok {
                    return json!({"t": "error", "why": "agent is a wallet, 0x and forty hex digits"});
                }
                cfg[k] = json!(a);
            }
            "browser_voice" => {
                let name = clean_text(v.get("name").and_then(Value::as_str).unwrap_or(""), 80);
                let rate = v.get("rate").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.5, 2.0);
                let pitch = v.get("pitch").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 2.0);
                cfg[k] = json!({"name": name, "rate": rate, "pitch": pitch});
            }
            "model" => {
                let model = pack_model(v, |f| {
                    let n = safe_file(f).filter(|n| model_ext(n).is_some()).ok_or_else(|| format!("{f:?}"))?;
                    if models.join(&n).is_file() { Ok(n) } else { Err(format!("{n} isn't uploaded")) }
                })
                .and_then(|mut m| bake_moves(models, &mut m).map(|_| m));
                match model {
                    Ok(m) => cfg[k] = m,
                    Err(why) => return json!({"t": "error", "why": why}),
                }
            }
            "chat" => match chat_setting(cfg.get("chat"), v) {
                Ok(c) => cfg[k] = c,
                Err(why) => return json!({"t": "error", "why": why}),
            },
            // Switches only: the ears listen (`on`), the agent's wallet is
            // followed on the island (`follow`). The engine is not here.
            "ears" | "game" => {
                let field = if k == "ears" { "on" } else { "follow" };
                let Some(on) = v.get(field).and_then(Value::as_bool) else {
                    return json!({"t": "error", "why": format!("{k} is {{\"{field}\": true|false}}")});
                };
                cfg[k] = json!({field: on});
            }
            other => return json!({"t": "error", "why": format!("{other:?} isn't a stage setting")}),
        }
    }
    json!({"t": "ack", "op": "config"})
}

fn socket(stream: TcpStream, leftover: Vec<u8>, hub: Shared, role: &str, name: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(40)));
    let _ = stream.set_nodelay(true);
    let cfg = WebSocketConfig::default()
        .max_message_size(Some(BODY_MAX))
        .max_frame_size(Some(BODY_MAX));
    let mut ws = WebSocket::from_partially_read(stream, leftover, Role::Server, Some(cfg));
    let (tx, rx): (Sender<String>, Receiver<String>) = channel();
    let id = {
        let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
        let id = h.next;
        h.next += 1;
        let role = if ["view", "drive", "desk"].contains(&role) { role } else { "drive" };
        h.clients.push(Client { id, role: role.into(), name: clean_text(name, NAME_MAX), tx });
        let snap = h.snapshot();
        let _ = ws.send(Message::text(snap.to_string()));
        h.broadcast(&json!({"t": "clients", "clients": snap["clients"]}));
        id
    };
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                let reply = match serde_json::from_str::<Value>(t.as_str()) {
                    Ok(op) => {
                        let mut reply = apply(&hub, &op, Some(id));
                        if let Some(rid) = op.get("rid") {
                            reply["rid"] = rid.clone();
                        }
                        reply
                    }
                    Err(e) => json!({"t": "error", "why": e.to_string()}),
                };
                if ws.send(Message::text(reply.to_string())).is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        let mut dead = false;
        while let Ok(out) = rx.try_recv() {
            if ws.send(Message::text(out)).is_err() {
                dead = true;
                break;
            }
        }
        if dead {
            break;
        }
    }
    let mut h = hub.lock().unwrap_or_else(|p| p.into_inner());
    h.clients.retain(|c| c.id != id);
    let clients = h.snapshot()["clients"].clone();
    h.broadcast(&json!({"t": "clients", "clients": clients}));
    drop(h);
    let _ = ws.get_mut().shutdown(Shutdown::Both);
}

/// Bind and serve on a thread. Returns once the port is held, so a caller can
/// connect straight after.
pub fn spawn(state: &Path, port: u16) -> std::io::Result<(Shared, u16)> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    let hub: Shared = Arc::new(Mutex::new(Hub::new(state, token(state)?, port)));
    let h2 = Arc::clone(&hub);
    std::thread::spawn(move || serve(listener, h2));
    restart_readers(&hub);
    Ok((hub, port))
}

fn serve(listener: TcpListener, hub: Shared) {
    let live = Arc::new(AtomicUsize::new(0));
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        if live.load(Ordering::SeqCst) >= CONN_MAX {
            let _ = conn.shutdown(Shutdown::Both);
            continue;
        }
        live.fetch_add(1, Ordering::SeqCst);
        let (hub, live) = (Arc::clone(&hub), Arc::clone(&live));
        std::thread::spawn(move || {
            handle(conn, hub);
            live.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// A POST to a running hub, for the CLI. `Err` when no hub is listening.
pub fn post_local(port: u16, path: &str, token: &str, body: &Value) -> Result<Value, String> {
    let mut s = TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(1))
        .map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let body = body.to_string();
    let req = format!(
        "POST {path}?k={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = String::new();
    s.read_to_string(&mut out).map_err(|e| e.to_string())?;
    let (_, json_body) = out.split_once("\r\n\r\n").ok_or("no reply")?;
    serde_json::from_str(json_body).map_err(|e| format!("{e}: {}", out.lines().next().unwrap_or("")))
}

pub fn urls(port: u16, token: &str) -> (String, String) {
    (
        format!("http://127.0.0.1:{port}/?k={token}"),
        format!("http://127.0.0.1:{port}/stage?k={token}"),
    )
}

// ── a driver: what `elo stage say` and `elo stage mcp` hold ──────────────────

pub struct Driver {
    ws: WebSocket<TcpStream>,
    rid: u64,
    /// Events read while waiting for a reply, kept for whoever waits on them.
    pub seen: VecDeque<Value>,
    /// The last line waited on was cut short: the owner started talking.
    pub cut: bool,
}

impl Driver {
    pub fn connect(port: u16, token: &str, name: &str) -> Result<Driver, String> {
        let stream = TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_secs(2),
        )
        .map_err(|e| format!("no stage on 127.0.0.1:{port} ({e})"))?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let url = format!(
            "ws://127.0.0.1:{port}/ws?k={token}&role=drive&name={}",
            name.bytes()
                .map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") })
                .collect::<String>()
        );
        let (ws, _) = tungstenite::client::client(url.as_str(), stream)
            .map_err(|e| format!("the stage refused the connection: {e}"))?;
        let mut d = Driver { ws, rid: 0, seen: VecDeque::new(), cut: false };
        // The first frame is always the state snapshot.
        d.next_event(Duration::from_secs(3)).ok_or("the stage said nothing")?;
        Ok(d)
    }

    pub fn next_event(&mut self, wait: Duration) -> Option<Value> {
        if let Some(v) = self.seen.pop_front() {
            return Some(v);
        }
        self.ws.get_mut().set_read_timeout(Some(wait.max(Duration::from_millis(1)))).ok();
        loop {
            match self.ws.read() {
                Ok(Message::Text(t)) => return serde_json::from_str(t.as_str()).ok(),
                Ok(Message::Close(_)) => return None,
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    /// Send one op and wait for its reply. Broadcast events read on the way
    /// are kept in `seen`.
    pub fn ask(&mut self, mut op: Value) -> Result<Value, String> {
        self.rid += 1;
        op["rid"] = json!(self.rid);
        self.ws
            .send(Message::text(op.to_string()))
            .map_err(|e| format!("the stage went away: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut kept = VecDeque::new();
        while Instant::now() < deadline {
            self.ws.get_mut().set_read_timeout(Some(Duration::from_millis(250))).ok();
            match self.ws.read() {
                Ok(Message::Text(t)) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap_or_default();
                    if v.get("rid") == Some(&json!(self.rid)) {
                        self.keep(kept);
                        return Ok(v);
                    }
                    kept.push_back(v);
                }
                Ok(Message::Close(_)) => return Err("the stage closed".into()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) => return Err(format!("the stage went away: {e}")),
            }
        }
        self.keep(kept);
        Err("the stage didn't answer in 5 s".into())
    }

    /// Wait until line `id` has been heard on a stage, or `limit` passes.
    /// `Ok(true)` heard; `Ok(false)` no stage reported it in time.
    pub fn wait_spoken(&mut self, id: u64, limit: Duration) -> Result<bool, String> {
        let deadline = Instant::now() + limit;
        // Events already read while waiting for the ack.
        let mut rest = VecDeque::new();
        let mut heard = false;
        self.cut = false;
        while let Some(v) = self.seen.pop_front() {
            if v["t"] == "spoken" && v["id"] == json!(id) {
                heard = true;
                self.cut = v["cut"] == true;
            } else {
                rest.push_back(v);
            }
        }
        self.seen = rest;
        while !heard && Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.next_event(left.min(Duration::from_millis(500))) {
                Some(v) if v["t"] == "spoken" && v["id"] == json!(id) => {
                    heard = true;
                    self.cut = v["cut"] == true;
                }
                Some(v) if v["t"] == "say" && v["id"] == json!(id) && v["voice_error"].is_string() => {
                    // Keep waiting for the stage to show it; the error is reported by the caller.
                    self.seen.push_back(v);
                    if self.seen.len() > 64 {
                        self.seen.pop_front();
                    }
                }
                Some(_) => {}
                None => {
                    if !self.alive() {
                        return Err("the stage went away".into());
                    }
                }
            }
        }
        Ok(heard)
    }

    /// A driver that never waits would otherwise keep every event forever.
    fn keep(&mut self, events: VecDeque<Value>) {
        self.seen.extend(events);
        while self.seen.len() > 64 {
            self.seen.pop_front();
        }
    }

    fn alive(&mut self) -> bool {
        self.ws.can_write()
    }
}

/// How long a line should take, for a caller with no stage to report it:
/// about fifteen characters a second, plus a beat.
pub fn speak_estimate(text: &str) -> Duration {
    Duration::from_millis(1200 + 66 * text.chars().count() as u64).min(Duration::from_secs(60))
}
