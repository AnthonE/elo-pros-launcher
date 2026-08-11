//! Install, verify, uninstall — and the local-reuse plan the updater rides on.
//!
//! Installing is: fetch every file, hash it, and **only then** move the whole
//! build into place from a staging directory. A killed or corrupted install
//! leaves nothing — no half-build that the library would count as real.

use crate::depot::Depot;
use crate::digest::sha256_file;
use crate::error::{DepotError, Result};
use crate::path::resolve_into;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const RECEIPT_NAME: &str = ".scry-install.json";

/// Where bytes come from. Supplied by the caller so this crate needs no
/// network stack and every rule in it is testable offline.
pub trait Fetcher {
    fn fetch(&self, url: &str, dest: &Path) -> Result<()>;
}

impl<F> Fetcher for F
where
    F: Fn(&str, &Path) -> Result<()>,
{
    fn fetch(&self, url: &str, dest: &Path) -> Result<()> {
        self(url, dest)
    }
}

/// Per-file progress. `(done_bytes, total_bytes, path, reused)`.
pub type Progress<'a> = &'a mut dyn FnMut(i64, i64, &str, bool);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub slug: String,
    pub build: String,
    pub platform: String,
    pub depot_digest: String,
    pub root: String,
    pub files: Vec<crate::depot::DepotFile>,
    pub launch_exec: String,
    pub launch_args: Vec<String>,
    pub launch_cwd: String,
    pub launch_env: BTreeMap<String, String>,
    pub total_bytes: i64,
    pub path: PathBuf,
}

pub fn build_dir(games_root: &Path, slug: &str, build: &str) -> Result<PathBuf> {
    // slug and build are path components too, and a depot names both.
    crate::path::safe_relpath(&format!("{slug}/{build}"))?;
    Ok(games_root.join(slug).join(build))
}

/// What an install would fetch and what it can take from disk.
///
/// **This is the delta step, and it needs no delta format.** Every file in a
/// depot is content-addressed by sha256, so a file that did not change between
/// two builds is *the same file* — and a prior install of the same title
/// already has it. Steam dedups at chunk level; file level gets most of that
/// win for a fraction of the machinery, because a game patch usually rewrites
/// a few large paks and leaves the rest untouched.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub reuse: BTreeMap<String, PathBuf>,
    pub fetch_bytes: i64,
    pub reuse_bytes: i64,
}

impl Plan {
    pub fn total_bytes(&self) -> i64 {
        self.fetch_bytes + self.reuse_bytes
    }
    pub fn line(&self) -> String {
        if self.reuse.is_empty() {
            return format!("{} to fetch", human(self.fetch_bytes));
        }
        format!(
            "{} to fetch, {} reused from builds already on disk ({} files)",
            human(self.fetch_bytes),
            human(self.reuse_bytes),
            self.reuse.len()
        )
    }
}

pub fn human(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut n = bytes as f64;
    let mut i = 0;
    while n >= 1024.0 && i < UNITS.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{n:.1} {}", UNITS[i])
    }
}

/// Build a reuse plan by scanning existing installs of the same slug.
///
/// A candidate is only accepted after its hash is re-read off disk. A receipt
/// says what a file *was*; the plan needs to know what it *is*, and the two
/// diverge exactly when reusing would be wrong.
pub fn plan(depot: &Depot, games_root: &Path) -> Plan {
    let mut have: BTreeMap<String, PathBuf> = BTreeMap::new();
    for other in installed(games_root) {
        if other.slug != depot.slug || other.build == depot.build {
            continue;
        }
        for f in &other.files {
            if have.contains_key(&f.sha256) {
                continue;
            }
            let candidate = other.path.join(&f.path);
            if candidate.is_file() {
                have.insert(f.sha256.clone(), candidate);
            }
        }
    }
    let mut out = Plan::default();
    for f in &depot.files {
        match have.get(&f.sha256) {
            Some(src) if sha256_file(src).map(|h| h == f.sha256).unwrap_or(false) => {
                out.reuse.insert(f.path.clone(), src.clone());
                out.reuse_bytes += f.bytes;
            }
            _ => out.fetch_bytes += f.bytes,
        }
    }
    out
}

