//! Workshop mods on this machine: what a player subscribed to, fetched from
//! each author's GitHub release and held to the sha256 the author published.
//!
//! The origin half is `meter/workshop.py`. `GET /api/workshop/<game>/
//! subscribed/<wallet>` names every file with its url, size and sha256, and
//! the origin stores none of the bytes. This module lands them, and it lives
//! in this crate for the crate's own reason: it writes files a stranger named,
//! so the two path checks an install makes (`path::safe_relpath`,
//! `path::resolve_into`) stand between a name off the wire and the player's
//! disk. No network here either; the caller hands in a [`Fetcher`].
//!
//! The layout, which a game reads:
//!
//! ```text
//! <mods root>/<game>/mods.json           what checked out, in subscription order
//! <mods root>/<game>/<mod id>/<file>     each file, byte for byte the author's
//! ```
//!
//! Three rules, each a refusal:
//!
//! * **A file lands only after its bytes check.** It streams to a temp file in
//!   the mod's folder, is held to its size and sha256, and only then takes its
//!   name. Bytes that differ are deleted, never kept under any name.
//! * **A url is the author's own release:** `https://github.com/<owner>/<repo>/
//!   releases/download/<tag>/<file>`, for the repo the mod is published from,
//!   the same rule the origin applies when it reads `elo-mod.json`. An answer
//!   pointing anywhere else is refused here too, so an origin cannot aim this
//!   machine at a host the author never chose.
//! * **Only a definite answer removes anything.** A mod folder goes when a
//!   subscribed list that parsed no longer carries it, and only a folder named
//!   like a mod id, inside that game's folder. A read that failed changes
//!   nothing on disk.

use crate::digest::sha256_file;
use crate::error::{DepotError, Result};
use crate::install::Fetcher;
use crate::path::{alias_key, resolve_into, safe_relpath};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The largest file a mod may list: GitHub's own cap on one release asset,
/// and `meter/workshop.FILE_SIZE_MAX`.
pub const FILE_SIZE_MAX: i64 = 2 * 1024 * 1024 * 1024;
/// How many files one mod may list (`meter/workshop.FILES_MAX`).
pub const FILES_MAX: usize = 8;
/// The file in a game's mods folder that says what checked out.
pub const INDEX_NAME: &str = "mods.json";
/// The temp-file prefix. A listed name starts with a letter or digit, so a
/// part file can never be mistaken for one.
const PART_PREFIX: &str = ".part-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModFile {
    pub name: String,
    pub url: String,
    /// 64 lowercase hex digits.
    pub sha256: String,
    pub size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mod {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub version: String,
    pub files: Vec<ModFile>,
    pub requires: Vec<String>,
    /// The GitHub repo the mod is published from.
    pub owner: String,
    pub repo: String,
    pub license: Option<String>,
}

/// One player's subscriptions in one game, as this client will act on them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Subscribed {
    pub game: String,
    pub wallet: String,
    /// Whether the workshop takes new subscriptions. Published mods stay
    /// readable when it closes.
    pub open: bool,
    pub runs_code: bool,
    /// The origin's sentence when the game's mods run code.
    pub warning: Option<String>,
    /// Why the game takes no mods, when it never opened a workshop.
    pub why: Option<String>,
    pub mods: Vec<Mod>,
    /// `(mod id, why)` the origin holds back: taken down, hidden, gone.
    pub withheld: Vec<(String, String)>,
    /// `(mod id, why)` this client will not fetch: a name, url, size or
    /// sha256 that fails the checks here. Their folders are left as they are.
    pub refused: Vec<(String, String)>,
}

/// A game slug, as the store writes one: `[a-z0-9][a-z0-9-]{0,62}`.
pub fn is_slug(s: &str) -> bool {
    id_like(s, 63)
}

/// A mod id, as the workshop writes one: `[a-z0-9][a-z0-9-]{0,47}`.
pub fn is_mod_id(s: &str) -> bool {
    id_like(s, 48)
}

fn id_like(s: &str, max: usize) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= max
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// A GitHub owner or repo name: `[A-Za-z0-9][A-Za-z0-9._-]*`.
fn is_github_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 100
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(c))
}

