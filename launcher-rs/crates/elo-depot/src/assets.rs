//! Assets in a project: what a builder added from the asset store, fetched
//! from each author's GitHub release (or, for an asset uploaded to the store,
//! from an IPFS gateway by its CID), held to the sha256 the author published,
//! and pinned in a lockfile beside the project.
//!
//! The origin half is `meter/assetstore.py`. `GET /api/assets/<id>/install`
//! names the files for one engine, the folder they go in, the licence and the
//! credit line, and stores none of the bytes. This module lands them, for the
//! reason `workshop.rs` lives in this crate: it writes files a stranger named,
//! so `path::safe_relpath` and `path::resolve_into` stand between a name off
//! the wire and the builder's project. The landing is the Workshop's own
//! (`workshop::place_file`). No network here; the caller hands in a
//! [`Fetcher`].
//!
//! The layout:
//!
//! ```text
//! <project>/elo-assets.lock              what was added, pinned by version and sha256
//! <project>/<path the origin named>/…    each file, byte for byte the author's
//! <project>/elo-credits.md               written by `credits`: who made what, under which licence
//! ```
//!
//! The rules, each a refusal:
//!
//! * **A file lands only after its bytes check** — `place_file`'s rule.
//! * **A url is the author's own release** — `workshop::check_release_url` —
//!   or, for an uploaded asset (`author.host == "elo"`), an IPFS gateway's
//!   `https://<host>/ipfs/<cid>` ([`check_ipfs_url`]): content-addressed, so
//!   the gateway is only a door, and the sha256 is still checked.
//! * **The folder is the asset's.** The origin's `path` must be relative,
//!   canonical, and end in the asset's id (`assets/elo/<id>`), so the folder
//!   an install tidies and a remove deletes can only ever be one an asset
//!   named for itself, never `src/`.
//! * **A sync reads the lock, not the origin.** The lock already holds every
//!   url and sha256, so a teammate gets the same bytes whatever the store
//!   says today; `update` is the one verb that asks for newer ones.

use crate::digest::sha256_file;
use crate::error::{DepotError, Result};
use crate::install::Fetcher;
use crate::path::{alias_key, resolve_into, safe_relpath};
use crate::workshop::{
    check_file_name, check_release_url, is_mod_id, place_file, remove_any, FileDone, ModFile,
    Outcome, FILE_SIZE_MAX,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The lockfile, at the project's root.
pub const LOCK_NAME: &str = "elo-assets.lock";
/// The credits file `credits` writes, at the project's root.
pub const CREDITS_NAME: &str = "elo-credits.md";
/// The manifest an author commits (`meter/assetstore.ASSET_FILE`).
pub const MANIFEST_NAME: &str = "elo-asset.json";
/// How many files one asset may list (`meter/assetstore.FILES_MAX`).
pub const FILES_MAX: usize = 16;
const PART_PREFIX: &str = ".part-";

/// One asset as the project holds it: what the install read named, and what
/// the lock keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub id: String,
    pub version: String,
    pub engine: String,
    /// The folder, relative to the project, ending in the id.
    pub path: String,
    pub files: Vec<ModFile>,
    pub license: String,
    pub credit: String,
    /// A GitHub account, or for an uploaded asset its wallet (or `elopros`).
    pub owner: String,
    /// Empty for an uploaded asset.
    pub repo: String,
    /// Uploaded to the store and pinned to IPFS, not a GitHub release.
    pub hosted: bool,
    pub source: Option<String>,
    pub requires: Vec<String>,
}

/// The house's own shelf on the store (`meter/assetstore.HOUSE`).
pub const HOUSE: &str = "elopros";

/// An uploaded file's url: `https://<gateway host>/ipfs/<cid>`, nothing more.
pub fn check_ipfs_url(url: &str) -> Result<()> {
    let refuse = || DepotError::new(format!("{url:?} is not a file on an IPFS gateway"));
    let rest = url.strip_prefix("https://").ok_or_else(refuse)?;
    let (host, path) = rest.split_once('/').ok_or_else(refuse)?;
    let cid = path.strip_prefix("ipfs/").ok_or_else(refuse)?;
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        && host.contains('.');
    let cid_ok = (20..=130).contains(&cid.len()) && cid.bytes().all(|c| c.is_ascii_alphanumeric());
    if host_ok && cid_ok {
        Ok(())
    } else {
        Err(refuse())
    }
}

fn is_hosted_owner(s: &str) -> bool {
    s == HOUSE
        || (s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|c| c.is_ascii_hexdigit()))
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn is_github_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 100
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(c))
}

