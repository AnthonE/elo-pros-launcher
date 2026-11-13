//! `elo publish <folder>` — a browser game from a folder to a page players
//! open, in one command.
//!
//! The origin half is three routes that already take one signature each
//! (`meter/openlane.py` lists a game, `meter/webhost.py` hosts its build); this
//! is the hand that makes the three calls in order, so nobody has to:
//!
//! ```text
//!   list     →  if the game is not on the store yet, list it under this
//!               machine's account (one signature: the listing's warranty)
//!   pack     →  zip the folder here, hash it here
//!   send     →  sign the zip's sha256 and send the zip; the origin hashes
//!               what arrived, checks it and unpacks it
//!   wait     →  read the build's report until it is live, or says why not
//! ```
//!
//! **Nothing here is a second way in.** Every write is the dev desk's prepare →
//! sign → submit, with the account on this machine (`signer::choose`), and the
//! origin re-checks the signature against the bytes it received. The key never
//! leaves this machine.
//!
//! ## Why the zip is written by hand
//!
//! The workspace's dependency policy (`Cargo.toml`) counts crates as a budget,
//! and a store-only zip is two fixed headers and a CRC. So the folder is packed
//! uncompressed, here, with no archive crate. A build that wants compression
//! is zipped by its own toolchain and passed as the `.zip` itself; a browser
//! build's weight is mostly art and wasm that are already compressed, and the
//! origin serves `.br` and `.gz` files a build ships beside its own.

use crate::args::Args;
use crate::signer;
use elo_broker::signer::Signer;
use elo_depot::DepotError;
use elo_net::{Fetched, Net};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const USAGE: &str = "\
elo publish — a browser game, from a folder to a page players open

  elo publish <folder>             list it if it is new, send the build, wait
                                   for the checks, print the page
  elo publish <build.zip>          the same, for a zip you made yourself

  --slug SLUG        the game's address on the store (default: from --name,
                     else the folder's name)
  --name \"NAME\"      its name, for a new listing
  --multiplayer M    how people play together, for a new listing:
                     shard | match | persistent | co-op
  --made-with LIST   where a model was used, comma separated (code,art,…);
                     leave it out for none
  --entry FILE       the page that starts the game (default index.html)
  --no-frame         players go to the game's own address instead of the
                     framed page on the store (a threaded wasm build)
  --dry-run          list nothing and send nothing: what would be signed
  --json             machine-readable output

A build is at most 96 MB sent this way; a bigger one ships as a GitHub release
in elo.json (`elo dev help`). The page is checked before it goes live: only the
kinds of file a web game ships, no programs, no miners.
";

/// Files a folder may carry that are never part of a build.
const SKIP_DIRS: [&str; 4] = [".git", "node_modules", ".github", ".vscode"];
const MAX_FILES: usize = 5000;
/// The origin's ceiling on one upload (`webhost.UPLOAD_MAX_BYTES`). Checked
/// here first so nobody waits on a send that will be refused.
const MAX_ZIP_BYTES: u64 = 96 * 1024 * 1024;
const WAIT: Duration = Duration::from_secs(180);
const MULTIPLAYER: [&str; 4] = ["shard", "match", "persistent", "co-op"];

fn told(got: Fetched<Value>, what: &str) -> Result<Value, DepotError> {
    if got.ok {
        return Ok(got.value.unwrap_or(Value::Null));
    }
    if !got.reachable {
        return Err(DepotError::new(format!("could not reach the origin: {}", got.why)));
    }
    Err(DepotError::new(format!("{what} refused: {}", got.why)))
}

fn sign(a: &Args, host: &str, message: &str, why: &str) -> Result<(String, String), DepotError> {
    let mut signer = signer::choose(a, host).map_err(DepotError::new)?;
    let out = signer.sign(message, why).map_err(|r| DepotError::new(r.reason))?;
    let pick = |k: &str| out.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let (addr, sig) = (pick("address"), pick("signature"));
    if addr.is_empty() || sig.is_empty() {
        return Err(DepotError::new("the signer returned no signature"));
    }
    Ok((addr, sig))
}