/// Accept a mod's file name or refuse it.
///
/// The origin's rule first (`[A-Za-z0-9][A-Za-z0-9._+-]{0,99}`: one plain
/// name, no folder), then the depot path rule, which also refuses the names
/// Windows reads as another file or a device (`map.`, `con.pak`). The name
/// comes off the network, and this is what keeps it inside the mod's folder.
pub fn check_file_name(name: &str) -> Result<()> {
    let b = name.as_bytes();
    let plain = !b.is_empty()
        && b.len() <= 100
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(c));
    if !plain {
        return Err(DepotError::new(format!("{name:?} is not a plain file name")));
    }
    safe_relpath(name).map(|_| ())
}

/// Accept a file url only as a release asset of the mod's own repo.
///
/// The host is fixed by the prefix, so nothing after it can name another
/// one; the owner and repo are compared the way GitHub does, without case.
pub fn check_release_url(url: &str, owner: &str, repo: &str) -> Result<()> {
    let refuse = || {
        DepotError::new(format!(
            "{url:?} is not a release asset of github.com/{owner}/{repo}"
        ))
    };
    let rest = url.strip_prefix("https://github.com/").ok_or_else(refuse)?;
    let parts: Vec<&str> = rest.split('/').collect();
    let piece = |p: &str| {
        !p.is_empty()
            && p != "."
            && p != ".."
            && p.bytes().all(|c| c.is_ascii_graphic() && !b"?#\\".contains(&c))
    };
    let ok = parts.len() == 6
        && parts[0].eq_ignore_ascii_case(owner)
        && parts[1].eq_ignore_ascii_case(repo)
        && parts[2] == "releases"
        && parts[3] == "download"
        && piece(parts[4])
        && piece(parts[5]);
    if ok {
        Ok(())
    } else {
        Err(refuse())
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// The subscribed list, parsed. A document that is not one is an error; a
/// mod inside it that fails a check is set aside in `refused`, so one bad
/// entry never stops the rest.
pub fn parse_subscribed(v: &Value) -> Result<Subscribed> {
    let game = text(v, "game")
        .filter(|g| is_slug(g))
        .ok_or_else(|| DepotError::new("the subscribed list names no game"))?;
    let mods = v
        .get("mods")
        .and_then(Value::as_array)
        .ok_or_else(|| DepotError::new("the subscribed list carries no mods"))?;
    let mut out = Subscribed {
        game: game.to_string(),
        wallet: text(v, "wallet").unwrap_or_default().to_string(),
        open: v.get("open").and_then(Value::as_bool).unwrap_or(false),
        runs_code: v.get("runs_code").and_then(Value::as_bool).unwrap_or(false),
        warning: text(v, "warning").map(str::to_string),
        why: text(v, "why").map(str::to_string),
        ..Subscribed::default()
    };
    let mut seen = BTreeSet::new();
    for m in mods {
        match parse_mod(m) {
            Ok(m) if seen.insert(m.id.clone()) => out.mods.push(m),
            Ok(m) => out.refused.push((m.id, "listed twice".into())),
            Err(e) => out
                .refused
                .push((text(m, "id").unwrap_or("?").to_string(), e.to_string())),
        }
    }
    for w in v.get("withheld").and_then(Value::as_array).into_iter().flatten() {
        out.withheld.push((
            text(w, "mod").unwrap_or("?").to_string(),
            text(w, "why").unwrap_or("held back").to_string(),
        ));
    }
    Ok(out)
}

fn parse_mod(m: &Value) -> Result<Mod> {
    let id = text(m, "id")
        .filter(|i| is_mod_id(i))
        .ok_or_else(|| DepotError::new("its id is not a mod id"))?;
    // The id is a folder name too, held to the depot rule: `con` is a mod id
    // and a Windows device.
    safe_relpath(id)?;
    let author = m.get("author").unwrap_or(&Value::Null);
    let (owner, repo) = match (text(author, "owner"), text(author, "repo")) {
        (Some(o), Some(r)) if is_github_name(o) && is_github_name(r) => (o, r),
        _ => return Err(DepotError::new("it names no GitHub repo")),
    };
    let rows = m
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
        // One name per file on every disk: `Map.pak` and `map.pak` are one
        // file on Windows and macOS, and whichever landed last would win.
        if !names.insert(alias_key(name)) {
            return Err(DepotError::new(format!("file {n}: {name:?} is listed twice")));
        }
        let url = text(f, "url").unwrap_or_default();
        check_release_url(url, owner, repo).map_err(|e| DepotError::new(format!("file {n}: {e}")))?;
        let sha256 = text(f, "sha256")
            .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| DepotError::new(format!("file {n}: no sha256 to check it by")))?
            .to_ascii_lowercase();
        let size = f
            .get("size")
            .and_then(Value::as_i64)
            .filter(|s| (1..=FILE_SIZE_MAX).contains(s))
            .ok_or_else(|| {
                DepotError::new(format!("file {n}: its size is not 1 byte to 2 GiB"))
            })?;
        files.push(ModFile { name: name.to_string(), url: url.to_string(), sha256, size });
    }
    let requires = m
        .get("requires")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|r| is_mod_id(r))
        .map(str::to_string)
        .collect();
    Ok(Mod {
        id: id.to_string(),
        name: text(m, "name").unwrap_or(id).to_string(),
        kind: text(m, "kind").unwrap_or_default().to_string(),
        version: text(m, "version").unwrap_or_default().to_string(),
        files,
        requires,
        owner: owner.to_string(),
        repo: repo.to_string(),
        license: text(m, "license").map(str::to_string),
    })
}

