//! Posting a recording to YouTube, from the desk.
//!
//! Operator, 2026-10-29: *"with us and youtube id probably upload over
//! streaming."*
//!
//! The stage keeps a recording of each stream (`stage_live.rs`); this sends
//! one to the streamer's own YouTube channel. Google's door for that is OAuth
//! with the `youtube.upload` scope, from an OAuth client of the streamer's own
//! (a "Desktop app" client in the same Google Cloud project as their YouTube
//! API key), so the app asking is theirs and no app of ours needs Google's
//! review. The consent page sends its answer to this hub on loopback, with a
//! PKCE check and a one-time state; the refresh token is kept in
//! `stage/youtube-oauth.json` (0600) and goes only to Google. The upload is
//! YouTube's resumable one, in 8 MiB pieces, so a large file survives a slow
//! line. Each upload costs 1,600 of the project's 10,000 daily quota units.

use crate::stage;
use elo_net::Net;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const AUTH: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const SCOPE: &str = "https://www.googleapis.com/auth/youtube.upload";
const CHUNK: u64 = 8 * 1024 * 1024;

/// A piece of the upload: 8 MiB, or `ELO_YOUTUBE_CHUNK` (a multiple of 256
/// KiB, as YouTube requires) for the suite.
fn chunk() -> u64 {
    std::env::var("ELO_YOUTUBE_CHUNK").ok().and_then(|v| v.parse::<u64>().ok())
        .filter(|c| *c >= 256 * 1024 && c % (256 * 1024) == 0).unwrap_or(CHUNK)
}

fn token_url() -> String {
    std::env::var("ELO_GOOGLE_TOKEN_URL").ok().filter(|u| !u.is_empty())
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".into())
}

fn upload_url() -> String {
    std::env::var("ELO_YOUTUBE_UPLOAD_URL").ok().filter(|u| !u.is_empty())
        .unwrap_or_else(|| "https://www.googleapis.com/upload/youtube/v3/videos".into())
}

fn path(state: &Path) -> PathBuf {
    stage::dir(state).join("youtube-oauth.json")
}

