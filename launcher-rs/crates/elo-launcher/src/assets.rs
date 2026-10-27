//! `elo assets` — files from the asset store, in the project you are building.
//!
//! The origin half is `meter/assetstore.py`: a search, and an install read
//! that names one asset's files for one engine, the folder they go in, the
//! licence and the credit line. The origin stores none of the bytes. Landing
//! them is `elo_depot::assets` — the crate that already decides where a
//! stranger's bytes go on this disk — so this file reads, prints and asks; it
//! never writes an asset file itself.
//!
//! The project is the current folder, or `--dir`. Its `elo-assets.lock` is
//! the record: `sync` rebuilds from it without asking the origin, so a
//! teammate or a CI box gets the same bytes the lock pinned.

use crate::args::Args;
use crate::friends::{told, whose_for};
use elo_depot::assets::{self, Asset, Landed, Lock, Meta};
use elo_depot::install::human;
use elo_depot::workshop::{is_mod_id, Outcome};
use elo_depot::DepotError;
use elo_hive::render::{clip, safe_line};
use elo_net::Net;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
elo assets — models, sprites, sounds and plugins for the game you are making,
each 99¢ or less, managed for you

  elo assets                          what this project has, from elo-assets.lock
  elo assets search <words…>          the store (--kind model, --engine godot)
  elo assets show <id>                one asset: files, licence, credit, price
  elo assets add <id>…                put it in this project, every file's sha256
                                      checked, and pin it in elo-assets.lock
  elo assets remove <id>              take it out, folder and lock line
  elo assets sync                     the lock's exact bytes, on this machine
  elo assets outdated                 what has a newer version in the store
  elo assets update [<id>]            take the newer versions, and re-pin
  elo assets credits                  write elo-credits.md: who made what, and
                                      under which licence

  for authors (your agent can run these too):
  elo assets init <folder> --repo https://github.com/you/thing --tag v1.0.0
                                      write elo-asset.json for the files in
                                      <folder>, with their sha256s and release urls
                                      (--id --name --about --kind --license
                                      --engine --price CENTS)
  elo assets check --repo URL         validate ./elo-asset.json before you commit
  elo assets publish --repo URL       list it: the store reads the committed one

  --dir PATH       the project (default: this folder)
  --engine NAME    godot, bevy, unity, unreal, three, fyrox, any (default: read
                   off the project, then the lock)
  --wallet ADDR    the wallet that holds a priced asset's licence (default:
                   ELO_WALLET, then this machine's signer)
  --dry-run        say what would be downloaded or written, do nothing
  --json           machine-readable output

A file whose bytes differ from what the author published is deleted, never
kept. A priced asset installs only for a wallet that holds its licence.
";

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let net = Net::new();
    match a.positional.first().map(String::as_str) {
        None | Some("list") => list(a, json_out),
        Some("help") => {
            print!("{USAGE}");
            Ok(0)
        }
        Some("search") => search(&net, a, host, json_out),
        Some("show") => show(&net, a, host, json_out),
        Some("add") => add(&net, a, host, json_out),
        Some("remove") => remove(a, json_out),
        Some("sync") => sync(&net, a, json_out),
        Some("outdated") => outdated(&net, a, host, json_out).map(|_| 0),
        Some("update") => update(&net, a, host, json_out),
        Some("credits") => credits(a, json_out),
        Some("init") => init(a, json_out),
        Some("check") => check(&net, a, host, json_out),
        Some("publish") => publish(&net, a, host, json_out),
        Some(other) => Err(DepotError::new(format!(
            "unknown assets command {} — `elo assets help`",
            safe_line(other)
        ))),
    }
}

fn project(a: &Args) -> PathBuf {
    PathBuf::from(a.flag("--dir").unwrap_or("."))
}

fn read_lock(root: &Path) -> Result<Lock, DepotError> {
    Ok(Lock::read(root)?.unwrap_or_default())
}

/// The engine to install for: the flag, else what the lock already says,
/// else read off the project.
fn engine_for(a: &Args, root: &Path, lock: &Lock) -> Result<String, DepotError> {
    if let Some(e) = a.flag("--engine") {
        let e = e.trim().to_ascii_lowercase();
        if !assets::is_engine(&e) {
            return Err(DepotError::new(format!("{} is not an engine name", safe_line(&e))));
        }
        return Ok(e);
    }
    if !lock.engine.is_empty() && lock.engine != "any" {
        return Ok(lock.engine.clone());
    }
    Ok(assets::detect_engine(root).to_string())
}