/// A store slug from a name: lower case, digits and hyphens, 3 to 41 long.
pub fn slug_of(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        match c {
            'a'..='z' | '0'..='9' => out.push(c),
            'A'..='Z' => out.push(c.to_ascii_lowercase()),
            _ if !out.is_empty() && !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    let out = out.trim_matches('-').to_string();
    out.chars().take(41).collect::<String>().trim_matches('-').to_string()
}

// ── the zip, store-only ──────────────────────────────────────────────────────

fn crc32(bytes: &[u8]) -> u32 {
    // the reflected IEEE polynomial, a nibble at a time: no table to ship
    const T: [u32; 16] = [
        0x0000_0000, 0x1db7_1064, 0x3b6e_20c8, 0x26d9_30ac, 0x76dc_4190, 0x6b6b_51f4, 0x4db2_6158,
        0x5005_713c, 0xedb8_8320, 0xf00f_9344, 0xd6d6_a3e8, 0xcb61_b38c, 0x9b64_c2b0, 0x86d3_d2d4,
        0xa00a_e278, 0xbdbd_f21c,
    ];
    let mut c = !0u32;
    for b in bytes {
        c = T[((c ^ u32::from(*b)) & 0x0f) as usize] ^ (c >> 4);
        c = T[((c ^ (u32::from(*b) >> 4)) & 0x0f) as usize] ^ (c >> 4);
    }
    !c
}

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A zip of `files` (path inside the zip, bytes), every entry stored. Paths are
/// forward-slashed and relative; the date is the zip epoch, so the same folder
/// packs to the same bytes and the same sha256 on any machine, any day.
pub fn zip_stored(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, DepotError> {
    const DOS_DATE: u16 = 0x0021; // 1980-01-01
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in files {
        let n = name.as_bytes();
        if n.len() > usize::from(u16::MAX) || data.len() as u64 > u64::from(u32::MAX) {
            return Err(DepotError::new(format!("{name}: too large for one upload")));
        }
        let (crc, size, at) = (crc32(data), data.len() as u32, out.len() as u32);
        // local file header; bit 11 of the flags says the name is UTF-8
        le32(&mut out, 0x0403_4b50);
        le16(&mut out, 20);
        le16(&mut out, 0x0800);
        le16(&mut out, 0);
        le16(&mut out, 0);
        le16(&mut out, DOS_DATE);
        le32(&mut out, crc);
        le32(&mut out, size);
        le32(&mut out, size);
        le16(&mut out, n.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(n);
        out.extend_from_slice(data);
        // its central directory entry
        le32(&mut central, 0x0201_4b50);
        le16(&mut central, 20);
        le16(&mut central, 20);
        le16(&mut central, 0x0800);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, DOS_DATE);
        le32(&mut central, crc);
        le32(&mut central, size);
        le32(&mut central, size);
        le16(&mut central, n.len() as u16);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le32(&mut central, 0);
        le32(&mut central, at);
        central.extend_from_slice(n);
    }
    if files.len() > usize::from(u16::MAX) || out.len() as u64 + central.len() as u64 > u64::from(u32::MAX) {
        return Err(DepotError::new("the build is too large for one upload"));
    }
    let (dir_at, dir_len) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4b50);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, files.len() as u16);
    le16(&mut out, files.len() as u16);
    le32(&mut out, dir_len);
    le32(&mut out, dir_at);
    le16(&mut out, 0);
    Ok(out)
}

/// Every file under `root`, as (forward-slashed relative path, bytes), sorted
/// by path. Hidden files and folders, links, and the folders in `SKIP_DIRS`
/// are left out: the origin refuses them anyway, and a `.git` is not a build.
pub fn gather(root: &Path) -> Result<Vec<(String, Vec<u8>)>, DepotError> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| DepotError::new(format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| DepotError::new(format!("{}: {e}", dir.display())))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            // `file_type` does not follow a link, so a link is neither below
            let kind = entry.file_type().map_err(|e| DepotError::new(format!("{name}: {e}")))?;
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if kind.is_dir() {
                stack.push((entry.path(), rel));
            } else if kind.is_file() {
                found.push((rel, entry.path()));
                if found.len() > MAX_FILES {
                    return Err(DepotError::new(format!(
                        "more than {MAX_FILES} files — is this the build's folder, or the whole project?"
                    )));
                }
            }
        }
    }
    found.sort();
    let mut total: u64 = 0;
    let mut out = Vec::with_capacity(found.len());
    for (rel, path) in found {
        let bytes = std::fs::read(&path).map_err(|e| DepotError::new(format!("{}: {e}", path.display())))?;
        total += bytes.len() as u64;
        if total > MAX_ZIP_BYTES {
            return Err(DepotError::new(format!(
                "the folder is over {} MB, the most one upload takes — ship it as a GitHub \
                 release in elo.json instead (`elo dev help`)",
                MAX_ZIP_BYTES / (1024 * 1024)
            )));
        }
        out.push((rel, bytes));
    }
    Ok(out)
}