/// What happened to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Already on disk, at its size and sha256.
    Kept,
    /// Downloaded, and its bytes checked.
    Fetched,
    /// A dry run: it would be downloaded.
    WouldFetch,
    /// Downloaded, and the bytes are not what the author published. Deleted.
    Mismatch(String),
    /// Could not be fetched or put in place. Nothing is left under its name.
    Failed(String),
}

impl Outcome {
    /// On disk and checked.
    pub fn ok(&self) -> bool {
        matches!(self, Outcome::Kept | Outcome::Fetched)
    }
    pub fn bad(&self) -> bool {
        matches!(self, Outcome::Mismatch(_) | Outcome::Failed(_))
    }
}

#[derive(Debug, Clone)]
pub struct FileDone {
    pub name: String,
    pub path: PathBuf,
    pub size: i64,
    pub outcome: Outcome,
}

#[derive(Debug, Clone)]
pub struct ModDone {
    pub id: String,
    pub dir: PathBuf,
    pub files: Vec<FileDone>,
}

impl ModDone {
    /// Every file on disk and checked: what `mods.json` lists.
    pub fn complete(&self) -> bool {
        self.files.iter().all(|f| f.outcome.ok())
    }
}

#[derive(Debug, Clone)]
pub struct Synced {
    /// The game's mods folder.
    pub dir: PathBuf,
    pub index: PathBuf,
    pub mods: Vec<ModDone>,
    /// Mod folders removed (or, in a dry run, that would be).
    pub removed: Vec<String>,
    pub dry_run: bool,
}

impl Synced {
    /// Files that did not check out or could not be fetched.
    pub fn bad(&self) -> usize {
        self.mods.iter().flat_map(|m| &m.files).filter(|f| f.outcome.bad()).count()
    }
}

/// A game's mods folder under `root`.
pub fn game_dir(root: &Path, game: &str) -> Result<PathBuf> {
    if !is_slug(game) {
        return Err(DepotError::new(format!("{game:?} is not a game slug")));
    }
    Ok(root.join(safe_relpath(game)?))
}