fn id_at(a: &Args, at: usize, verb: &str) -> Result<String, DepotError> {
    let typed = a
        .positional
        .get(at)
        .ok_or_else(|| DepotError::new(format!("{verb} which asset? — its id, from `elo assets search`")))?;
    let id = typed.trim().to_ascii_lowercase();
    if !is_mod_id(&id) {
        return Err(DepotError::new(format!(
            "{} is not an asset id — lowercase letters, digits and '-'",
            safe_line(typed)
        )));
    }
    Ok(id)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The origin's sentence for a plugin or template, which put the author's
/// code in your project; `None` for anything else. Made safe to print.
pub fn runs_code_warning(v: &Value) -> Option<String> {
    if v.get("runs_code").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(safe_line(
        v.get("warning").and_then(Value::as_str).unwrap_or("this asset runs code in your project"),
    ))
}

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

// ── reading the store ────────────────────────────────────────────────────────
fn search(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let words = a.positional[1..].join(" ");
    let mut url = format!("{host}/api/assets?q={}", urlencode(words.trim()));
    for (flag, key) in [("--kind", "kind"), ("--engine", "engine")] {
        if let Some(v) = a.flag(flag) {
            url.push_str(&format!("&{key}={}", urlencode(v)));
        }
    }
    let got = told(net.get_json_told(&url))?;
    if json_out {
        print_json(&got);
        return Ok(0);
    }
    for line in search_lines(&got) {
        println!("{line}");
    }
    Ok(0)
}

/// A search as terminal lines. Every string an author wrote goes through
/// `safe_line`: a name is not allowed to drive the terminal it is printed on.
pub fn search_lines(got: &Value) -> Vec<String> {
    let rows = got.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
    let total = got.get("total").and_then(Value::as_u64).unwrap_or(rows.len() as u64);
    let mut out = vec![format!("{total} asset{}", if total == 1 { "" } else { "s" })];
    for r in &rows {
        let engines: Vec<&str> = r
            .get("engines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        out.push(format!(
            "  {:<24} {:<6} {} · {} · {}",
            clip(&safe_line(s(r, "id")), 24),
            safe_line(s(r, "price")),
            safe_line(s(r, "name")),
            safe_line(s(r, "kind")),
            safe_line(&engines.join(", "))
        ));
        out.push(format!(
            "  {:<24} {:<6} {} · github.com/{}",
            "",
            "",
            safe_line(s(r, "license")),
            safe_line(&format!(
                "{}/{}",
                s(r.get("author").unwrap_or(&Value::Null), "owner"),
                s(r.get("author").unwrap_or(&Value::Null), "repo")
            ))
        ));
    }
    if !rows.is_empty() {
        out.push(String::new());
        out.push("`elo assets add <id>` puts one in this project".into());
    }
    out
}

fn show(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let id = id_at(a, 1, "show")?;
    let got = told(net.get_json_told(&format!("{host}/api/assets/{id}")))?;
    if json_out {
        print_json(&got);
        return Ok(0);
    }
    println!("{} {} · {}", id, safe_line(s(&got, "version")), safe_line(s(&got, "name")));
    println!("  {}", safe_line(s(&got, "about")));
    println!("  {} · licence {}", safe_line(s(&got, "price")), safe_line(s(&got, "license")));
    println!("  credit: {}", safe_line(s(&got, "credit")));
    if let Some(w) = runs_code_warning(&got) {
        println!("  ⚠ {w}");
    }
    for f in got.get("files").and_then(Value::as_array).into_iter().flatten() {
        let size = f.get("size").and_then(Value::as_i64).unwrap_or(0);
        println!("  {:<32} {}", clip(&safe_line(s(f, "name")), 32), human(size));
    }
    let buy = got.get("buy").unwrap_or(&Value::Null);
    if buy.get("free").and_then(Value::as_bool) != Some(true) {
        if let Some(why) = buy.get("why").and_then(Value::as_str) {
            println!("  {}", safe_line(why));
        }
    }
    println!("\n`elo assets add {id}` puts it in this project");
    Ok(0)
}