/// An engine name: `[a-z0-9][a-z0-9-]{0,23}`, the origin's rule.
pub fn is_engine(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 24
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// One line the origin wrote, kept printable and short: a credit line or a
/// licence id ends up in a file a game ships.
fn plain(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// The install read (or a lock entry), parsed and checked. Anything that
/// fails a check is an error: an asset is added whole or not at all.
pub fn parse_asset(v: &Value) -> Result<Asset> {
    let id = text(v, "id")
        .filter(|i| is_mod_id(i))
        .ok_or_else(|| DepotError::new("the asset has no id"))?;
    safe_relpath(id)?;
    let path = text(v, "path").unwrap_or_default();
    let path = safe_relpath(path)
        .map_err(|e| DepotError::new(format!("its folder is not a project path: {e}")))?;
    if path.rsplit('/').next() != Some(id) || !path.contains('/') {
        return Err(DepotError::new(format!(
            "its folder {path:?} does not end in /{id} — an asset's folder is named for it"
        )));
    }
    let engine = text(v, "engine").filter(|e| is_engine(e)).unwrap_or("any");
    let author = v.get("author").unwrap_or(&Value::Null);
    let hosted = text(author, "host") == Some("elo");
    let (owner, repo) = match (text(author, "owner"), text(author, "repo")) {
        (Some(o), _) if hosted && is_hosted_owner(o) => (o, ""),
        (Some(o), Some(r)) if !hosted && is_github_name(o) && is_github_name(r) => (o, r),
        _ if hosted => return Err(DepotError::new("it names no author the store knows")),
        _ => return Err(DepotError::new("it names no GitHub repo")),
    };
    let rows = v
        .get("files")
        .and_then(Value::as_array)
        .filter(|f| (1..=FILES_MAX).contains(&f.len()))
        .ok_or_else(|| DepotError::new(format!("it lists no files, or more than {FILES_MAX}")))?;
    let mut files = Vec::with_capacity(rows.len());
    let mut names = BTreeSet::new();
    for (i, f) in rows.iter().enumerate() {
        let n = i + 1;
        let name = text(f, "name").unwrap_or_default();
        check_file_name(name).map_err(|e| DepotError::new(format!("file {n}: {e}")))?;
        if !names.insert(alias_key(name)) {
            return Err(DepotError::new(format!("file {n}: {name:?} is listed twice")));
        }
        let url = text(f, "url").unwrap_or_default();
        if hosted {
            check_ipfs_url(url)
        } else {
            check_release_url(url, owner, repo)
        }
        .map_err(|e| DepotError::new(format!("file {n}: {e}")))?;
        let sha256 = text(f, "sha256")
            .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| DepotError::new(format!("file {n}: no sha256 to check it by")))?
            .to_ascii_lowercase();
        let size = f
            .get("size")
            .and_then(Value::as_i64)
            .filter(|s| (1..=FILE_SIZE_MAX).contains(s))
            .ok_or_else(|| DepotError::new(format!("file {n}: its size is not 1 byte to 2 GiB")))?;
        files.push(ModFile { name: name.to_string(), url: url.to_string(), sha256, size });
    }
    let source = text(v, "source")
        .filter(|s| s.starts_with("https://") && s.len() <= 300 && !s.chars().any(char::is_control))
        .map(str::to_string);
    Ok(Asset {
        id: id.to_string(),
        version: plain(text(v, "version").unwrap_or_default(), 32),
        engine: engine.to_string(),
        path,
        files,
        license: plain(text(v, "license").unwrap_or("not stated"), 64),
        credit: plain(text(v, "credit").unwrap_or(id), 200),
        owner: owner.to_string(),
        repo: repo.to_string(),
        hosted,
        source,
        requires: v
            .get("requires")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|r| is_mod_id(r))
            .map(str::to_string)
            .collect(),
    })
}

/// One asset as the lock writes it.
pub fn asset_json(a: &Asset) -> Value {
    json!({
        "id": a.id, "version": a.version, "engine": a.engine, "path": a.path,
        "license": a.license, "credit": a.credit, "source": a.source, "requires": a.requires,
        "author": if a.hosted { json!({"host": "elo", "owner": a.owner, "repo": ""}) }
                  else { json!({"owner": a.owner, "repo": a.repo}) },
        "files": a.files.iter().map(|f| json!({
            "name": f.name, "url": f.url, "sha256": f.sha256, "size": f.size,
        })).collect::<Vec<_>>(),
    })
}

/// `elo-assets.lock`: the project's engine and every asset added, in the
/// order they were added.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lock {
    pub engine: String,
    pub assets: Vec<Asset>,
}

