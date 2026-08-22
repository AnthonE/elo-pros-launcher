//! The depot — one build: a flat list of files with sha256s, plus the single
//! command that starts it. It changes every build.

use crate::digest::digest;
use crate::error::{DepotError, Result};
use crate::path::safe_relpath;
use serde_json::Value;
use std::collections::BTreeMap;

pub const DEPOT_VERSION: i64 = 1;
/// A sanity bound, not a policy.
pub const MAX_FILE_BYTES: i64 = 8 * 1024 * 1024 * 1024;
pub const MAX_FILES: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotFile {
    pub path: String,
    pub sha256: String,
    pub bytes: i64,
    pub executable: bool,
}

/// Hostnames this platform served from and has since retired.
///
/// **This is not a naming rule and not a label to be tidied — it is a list of
/// dead addresses.** A depot bakes its `root` at package time and that root is
/// inside the notarized digest, so a build packaged before a domain move keeps
/// naming the old host forever: the document is current, the digest is right,
/// and every byte it points at answers `410 Gone`. The move itself is recorded
/// in `deploy/nginx/scry.moreright.xyz-gone.conf`, which listed the launcher
/// binaries that would break and did not foresee this one — the depot
/// documents, which break a launcher that is *not* stale.
///
/// Same category as `scry play` in `elo-broker` and the `x-scry-grant` header:
/// a reader kept for a rename where the two sides move on different days. It
/// goes when nothing published names one of these.
pub const RETIRED_ORIGINS: &[&str] = &["scry.moreright.xyz"];

/// `scheme://host[:port]` of a url. Everything from the third `/` is dropped,
/// which is all a base needs to be.
pub fn origin_of(url: &str) -> String {
    let Some(rest) = url.find("://").map(|i| i + 3) else {
        return url.trim_end_matches('/').to_string();
    };
    match url[rest..].find('/') {
        Some(i) => url[..rest + i].to_string(),
        None => url.trim_end_matches('/').to_string(),
    }
}

/// Is this url's host one the platform has retired?
fn on_retired_origin(url: &str) -> bool {
    let origin = origin_of(url);
    let host = origin
        .split_once("://")
        .map(|(_, h)| h)
        .unwrap_or(&origin)
        .split(':')
        .next()
        .unwrap_or("");
    RETIRED_ORIGINS
        .iter()
        .any(|r| host.eq_ignore_ascii_case(r))
}

#[derive(Debug, Clone)]
pub struct Depot {
    pub slug: String,
    pub build: String,
    pub platform: String,
    pub root: String,
    /// The root this depot was packaged with, when [`Depot::heal_retired_root`]
    /// has moved downloads off a retired host. `None` is the normal state.
    ///
    /// Kept so the client can SAY it did this. A silent redirect to a host the
    /// document did not name is the kind of helpfulness that reads as a
    /// compromise the first time anyone looks at it closely.
    pub healed_from: Option<String>,
    pub files: Vec<DepotFile>,
    pub launch_exec: String,
    pub launch_args: Vec<String>,
    pub launch_cwd: String,
    pub launch_env: BTreeMap<String, String>,
    pub total_bytes: i64,
    pub notary: Option<Value>,
    pub raw: Value,
}

impl Depot {
    pub fn digest(&self) -> Result<String> {
        digest(&self.raw)
    }

    /// Where a file is fetched from. One join, in one place, so a depot root
    /// with or without a trailing slash cannot produce two different URLs.
    pub fn file_url(&self, rel: &str) -> String {
        format!("{}/{}", self.root.trim_end_matches('/'), rel)
    }

    /// Move downloads off a retired host onto the origin that served this
    /// document, keeping the path. Returns the root it replaced.
    ///
    /// **The digest does not move, and that is the whole point.** `raw` is
    /// untouched, so [`Depot::digest`] still hashes the document exactly as it
    /// was served and still resolves against what the notary committed. Only
    /// the LOCATOR changes — and a locator is not an integrity claim here:
    /// every file carries its own `sha256` inside that digest and `install`
    /// re-hashes each one, so bytes fetched from a different host are held to
    /// the same number as bytes fetched from the baked one. Nothing is trusted
    /// more for having come from the address the packager typed.
    ///
    /// Deliberately narrow. A depot whose root is simply a CDN — any host that
    /// is not on [`RETIRED_ORIGINS`] — is left alone, because naming a
    /// separate download host is a thing a publisher is allowed to do and this
    /// is not a policy about where bytes may live. It is one dead name, healed.
    pub fn heal_retired_root(&mut self, document_url: &str) -> Option<String> {
        if !on_retired_origin(&self.root) {
            return None;
        }
        let origin = origin_of(document_url);
        if origin.is_empty() || on_retired_origin(&origin) {
            // Nowhere better to point. Leave the root as packaged and let the
            // download fail loudly against the address the document names,
            // rather than inventing a host nobody served this from.
            return None;
        }
        let path = match self.root.find("://").map(|i| i + 3) {
            Some(rest) => match self.root[rest..].find('/') {
                Some(i) => self.root[rest + i..].to_string(),
                None => String::new(),
            },
            None => String::new(),
        };
        let was = std::mem::replace(&mut self.root, format!("{origin}{path}"));
        self.healed_from = Some(was.clone());
        Some(was)
    }
}