// ── changing the project ─────────────────────────────────────────────────────
/// The install read for one asset. A priced one goes with the wallet that
/// holds its licence, when this machine knows one.
fn fetch_install(net: &Net, a: &Args, host: &str, id: &str, engine: &str) -> Result<Asset, DepotError> {
    let mut url = format!("{host}/api/assets/{id}/install?engine={}", urlencode(engine));
    if let Ok(w) = whose_for(a, host, "which wallet holds it?") {
        url.push_str(&format!("&wallet={w}"));
    }
    let got = told(net.get_json_told(&url))?;
    // stderr, so a `--json` caller's stdout stays one document
    if let Some(w) = runs_code_warning(&got) {
        eprintln!("⚠ {id}: {w}");
    }
    let asset = assets::parse_asset(&got).map_err(|e| {
        DepotError::new(format!("{id} did not read — {}", safe_line(&e.to_string())))
    })?;
    if asset.id != id {
        return Err(DepotError::new(format!(
            "asked for {id} and the origin answered for {}",
            safe_line(&asset.id)
        )));
    }
    Ok(asset)
}

fn land_one(net: &Net, root: &Path, asset: &Asset, dry: bool) -> Landed {
    // stderr, so a `--json` caller's stdout stays one document.
    let mut tell = |a: &Asset, f: &elo_depot::workshop::ModFile| {
        eprintln!("  fetching {}/{} ({})", a.id, f.name, human(f.size));
    };
    assets::land(root, asset, net, dry, &mut tell)
}

pub fn landed_lines(a: &Asset, l: &Landed) -> Vec<String> {
    let mut out = vec![format!("  {} {} → {}", a.id, safe_line(&a.version), l.dir.display())];
    for f in &l.files {
        let said = match &f.outcome {
            Outcome::Kept => "already here".to_string(),
            Outcome::Fetched => format!("downloaded, {}, sha256 checked", human(f.size)),
            Outcome::WouldFetch => format!("would download, {}", human(f.size)),
            Outcome::Mismatch(why) => format!("NOT KEPT — {}", safe_line(why)),
            Outcome::Failed(why) => format!("not downloaded — {}", safe_line(why)),
        };
        out.push(format!("    {:<28} {said}", clip(&f.name, 28)));
    }
    out
}

fn landed_json(a: &Asset, l: &Landed) -> Value {
    json!({
        "id": a.id, "version": a.version, "dir": l.dir.display().to_string(),
        "complete": l.complete(),
        "files": l.files.iter().map(|f| json!({
            "name": f.name, "path": f.path.display().to_string(), "size": f.size,
            "outcome": match &f.outcome {
                Outcome::Kept => "kept", Outcome::Fetched => "fetched",
                Outcome::WouldFetch => "would_fetch", Outcome::Mismatch(_) => "mismatch",
                Outcome::Failed(_) => "failed",
            },
        })).collect::<Vec<_>>(),
    })
}

/// Land each asset; the lock takes only those whose every file checked.
fn add_all(
    net: &Net,
    a: &Args,
    root: &Path,
    lock: &mut Lock,
    picked: Vec<Asset>,
    json_out: bool,
) -> Result<i32, DepotError> {
    let dry = a.has("--dry-run");
    let mut bad = 0;
    let mut rows = Vec::new();
    for asset in picked {
        let l = land_one(net, root, &asset, dry);
        bad += l.bad();
        if json_out {
            rows.push(landed_json(&asset, &l));
        } else {
            for line in landed_lines(&asset, &l) {
                println!("{line}");
            }
        }
        if l.complete() && !dry {
            for r in &asset.requires {
                if lock.get(r).is_none() {
                    eprintln!("  {} needs {r} — `elo assets add {r}`", asset.id);
                }
            }
            lock.put(asset);
        }
    }
    if !dry {
        lock.write(root)?;
    }
    if json_out {
        print_json(&json!({"dry_run": dry, "engine": lock.engine, "assets": rows, "bad": bad,
                           "lock": root.join(assets::LOCK_NAME).display().to_string()}));
    } else if dry {
        println!("nothing was downloaded or written — drop --dry-run to do it");
    } else {
        println!("pinned in {} — `elo assets credits` writes the credits file",
                 root.join(assets::LOCK_NAME).display());
    }
    if bad > 0 {
        eprintln!("elo: {bad} file{} did not check out — nothing unchecked was kept",
                  if bad == 1 { "" } else { "s" });
        return Ok(1);
    }
    Ok(0)
}