/// Bring a game's mods folder in line with a subscribed list.
///
/// Each listed file is kept when it is already there at its size and sha256,
/// else fetched and checked; files a mod no longer lists are removed from its
/// folder, folders of mods no longer subscribed are removed, and `mods.json`
/// is rewritten to list the mods whose every file checked. `progress` hears
/// about each file just before it is fetched. A dry run fetches, removes and
/// writes nothing, and says what it would do.
pub fn sync(
    sub: &Subscribed,
    root: &Path,
    fetcher: &dyn Fetcher,
    dry_run: bool,
    progress: &mut dyn FnMut(&Mod, &ModFile),
) -> Result<Synced> {
    let dir = game_dir(root, &sub.game)?;
    if !dry_run {
        std::fs::create_dir_all(&dir)
            .map_err(|e| DepotError::new(format!("{}: {e}", dir.display())))?;
    }
    let mut mods = Vec::with_capacity(sub.mods.len());
    for m in &sub.mods {
        // A mod folder that is a link out of the game's folder is refused
        // here, before anything is written through it.
        let mdir = resolve_into(&dir, &m.id).and_then(|d| {
            if !dry_run {
                std::fs::create_dir_all(&d)?;
            }
            Ok(d)
        });
        let files = m
            .files
            .iter()
            .map(|f| {
                let rel = format!("{}/{}", m.id, f.name);
                let (path, outcome) = match (&mdir, resolve_into(&dir, &rel)) {
                    (Ok(md), Ok(dest)) => {
                        let o = place(m, f, md, &dest, fetcher, dry_run, progress);
                        (dest, o)
                    }
                    (Err(e), _) => (dir.join(&rel), Outcome::Failed(e.to_string())),
                    (_, Err(e)) => (dir.join(&rel), Outcome::Failed(e.to_string())),
                };
                FileDone { name: f.name.clone(), path, size: f.size, outcome }
            })
            .collect();
        if let (Ok(md), false) = (&mdir, dry_run) {
            tidy(md, &m.files);
        }
        mods.push(ModDone { id: m.id.clone(), dir: dir.join(&m.id), files });
    }

    // Folders of mods still subscribed but refused here are left alone: the
    // list said they are wanted, and this client only could not check them.
    let keep: BTreeSet<&str> = sub
        .mods
        .iter()
        .map(|m| m.id.as_str())
        .chain(sub.refused.iter().map(|(id, _)| id.as_str()))
        .collect();
    let removed = prune(&dir, &keep, dry_run);
    let index = dir.join(INDEX_NAME);
    if !dry_run {
        write_index(&dir, sub, &mods)?;
    }
    Ok(Synced { dir, index, mods, removed, dry_run })
}

/// Put one file at `dest`, or say why it is not there.
fn place(
    m: &Mod,
    f: &ModFile,
    mdir: &Path,
    dest: &Path,
    fetcher: &dyn Fetcher,
    dry_run: bool,
    progress: &mut dyn FnMut(&Mod, &ModFile),
) -> Outcome {
    place_file(f, mdir, dest, fetcher, dry_run, &mut || progress(m, f))
}

/// The landing itself, for anything that names a file by url, size and
/// sha256: a Workshop mod's here, an asset's in `assets.rs`. `before_fetch`
/// runs just before the download starts.
pub(crate) fn place_file(
    f: &ModFile,
    mdir: &Path,
    dest: &Path,
    fetcher: &dyn Fetcher,
    dry_run: bool,
    before_fetch: &mut dyn FnMut(),
) -> Outcome {
    // Size first, so a file that cannot be right is not hashed.
    if let Ok(md) = std::fs::symlink_metadata(dest) {
        if md.is_file()
            && md.len() == f.size as u64
            && sha256_file(dest).is_ok_and(|got| got == f.sha256)
        {
            return Outcome::Kept;
        }
    }
    if dry_run {
        return Outcome::WouldFetch;
    }
    // Whatever is there is not the file the author published. It goes before
    // the fetch, so a fetch that fails leaves nothing rather than old bytes
    // under a name a game trusts.
    if let Err(e) = remove_any(dest) {
        return Outcome::Failed(format!("could not remove the old {}: {e}", f.name));
    }
    before_fetch();
    let part = match tempfile::Builder::new().prefix(PART_PREFIX).tempfile_in(mdir) {
        Ok(t) => t.into_temp_path(),
        Err(e) => return Outcome::Failed(format!("{}: {e}", mdir.display())),
    };
    // From here every early return drops `part`, which deletes it.
    if let Err(e) = fetcher.fetch_sized(&f.url, &part, f.size) {
        return Outcome::Failed(e.to_string());
    }
    let len = std::fs::metadata(&part).map(|md| md.len()).unwrap_or(0);
    if len != f.size as u64 {
        return Outcome::Mismatch(format!(
            "{len} bytes arrived, and the author published {} — deleted",
            f.size
        ));
    }
    match sha256_file(&part) {
        Ok(got) if got == f.sha256 => {}
        Ok(got) => {
            return Outcome::Mismatch(format!(
                "its sha256 is {}…, and the author published {}… — deleted",
                &got[..16],
                &f.sha256[..16]
            ))
        }
        Err(e) => return Outcome::Failed(e.to_string()),
    }
    match part.persist(dest) {
        Ok(()) => Outcome::Fetched,
        Err(e) => Outcome::Failed(format!("could not move it into place: {}", e.error)),
    }
}