/// Fetch (or reuse) every file, hash it, then move the whole build into place.
pub fn install(
    depot: &Depot,
    games_root: &Path,
    fetcher: &dyn Fetcher,
    reuse: Option<&Plan>,
    mut progress: Option<Progress<'_>>,
) -> Result<PathBuf> {
    let target = build_dir(games_root, &depot.slug, &depot.build)?;
    let parent = target
        .parent()
        .ok_or_else(|| DepotError::new("bad install root"))?;
    std::fs::create_dir_all(parent)?;
    let stage = tempfile::Builder::new()
        .prefix(&format!(".{}-{}-", depot.slug, depot.build))
        .tempdir_in(parent)?;

    let result = (|| -> Result<()> {
        let mut done: i64 = 0;
        for f in &depot.files {
            let dest = resolve_into(stage.path(), &f.path)?;
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            let reused = match reuse.and_then(|p| p.reuse.get(&f.path)) {
                Some(src) => {
                    std::fs::copy(src, &dest).map_err(|e| {
                        DepotError::new(format!("could not reuse {}: {e}", src.display()))
                    })?;
                    true
                }
                None => {
                    fetcher.fetch(&depot.file_url(&f.path), &dest)?;
                    false
                }
            };
            // Hashed the same way whether it came off the wire or off the
            // disk. A reused file is not trusted more than a fetched one —
            // the prior build's directory is writable by whoever runs this.
            let got = sha256_file(&dest)?;
            if got != f.sha256 {
                return Err(DepotError::new(format!(
                    "{}: hash mismatch — depot says {}…, the file is {}…. Nothing was installed.",
                    f.path,
                    &f.sha256[..16],
                    &got[..16]
                )));
            }
            set_executable(&dest, f.executable)?;
            done += f.bytes;
            if let Some(cb) = progress.as_deref_mut() {
                cb(done, depot.total_bytes, &f.path, reused);
            }
        }
        let receipt = receipt_json(depot)?;
        std::fs::write(
            stage.path().join(RECEIPT_NAME),
            serde_json::to_string_pretty(&receipt)
                .map_err(|e| DepotError::new(e.to_string()))?
                + "\n",
        )?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            let staged = stage.keep();
            if target.exists() {
                std::fs::remove_dir_all(&target)?;
            }
            std::fs::rename(&staged, &target)?;
            Ok(target)
        }
        Err(e) => {
            drop(stage); // removes the staging tree
            // Only succeeds if we created it and it is empty, so this never
            // touches another build of the same title.
            let _ = std::fs::remove_dir(parent);
            Err(e)
        }
    }
}

/// A file is executable iff the depot said so — never whatever the transfer's
/// umask or the source file's mode produced.
fn set_executable(path: &Path, executable: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        let mode = perms.mode();
        perms.set_mode(if executable {
            mode | 0o110
        } else {
            mode & !0o111
        });
        std::fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    let _ = (path, executable);
    Ok(())
}

fn receipt_json(depot: &Depot) -> Result<Value> {
    let files: Vec<Value> = depot
        .files
        .iter()
        .map(|f| {
            json!({
                "path": f.path, "sha256": f.sha256,
                "bytes": f.bytes, "executable": f.executable,
            })
        })
        .collect();
    Ok(json!({
        "slug": depot.slug,
        "build": depot.build,
        "platform": depot.platform,
        "depot_digest": depot.digest()?,
        "root": depot.root,
        "files": files,
        "launch": {
            "exec": depot.launch_exec,
            "args": depot.launch_args,
            "cwd": depot.launch_cwd,
            "env": depot.launch_env,
        },
        "total_bytes": depot.total_bytes,
        "notary": depot.notary,
    }))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    pub ok: bool,
    pub checked: usize,
    pub missing: Vec<String>,
    pub changed: Vec<String>,
    pub extra: Vec<String>,
    pub error: Option<String>,
}

impl Verdict {
    pub fn line(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.ok {
            return format!("{} files, every hash matches", self.checked);
        }
        let mut bits = Vec::new();
        if !self.missing.is_empty() {
            bits.push(format!("{} missing", self.missing.len()));
        }
        if !self.changed.is_empty() {
            bits.push(format!("{} changed", self.changed.len()));
        }
        if !self.extra.is_empty() {
            bits.push(format!("{} not in the depot", self.extra.len()));
        }
        if bits.is_empty() {
            "failed".into()
        } else {
            bits.join(", ")
        }
    }
}