fn add(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let root = project(a);
    let mut lock = read_lock(&root)?;
    let engine = engine_for(a, &root, &lock)?;
    lock.engine = engine.clone();
    if a.positional.len() < 2 {
        return Err(DepotError::new("add which asset? — its id, from `elo assets search`"));
    }
    let mut picked = Vec::new();
    for i in 1..a.positional.len() {
        let id = id_at(a, i, "add")?;
        picked.push(fetch_install(net, a, host, &id, &engine)?);
    }
    add_all(net, a, &root, &mut lock, picked, json_out)
}

fn remove(a: &Args, json_out: bool) -> Result<i32, DepotError> {
    let root = project(a);
    let id = id_at(a, 1, "remove")?;
    let mut lock = read_lock(&root)?;
    let asset = lock
        .drop_id(&id)
        .ok_or_else(|| DepotError::new(format!("{id} is not in {}", assets::LOCK_NAME)))?;
    if a.has("--dry-run") {
        println!("would remove {} and its line in {}", root.join(&asset.path).display(), assets::LOCK_NAME);
        return Ok(0);
    }
    let gone = assets::remove(&root, &asset)?;
    lock.write(&root)?;
    if json_out {
        print_json(&json!({"removed": id, "folder_removed": gone}));
    } else {
        println!("removed {id}{}", if gone { format!(" and {}", root.join(&asset.path).display()) } else { String::new() });
    }
    Ok(0)
}

fn sync(net: &Net, a: &Args, json_out: bool) -> Result<i32, DepotError> {
    let root = project(a);
    let lock = Lock::read(&root)?.ok_or_else(|| {
        DepotError::new(format!("no {} here — `elo assets add <id>` makes one", assets::LOCK_NAME))
    })?;
    let mut copy = lock.clone();
    let picked = lock.assets.clone();
    add_all(net, a, &root, &mut copy, picked, json_out)
}