impl Lock {
    /// The lock at `root`: `Ok(None)` when there is none yet, an error when
    /// one is there and does not read — never an empty lock in its place,
    /// which a later write would save over the real one.
    pub fn read(root: &Path) -> Result<Option<Lock>> {
        let p = root.join(LOCK_NAME);
        let body = match std::fs::read_to_string(&p) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(DepotError::new(format!("{}: {e}", p.display()))),
        };
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| DepotError::new(format!("{LOCK_NAME} is not JSON: {e}")))?;
        if v.get("elo_assets").and_then(Value::as_i64) != Some(1) {
            return Err(DepotError::new(format!("{LOCK_NAME} is not an elo assets lock (\"elo_assets\": 1)")));
        }
        let mut assets = Vec::new();
        for (i, a) in v.get("assets").and_then(Value::as_array).into_iter().flatten().enumerate() {
            assets.push(parse_asset(a).map_err(|e| {
                DepotError::new(format!("{LOCK_NAME}: asset {}: {e}", i + 1))
            })?);
        }
        let engine = text(&v, "engine").filter(|e| is_engine(e)).unwrap_or("any").to_string();
        Ok(Some(Lock { engine, assets }))
    }

    pub fn to_json(&self) -> Value {
        json!({"elo_assets": 1, "engine": self.engine,
               "assets": self.assets.iter().map(asset_json).collect::<Vec<_>>()})
    }

    /// Written whole or not at all.
    pub fn write(&self, root: &Path) -> Result<PathBuf> {
        write_whole(root, LOCK_NAME, &(serde_json::to_string_pretty(&self.to_json())
            .map_err(|e| DepotError::new(e.to_string()))? + "\n"))
    }

    pub fn get(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// Add an asset, or replace the one with its id in place.
    pub fn put(&mut self, a: Asset) {
        match self.assets.iter_mut().find(|x| x.id == a.id) {
            Some(slot) => *slot = a,
            None => self.assets.push(a),
        }
    }

    pub fn drop_id(&mut self, id: &str) -> Option<Asset> {
        let i = self.assets.iter().position(|a| a.id == id)?;
        Some(self.assets.remove(i))
    }
}

fn write_whole(root: &Path, name: &str, body: &str) -> Result<PathBuf> {
    let mut tmp = tempfile::Builder::new().prefix(PART_PREFIX).tempfile_in(root)?;
    tmp.write_all(body.as_bytes())?;
    let dest = root.join(name);
    tmp.persist(&dest)
        .map_err(|e| DepotError::new(format!("could not write {name}: {}", e.error)))?;
    Ok(dest)
}

/// Which engine a project is, from the files every one of them leaves at its
/// root. A guess the builder can override with `--engine`.
pub fn detect_engine(root: &Path) -> &'static str {
    let has = |n: &str| root.join(n).exists();
    if has("project.godot") {
        return "godot";
    }
    if has("ProjectSettings") && has("Assets") {
        return "unity";
    }
    if std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().ends_with(".uproject"))
    {
        return "unreal";
    }
    let read = |n: &str| std::fs::read_to_string(root.join(n)).unwrap_or_default();
    let cargo = read("Cargo.toml");
    if cargo.contains("bevy") {
        return "bevy";
    }
    if cargo.contains("fyrox") {
        return "fyrox";
    }
    if read("package.json").contains("\"three\"") {
        return "three";
    }
    "any"
}

/// What landing one asset did.
#[derive(Debug, Clone)]
pub struct Landed {
    pub id: String,
    pub dir: PathBuf,
    pub files: Vec<FileDone>,
}

impl Landed {
    pub fn complete(&self) -> bool {
        self.files.iter().all(|f| f.outcome.ok())
    }
    pub fn bad(&self) -> usize {
        self.files.iter().filter(|f| f.outcome.bad()).count()
    }
}

/// Put one asset's files in its folder under `root`, each checked, and clear
/// out what the folder holds that the asset no longer lists. A dry run
/// fetches and removes nothing.
pub fn land(
    root: &Path,
    a: &Asset,
    fetcher: &dyn Fetcher,
    dry_run: bool,
    progress: &mut dyn FnMut(&Asset, &ModFile),
) -> Landed {
    let dir = resolve_into(root, &a.path).and_then(|d| {
        if !dry_run {
            std::fs::create_dir_all(&d)?;
        }
        Ok(d)
    });
    let files = a
        .files
        .iter()
        .map(|f| {
            let rel = format!("{}/{}", a.path, f.name);
            let (path, outcome) = match (&dir, resolve_into(root, &rel)) {
                (Ok(d), Ok(dest)) => {
                    let o = place_file(f, d, &dest, fetcher, dry_run, &mut || progress(a, f));
                    (dest, o)
                }
                (Err(e), _) => (root.join(&rel), Outcome::Failed(e.to_string())),
                (_, Err(e)) => (root.join(&rel), Outcome::Failed(e.to_string())),
            };
            FileDone { name: f.name.clone(), path, size: f.size, outcome }
        })
        .collect();
    if let (Ok(d), false) = (&dir, dry_run) {
        tidy(d, &a.files);
    }
    Landed { id: a.id.clone(), dir: root.join(&a.path), files }
}