fn sha256_hex(bytes: &[u8]) -> String {
    elo_depot::digest::sha256_bytes(bytes)
}

fn listed(net: &Net, host: &str, slug: &str) -> Result<Option<Value>, DepotError> {
    let got = net.get_json(&format!("{host}/api/store/catalog"));
    let cat = match got.value {
        Some(v) if got.ok => v,
        _ if !got.reachable => return Err(DepotError::new(format!("could not reach the store: {}", got.why))),
        _ => return Err(DepotError::new(format!("the store's catalog: {}", got.why))),
    };
    Ok(cat
        .get("titles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|t| t.get("slug").and_then(Value::as_str) == Some(slug))
        .cloned())
}

fn say_warnings(v: &Value) {
    for w in v.get("warnings").and_then(Value::as_array).into_iter().flatten() {
        if let Some(s) = w.as_str() {
            eprintln!("  ⚠ {s}");
        }
    }
}

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let Some(target) = a.positional.first() else {
        print!("{USAGE}");
        return Ok(2);
    };
    if target == "help" || a.has("--help") {
        print!("{USAGE}");
        return Ok(0);
    }
    let path = Path::new(target);
    let meta = std::fs::metadata(path).map_err(|e| DepotError::new(format!("{target}: {e}")))?;
    let is_zip = meta.is_file() && target.to_ascii_lowercase().ends_with(".zip");
    if !meta.is_dir() && !is_zip {
        return Err(DepotError::new(format!(
            "{target} is not a folder or a .zip — point at the build's folder, the one with index.html in it"
        )));
    }
    let entry = a.flag("--entry").unwrap_or("index.html");

    // ── pack ─────────────────────────────────────────────────────────────────
    let zip = if is_zip {
        if meta.len() > MAX_ZIP_BYTES {
            return Err(DepotError::new(format!(
                "{target} is over {} MB, the most one upload takes — ship it as a GitHub \
                 release in elo.json instead (`elo dev help`)",
                MAX_ZIP_BYTES / (1024 * 1024)
            )));
        }
        std::fs::read(path).map_err(|e| DepotError::new(format!("{target}: {e}")))?
    } else {
        let files = gather(path)?;
        if !files.iter().any(|(name, _)| name == entry) {
            return Err(DepotError::new(format!(
                "no {entry} at the top of {target} — that is the page the game starts on. \
                 Point at the folder that holds it, or name the page with --entry"
            )));
        }
        zip_stored(&files)?
    };
    let sha = sha256_hex(&zip);

    // ── which game ───────────────────────────────────────────────────────────
    let folder_name = path
        .canonicalize()
        .ok()
        .and_then(|p| if is_zip { p.file_stem().map(|s| s.to_os_string()) } else { p.file_name().map(|s| s.to_os_string()) })
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let slug = match a.flag("--slug") {
        Some(s) => s.to_string(),
        None => slug_of(a.flag("--name").unwrap_or(&folder_name)),
    };
    if slug.len() < 3 {
        return Err(DepotError::new(
            "which game? — name it with --slug my-game (3 to 41 lowercase letters, digits and hyphens)",
        ));
    }
    let net = Net::new();
    let row = listed(&net, host, &slug)?;
    let dry = a.has("--dry-run");
    let mut steps: Vec<Value> = Vec::new();

    // ── list, when it is new ─────────────────────────────────────────────────
    if row.is_none() {
        let name = a.flag("--name").map(str::to_string).unwrap_or_else(|| folder_name.clone());
        let Some(mp) = a.flag("--multiplayer") else {
            return Err(DepotError::new(format!(
                "'{slug}' is not on the store yet, so this lists it first, and a listing says \
                 how people play together: add --multiplayer {} (and --name \"Your Game\")",
                MULTIPLAYER.join(" | ")
            )));
        };
        if !MULTIPLAYER.contains(&mp) {
            return Err(DepotError::new(format!("--multiplayer is one of {}", MULTIPLAYER.join(", "))));
        }
        let parts: Vec<&str> = a
            .flag("--made-with")
            .map(|s| s.split(',').map(str::trim).filter(|p| !p.is_empty()).collect())
            .unwrap_or_default();
        let fields = json!({"name": name, "multiplayer": mp, "wallet": false, "made_with": {"parts": parts}});
        let me = signer::choose(a, host).ok().and_then(|s| s.address()).unwrap_or_default();
        let prepared = told(
            net.post_json_told(
                &format!("{host}/api/store/claim/prepare"),
                &json!({"game": slug, "fields": fields, "wallet": me}),
            ),
            "the listing",
        )?;
        say_warnings(&prepared);
        let message = prepared
            .get("sign_this")
            .and_then(Value::as_str)
            .ok_or_else(|| DepotError::new("the origin returned no text to sign"))?;
        if dry {
            steps.push(json!({"step": "list", "would_sign": message}));
        } else {
            let (address, signature) = sign(a, host, message, &format!("list {name} on the store"))?;
            let mut body = json!({"game": slug, "fields": prepared.get("fields").cloned().unwrap_or(fields),
                                  "wallet": address, "signature": signature});
            if let Some(i) = prepared.get("index").and_then(Value::as_u64) {
                body["index"] = json!(i);
            }
            let done = told(net.post_json_told(&format!("{host}/api/store/claim"), &body), "the listing")?;
            say_warnings(&done);
            if !json_out {
                println!("listed      {slug} — {name}");
            }
            steps.push(json!({"step": "list", "done": true}));
        }
    }

    // ── sign the zip and send it ─────────────────────────────────────────────
    let mut fields = json!({"sha256": sha});
    if entry != "index.html" {
        fields["entry"] = json!(entry);
    }
    if a.has("--no-frame") {
        fields["frame"] = json!(false);
    }
    if dry && row.is_none() {
        // the build's text binds the desk's log, which the listing above would move
        steps.push(json!({"step": "send", "bytes": zip.len(), "sha256": sha,
                          "note": "signed after the listing lands"}));
        return finish_dry(json_out, &slug, &steps, zip.len(), &sha);
    }
    let prepared = told(
        net.post_json_told(
            &format!("{host}/api/store/dev/prepare"),
            &json!({"act": "web", "game": slug, "fields": fields}),
        ),
        "the build",
    )?;
    let message = prepared
        .get("sign_this")
        .and_then(Value::as_str)
        .ok_or_else(|| DepotError::new("the origin returned no text to sign"))?;
    if dry {
        steps.push(json!({"step": "send", "bytes": zip.len(), "sha256": sha, "would_sign": message}));
        return finish_dry(json_out, &slug, &steps, zip.len(), &sha);
    }
    let (address, signature) = sign(a, host, message, &format!("send {slug}'s build"))?;
    let mut url = format!(
        "{host}/api/store/dev/web?game={slug}&wallet={address}&signature={signature}&sha256={sha}"
    );
    if let Some(i) = prepared.get("index").and_then(Value::as_u64) {
        url.push_str(&format!("&index={i}"));
    }
    if entry != "index.html" {
        url.push_str(&format!("&entry={}", urlish(entry)));
    }
    if a.has("--no-frame") {
        url.push_str("&frame=false");
    }
    let bytes = zip.len();
    if !json_out {
        let size = if bytes < 1_048_576 {
            format!("{} KB", bytes.div_ceil(1024))
        } else {
            format!("{} MB", (bytes + 524_288) / 1_048_576)
        };
        println!("sending     {size}, sha256 {}…", &sha[..12]);
    }
    let sent = told(net.put_or_post_zip(&url, zip), "the build")?;
    steps.push(json!({"step": "send", "done": true, "bytes": bytes, "sha256": sha}));

    // ── wait for the checks ──────────────────────────────────────────────────
    let play = sent.get("play").and_then(Value::as_str).unwrap_or_default().to_string();
    let status_url = format!("{host}/api/web/{slug}");
    let began = Instant::now();
    let mut why: Option<String> = None;
    let mut live = false;
    while began.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(1500));
        let got = net.get_json(&status_url);
        let Some(v) = got.value else { continue };
        if v.pointer("/live/sha256").and_then(Value::as_str) == Some(sha.as_str()) {
            live = true;
            break;
        }
        // a report for THIS zip that says no: the checks refused it
        let wanted_this = v.pointer("/wanted/sha256").and_then(Value::as_str) == Some(sha.as_str());
        if wanted_this && v.pointer("/last/ok").and_then(Value::as_bool) == Some(false) {
            why = v.pointer("/last/why").and_then(Value::as_str).map(str::to_string);
            break;
        }
    }
    let out = json!({"game": slug, "live": live, "play": play, "why": why, "steps": steps,
                     "page": format!("{host}/game.html?slug={slug}"), "status": status_url});
    if json_out {
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else if live {
        println!("live        {play}");
        println!("store page  {host}/game.html?slug={slug}");
        println!("next        art and words: elo dev edit {slug} --set blurb=\"…\" · elo dev image {slug} capsule.png");
    } else if let Some(w) = &why {
        println!("refused     {w}");
        println!("            fix the build and run this again; what was live stays live");
    } else {
        println!("still checking after {} s — read {status_url}", WAIT.as_secs());
    }
    Ok(if live { 0 } else { 1 })
}