/// `(id, pinned version, store version)` for each asset whose store version differs.
fn outdated(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<Vec<(String, String, String)>, DepotError> {
    let root = project(a);
    let lock = read_lock(&root)?;
    let mut out = Vec::new();
    let mut unread = Vec::new();
    for asset in &lock.assets {
        let got = net.get_json_told(&format!("{host}/api/assets/{}", asset.id));
        match got.value {
            Some(v) if got.ok => {
                let now = s(&v, "version").to_string();
                if now != asset.version {
                    out.push((asset.id.clone(), asset.version.clone(), now));
                }
            }
            _ => unread.push((asset.id.clone(), got.why)),
        }
    }
    if json_out {
        print_json(&json!({
            "outdated": out.iter().map(|(i, p, n)| json!({"id": i, "pinned": p, "store": safe_line(n)})).collect::<Vec<_>>(),
            "unread": unread.iter().map(|(i, w)| json!({"id": i, "why": safe_line(w)})).collect::<Vec<_>>(),
        }));
    } else {
        if out.is_empty() {
            println!("every asset is at the store's version");
        }
        for (id, pinned, now) in &out {
            println!("  {id:<24} {} → {}", safe_line(pinned), safe_line(now));
        }
        for (id, why) in &unread {
            println!("  {id:<24} could not read — {}", safe_line(why));
        }
        if !out.is_empty() {
            println!("\n`elo assets update` takes them");
        }
    }
    Ok(out)
}

fn update(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let root = project(a);
    let mut lock = read_lock(&root)?;
    let only = if a.positional.len() > 1 { Some(id_at(a, 1, "update")?) } else { None };
    let engine = engine_for(a, &root, &lock)?;
    let mut picked = Vec::new();
    for asset in lock.assets.clone() {
        if only.as_deref().is_some_and(|o| o != asset.id) {
            continue;
        }
        let fresh = fetch_install(net, a, host, &asset.id, &engine)?;
        if fresh.version != asset.version || fresh.files != asset.files {
            picked.push(fresh);
        }
    }
    if let Some(o) = &only {
        if lock.get(o).is_none() {
            return Err(DepotError::new(format!("{o} is not in {}", assets::LOCK_NAME)));
        }
    }
    if picked.is_empty() {
        println!("nothing to update");
        return Ok(0);
    }
    add_all(net, a, &root, &mut lock, picked, json_out)
}

fn credits(a: &Args, json_out: bool) -> Result<i32, DepotError> {
    let root = project(a);
    let lock = Lock::read(&root)?.ok_or_else(|| {
        DepotError::new(format!("no {} here — nothing to credit yet", assets::LOCK_NAME))
    })?;
    if a.has("--dry-run") {
        print!("{}", assets::credits_markdown(&lock));
        return Ok(0);
    }
    let p = assets::write_credits(&root, &lock)?;
    if json_out {
        print_json(&json!({"written": p.display().to_string(), "assets": lock.assets.len()}));
    } else {
        println!("wrote {} — {} asset{}", p.display(), lock.assets.len(),
                 if lock.assets.len() == 1 { "" } else { "s" });
    }
    Ok(0)
}

fn list(a: &Args, json_out: bool) -> Result<i32, DepotError> {
    let root = project(a);
    let lock = read_lock(&root)?;
    if json_out {
        print_json(&lock.to_json());
        return Ok(0);
    }
    let n = lock.assets.len();
    let engine = if lock.engine.is_empty() { assets::detect_engine(&root) } else { lock.engine.as_str() };
    println!("{} · {engine} · {n} asset{}", root.display(), if n == 1 { "" } else { "s" });
    if n == 0 {
        println!("  nothing added yet — `elo assets search <words>`, then `elo assets add <id>`");
    }
    for x in &lock.assets {
        println!("  {:<24} {} · {} · {}", clip(&x.id, 24), safe_line(&x.version),
                 safe_line(&x.license), x.path);
    }
    Ok(0)
}

// ── for authors ──────────────────────────────────────────────────────────────
fn repo_of(a: &Args) -> Result<(String, String), DepotError> {
    let url = a
        .flag("--repo")
        .ok_or_else(|| DepotError::new("which repo? — --repo https://github.com/you/your-asset"))?;
    let rest = url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .strip_prefix("https://github.com/")
        .ok_or_else(|| DepotError::new("the repo is https://github.com/<owner>/<repo>"))?;
    match rest.split('/').collect::<Vec<_>>()[..] {
        [o, r] if !o.is_empty() && !r.is_empty() => Ok((o.to_string(), r.to_string())),
        _ => Err(DepotError::new("the repo is https://github.com/<owner>/<repo>")),
    }
}

fn init(a: &Args, json_out: bool) -> Result<i32, DepotError> {
    let folder = a
        .positional
        .get(1)
        .ok_or_else(|| DepotError::new("which folder holds the files? — `elo assets init <folder> --repo … --tag …`"))?;
    let (owner, repo) = repo_of(a)?;
    let tag = a.flag("--tag").ok_or_else(|| DepotError::new("which release tag? — --tag v1.0.0"))?;
    let id = a
        .flag("--id")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| repo.to_ascii_lowercase());
    let price = match a.flag("--price") {
        None => 0,
        Some(p) => p.trim().parse::<u32>().ok().filter(|c| *c <= 99).ok_or_else(|| {
            DepotError::new("--price is a whole number of cents, 0 to 99")
        })?,
    };
    let meta = Meta {
        id,
        name: a.flag("--name").unwrap_or_default().to_string(),
        about: a.flag("--about").unwrap_or_default().to_string(),
        kind: a.flag("--kind").unwrap_or_default().to_string(),
        version: String::new(),
        license: a.flag("--license").unwrap_or_default().to_string(),
        engines: a.repeats.get("--engine").cloned().unwrap_or_default(),
        price_cents: price,
    };
    let m = assets::build_manifest(Path::new(folder), &owner, &repo, tag, &meta)?;
    let dest = project(a).join(assets::MANIFEST_NAME);
    if a.has("--dry-run") || json_out {
        print_json(&m);
        if a.has("--dry-run") {
            return Ok(0);
        }
    }
    let body = serde_json::to_string_pretty(&m).map_err(|e| DepotError::new(e.to_string()))? + "\n";
    std::fs::write(&dest, body).map_err(|e| DepotError::new(format!("{}: {e}", dest.display())))?;
    if !json_out {
        println!("wrote {}", dest.display());
        println!("  1. upload every file in {folder} to the {tag} release of github.com/{owner}/{repo}");
        println!("  2. fill in about and kind, then `elo assets check --repo https://github.com/{owner}/{repo}`");
        println!("  3. commit {} and `elo assets publish --repo https://github.com/{owner}/{repo}`",
                 assets::MANIFEST_NAME);
    }
    Ok(0)
}

