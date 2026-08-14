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

#[derive(Debug, Clone)]
pub struct Depot {
    pub slug: String,
    pub build: String,
    pub platform: String,
    pub root: String,
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