fn tidy(dir: &Path, files: &[ModFile]) {
    let listed: BTreeSet<String> = files.iter().map(|f| alias_key(&f.name)).collect();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !listed.contains(&alias_key(&name)) {
            let _ = remove_any(&e.path());
        }
    }
}

/// Remove an asset's folder. Only ever the folder a lock entry names, which
/// `parse_asset` already held to ending in the asset's id.
pub fn remove(root: &Path, a: &Asset) -> Result<bool> {
    let dir = resolve_into(root, &a.path)?;
    if !dir.exists() {
        return Ok(false);
    }
    remove_any(&dir).map_err(|e| DepotError::new(format!("{}: {e}", dir.display())))?;
    Ok(true)
}

/// The credits file: one entry per asset, in the order added, with the
/// author's own credit line, the licence and where the work came from.
pub fn credits_markdown(lock: &Lock) -> String {
    let mut out = String::from(
        "# Credits\n\nThe assets in this project, and who made them. Written by `elo assets \
         credits` from elo-assets.lock; run it again after adding one.\n",
    );
    for a in &lock.assets {
        out.push_str(&format!("\n## {}\n\n", a.id));
        out.push_str(&format!("- {}\n", a.credit));
        out.push_str(&format!("- version {} · licence {}\n", a.version, a.license));
        if a.hosted {
            out.push_str(&format!("- https://elopros.com/assets.html#{}\n", a.id));
        } else {
            out.push_str(&format!("- https://github.com/{}/{}\n", a.owner, a.repo));
        }
        if let Some(s) = &a.source {
            out.push_str(&format!("- source: {s}\n"));
        }
    }
    out
}

pub fn write_credits(root: &Path, lock: &Lock) -> Result<PathBuf> {
    write_whole(root, CREDITS_NAME, &credits_markdown(lock))
}

/// What an author fills in for `elo assets init`; the files say the rest.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub id: String,
    pub name: String,
    pub about: String,
    pub kind: String,
    pub version: String,
    pub license: String,
    pub engines: Vec<String>,
    pub price_cents: u32,
}

/// `elo-asset.json` for the files in `dir`, as they will be uploaded to the
/// release `tag` of `owner/repo`: each file's name, release url, sha256 and
/// size, computed here. Hidden files, folders and the manifest itself are
/// skipped; the files are sorted so the manifest is the same on every run.
pub fn build_manifest(dir: &Path, owner: &str, repo: &str, tag: &str, meta: &Meta) -> Result<Value> {
    if !is_github_name(owner) || !is_github_name(repo) {
        return Err(DepotError::new("the repo is https://github.com/<owner>/<repo>"));
    }
    let tag_ok = !tag.is_empty()
        && tag.len() <= 128
        && tag != "."
        && tag != ".."
        && tag.bytes().all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c));
    if !tag_ok {
        return Err(DepotError::new(format!("{tag:?} is not a release tag — like v1.0.0")));
    }
    if !is_mod_id(&meta.id) {
        return Err(DepotError::new("the id is lowercase letters, digits and '-'"));
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| DepotError::new(format!("{}: {e}", dir.display())))?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.') && n != MANIFEST_NAME)
        .collect();
    names.sort();
    if names.is_empty() {
        return Err(DepotError::new(format!("{} holds no files to list", dir.display())));
    }
    if names.len() > FILES_MAX {
        return Err(DepotError::new(format!(
            "{} holds {} files and an asset lists {FILES_MAX} at most — zip them",
            dir.display(),
            names.len()
        )));
    }
    let mut files = Vec::with_capacity(names.len());
    for n in &names {
        check_file_name(n).map_err(|e| {
            DepotError::new(format!("{e} — rename it before it goes in a release"))
        })?;
        let p = dir.join(n);
        let size = std::fs::metadata(&p)?.len() as i64;
        if !(1..=FILE_SIZE_MAX).contains(&size) {
            return Err(DepotError::new(format!("{n} is {size} bytes; a file is 1 byte to 2 GiB")));
        }
        files.push(json!({
            "name": n,
            "url": format!("https://github.com/{owner}/{repo}/releases/download/{tag}/{n}"),
            "sha256": sha256_file(&p)?,
            "size": size,
        }));
    }
    let preview = names.iter().find(|n| {
        let l = n.to_ascii_lowercase();
        l.starts_with("preview") && (l.ends_with(".png") || l.ends_with(".jpg"))
    });
    let engines = if meta.engines.is_empty() { vec!["any".to_string()] } else { meta.engines.clone() };
    let mut m = json!({
        "elo_asset": 1, "id": meta.id,
        "name": if meta.name.is_empty() { meta.id.clone() } else { meta.name.clone() },
        "about": if meta.about.is_empty() { "say what it is, and what is in it".to_string() }
                 else { meta.about.clone() },
        "kind": if meta.kind.is_empty() { "model" } else { meta.kind.as_str() },
        "version": if meta.version.is_empty() { tag.trim_start_matches('v').to_string() }
                   else { meta.version.clone() },
        "license": if meta.license.is_empty() { "CC-BY-4.0" } else { meta.license.as_str() },
        "engines": engines, "price_cents": meta.price_cents, "files": files,
    });
    if let Some(p) = preview {
        // A preview the card shows is read from the repo, not the release.
        m["_preview_hint"] = json!(format!("commit {p} to the repo too, and set \"preview\": \"{p}\""));
    }
    Ok(m)
}

