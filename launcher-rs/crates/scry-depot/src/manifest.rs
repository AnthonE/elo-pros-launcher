//! The manifest — what a title is and which builds exist.
//!
//! Stable; it changes when a game changes shape, not when it ships a build.
//! The launcher knows nothing about any game beyond this document and the
//! depot a build row points at.

use crate::error::{DepotError, Result};
use serde_json::Value;

pub const MANIFEST_VERSION: i64 = 1;

/// Builds a manifest may declare. `browser` needs no depot; every town room
/// uses it. `native` is the one that installs.
pub const BUILD_KINDS: [&str; 2] = ["browser", "native"];
pub const PLATFORMS: [&str; 6] = [
    "any",
    "linux-x86_64",
    "linux-aarch64",
    "win-x86_64",
    "mac-arm64",
    "mac-x86_64",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub kind: String,
    pub platform: String,
    /// browser: where it opens.
    pub url: Option<String>,
    /// native: where the depot index lives.
    pub depot_url: Option<String>,
    pub version: Option<String>,
    pub bytes: Option<i64>,
    pub notes: String,
}

impl Build {
    /// Honest, and the launcher never guesses past it: a build row with no url
    /// and no depot is a **slot**, not a build. It renders as "not published"
    /// rather than as a broken download, and that is exactly Gates' shape
    /// today — both rows deliberately empty.
    pub fn published(&self) -> bool {
        if self.kind == "browser" {
            self.url.as_deref().is_some_and(|u| !u.is_empty())
        } else {
            self.depot_url.as_deref().is_some_and(|u| !u.is_empty())
        }
    }
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub slug: String,
    pub name: String,
    pub version: i64,
    pub publisher: String,
    pub state: String,
    pub blurb: String,
    pub repo: String,
    pub source_required: bool,
    pub builds: Vec<Build>,
    pub servers_url: Option<String>,
    pub notary: Option<Value>,
    pub raw: Value,
}

impl Manifest {
    /// Best build for this machine. `any` matches everything, an exact
    /// platform beats it, and an unpublished slot never beats a published
    /// build of the other kind.
    pub fn build_for(&self, platform: &str, kind: Option<&str>) -> Option<&Build> {
        let mut cands: Vec<&Build> = self
            .builds
            .iter()
            .filter(|b| kind.is_none_or(|k| b.kind == k))
            .filter(|b| b.platform == platform || b.platform == "any")
            .collect();
        cands.sort_by_key(|b| {
            (
                !b.published(),
                b.platform == "any",
                b.kind != "native",
            )
        });
        cands.first().copied()
    }

    pub fn playable_now(&self) -> bool {
        self.builds.iter().any(|b| b.published())
    }

    /// Resolve path-absolute urls against the origin this manifest came from.
    ///
    /// An origin serving a manifest writes `"depot": "/api/launcher/depot/…"`
    /// rather than a full url, and it does that for a reason worth stating:
    /// **it does not reliably know its own public name.** Behind nginx and a
    /// CDN the only candidate is the request's `Host` header, which is
    /// attacker-controlled on any request an attacker makes — so an origin
    /// that built absolute download urls out of it would be handing every
    /// caller a url the caller supplied. The client knows where it asked;
    /// the server does not. So the client resolves.
    ///
    /// The rules, and each refusal is the interesting half:
    ///
    /// * `https://…` — left exactly alone. A title may host its depot
    ///   anywhere, and most will.
    /// * `/api/…` — rebased onto `base`. This is the same-origin case.
    /// * `//host/…` — **refused**, blanked rather than rebased. It is
    ///   protocol-relative, so prefixing a scheme would silently point the
    ///   download at `host` while looking local in the manifest. That is a
    ///   redirect wearing a relative path, and the row becomes a slot instead.
    /// * anything else relative (`x/y`, `../z`) — refused for the same
    ///   reason there is no path arithmetic anywhere else in this crate.
    ///
    /// A refused row is blanked, which makes it a SLOT — rendered as "no
    /// desktop build published" rather than as a broken download. Losing a
    /// build row is recoverable; fetching from the wrong host is not.
    pub fn rebase(&mut self, base: &str) {
        let base = base.trim_end_matches('/').to_string();
        let fix = |u: &mut Option<String>| {
            let Some(v) = u.as_deref() else { return };
            if v.starts_with("https://") || v.starts_with("http://") {
                return;
            }
            if v.starts_with("//") || !v.starts_with('/') {
                *u = None;
                return;
            }
            *u = Some(format!("{base}{v}"));
        };
        for b in self.builds.iter_mut() {
            fix(&mut b.url);
            fix(&mut b.depot_url);
        }
        fix(&mut self.servers_url);
    }
}

fn as_str(v: Option<&Value>) -> String {
    v.and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn opt_str(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn parse_manifest(raw: &Value) -> Result<Manifest> {
    let obj = raw
        .as_object()
        .ok_or_else(|| DepotError::new("manifest is not an object"))?;
    let ver = obj
        .get("manifest_version")
        .and_then(|v| v.as_i64())
        .unwrap_or(MANIFEST_VERSION);
    if ver != MANIFEST_VERSION {
        return Err(DepotError::new(format!(
            "manifest_version {ver} — this launcher speaks {MANIFEST_VERSION}"
        )));
    }
    let slug = as_str(obj.get("slug")).trim().to_string();
    if slug.is_empty() || !slug.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(DepotError::new(format!("bad slug: {slug:?}")));
    }
    let mut builds = Vec::new();
    for b in obj.get("builds").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
        let Some(b) = b.as_object() else { continue };
        let kind = as_str(b.get("kind"));
        if !BUILD_KINDS.contains(&kind.as_str()) {
            return Err(DepotError::new(format!(
                "unknown build kind {kind:?} in {slug}"
            )));
        }
        let platform = opt_str(b.get("platform")).unwrap_or_else(|| "any".into());
        if !PLATFORMS.contains(&platform.as_str()) {
            return Err(DepotError::new(format!(
                "unknown platform {platform:?} in {slug}"
            )));
        }
        builds.push(Build {
            kind,
            platform,
            url: opt_str(b.get("url")),
            depot_url: opt_str(b.get("depot")),
            version: opt_str(b.get("version")),
            bytes: b.get("bytes").and_then(|v| v.as_i64()),
            notes: as_str(b.get("notes")),
        });
    }
    let servers_url = obj
        .get("servers")
        .and_then(|v| v.as_object())
        .and_then(|s| s.get("url"))
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(Manifest {
        name: opt_str(obj.get("name")).unwrap_or_else(|| slug.clone()),
        slug,
        version: ver,
        publisher: as_str(obj.get("publisher")),
        state: opt_str(obj.get("state")).unwrap_or_else(|| "in-build".into()),
        blurb: as_str(obj.get("blurb")),
        repo: as_str(obj.get("repo")),
        source_required: obj
            .get("source_required")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        builds,
        servers_url,
        notary: obj.get("notary").filter(|v| v.is_object()).cloned(),
        raw: raw.clone(),
    })
}