fn finish_dry(json_out: bool, slug: &str, steps: &[Value], bytes: usize, sha: &str) -> Result<i32, DepotError> {
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"game": slug, "dry_run": true, "steps": steps}))
                .unwrap_or_default()
        );
        return Ok(0);
    }
    println!("would publish {slug}: {bytes} bytes, sha256 {sha}");
    for s in steps {
        if let Some(text) = s.get("would_sign").and_then(Value::as_str) {
            println!("\nwould sign ({}):\n{text}", s.get("step").and_then(Value::as_str).unwrap_or("?"));
        }
    }
    println!("\nnothing was listed or sent — drop --dry-run to do it");
    Ok(0)
}

fn urlish(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' | '/' => out.push(c),
            _ => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_published_check_value() {
        // the CRC-32/ISO-HDLC check value every zip tool agrees on
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_slug_is_what_the_store_takes() {
        assert_eq!(slug_of("Snow Fort!"), "snow-fort");
        assert_eq!(slug_of("  --My_Game 2  "), "my-game-2");
        assert_eq!(slug_of("ÉCLAT"), "clat");
        assert!(slug_of(&"a".repeat(80)).len() <= 41);
        assert_eq!(slug_of("!!!"), "");
    }

    #[test]
    fn the_zip_is_stored_and_the_same_bytes_every_time() {
        let files = vec![
            ("index.html".to_string(), b"<!doctype html>".to_vec()),
            ("art/hero.png".to_string(), vec![0x89, b'P', b'N', b'G']),
        ];
        let z = zip_stored(&files).unwrap();
        assert_eq!(z, zip_stored(&files).unwrap(), "no clock and no order in the bytes");
        // a local header, then the name, then the bytes as they are
        assert_eq!(&z[0..4], &[0x50, 0x4b, 0x03, 0x04]);
        assert_eq!(u16::from_le_bytes([z[8], z[9]]), 0, "method 0: stored");
        assert_eq!(&z[30..40], b"index.html");
        assert_eq!(&z[40..55], b"<!doctype html>");
        // the end record names two entries and where the directory starts
        let end = z.len() - 22;
        assert_eq!(&z[end..end + 4], &[0x50, 0x4b, 0x05, 0x06]);
        assert_eq!(u16::from_le_bytes([z[end + 10], z[end + 11]]), 2);
        let dir_at = u32::from_le_bytes([z[end + 16], z[end + 17], z[end + 18], z[end + 19]]) as usize;
        assert_eq!(&z[dir_at..dir_at + 4], &[0x50, 0x4b, 0x01, 0x02]);
    }

    #[test]
    fn a_folder_packs_without_its_hidden_files_and_in_order() {
        let dir = std::env::temp_dir().join(format!("elo-publish-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("art")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules/x")).unwrap();
        std::fs::write(dir.join("index.html"), b"<!doctype html>").unwrap();
        std::fs::write(dir.join("art/b.png"), b"b").unwrap();
        std::fs::write(dir.join("art/a.png"), b"a").unwrap();
        std::fs::write(dir.join(".env"), b"SECRET=1").unwrap();
        std::fs::write(dir.join(".git/config"), b"x").unwrap();
        std::fs::write(dir.join("node_modules/x/i.js"), b"x").unwrap();
        let got = gather(&dir).unwrap();
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["art/a.png", "art/b.png", "index.html"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