fn require_https(url: &str, allow_insecure: bool) -> Result<String> {
    let scheme = url.split_once("://").map(|(s, _)| s).unwrap_or("");
    if scheme == "https" || (allow_insecure && (scheme == "http" || scheme == "file")) {
        return Ok(url.to_string());
    }
    Err(DepotError::new(format!(
        "depot root must be https, got {:?}",
        if scheme.is_empty() { "nothing" } else { scheme }
    )))
}

pub fn parse_depot(raw: &Value, allow_insecure: bool) -> Result<Depot> {
    let obj = raw
        .as_object()
        .ok_or_else(|| DepotError::new("depot is not an object"))?;
    let ver = obj
        .get("depot_version")
        .and_then(|v| v.as_i64())
        .unwrap_or(DEPOT_VERSION);
    if ver != DEPOT_VERSION {
        return Err(DepotError::new(format!(
            "depot_version {ver} — this launcher speaks {DEPOT_VERSION}"
        )));
    }
    let root = require_https(
        obj.get("root").and_then(|v| v.as_str()).unwrap_or(""),
        allow_insecure,
    )?;
    let files_raw = obj
        .get("files")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| DepotError::new("depot lists no files"))?;
    if files_raw.len() > MAX_FILES {
        return Err(DepotError::new(format!(
            "depot lists {} files (cap {MAX_FILES})",
            files_raw.len()
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::with_capacity(files_raw.len());
    let mut total: i64 = 0;
    for f in files_raw {
        let f = f
            .as_object()
            .ok_or_else(|| DepotError::new("a depot file entry is not an object"))?;
        let rel = safe_relpath(f.get("path").and_then(|v| v.as_str()).unwrap_or(""))?;
        // Case-insensitive: two entries differing only in case overwrite each
        // other on a mac/win install and the second hash wins with no error.
        if !seen.insert(rel.to_lowercase()) {
            return Err(DepotError::new(format!("depot lists {rel:?} twice")));
        }
        let sha = f
            .get("sha256")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let sha = sha.strip_prefix("0x").unwrap_or(&sha).to_string();
        if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DepotError::new(format!("{rel}: sha256 is not 32 hex bytes")));
        }
        let size = f.get("bytes").and_then(|v| v.as_i64());
        let size = match size {
            Some(n) if (0..=MAX_FILE_BYTES).contains(&n) => n,
            other => {
                return Err(DepotError::new(format!(
                    "{rel}: bad size {}",
                    other.map(|n| n.to_string()).unwrap_or_else(|| "None".into())
                )))
            }
        };
        total += size;
        files.push(DepotFile {
            path: rel,
            sha256: sha,
            bytes: size,
            executable: f
                .get("executable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        });
    }
    let launch = obj
        .get("launch")
        .and_then(|v| v.as_object())
        .filter(|l| l.get("exec").and_then(|e| e.as_str()).is_some_and(|s| !s.is_empty()))
        .ok_or_else(|| DepotError::new("depot declares no launch command"))?;
    let exec_rel = safe_relpath(launch["exec"].as_str().unwrap_or(""))?;
    if !files.iter().any(|f| f.path == exec_rel) {
        return Err(DepotError::new(format!(
            "launch exec {exec_rel:?} is not one of the depot's files"
        )));
    }
    let mut args = Vec::new();
    if let Some(list) = launch.get("args") {
        let list = list
            .as_array()
            .ok_or_else(|| DepotError::new("launch args must be a list of strings"))?;
        for a in list {
            args.push(
                a.as_str()
                    .ok_or_else(|| DepotError::new("launch args must be a list of strings"))?
                    .to_string(),
            );
        }
    }
    // The launch command's placeholders are checked HERE, not only at launch:
    // a depot that writes `{servers}` for `{server}` must be refused while it
    // is still a download. Unchecked it installs, verifies clean, sits in the
    // library as "up to date" — and every Play of it fails.
    crate::launch::check_args(&args)?;
    let mut env = BTreeMap::new();
    if let Some(map) = launch.get("env") {
        let map = map
            .as_object()
            .ok_or_else(|| DepotError::new("launch env must be a string→string map"))?;
        for (k, v) in map {
            env.insert(
                k.clone(),
                v.as_str()
                    .ok_or_else(|| DepotError::new("launch env must be a string→string map"))?
                    .to_string(),
            );
        }
    }
    let cwd = launch
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(".")
        .to_string();
    if cwd != "." {
        safe_relpath(&cwd)?; // must also stay inside
    }
    Ok(Depot {
        slug: obj.get("slug").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        build: obj
            .get("build")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("0")
            .to_string(),
        platform: obj
            .get("platform")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("any")
            .to_string(),
        root,
        healed_from: None,
        files,
        launch_exec: exec_rel,
        launch_args: args,
        launch_cwd: cwd,
        launch_env: env,
        total_bytes: total,
        notary: obj.get("notary").filter(|v| v.is_object()).cloned(),
        raw: raw.clone(),
    })
}
