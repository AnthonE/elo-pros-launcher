//! `elo stage …` — the verbs. The hub is `stage.rs`; the agent's tool is
//! `stage_mcp.rs`.

use crate::args::Args;
use crate::stage::{self, DEFAULT_PORT};
use crate::{stage_chat, stage_ears, stage_live, stage_sources, stage_voice};
use elo_depot::DepotError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
elo stage — an agent's body on your screen: a VTuber you run from this machine

  elo stage                       start the stage and open its desk
  elo stage serve                 start it without opening anything
  elo stage url                   the desk's url, and the one to add to OBS
  elo stage model <file|dir>…     use a model: one .vrm, a pack folder with a
                                  stage-pack.json, or pictures named for their
                                  expressions (neutral.png, happy.png, …). A
                                  glb pack's `moves` file is put on its body
                                  here, once (animate.html makes one)
  elo stage set key=value…        name, label, accent (#rrggbb), agent (0x…)
  elo stage voice elevenlabs [voice] [model=… speed=… cap=…]
                                  speak with ElevenLabs: asks for your API key
                                  once (kept here, 0600), then lists your voices
  elo stage voice -- <command…>   speak with a local voice: the line arrives on
                                  stdin; {out} in an argument is the file to
                                  write, else the audio is read from stdout
  elo stage voice test [text]     make one line with the voice that is set
  elo stage voice off             back to the stage page's own voice
  elo stage chat                  where chat comes from, and the settings
  elo stage chat twitch <channel> read a Twitch channel's chat (no account
                                  needed); `twitch off` stops
  elo stage chat youtube <@handle | url>
                                  read YouTube chat with your own API key
                                  (`elo stage chat youtube-key` saves it)
  elo stage chat kick <channel>   read Kick chat, through elopros.com
  elo stage chat mode open|approve|off
                                  approve: you pass each message on the desk
  elo stage chat set key=value…   trigger (e.g. !ask), cooldown (seconds),
                                  block_words / block_users (a,b,c), links,
                                  bots (true|false)
  elo stage chat as <name> <text…>    a message as a viewer, to try it
  elo stage live                  going live everywhere: the places, and what
                                  to put in OBS (one stream in, a copy to each)
  elo stage live add <platform> [url=…]   twitch, youtube, kick, x, tiktok,
                                  facebook, pumpfun or custom; asks for the
                                  stream key
  elo stage live remove <id>      take a place off
  elo stage live go | stop        start or stop sending (needs ffmpeg)
  elo stage ears                  hearing you: the engine, and whether it is on
  elo stage ears elevenlabs       turn your speech into text with ElevenLabs
                                  (the key the voice uses)
  elo stage ears -- <command…>    with a local command: the WAV at {in}, else
                                  on stdin; the text on stdout
  elo stage ears on | off         listen to your microphone (the desk asks
                                  the browser for it) or stop
  elo stage follow on | off       tell the agent what its body does on the
                                  island (the `agent` wallet, set first)
  elo stage event <text…>         report something from a game to the agent
  elo stage say <text…>           say a line now (any script can drive it)
  elo stage face <expression>     change the face
  elo stage move <name>           play one of the body's moves (wave, nod…)
  elo stage layout <name>         corner, center, full, hidden or brb
  elo stage mcp                   the stage as an MCP tool, for an agent
                                  harness (starts the stage if none is running)

  --port N         the port on 127.0.0.1 (default 7357)
  --name NAME      for `mcp`: whose hand this is, shown on the desk
  --json           machine-readable output