pub fn load(state: &Path) -> Value {
    std::fs::read(path(state)).ok().and_then(|b| serde_json::from_slice(&b).ok())
        .filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn save(state: &Path, v: &Value) -> Result<(), String> {
    std::fs::create_dir_all(stage::dir(state)).map_err(|e| e.to_string())?;
    stage::write_private(&path(state), v.to_string().as_bytes()).map_err(|e| e.to_string())
}

/// What the desk may see: whether a client and a channel are connected.
pub fn public(state: &Path) -> Value {
    let v = load(state);
    json!({"client": v["client_id"].is_string(), "connected": v["refresh_token"].is_string(),
           "channel": v["channel"]})
}

/// The streamer's OAuth client, from Google Cloud → Credentials → OAuth
/// client ID → Desktop app.
pub fn set_client(state: &Path, id: &str, secret: &str) -> Result<(), String> {
    let (id, secret) = (id.trim(), secret.trim());
    if !id.ends_with(".apps.googleusercontent.com") || id.len() > 200 || id.contains(char::is_whitespace) {
        return Err("a Google OAuth client id ends .apps.googleusercontent.com".into());
    }
    if secret.is_empty() || secret.len() > 200 || secret.contains(char::is_whitespace) {
        return Err("paste the client secret too".into());
    }
    save(state, &json!({"client_id": id, "client_secret": secret}))
}

fn b64url(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..=c.len() {
            out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/youtube/oauth")
}

/// Start a sign-in: the consent page's url, and the (state, verifier) to
/// keep for the answer.
pub fn begin(state: &Path, port: u16) -> Result<(String, String, String), String> {
    use sha2::Digest;
    let v = load(state);
    let id = v["client_id"].as_str().ok_or("add your OAuth client first")?;
    let verifier = format!("{:016x}{:016x}{:016x}{:016x}", stage::rand_u64(), stage::rand_u64(),
                           stage::rand_u64(), stage::rand_u64());
    let challenge = b64url(&sha2::Sha256::digest(verifier.as_bytes()));
    let st = format!("{:016x}{:016x}", stage::rand_u64(), stage::rand_u64());
    let url = format!(
        "{AUTH}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline\
         &prompt=consent&code_challenge={challenge}&code_challenge_method=S256&state={st}",
        enc(id), enc(&redirect_uri(port)), enc(SCOPE)
    );
    Ok((url, st, verifier))
}

fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    pairs.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect::<Vec<_>>().join("&").into_bytes()
}

fn token_call(net: &Net, body: Vec<u8>) -> Result<Value, String> {
    let (code, _, b) = net.request_full("POST", &token_url(), &[("content-type", "application/x-www-form-urlencoded")],
                                        Some(&body), 1 << 20)?;
    let v: Value = serde_json::from_slice(&b).unwrap_or_default();
    if code != 200 {
        return Err(format!("Google said {}", v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("no")));
    }
    Ok(v)
}

/// The consent page's answer: trade the code for a refresh token and keep it.
pub fn finish(state: &Path, port: u16, code: &str, verifier: &str) -> Result<(), String> {
    let mut v = load(state);
    let (id, secret) = (v["client_id"].as_str().unwrap_or("").to_string(), v["client_secret"].as_str().unwrap_or("").to_string());
    let r = token_call(&Net::new(), form(&[("code", code), ("client_id", &id), ("client_secret", &secret),
                                           ("redirect_uri", &redirect_uri(port)), ("grant_type", "authorization_code"),
                                           ("code_verifier", verifier)]))?;
    let refresh = r["refresh_token"].as_str().ok_or("Google gave no refresh token; remove the app's access at myaccount.google.com and connect again")?;
    v["refresh_token"] = json!(refresh);
    save(state, &v)
}

fn access_token(net: &Net, state: &Path) -> Result<String, String> {
    let v = load(state);
    let (id, secret, refresh) = (v["client_id"].as_str(), v["client_secret"].as_str(), v["refresh_token"].as_str());
    let (Some(id), Some(secret), Some(refresh)) = (id, secret, refresh) else {
        return Err("connect YouTube on the desk first".into());
    };
    let r = token_call(net, form(&[("client_id", id), ("client_secret", secret), ("refresh_token", refresh),
                                   ("grant_type", "refresh_token")]))?;
    r["access_token"].as_str().map(String::from).ok_or_else(|| "Google gave no access token".into())
}

pub fn forget(state: &Path) -> Result<(), String> {
    let mut v = load(state);
    v.as_object_mut().map(|o| o.remove("refresh_token"));
    save(state, &v)
}

/// Upload one recording; `progress(sent, total)` after each piece. The new
/// video's id.
pub fn upload(state: &Path, file: &Path, title: &str, description: &str, privacy: &str,
              mut progress: impl FnMut(u64, u64)) -> Result<String, String> {
    let net = Net::new();
    let token = access_token(&net, state)?;
    let auth = format!("Bearer {token}");
    let mut f = std::fs::File::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let total = f.metadata().map_err(|e| e.to_string())?.len();
    if total == 0 {
        return Err("that recording is empty".into());
    }
    let privacy = if ["private", "unlisted", "public"].contains(&privacy) { privacy } else { "private" };
    let meta = json!({
        "snippet": {"title": stage::clean_text(title, 100), "description": stage::clean_text(description, 4500),
                    "categoryId": "20"},
        "status": {"privacyStatus": privacy, "selfDeclaredMadeForKids": false},
    });
    let start = format!("{}?uploadType=resumable&part=snippet,status", upload_url());
    let len = total.to_string();
    let (code, hs, body) = net.request_full("POST", &start, &[
        ("authorization", &auth), ("content-type", "application/json; charset=UTF-8"),
        ("x-upload-content-type", "video/x-matroska"), ("x-upload-content-length", &len)],
        Some(meta.to_string().as_bytes()), 1 << 20)?;
    if code != 200 {
        return Err(why(code, &body));
    }
    let session = hs.iter().find(|(k, _)| k == "location").map(|(_, v)| v.clone())
        .ok_or("YouTube gave no upload address")?;
    let mut sent = 0u64;
    let piece = chunk();
    let mut buf = vec![0u8; piece as usize];
    let mut tries = 0;
    loop {
        let n = ((total - sent).min(piece)) as usize;
        f.seek(SeekFrom::Start(sent)).map_err(|e| e.to_string())?;
        f.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
        let range = format!("bytes {}-{}/{}", sent, sent + n as u64 - 1, total);
        let got = net.request_full("PUT", &session, &[("authorization", &auth), ("content-range", &range)],
                                   Some(&buf[..n]), 1 << 20);
        match got {
            Ok((200 | 201, _, body)) => {
                progress(total, total);
                let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                return v["id"].as_str().map(String::from).ok_or_else(|| "YouTube took it but gave no id".into());
            }
            Ok((308, hs, _)) => {
                // "Range: bytes=0-N" is what YouTube has; carry on from N+1.
                sent = hs.iter().find(|(k, _)| k == "range")
                    .and_then(|(_, v)| v.rsplit('-').next()?.parse::<u64>().ok())
                    .map(|end| end + 1)
                    .unwrap_or(sent);
                tries = 0;
                progress(sent, total);
            }
            Ok((code, _, body)) if code >= 500 && tries < 5 => {
                tries += 1;
                let _ = body;
                std::thread::sleep(std::time::Duration::from_secs(2u64.pow(tries)));
            }
            Ok((code, _, body)) => return Err(why(code, &body)),
            Err(e) if tries < 5 => {
                tries += 1;
                let _ = e;
                std::thread::sleep(std::time::Duration::from_secs(2u64.pow(tries)));
            }
            Err(e) => return Err(e),
        }
    }
}

fn why(code: u16, body: &[u8]) -> String {
    let v: Value = serde_json::from_slice(body).unwrap_or_default();
    let reason = v.pointer("/error/errors/0/reason").and_then(Value::as_str).unwrap_or("");
    match reason {
        "quotaExceeded" => "YouTube's quota for today is spent (an upload costs 1,600 of 10,000 units)".into(),
        "uploadLimitExceeded" => "YouTube says this channel has uploaded its limit for today".into(),
        _ => format!("YouTube answered {code}{}",
                     v.pointer("/error/message").and_then(Value::as_str).map(|m| format!(": {m}")).unwrap_or_default()),
    }
}