/// Re-hash an install against its own receipt — Steam's *verify integrity of
/// game files*. A build that passes here is byte-identical to the one whose
/// digest was notarized.
pub fn verify(path: &Path) -> Verdict {
    let receipt = match read_receipt(path) {
        Ok(r) => r,
        Err(e) => {
            return Verdict {
                ok: false,
                error: Some(e.0),
                ..Default::default()
            }
        }
    };
    let mut declared = std::collections::HashSet::new();
    let (mut missing, mut changed) = (Vec::new(), Vec::new());
    for f in receipt.get("files").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
        let rel = f.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let want = f.get("sha256").and_then(|v| v.as_str()).unwrap_or("");
        declared.insert(rel.to_string());
        let target = path.join(rel);
        if !target.is_file() {
            missing.push(rel.to_string());
            continue;
        }
        if sha256_file(&target).map(|h| h != want).unwrap_or(true) {
            changed.push(rel.to_string());
        }
    }
    let mut extra = Vec::new();
    walk_files(path, path, &mut |rel| {
        if rel != RECEIPT_NAME && !declared.contains(rel) {
            extra.push(rel.to_string());
        }
    });
    extra.sort();
    Verdict {
        ok: missing.is_empty() && changed.is_empty(),
        checked: declared.len(),
        missing,
        changed,
        extra,
        error: None,
    }
}

fn walk_files(root: &Path, dir: &Path, f: &mut dyn FnMut(&str)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_files(root, &p, f);
        } else if p.is_file() {
            if let Ok(rel) = p.strip_prefix(root) {
                f(&rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn read_receipt(path: &Path) -> Result<Value> {
    let raw = std::fs::read_to_string(path.join(RECEIPT_NAME)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            DepotError::new("no install receipt — not installed by this launcher")
        } else {
            DepotError::new(format!("unreadable receipt: {e}"))
        }
    })?;
    serde_json::from_str(&raw).map_err(|e| DepotError::new(format!("unreadable receipt: {e}")))
}

/// Every complete install under the games root.
///
/// Reads receipts only. **A directory with no receipt is not an install** — it
/// is someone else's folder, and the launcher does not claim it, count it, or
/// delete it.
pub fn installed(games_root: &Path) -> Vec<Install> {
    let mut out = Vec::new();
    let Ok(slugs) = std::fs::read_dir(games_root) else {
        return out;
    };
    let mut slug_dirs: Vec<PathBuf> = slugs.flatten().map(|e| e.path()).collect();
    slug_dirs.sort();
    for slug_dir in slug_dirs.iter().filter(|p| p.is_dir()) {
        let Ok(builds) = std::fs::read_dir(slug_dir) else {
            continue;
        };
        let mut build_dirs: Vec<PathBuf> = builds.flatten().map(|e| e.path()).collect();
        build_dirs.sort();
        for build in build_dirs.iter().filter(|p| p.is_dir()) {
            let Ok(receipt) = read_receipt(build) else {
                continue;
            };
            if let Some(rec) = parse_install(&receipt, build) {
                out.push(rec);
            }
        }
    }
    out
}

fn parse_install(receipt: &Value, path: &Path) -> Option<Install> {
    let files = receipt
        .get("files")?
        .as_array()?
        .iter()
        .map(|f| crate::depot::DepotFile {
            path: f.get("path").and_then(|v| v.as_str()).unwrap_or("").into(),
            sha256: f.get("sha256").and_then(|v| v.as_str()).unwrap_or("").into(),
            bytes: f.get("bytes").and_then(|v| v.as_i64()).unwrap_or(0),
            executable: f.get("executable").and_then(|v| v.as_bool()).unwrap_or(false),
        })
        .collect();
    let launch = receipt.get("launch").cloned().unwrap_or(json!({}));
    let mut env = BTreeMap::new();
    if let Some(map) = launch.get("env").and_then(|v| v.as_object()) {
        for (k, v) in map {
            env.insert(k.clone(), v.as_str().unwrap_or("").to_string());
        }
    }
    Some(Install {
        slug: receipt.get("slug")?.as_str()?.to_string(),
        build: receipt.get("build")?.as_str()?.to_string(),
        platform: receipt
            .get("platform")
            .and_then(|v| v.as_str())
            .unwrap_or("any")
            .into(),
        depot_digest: receipt
            .get("depot_digest")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        root: receipt.get("root").and_then(|v| v.as_str()).unwrap_or("").into(),
        files,
        launch_exec: launch.get("exec").and_then(|v| v.as_str()).unwrap_or("").into(),
        launch_args: launch
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).map(str::to_string).collect())
            .unwrap_or_default(),
        launch_cwd: launch
            .get("cwd")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(".")
            .into(),
        launch_env: env,
        total_bytes: receipt
            .get("total_bytes")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        path: path.to_path_buf(),
    })
}

/// Remove one build. Refuses anything without a receipt, so a mistyped install
/// root cannot turn this into `rm -rf` on a home directory.
pub fn uninstall(path: &Path) -> Result<()> {
    if !path.join(RECEIPT_NAME).is_file() {
        return Err(DepotError::new(format!(
            "{} has no install receipt — refusing to delete it",
            path.display()
        )));
    }
    std::fs::remove_dir_all(path)?;
    Ok(())
}