Everything stays on this machine: the models, the voice, the audio. OBS adds
the stage as a Browser source (1920×1080; check \"Control audio via OBS\").
";

fn port(a: &Args) -> Result<u16, DepotError> {
    match a.flags.get("--port") {
        None => Ok(std::env::var("ELO_STAGE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT)),
        Some(p) => p
            .parse::<u16>()
            .ok()
            .filter(|p| *p >= 1024)
            .ok_or_else(|| DepotError::new("--port is a number from 1024 to 65535")),
    }
}

fn err(e: impl std::fmt::Display) -> DepotError {
    DepotError::new(e.to_string())
}

pub fn run(a: &Args, state: &Path, json_out: bool) -> Result<i32, DepotError> {
    let port = port(a)?;
    let token = stage::token(state).map_err(|e| err(format!("the stage's key: {e}")))?;
    let (desk, obs) = stage::urls(port, &token);
    let sub = a.positional.first().map(String::as_str);
    match sub {
        None | Some("serve") => {
            let (_hub, bound) = stage::spawn(state, port).map_err(|e| {
                err(format!(
                    "127.0.0.1:{port} is taken ({e}) — is a stage already running? \
                     `elo stage url` prints its address, or pick another with --port"
                ))
            })?;
            let (desk, obs) = stage::urls(bound, &token);
            if json_out {
                println!("{}", json!({"desk": desk, "obs": obs, "port": bound}));
            } else {
                println!("the stage is up on 127.0.0.1:{bound}");
                println!("  desk   {desk}");
                println!("  OBS    {obs}");
                println!("         a Browser source, 1920×1080, \"Control audio via OBS\" on");
                println!("  agent  elo stage mcp   (an MCP server; `elo stage help`)");
                println!("ctrl-c stops it. The urls carry this stage's key: keep them off stream.");
            }
            if sub.is_none() {
                let _ = elo_depot::launch::open_page(&desk);
            }
            loop {
                std::thread::park();
            }
        }
        Some("help") => {
            print!("{USAGE}");
            Ok(0)
        }
        Some("url") => {
            if json_out {
                println!("{}", json!({"desk": desk, "obs": obs, "port": port}));
            } else {
                println!("desk  {desk}\nOBS   {obs}");
            }
            Ok(0)
        }
        Some("mcp") => {
            let name = a.flags.get("--name").cloned().unwrap_or_else(|| {
                stage::load_config(state)["name"].as_str().unwrap_or("agent").to_string()
            });
            crate::stage_mcp::Server::new(state, port, &name)
                .serve()
                .map_err(|e| err(format!("stage mcp: {e}")))?;
            Ok(0)
        }
        Some("model") => {
            let paths: Vec<PathBuf> = a.positional[1..].iter().map(PathBuf::from).collect();
            if paths.is_empty() {
                return Err(err("which model? — `elo stage model <file.vrm | pack folder | pictures…>`"));
            }
            let model = stage::import(state, &paths).map_err(err)?;
            let live = sync(port, &token, state);
            if json_out {
                println!("{}", json!({"model": model, "live": live}));
            } else {
                println!("model set: {}", describe(&model));
                println!("{}", if live { "  the running stage switched to it" } else { "  the stage uses it when it starts" });
            }
            Ok(0)
        }
        Some("set") => {
            let mut patch = json!({});
            for kv in &a.positional[1..] {
                let (k, v) = kv.split_once('=').ok_or_else(|| err(format!("{kv:?}: write key=value")))?;
                patch[k] = json!(v);
            }
            let mut cfg = stage::load_config(state);
            let models = stage::dir(state).join("models");
            let reply = stage::set_config(&mut cfg, &patch, &models);
            if reply["t"] == "error" {
                return Err(err(reply["why"].as_str().unwrap_or("refused")));
            }
            stage::save_config(state, &cfg).map_err(err)?;
            sync(port, &token, state);
            println!("set");
            Ok(0)
        }
        Some("voice") => voice(a, state, port, &token),
        Some("chat") => chat(a, state, port, &token, json_out),
        Some("live") => live(a, state, port, &token, json_out),
        Some("ears") => ears(a, state, port, &token),
        Some("follow") => {
            let on = match a.positional.get(1).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => return Err(err("`elo stage follow on` or `off`")),
            };
            let mut cfg = stage::load_config(state);
            if on && cfg["agent"].as_str().is_none_or(str::is_empty) {
                return Err(err("set the agent's wallet first: `elo stage set agent=0x…`"));
            }
            cfg["game"] = json!({"follow": on});
            stage::save_config(state, &cfg).map_err(err)?;
            sync(port, &token, state);
            println!("{}", if on { "following the agent on the island" } else { "not following" });
            Ok(0)
        }
        Some("event") => {
            let text = a.positional[1..].join(" ");
            let reply = stage::post_local(port, "/event", &token, &json!({"text": text, "from": "a script"}))
                .map_err(|e| err(format!("no stage on 127.0.0.1:{port} ({e}) — start one with `elo stage`")))?;
            if reply["t"] == "error" {
                return Err(err(reply["why"].as_str().unwrap_or("refused")));
            }
            if json_out {
                println!("{reply}");
            }
            Ok(0)
        }
        Some(verb @ ("say" | "face" | "move" | "layout")) => {
            let rest = a.positional[1..].join(" ");
            let op = match verb {
                "say" => json!({"op": "say", "text": rest}),
                "face" => json!({"op": "expression", "name": rest}),
                "move" => json!({"op": "move", "name": rest}),
                _ => json!({"op": "layout", "name": rest}),
            };
            let reply = stage::post_local(port, "/op", &token, &op)
                .map_err(|e| err(format!("no stage on 127.0.0.1:{port} ({e}) — start one with `elo stage`")))?;
            if reply["t"] == "error" {
                return Err(err(reply["why"].as_str().unwrap_or("refused")));
            }
            if json_out {
                println!("{reply}");
            }
            Ok(0)
        }
        Some(other) => Err(err(format!("unknown stage command {other:?} — `elo stage help`"))),
    }
}

fn ears(a: &Args, state: &Path, port: u16, token: &str) -> Result<i32, DepotError> {
    let rest: Vec<String> = a.positional[1..].to_vec();
    let mut cfg = stage::load_config(state);
    match rest.first().map(String::as_str) {
        None => {
            let on = cfg.pointer("/ears/on").and_then(Value::as_bool).unwrap_or(false);
            match stage_ears::engine(&cfg) {
                stage_ears::Engine::None => println!("hearing: none — `elo stage ears elevenlabs` or `elo stage ears -- <command…>`"),
                stage_ears::Engine::Command(argv) => println!("hearing: {}", argv.join(" ")),
                stage_ears::Engine::ElevenLabs { model } => println!("hearing: ElevenLabs, {model}"),
            }
            println!("  {}", if on { "on: the desk listens to your microphone" } else { "off: `elo stage ears on`" });
            return Ok(0);
        }
        Some(sw @ ("on" | "off")) => cfg["ears"] = json!({"on": sw == "on"}),
        Some("elevenlabs") => {
            if stage_voice::key(state).is_none() {
                return Err(err("no ElevenLabs key yet: `elo stage voice elevenlabs` adds it"));
            }
            let model = rest.get(1).filter(|m| stage_voice::id_ok(m)).cloned()
                .unwrap_or_else(|| stage_ears::DEFAULT_STT_MODEL.into());
            cfg["hearing"] = json!({"elevenlabs": {"model": model}});
        }
        Some("none") => {
            cfg.as_object_mut().map(|o| o.remove("hearing"));
        }
        Some(_) => {
            if !runnable(&rest[0]) {
                return Err(err(format!("{} isn't a command this machine can run", rest[0])));
            }
            cfg["hearing"] = json!({"cmd": rest});
        }
    }
    stage::save_config(state, &cfg).map_err(err)?;
    sync(port, token, state);
    println!("set");
    Ok(0)
}

fn voice(a: &Args, state: &Path, port: u16, token: &str) -> Result<i32, DepotError> {
    let rest: Vec<String> = a.positional[1..].to_vec();
    let mut cfg = stage::load_config(state);
    match rest.first().map(String::as_str) {
        None => {
            match stage_voice::of(&cfg) {
                stage_voice::Voice::Page => {
                    println!("voice: the stage page's own");
                    println!("  `elo stage voice elevenlabs` or `elo stage voice -- <command…>` sets one");
                }
                stage_voice::Voice::Command(argv) => println!("voice: {}", argv.join(" ")),
                stage_voice::Voice::ElevenLabs { voice_id, model, daily_chars, .. } => {
                    let name = cfg.pointer("/voice/elevenlabs/voice_name").and_then(Value::as_str).unwrap_or("");
                    println!("voice: ElevenLabs, {} ({voice_id}), {model}", if name.is_empty() { "?" } else { name });
                    let used = stage_voice::used_today(state);
                    match daily_chars {
                        0 => println!("  {used} characters today, no cap"),
                        cap => println!("  {used} of {cap} characters today"),
                    }
                    if stage_voice::key(state).is_none() {
                        println!("  no key: run `elo stage voice elevenlabs` to add one");
                    }
                }
            }
            return Ok(0);
        }
        Some("off") => {
            cfg.as_object_mut().map(|o| o.remove("voice"));
        }
        Some("test") => {
            let text = if rest.len() > 1 { rest[1..].join(" ") } else { "testing, one two.".into() };
            match stage_voice::of(&cfg) {
                stage_voice::Voice::Page => {
                    println!("no voice is set: the stage page speaks with the browser's");
                    return Ok(0);
                }
                v => {
                    let (bytes, kind) = stage_voice::render(state, &v, &text).map_err(err)?;
                    println!("the voice made {} bytes of {kind}", bytes.len());
                    return Ok(0);
                }
            }
        }
        Some("elevenlabs") => return elevenlabs(&rest[1..], state, port, token),
        Some(_) => {
            if !runnable(&rest[0]) {
                return Err(err(format!("{} isn't a command this machine can run", rest[0])));
            }
            cfg["voice"] = json!({"cmd": rest});
        }
    }
    stage::save_config(state, &cfg).map_err(err)?;
    let _ = stage::post_local(port, "/config", token, &json!({}));
    println!("voice set — `elo stage voice test` tries it");
    Ok(0)
}

/// `elo stage voice elevenlabs [voice] [model=… speed=… cap=…]`: the key once,
/// then a voice from your own list.
fn elevenlabs(args: &[String], state: &Path, port: u16, token: &str) -> Result<i32, DepotError> {
    let net = elo_net::Net::new();
    let key = match stage_voice::key(state) {
        Some(k) => k,
        None => {
            println!("ElevenLabs needs an API key: elevenlabs.io → Developers → API keys.");
            println!("It is kept on this machine ({}), sent only to ElevenLabs.", stage_voice::key_path(state).display());
            let k = crate::prompt::secret("ElevenLabs API key: ").map_err(err)?;
            let k = k.trim().to_string();
            stage_voice::voices(&net, &k).map_err(err)?;
            stage_voice::save_key(state, &k).map_err(err)?;
            println!("  key saved");
            k
        }
    };
    let (mut pick, mut extra) = (Vec::new(), json!({"settings": {}}));
    for x in args {
        match x.split_once('=') {
            Some(("model", m)) => extra["model"] = json!(m),
            Some(("speed", v)) => extra["settings"]["speed"] = json!(v.parse::<f64>().map_err(|_| err("speed is a number, 0.7 to 1.2"))?),
            Some(("stability", v)) => extra["settings"]["stability"] = json!(v.parse::<f64>().map_err(|_| err("stability is 0 to 1"))?),
            Some(("cap", v)) => extra["daily_chars"] = json!(v.parse::<u64>().map_err(|_| err("cap is characters a day"))?),
            Some((k, _)) => return Err(err(format!("{k:?} — model, speed, stability or cap"))),
            None => pick.push(x.clone()),
        }
    }
    let list = stage_voice::voices(&net, &key).map_err(err)?;
    if list.is_empty() {
        return Err(err("your ElevenLabs account has no voices yet: add one at elevenlabs.io → Voices"));
    }
    let want = pick.join(" ").to_lowercase();
    let chosen = if want.is_empty() {
        for (i, v) in list.iter().enumerate() {
            println!("{:>3}. {}  {}  {}", i + 1, v["name"].as_str().unwrap_or(""),
                     v["category"].as_str().unwrap_or(""), v["about"].as_str().unwrap_or(""));
        }
        print!("which voice? number: ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map_err(err)?;
        let n: usize = line.trim().parse().map_err(|_| err("a number from the list — or name the voice: `elo stage voice elevenlabs <name>`"))?;
        list.get(n.wrapping_sub(1)).cloned().ok_or_else(|| err("that number isn't on the list"))?
    } else {
        list.iter()
            .find(|v| v["voice_id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(&want)))
            .or_else(|| list.iter().find(|v| v["name"].as_str().is_some_and(|n| n.to_lowercase() == want)))
            .or_else(|| list.iter().find(|v| v["name"].as_str().is_some_and(|n| n.to_lowercase().starts_with(&want))))
            .cloned()
            .ok_or_else(|| err(format!("no voice called {want:?} on your account — `elo stage voice elevenlabs` lists them")))?
    };
    extra["voice_id"] = chosen["voice_id"].clone();
    extra["voice_name"] = chosen["name"].clone();
    let setting = stage::elevenlabs_setting(&extra).map_err(err)?;
    let live = stage::post_local(port, "/voice/elevenlabs", token, &setting).is_ok_and(|r| r["t"] == "ack");
    if !live {
        let mut cfg = stage::load_config(state);
        cfg["voice"] = json!({"elevenlabs": setting});
        stage::save_config(state, &cfg).map_err(err)?;
    }
    println!("voice: ElevenLabs, {} with {}", chosen["name"].as_str().unwrap_or("?"),
             setting["model"].as_str().unwrap_or(stage_voice::DEFAULT_MODEL));
    println!("  `elo stage voice test` makes a line; `elo stage say hello` speaks one on the stage");
    Ok(0)
}

fn chat(a: &Args, state: &Path, port: u16, token: &str, json_out: bool) -> Result<i32, DepotError> {
    let rest: Vec<String> = a.positional[1..].to_vec();
    let patch = match rest.first().map(String::as_str) {
        None => {
            let cfg = stage::load_config(state);
            let live = http_get_state(port, token);
            if json_out {
                println!("{}", json!({"chat": cfg.get("chat"), "status": live.as_ref().map(|s| &s["live"]["chat_status"])}));
                return Ok(0);
            }
            let c = cfg.get("chat").cloned().unwrap_or_else(|| json!({}));
            let f = stage_chat::Filter::from_cfg(&cfg);
            match c["twitch"].as_str().filter(|t| !t.is_empty()) {
                Some(t) => println!("twitch: #{t}"),
                None => println!("twitch: none (`elo stage chat twitch <channel>`)"),
            }
            match c["youtube"].as_str().filter(|t| !t.is_empty()) {
                Some(t) => println!("youtube: {t}{}", if stage_sources::youtube_key(state).is_some() { "" } else {
                    " — needs an API key (`elo stage chat youtube-key`)" }),
                None => println!("youtube: none (`elo stage chat youtube <@handle or url>`)"),
            }
            match c["kick"].as_str().filter(|t| !t.is_empty()) {
                Some(t) => println!("kick: {t}"),
                None => println!("kick: none (`elo stage chat kick <channel>`)"),
            }
            println!("mode: {}{}", f.mode, if f.mode == "approve" { " (you pass each message on the desk)" } else { "" });
            println!("trigger: {}", if f.trigger.is_empty() { "none: every message" } else { &f.trigger });
            println!("cooldown: {} s a viewer", f.cooldown);
            println!("blocked: {} words, {} viewers; links {}, chat bots {}", f.block_words.len(), f.block_users.len(),
                     if f.links { "allowed" } else { "dropped" }, if f.bots { "ignored" } else { "read" });
            println!("other platforms: POST {{\"from\", \"text\", \"platform\"}} to http://127.0.0.1:{port}/chat?k=…");
            if let Some(s) = live {
                println!("now: {}", crate::stage_mcp::chat_sources(&s["config"], &s["live"]["chat_status"]));
            }
            return Ok(0);
        }
        Some(src @ ("twitch" | "youtube" | "kick")) => {
            let ch = rest.get(1).map(String::as_str).unwrap_or("");
            json!({src: if ch == "off" { "" } else { ch }})
        }
        Some("youtube-key") => {
            println!("A YouTube Data API key: console.cloud.google.com → APIs & Services → enable");
            println!("\"YouTube Data API v3\" → Credentials → Create credentials → API key.");
            let k = crate::prompt::secret("YouTube API key: ").map_err(err)?;
            stage_sources::save_youtube_key(state, &k).map_err(err)?;
            println!("  saved on this machine ({})", stage_sources::youtube_key_path(state).display());
            return Ok(0);
        }
        Some("mode") => json!({"mode": rest.get(1).cloned().unwrap_or_default()}),
        Some("set") => {
            let mut p = json!({});
            for kv in &rest[1..] {
                let (k, v) = kv.split_once('=').ok_or_else(|| err(format!("{kv:?}: write key=value")))?;
                p[k] = match k {
                    "cooldown" => json!(v.parse::<u64>().map_err(|_| err("cooldown is seconds"))?),
                    "links" | "bots" => json!(v == "true" || v == "yes" || v == "on"),
                    _ => json!(v),
                };
            }
            p
        }
        Some("as") => {
            let from = rest.get(1).ok_or_else(|| err("`elo stage chat as <name> <text…>`"))?;
            let text = rest[2..].join(" ");
            let reply = stage::post_local(port, "/chat", token, &json!({"from": from, "text": text, "platform": "test"}))
                .map_err(|e| err(format!("no stage on 127.0.0.1:{port} ({e}) — start one with `elo stage`")))?;
            match reply["t"].as_str() {
                Some("ack") => println!("#{} {}", reply["id"], reply["state"].as_str().unwrap_or("")),
                Some("dropped") => println!("dropped: {}", reply["why"].as_str().unwrap_or("")),
                _ => return Err(err(reply["why"].as_str().unwrap_or("refused"))),
            }
            return Ok(0);
        }
        Some(other) => return Err(err(format!("unknown chat command {other:?} — `elo stage help`"))),
    };
    let body = json!({"chat": patch});
    match stage::post_local(port, "/config", token, &body) {
        Ok(r) if r["t"] == "error" => return Err(err(r["why"].as_str().unwrap_or("refused"))),
        Ok(_) => println!("set, and the running stage switched"),
        Err(_) => {
            let mut cfg = stage::load_config(state);
            let models = stage::dir(state).join("models");
            let r = stage::set_config(&mut cfg, &body, &models);
            if r["t"] == "error" {
                return Err(err(r["why"].as_str().unwrap_or("refused")));
            }
            stage::save_config(state, &cfg).map_err(err)?;
            println!("set — the stage uses it when it starts");
        }
    }
    Ok(0)
}

fn live(a: &Args, state: &Path, port: u16, token: &str, json_out: bool) -> Result<i32, DepotError> {
    let rest: Vec<String> = a.positional[1..].to_vec();
    match rest.first().map(String::as_str) {
        None => {
            let v = stage_live::load(state);
            let snap = stage_live::public(state, &v, &stage_live::Live::default());
            if json_out {
                println!("{snap}");
                return Ok(0);
            }
            println!("in OBS: Settings → Stream → Service Custom");
            println!("  server      {}", snap["obs"]["server"].as_str().unwrap_or(""));
            println!("  stream key  {}", snap["obs"]["key"].as_str().unwrap_or(""));
            for d in snap["dests"].as_array().into_iter().flatten() {
                println!("{} {:<10} {:<16} {}  ({})", if d["on"] == true { "●" } else { "○" },
                         d["id"].as_str().unwrap_or(""), d["name"].as_str().unwrap_or(""),
                         d["key_hint"].as_str().unwrap_or(""), d["url"].as_str().unwrap_or(""));
            }
            if snap["dests"].as_array().is_none_or(|d| d.is_empty()) {
                println!("no places yet: `elo stage live add twitch`");
            }
            println!("recording: {}", if snap["record"] == true { "on" } else { "off" });
            if snap["ffmpeg"] != true {
                println!("ffmpeg isn't installed: it does the sending (winget, brew or apt install ffmpeg)");
            }
            Ok(0)
        }
        Some("add") => {
            let platform = rest.get(1).ok_or_else(|| err("which? twitch, youtube, kick, x, tiktok, facebook, pumpfun or custom"))?;
            let mut patch = json!({"platform": platform});
            for kv in &rest[2..] {
                match kv.split_once('=') {
                    Some(("url", u)) => patch["url"] = json!(u),
                    Some(("name", n)) => patch["name"] = json!(n),
                    _ => return Err(err(format!("{kv:?}: url=… or name=…"))),
                }
            }
            if let Some(p) = stage_live::PLATFORMS.iter().find(|p| p.0 == platform.as_str()) {
                println!("{}: {}", p.1, p.3);
            }
            let key = crate::prompt::secret("stream key: ").map_err(err)?;
            patch["key"] = json!(key.trim());
            stage_live::set_dest(state, &patch).map_err(err)?;
            println!("added — `elo stage live go` starts sending");
            Ok(0)
        }
        Some("remove") => {
            stage_live::remove_dest(state, rest.get(1).map(String::as_str).unwrap_or("")).map_err(err)?;
            println!("removed");
            Ok(0)
        }
        Some(verb @ ("go" | "stop")) => {
            let path = if verb == "go" { "/live/start" } else { "/live/stop" };
            let r = stage::post_local(port, path, token, &json!({}))
                .map_err(|e| err(format!("no stage on 127.0.0.1:{port} ({e}) — start one with `elo stage`")))?;
            if r["t"] == "error" {
                return Err(err(r["why"].as_str().unwrap_or("refused")));
            }
            println!("{}", r["live"]["state"].as_str().unwrap_or("ok"));
            Ok(0)
        }
        Some(other) => Err(err(format!("unknown live command {other:?} — `elo stage help`"))),
    }
}

fn http_get_state(port: u16, token: &str) -> Option<Value> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                                                     std::time::Duration::from_secs(1)).ok()?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    write!(s, "GET /state?k={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    serde_json::from_str(out.split_once("\r\n\r\n")?.1).ok()
}

/// Hand a running hub the config just written, so it switches without a
/// restart. `false` when no hub is listening (it reads the file on start).
fn sync(port: u16, token: &str, state: &Path) -> bool {
    let cfg = stage::load_config(state);
    let mut patch = json!({});
    for k in ["name", "label", "accent", "agent", "browser_voice", "model", "ears", "game"] {
        if let Some(v) = cfg.get(k) {
            patch[k] = v.clone();
        }
    }
    stage::post_local(port, "/config", token, &patch).is_ok_and(|r| r["t"] != "error")
}

fn describe(model: &Value) -> String {
    match model["kind"].as_str() {
        Some("vrm") => format!("the 3D model {}", model["file"].as_str().unwrap_or("?")),
        Some("glb") => format!(
            "the rigged model {}{}{}",
            model["file"].as_str().unwrap_or("?"),
            if model["face"].is_object() { ", with a face to draw" } else { " (no face: it won't blink or move its mouth)" },
            match model["clips"].as_object() {
                Some(c) if !c.is_empty() => format!(", moves: {}", c.keys().cloned().collect::<Vec<_>>().join(", ")),
                _ => String::new(),
            }
        ),
        _ => format!(
            "pictures, faces: {}",
            model["files"].as_object().map(|o| o.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default()
        ),
    }
}

/// Is this a program on PATH, or a file that exists?
fn runnable(cmd: &str) -> bool {
    let p = Path::new(cmd);
    if p.components().count() > 1 {
        return p.is_file();
    }
    let exts: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|d| exts.iter().any(|e| d.join(format!("{cmd}{e}")).is_file()))
        })
        .unwrap_or(false)
}