fn check(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let (owner, repo) = repo_of(a)?;
    let p = project(a).join(assets::MANIFEST_NAME);
    let body = match std::fs::read_to_string(&p) {
        Ok(b) => {
            let m: Value = serde_json::from_str(&b)
                .map_err(|e| DepotError::new(format!("{} is not JSON: {e}", p.display())))?;
            json!({"repo": format!("https://github.com/{owner}/{repo}"), "manifest": m})
        }
        // No local copy: check the one already committed.
        Err(_) => json!({"repo": format!("https://github.com/{owner}/{repo}")}),
    };
    let got = told(net.post_json_told(&format!("{host}/api/assets/check"), &body))?;
    if json_out {
        print_json(&got);
        return Ok(if got.get("ok").and_then(Value::as_bool) == Some(true) { 0 } else { 1 });
    }
    if got.get("ok").and_then(Value::as_bool) == Some(true) {
        println!("{} reads clean — credit line: {}", safe_line(s(&got, "id")), safe_line(s(&got, "credit")));
    } else {
        println!("not yet — {}", safe_line(s(&got, "why")));
    }
    for w in got.get("warnings").and_then(Value::as_array).into_iter().flatten() {
        println!("  · {}", safe_line(w.as_str().unwrap_or("")));
    }
    println!("  next: {}", safe_line(s(&got, "next")));
    Ok(0)
}

fn publish(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let (owner, repo) = repo_of(a)?;
    let got = told(net.post_json_told(
        &format!("{host}/api/assets/publish"),
        &json!({"repo": format!("https://github.com/{owner}/{repo}")}),
    ))?;
    if json_out {
        print_json(&got);
        return Ok(0);
    }
    let id = s(&got, "id");
    let verb = if got.get("created").and_then(Value::as_bool) == Some(true) { "listed" }
               else if got.get("updated").and_then(Value::as_bool) == Some(true) { "updated" }
               else { "re-read" };
    println!("{verb} {} — {host}/assets.html#{}", safe_line(id), safe_line(id));
    for w in got.get("warnings").and_then(Value::as_array).into_iter().flatten() {
        println!("  · {}", safe_line(w.as_str().unwrap_or("")));
    }
    println!("anyone adds it with: elo assets add {}", safe_line(id));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args;

    fn argv(parts: &[&str]) -> args::Args {
        args::parse(&parts.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn an_id_is_held_to_its_alphabet() {
        assert_eq!(id_at(&argv(&["assets", "add", "Low-Poly-Forest"]), 1, "add").unwrap(), "low-poly-forest");
        for bad in ["../x", "a/b", "", "-x", "a b"] {
            assert!(id_at(&argv(&["assets", "add", bad]), 1, "add").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_repo_is_a_github_repo() {
        assert_eq!(repo_of(&argv(&["assets", "check", "--repo", "https://github.com/ann/forest.git"]))
                       .unwrap(), ("ann".to_string(), "forest".to_string()));
        for bad in ["https://gitlab.com/a/b", "https://github.com/a", "https://github.com/a/b/c"] {
            assert!(repo_of(&argv(&["assets", "check", "--repo", bad])).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_search_cannot_drive_the_terminal() {
        let got = json!({"total": 1, "assets": [{
            "id": "neon", "price": "99¢", "name": "Neon\x1b[2J\x1b]52;c;aGk=\x07", "kind": "shader",
            "engines": ["unity"], "license": "MIT", "author": {"owner": "cara", "repo": "neon"}}]});
        let lines = search_lines(&got);
        assert!(lines[0] == "1 asset" && lines[1].contains("99¢"), "{lines:?}");
        for l in &lines {
            assert!(!l.contains('\x1b') && !l.contains('\x07'), "{l:?}");
        }
    }

    #[test]
    fn code_says_so_and_nothing_else_does() {
        let w = runs_code_warning(&json!({"runs_code": true, "warning": "runs code\x1b[2J here"}));
        assert_eq!(w.as_deref().map(|x| x.contains('\x1b')), Some(false));
        assert!(w.unwrap().contains("runs code"));
        assert_eq!(runs_code_warning(&json!({"runs_code": true})).as_deref(),
                   Some("this asset runs code in your project"));
        assert_eq!(runs_code_warning(&json!({"runs_code": false, "warning": "x"})), None);
        assert_eq!(runs_code_warning(&json!({})), None);
    }

    #[test]
    fn a_query_is_encoded() {
        assert_eq!(urlencode("low poly&kind=x"), "low%20poly%26kind%3Dx");
    }
}