/// Remove a file, a link or a folder. Absent is fine. A link is removed
/// itself and never followed.
pub(crate) fn remove_any(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Remove what a mod's folder holds that the mod no longer lists: an old
/// version's files, and a part file a killed run left behind.
///
/// Compared by `alias_key`, not by spelling: on a case-insensitive disk the
/// listed `Map.pak` may be on disk as `map.pak`, and that is the file just
/// kept, not a stray.
fn tidy(mdir: &Path, files: &[ModFile]) {
    let listed: BTreeSet<String> = files.iter().map(|f| alias_key(&f.name)).collect();
    let Ok(entries) = std::fs::read_dir(mdir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !listed.contains(&alias_key(&name)) {
            let _ = remove_any(&e.path());
        }
    }
}

/// Remove the folders of mods no longer subscribed. Only names that are mod
/// ids are touched, so `mods.json` and anything a player put here stay.
fn prune(dir: &Path, keep: &BTreeSet<&str>, dry_run: bool) -> Vec<String> {
    let mut removed = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return removed };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !is_mod_id(&name) || keep.contains(name.as_str()) {
            continue;
        }
        if dry_run || remove_any(&e.path()).is_ok() {
            removed.push(name);
        }
    }
    removed.sort();
    removed
}

/// `mods.json`: the mods whose every file checked, in subscription order,
/// with each file's path relative to the folder the index is in.
pub fn index_json(sub: &Subscribed, done: &[ModDone]) -> Value {
    let complete: BTreeSet<&str> =
        done.iter().filter(|d| d.complete()).map(|d| d.id.as_str()).collect();
    let mods: Vec<Value> = sub
        .mods
        .iter()
        .filter(|m| complete.contains(m.id.as_str()))
        .map(|m| {
            json!({
                "id": m.id, "name": m.name, "kind": m.kind, "version": m.version,
                "requires": m.requires,
                "author": {"owner": m.owner, "repo": m.repo},
                "license": m.license,
                "files": m.files.iter().map(|f| json!({
                    "name": f.name, "path": format!("{}/{}", m.id, f.name),
                    "sha256": f.sha256, "size": f.size,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let incomplete: Vec<&str> =
        done.iter().filter(|d| !d.complete()).map(|d| d.id.as_str()).collect();
    json!({
        "elo_mods": 1, "game": sub.game, "wallet": sub.wallet, "runs_code": sub.runs_code,
        "mods": mods, "incomplete": incomplete,
    })
}

/// Written whole or not at all: a game reading it mid-sync sees the old one.
fn write_index(dir: &Path, sub: &Subscribed, done: &[ModDone]) -> Result<()> {
    let body = serde_json::to_string_pretty(&index_json(sub, done))
        .map_err(|e| DepotError::new(e.to_string()))?
        + "\n";
    let mut tmp = tempfile::Builder::new().prefix(PART_PREFIX).tempfile_in(dir)?;
    tmp.write_all(body.as_bytes())?;
    tmp.persist(dir.join(INDEX_NAME))
        .map_err(|e| DepotError::new(format!("could not write {INDEX_NAME}: {}", e.error)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha256};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const OWNER: &str = "ash";
    const REPO: &str = "desert-run";

    fn sha(bytes: &[u8]) -> String {
        crate::digest::hex_lower(&Sha256::digest(bytes))
    }

    fn url(name: &str) -> String {
        format!("https://github.com/{OWNER}/{REPO}/releases/download/v1/{name}")
    }

    fn file(name: &str, bytes: &[u8]) -> Value {
        json!({"name": name, "url": url(name), "sha256": sha(bytes), "size": bytes.len()})
    }

    fn one(id: &str, files: Vec<Value>) -> Value {
        json!({"id": id, "name": id, "kind": "map", "version": "1.0.0", "files": files,
               "requires": [], "author": {"owner": OWNER, "repo": REPO,
               "url": format!("https://github.com/{OWNER}/{REPO}")}, "license": "MIT"})
    }

    fn doc(mods: Vec<Value>) -> Value {
        json!({"game": "gates", "wallet": "0x00000000000000000000000000000000000000aa",
               "open": true, "runs_code": false, "count": mods.len(), "mods": mods,
               "withheld": [{"mod": "old-run", "why": "taken down"}], "check": "…"})
    }

    /// A fetcher that serves `bytes` by url and counts what it was asked for.
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

    fn run(sub: &Subscribed, root: &Path, f: &Fake, dry: bool) -> Synced {
        sync(sub, root, f, dry, &mut |_, _| {}).unwrap()
    }

    #[test]
    fn a_file_name_never_leaves_the_mods_folder() {
        for good in ["map.pak", "Desert_Run-1.2.zip", "a+b.txt", "x"] {
            assert!(check_file_name(good).is_ok(), "{good}");
        }
        for bad in ["", "../x", "..", ".", "a/b", "a\\b", "/etc/passwd", ".hidden", "map.",
                    "con", "CON.pak", "nul.txt", "c:x", "é.pak", "a b", "a\0b",
                    &"x".repeat(101)] {
            assert!(check_file_name(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_url_is_the_authors_own_release_asset() {
        assert!(check_release_url(&url("map.pak"), OWNER, REPO).is_ok());
        assert!(check_release_url(&url("map.pak"), "ASH", "Desert-Run").is_ok(),
                "GitHub names compare without case");
        for bad in [
            "http://github.com/ash/desert-run/releases/download/v1/map.pak",
            "https://github.com.evil.tld/ash/desert-run/releases/download/v1/map.pak",
            "https://github.com@evil.tld/ash/desert-run/releases/download/v1/map.pak",
            "https://github.com/bo/desert-run/releases/download/v1/map.pak",
            "https://github.com/ash/other/releases/download/v1/map.pak",
            "https://github.com/ash/desert-run/raw/main/map.pak",
            "https://github.com/ash/desert-run/releases/download/v1/x/map.pak",
            "https://github.com/ash/desert-run/releases/download/v1/map.pak?x=1",
            "https://github.com/ash/desert-run/releases/download/../map.pak",
            "https://github.com/ash/desert-run/releases/download/v1/",
            "https://objects.githubusercontent.com/map.pak",
            "",
        ] {
            assert!(check_release_url(bad, OWNER, REPO).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_bad_entry_is_set_aside_and_the_rest_parse() {
        let mut upper = file("map.pak", b"abc");
        upper["sha256"] = json!(sha(b"abc").to_ascii_uppercase());
        let mut huge = file("big.pak", b"x");
        huge["size"] = json!(FILE_SIZE_MAX + 1);
        let mut elsewhere = file("x.pak", b"x");
        elsewhere["url"] = json!("https://example.com/x.pak");
        let v = doc(vec![
            one("good", vec![upper]),
            one("escape", vec![file("../x", b"x")]),
            one("huge", vec![huge]),
            one("elsewhere", vec![elsewhere]),
            one("twice", vec![file("Map.pak", b"a"), file("map.pak", b"b")]),
            one("good", vec![file("map.pak", b"abc")]),
            one("con", vec![file("map.pak", b"abc")]),
        ]);
        let sub = parse_subscribed(&v).unwrap();
        assert_eq!(sub.mods.len(), 1);
        assert_eq!(sub.mods[0].files[0].sha256, sha(b"abc"), "a digest is kept lowercase");
        let refused: Vec<&str> = sub.refused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(refused, ["escape", "huge", "elsewhere", "twice", "good", "con"]);
        assert_eq!(sub.withheld, [("old-run".to_string(), "taken down".to_string())]);
        assert!(parse_subscribed(&json!({"game": "gates"})).is_err());
        assert!(parse_subscribed(&json!({"game": "../x", "mods": []})).is_err());
    }

    #[test]
    fn a_file_lands_checked_and_is_not_fetched_twice() {
        let root = tempfile::tempdir().unwrap();
        let sub = parse_subscribed(&doc(vec![one("desert-run", vec![
            file("map.pak", b"the map"), file("README.txt", b"read me")])])).unwrap();
        let f = fake(&[("map.pak", b"the map"), ("README.txt", b"read me")]);
        let got = run(&sub, root.path(), &f, false);
        assert_eq!(got.bad(), 0);
        assert!(got.mods[0].files.iter().all(|x| x.outcome == Outcome::Fetched));
        let map = root.path().join("gates/desert-run/map.pak");
        assert_eq!(std::fs::read(&map).unwrap(), b"the map");
        let index: Value =
            serde_json::from_str(&std::fs::read_to_string(&got.index).unwrap()).unwrap();
        assert_eq!(index["mods"][0]["id"], "desert-run");
        assert_eq!(index["mods"][0]["files"][0]["path"], "desert-run/map.pak");

        let again = run(&sub, root.path(), &f, false);
        assert!(again.mods[0].files.iter().all(|x| x.outcome == Outcome::Kept));
        assert_eq!(f.asked.load(Ordering::SeqCst), 2, "a file already right is not fetched");

        // A file changed on disk is fetched again, not trusted.
        std::fs::write(&map, b"the mop").unwrap();
        let third = run(&sub, root.path(), &f, false);
        assert_eq!(third.mods[0].files[0].outcome, Outcome::Fetched);
        assert_eq!(std::fs::read(&map).unwrap(), b"the map");
    }

    #[test]
    fn bytes_that_differ_are_deleted_and_never_kept() {
        let root = tempfile::tempdir().unwrap();
        let sub = parse_subscribed(&doc(vec![one("desert-run", vec![
            file("map.pak", b"the map")])])).unwrap();
        // Same length, other bytes; then a host that sends too little.
        for served in [&b"the mop"[..], &b"the"[..]] {
            let got = run(&sub, root.path(), &fake(&[("map.pak", served)]), false);
            assert!(matches!(got.mods[0].files[0].outcome, Outcome::Mismatch(_)),
                    "{:?}", got.mods[0].files[0].outcome);
            assert_eq!(got.bad(), 1);
            let mdir = root.path().join("gates/desert-run");
            let left: Vec<_> = std::fs::read_dir(&mdir).unwrap().flatten().collect();
            assert!(left.is_empty(), "nothing may be left, under any name: {left:?}");
            let index: Value =
                serde_json::from_str(&std::fs::read_to_string(&got.index).unwrap()).unwrap();
            assert_eq!(index["mods"], json!([]), "an unchecked mod is not offered to the game");
            assert_eq!(index["incomplete"], json!(["desert-run"]));
        }

        // A fetch that fails leaves nothing under the name either, even where
        // an older file sat.
        std::fs::write(root.path().join("gates/desert-run/map.pak"), b"old map").unwrap();
        let got = run(&sub, root.path(), &fake(&[]), false);
        assert!(matches!(got.mods[0].files[0].outcome, Outcome::Failed(_)));
        assert!(!root.path().join("gates/desert-run/map.pak").exists());
    }

    #[test]
    fn what_is_no_longer_subscribed_goes_and_nothing_else_does() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("gates");
        for d in ["old-run", "my notes", "refused-one", "desert-run"] {
            std::fs::create_dir_all(game.join(d)).unwrap();
        }
        std::fs::write(game.join("old-run/x.pak"), b"x").unwrap();
        std::fs::write(game.join("desert-run/old-version.pak"), b"x").unwrap();
        std::fs::write(game.join("desert-run/.part-abc"), b"x").unwrap();
        std::fs::write(game.join("notes.txt"), b"mine").unwrap();
        let mut sub = parse_subscribed(&doc(vec![one("desert-run", vec![
            file("map.pak", b"the map")])])).unwrap();
        sub.refused.push(("refused-one".into(), "bad url".into()));

        // A dry run says so and touches nothing.
        let f = fake(&[("map.pak", b"the map")]);
        let dry = run(&sub, root.path(), &f, true);
        assert_eq!(dry.removed, ["old-run"]);
        assert_eq!(dry.mods[0].files[0].outcome, Outcome::WouldFetch);
        assert!(game.join("old-run/x.pak").exists() && !game.join(INDEX_NAME).exists());
        assert_eq!(f.asked.load(Ordering::SeqCst), 0);

        let got = run(&sub, root.path(), &f, false);
        assert_eq!(got.removed, ["old-run"]);
        assert!(!game.join("old-run").exists());
        assert!(game.join("my notes").is_dir(), "a folder that is not a mod id stays");
        assert!(game.join("refused-one").is_dir(), "a subscribed mod we could not check stays");
        assert!(game.join("notes.txt").exists());
        let mut left: Vec<String> = std::fs::read_dir(game.join("desert-run")).unwrap()
            .flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, ["map.pak"], "an old version's file and a part file go");
    }

    /// A mod folder that is a link out of the game's folder is never written
    /// through, whatever the list says.
    #[cfg(unix)]
    #[test]
    fn a_linked_mod_folder_is_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("gates")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("gates/desert-run")).unwrap();
        let sub = parse_subscribed(&doc(vec![one("desert-run", vec![
            file("map.pak", b"the map")])])).unwrap();
        let got = run(&sub, root.path(), &fake(&[("map.pak", b"the map")]), false);
        assert!(matches!(got.mods[0].files[0].outcome, Outcome::Failed(_)));
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0,
                   "nothing landed outside the mods folder");
    }
}