/// The picture `upload` sends for the card: `preview.png`, `.jpg` or `.jpeg`
/// in the folder, never one of the files.
fn is_preview_name(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    l.starts_with("preview") && (l.ends_with(".png") || l.ends_with(".jpg") || l.ends_with(".jpeg"))
}

/// `elo-asset.json` for uploading the files in `dir` to the store (no GitHub):
/// no url on any file (the store pins them and writes one), every file's
/// sha256 and size read off the disk now, and a `preview.png`/`.jpg` beside
/// them sent as the card's picture. `prior` is the manifest already in the
/// folder from an earlier upload: what it says stands (tags, made_with,
/// credit, source, payee…) unless `meta` says otherwise, so an update is the
/// same command with a new `--version`.
pub fn build_upload_manifest(dir: &Path, meta: &Meta, prior: Option<&Value>) -> Result<Value> {
    if prior.and_then(|p| p.get("files")).and_then(Value::as_array).into_iter().flatten()
        .any(|f| f.get("url").is_some())
    {
        return Err(DepotError::new(
            "this folder's elo-asset.json names release urls: it is published from GitHub \
             (`elo assets publish --repo …`), not uploaded",
        ));
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| DepotError::new(format!("{}: {e}", dir.display())))?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.') && n != MANIFEST_NAME)
        .collect();
    names.sort();
    let preview = names.iter().find(|n| is_preview_name(n)).cloned();
    names.retain(|n| Some(n) != preview.as_ref());
    if names.is_empty() {
        return Err(DepotError::new(format!("{} holds no files to upload", dir.display())));
    }
    if names.len() > FILES_MAX {
        return Err(DepotError::new(format!(
            "{} holds {} files and an asset lists {FILES_MAX} at most — zip them",
            dir.display(),
            names.len()
        )));
    }
    let mut files = Vec::with_capacity(names.len());
    for n in &names {
        check_file_name(n).map_err(|e| DepotError::new(format!("{e} — rename it")))?;
        let p = dir.join(n);
        let size = std::fs::metadata(&p)?.len() as i64;
        if !(1..=FILE_SIZE_MAX).contains(&size) {
            return Err(DepotError::new(format!("{n} is {size} bytes; a file is 1 byte to 2 GiB")));
        }
        let mut f = json!({"name": n, "sha256": sha256_file(&p)?, "size": size});
        // an engine the earlier manifest pinned this file to stays pinned
        let was = prior.and_then(|p| p.get("files")).and_then(Value::as_array).into_iter().flatten()
            .find(|x| x.get("name").and_then(Value::as_str) == Some(n.as_str()));
        if let Some(e) = was.and_then(|x| x.get("engine")) {
            f["engine"] = e.clone();
        }
        files.push(f);
    }
    let mut m = prior.cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    let pick = |m: &Value, k: &str, given: &str, fallback: &str| -> Value {
        if !given.is_empty() {
            json!(given)
        } else {
            m.get(k).cloned().unwrap_or_else(|| json!(fallback))
        }
    };
    let id = if meta.id.is_empty() { m.get("id").and_then(Value::as_str).unwrap_or("").to_string() }
             else { meta.id.clone() };
    if !is_mod_id(&id) {
        return Err(DepotError::new("the id is lowercase letters, digits and '-' — --id <id>"));
    }
    m["elo_asset"] = json!(1);
    m["id"] = json!(id);
    m["name"] = pick(&m, "name", &meta.name, &id);
    m["about"] = pick(&m, "about", &meta.about, "");
    m["kind"] = pick(&m, "kind", &meta.kind, "model");
    m["version"] = pick(&m, "version", &meta.version, "1.0.0");
    m["license"] = pick(&m, "license", &meta.license, "CC-BY-4.0");
    if !meta.engines.is_empty() {
        m["engines"] = json!(meta.engines);
    } else if m.get("engines").is_none() {
        m["engines"] = json!(["any"]);
    }
    if meta.price_cents > 0 || m.get("price_cents").is_none() {
        m["price_cents"] = json!(meta.price_cents);
    }
    m["files"] = json!(files);
    match preview {
        Some(p) => m["preview"] = json!(p),
        None => {
            if let Some(o) = m.as_object_mut() {
                o.remove("preview");
            }
        }
    }
    if m.get("about").and_then(Value::as_str).unwrap_or("").trim().is_empty() {
        return Err(DepotError::new("say what it is — --about \"…\" (or fill in about in elo-asset.json)"));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha256};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const OWNER: &str = "ann";
    const REPO: &str = "low-poly-forest";

    fn sha(bytes: &[u8]) -> String {
        crate::digest::hex_lower(&Sha256::digest(bytes))
    }

    fn url(name: &str) -> String {
        format!("https://github.com/{OWNER}/{REPO}/releases/download/v1/{name}")
    }

    fn file(name: &str, bytes: &[u8]) -> Value {
        json!({"name": name, "url": url(name), "sha256": sha(bytes), "size": bytes.len()})
    }

    fn install(files: Vec<Value>) -> Value {
        json!({"id": "low-poly-forest", "version": "1.1.0", "engine": "godot",
               "path": "assets/elo/low-poly-forest", "files": files, "license": "CC-BY-4.0",
               "credit": "Forest kit by Ann Lee (CC-BY-4.0)",
               "author": {"owner": OWNER, "repo": REPO}, "requires": []})
    }

    struct Fake {
        bytes: BTreeMap<String, Vec<u8>>,
        asked: AtomicUsize,
    }

    impl Fetcher for Fake {
        fn fetch(&self, url: &str, dest: &Path) -> Result<()> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let b = self.bytes.get(url).ok_or_else(|| DepotError::new("404"))?;
            std::fs::write(dest, b)?;
            Ok(())
        }
    }

    fn fake(pairs: &[(&str, &[u8])]) -> Fake {
        Fake {
            bytes: pairs.iter().map(|(n, b)| (url(n), b.to_vec())).collect(),
            asked: AtomicUsize::new(0),
        }
    }

    #[test]
    fn the_folder_is_always_the_assets_own() {
        assert!(parse_asset(&install(vec![file("tree.glb", b"t")])).is_ok());
        for bad in ["src", "assets/elo/other", "../low-poly-forest", "/abs/low-poly-forest",
                    "low-poly-forest", "assets//low-poly-forest", "assets/elo/low-poly-forest/"] {
            let mut v = install(vec![file("tree.glb", b"t")]);
            v["path"] = json!(bad);
            assert!(parse_asset(&v).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_file_off_the_authors_release_is_refused_whole() {
        let mut elsewhere = file("tree.glb", b"t");
        elsewhere["url"] = json!("https://example.com/tree.glb");
        assert!(parse_asset(&install(vec![file("a.png", b"a"), elsewhere])).is_err());
        assert!(parse_asset(&install(vec![file("../x", b"x")])).is_err());
        assert!(parse_asset(&install(vec![file("A.png", b"a"), file("a.png", b"b")])).is_err());
        let mut v = install(vec![file("tree.glb", b"t")]);
        v["author"] = json!({"owner": "../x", "repo": REPO});
        assert!(parse_asset(&v).is_err());
    }

    #[test]
    fn an_asset_lands_checked_pins_in_the_lock_and_syncs_from_it() {
        let root = tempfile::tempdir().unwrap();
        let a = parse_asset(&install(vec![file("tree.glb", b"a tree"), file("rock.glb", b"a rock")]))
            .unwrap();
        let f = fake(&[("tree.glb", b"a tree"), ("rock.glb", b"a rock")]);
        let got = land(root.path(), &a, &f, false, &mut |_, _| {});
        assert!(got.complete() && got.bad() == 0, "{:?}", got.files);
        assert_eq!(std::fs::read(root.path().join("assets/elo/low-poly-forest/tree.glb")).unwrap(),
                   b"a tree");
        let mut lock = Lock { engine: "godot".into(), assets: vec![] };
        lock.put(a.clone());
        lock.write(root.path()).unwrap();
        let back = Lock::read(root.path()).unwrap().unwrap();
        assert_eq!(back, lock, "the lock reads back as written");

        // A sync from the lock keeps what is right and fetches nothing.
        let again = land(root.path(), &back.assets[0], &f, false, &mut |_, _| {});
        assert!(again.files.iter().all(|x| x.outcome == Outcome::Kept));
        assert_eq!(f.asked.load(Ordering::SeqCst), 2);

        // An update drops the file the new version no longer lists.
        let b = parse_asset(&install(vec![file("tree.glb", b"a tree")])).unwrap();
        land(root.path(), &b, &f, false, &mut |_, _| {});
        assert!(!root.path().join("assets/elo/low-poly-forest/rock.glb").exists());
    }

    #[test]
    fn an_uploaded_asset_lands_from_ipfs_and_the_lock_keeps_its_host() {
        const CID: &str = "bafkreigh2akiscaildcqabsyg3dfr6chu3fgpregiymsck7e7aqa4s52zy";
        let gw = format!("https://gw.example/ipfs/{CID}");
        let w = "0x00000000000000000000000000000000000000ab";
        let v = json!({"id": "fel-wolf", "version": "1.0.0", "engine": "any",
                       "path": "assets/elo/fel-wolf", "license": "CC-BY-4.0",
                       "credit": "Fel wolf by Elo Pros, CC-BY-4.0",
                       "author": {"host": "elo", "owner": w, "repo": "", "name": "0x0000…00ab"},
                       "files": [{"name": "wolf.glb", "url": gw, "sha256": sha(b"a wolf"), "size": 6}]});
        let a = parse_asset(&v).unwrap();
        assert!(a.hosted && a.owner == w && a.repo.is_empty());
        let root = tempfile::tempdir().unwrap();
        let f = Fake { bytes: [(gw.clone(), b"a wolf".to_vec())].into_iter().collect(),
                       asked: AtomicUsize::new(0) };
        let got = land(root.path(), &a, &f, false, &mut |_, _| {});
        assert!(got.complete() && got.bad() == 0, "{:?}", got.files);
        let mut lock = Lock { engine: "any".into(), assets: vec![] };
        lock.put(a.clone());
        lock.write(root.path()).unwrap();
        assert_eq!(Lock::read(root.path()).unwrap().unwrap(), lock, "hosted survives the lock");
        assert!(credits_markdown(&lock).contains("https://elopros.com/assets.html#fel-wolf"));

        // The house is an author too; a GitHub-shaped name with host "elo" is not.
        let mut house = v.clone();
        house["author"]["owner"] = json!(HOUSE);
        assert!(parse_asset(&house).is_ok());
        let mut stranger = v.clone();
        stranger["author"]["owner"] = json!("ann");
        assert!(parse_asset(&stranger).is_err());
        // An uploaded asset's files are on a gateway, by CID, and nowhere else.
        for bad in ["https://gw.example/wolf.glb", "http://gw.example/ipfs/bafkreigh2akiscaildcqabsyg3dfr6chu3fgp",
                    "https://gw.example/ipfs/../../etc", "https://user@gw.example/ipfs/bafkreigh2akiscaildcqabsyg3dfr6chu3",
                    "https://github.com/ann/x/releases/download/v1/wolf.glb"] {
            let mut x = v.clone();
            x["files"][0]["url"] = json!(bad);
            assert!(parse_asset(&x).is_err(), "{bad}");
        }
        // …and a GitHub asset still may not point at a gateway.
        let mut gh = install(vec![file("tree.glb", b"t")]);
        gh["files"][0]["url"] = json!(gw);
        assert!(parse_asset(&gh).is_err());
    }

    #[test]
    fn an_upload_manifest_is_read_off_the_folder_and_keeps_what_it_said() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("crate.glb"), b"a crate").unwrap();
        std::fs::write(dir.path().join("crate.png"), b"its texture").unwrap();
        std::fs::write(dir.path().join("preview.jpg"), b"\xff\xd8\xffpic").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), b"x").unwrap();
        let meta = Meta { id: "wood-crate".into(), about: "A wooden crate.".into(), ..Meta::default() };
        let m = build_upload_manifest(dir.path(), &meta, None).unwrap();
        let names: Vec<&str> = m["files"].as_array().unwrap().iter()
            .map(|f| f["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["crate.glb", "crate.png"], "the picture and hidden files are not files");
        assert_eq!(m["preview"], "preview.jpg");
        assert_eq!(m["files"][0]["sha256"], sha(b"a crate"));
        assert!(m["files"][0].get("url").is_none(), "the store writes the url");
        assert_eq!((m["version"].as_str(), m["license"].as_str(), m["kind"].as_str()),
                   (Some("1.0.0"), Some("CC-BY-4.0"), Some("model")));

        // The next upload keeps what the manifest said, and takes a new version.
        let mut prior = m.clone();
        prior["tags"] = json!(["crate"]);
        prior["made_with"] = json!({"parts": []});
        prior["files"][0]["engine"] = json!("any");
        std::fs::write(dir.path().join("crate.glb"), b"a better crate").unwrap();
        let next = Meta { version: "1.1.0".into(), ..Meta::default() };
        let m2 = build_upload_manifest(dir.path(), &next, Some(&prior)).unwrap();
        assert_eq!((m2["id"].as_str(), m2["version"].as_str()), (Some("wood-crate"), Some("1.1.0")));
        assert_eq!(m2["tags"], json!(["crate"]));
        assert_eq!(m2["made_with"], json!({"parts": []}));
        assert_eq!(m2["files"][0]["engine"], "any");
        assert_eq!(m2["files"][0]["sha256"], sha(b"a better crate"), "sha256s are read off the disk again");

        // A GitHub manifest is not an upload's, and an upload says what it is.
        let mut gh = prior.clone();
        gh["files"][0]["url"] = json!("https://github.com/a/b/releases/download/v1/crate.glb");
        assert!(build_upload_manifest(dir.path(), &next, Some(&gh)).is_err());
        let bare = Meta { id: "x".into(), ..Meta::default() };
        assert!(build_upload_manifest(dir.path(), &bare, None).is_err());
    }

    #[test]
    fn bytes_that_differ_never_land() {
        let root = tempfile::tempdir().unwrap();
        let a = parse_asset(&install(vec![file("tree.glb", b"a tree")])).unwrap();
        let got = land(root.path(), &a, &fake(&[("tree.glb", b"a trap")]), false, &mut |_, _| {});
        assert_eq!(got.bad(), 1);
        assert_eq!(std::fs::read_dir(root.path().join("assets/elo/low-poly-forest")).unwrap().count(), 0);
    }

    #[test]
    fn a_broken_lock_is_an_error_never_an_empty_lock() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(Lock::read(root.path()).unwrap(), None);
        std::fs::write(root.path().join(LOCK_NAME), "{ not json").unwrap();
        assert!(Lock::read(root.path()).is_err());
        std::fs::write(root.path().join(LOCK_NAME), r#"{"elo_assets": 1, "assets": [{"id": "x"}]}"#)
            .unwrap();
        assert!(Lock::read(root.path()).is_err());
    }

    #[test]
    fn remove_takes_only_the_assets_folder() {
        let root = tempfile::tempdir().unwrap();
        let a = parse_asset(&install(vec![file("tree.glb", b"a tree")])).unwrap();
        land(root.path(), &a, &fake(&[("tree.glb", b"a tree")]), false, &mut |_, _| {});
        std::fs::write(root.path().join("assets/elo/mine.txt"), b"mine").unwrap();
        assert!(remove(root.path(), &a).unwrap());
        assert!(!root.path().join("assets/elo/low-poly-forest").exists());
        assert!(root.path().join("assets/elo/mine.txt").exists());
    }

    #[test]
    fn credits_name_every_author_and_licence() {
        let a = parse_asset(&install(vec![file("tree.glb", b"t")])).unwrap();
        let mut hostile = install(vec![file("tree.glb", b"t")]);
        hostile["id"] = json!("rocks");
        hostile["path"] = json!("assets/elo/rocks");
        hostile["credit"] = json!("Rocks\x1b[2J by Bo");
        hostile["source"] = json!("https://kenney.nl/assets");
        let b = parse_asset(&hostile).unwrap();
        let md = credits_markdown(&Lock { engine: "godot".into(), assets: vec![a, b] });
        assert!(md.contains("## low-poly-forest") && md.contains("Forest kit by Ann Lee"), "{md}");
        assert!(md.contains("licence CC-BY-4.0") && md.contains("github.com/ann/low-poly-forest"));
        assert!(md.contains("source: https://kenney.nl/assets") && !md.contains('\x1b'), "{md}");
    }

    #[test]
    fn a_manifest_is_written_from_the_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tree.glb"), b"a tree").unwrap();
        std::fs::write(dir.path().join("preview.png"), b"png").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), b"x").unwrap();
        std::fs::write(dir.path().join(MANIFEST_NAME), b"{}").unwrap();
        let meta = Meta { id: "low-poly-forest".into(), kind: "model".into(), ..Meta::default() };
        let m = build_manifest(dir.path(), OWNER, REPO, "v1.2.0", &meta).unwrap();
        assert_eq!(m["version"], "1.2.0");
        assert_eq!(m["engines"], json!(["any"]));
        let names: Vec<&str> = m["files"].as_array().unwrap().iter()
            .map(|f| f["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["preview.png", "tree.glb"], "hidden files and the manifest are skipped");
        assert_eq!(m["files"][1]["sha256"], sha(b"a tree"));
        assert_eq!(m["files"][1]["url"],
                   "https://github.com/ann/low-poly-forest/releases/download/v1.2.0/tree.glb");
        assert!(m["_preview_hint"].as_str().unwrap().contains("preview.png"));
        assert!(build_manifest(dir.path(), OWNER, REPO, "../x", &meta).is_err());
        assert!(build_manifest(dir.path(), "a/b", REPO, "v1", &meta).is_err());
        std::fs::write(dir.path().join("bad name.glb"), b"x").unwrap();
        assert!(build_manifest(dir.path(), OWNER, REPO, "v1", &meta).is_err());
    }

    #[test]
    fn the_engine_is_read_off_the_project() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(detect_engine(d.path()), "any");
        std::fs::write(d.path().join("Cargo.toml"), "[dependencies]\nbevy = \"0.16\"\n").unwrap();
        assert_eq!(detect_engine(d.path()), "bevy");
        std::fs::write(d.path().join("project.godot"), "").unwrap();
        assert_eq!(detect_engine(d.path()), "godot");
    }
}
