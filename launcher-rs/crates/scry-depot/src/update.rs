//! The updater — *"a game updater like steam has with builds"*.
//!
//! Operator, 2026-08-05. What Steam's updater actually does, minus the parts
//! that need a proprietary CDN:
//!
//! * notice a published build is newer than the installed one,
//! * download only what changed,
//! * keep the old build until the new one is whole,
//! * and never claim to be current when it could not look.
//!
//! The last is the one this repo has to be careful about, because it has a
//! name here: **a never-raises reader returns `[]` for both "nothing there"
//! and "we could not look"** (`CLAUDE.md` §traps). An updater that renders a
//! failed fetch as *"up to date"* is that trap with a patch schedule attached,
//! so [`UpdateState`] has a separate [`UpdateState::Unknown`] arm and nothing
//! collapses it into [`UpdateState::Current`].
//!
//! What makes an update cheap is not a delta format — it is that every depot
//! file is content-addressed, so an unchanged file is *already on disk* in the
//! prior build. See [`crate::install::plan`].

use crate::depot::{parse_depot, Depot};
use crate::install::{installed, plan, Install, Plan};
use crate::manifest::Manifest;
use serde_json::Value;
use std::path::Path;

/// Where a manifest or depot document comes from. `Err` is a *reason*, and it
/// is carried all the way to the surface rather than flattened to a bool.
pub trait JsonSource {
    fn get(&self, url: &str) -> Result<Value, String>;
}

impl<F> JsonSource for F
where
    F: Fn(&str) -> Result<Value, String>,
{
    fn get(&self, url: &str) -> Result<Value, String> {
        self(url)
    }
}

#[derive(Debug, Clone)]
pub enum UpdateState {
    /// Nothing of this title is installed for this platform.
    NotInstalled { published: bool },
    /// The installed build's digest is the published one.
    Current { build: String, digest: String },
    /// A different build is published. Carries the plan, so the caller can say
    /// what it will cost before anyone commits to it.
    Stale {
        from_build: String,
        to_build: String,
        from_digest: String,
        to_digest: String,
        plan: Plan,
    },
    /// The manifest has a build row for this platform with no depot behind it
    /// — a slot, not a build. Gates' shape today.
    Unpublished,
    /// **We could not look.** Never rendered as up to date.
    Unknown { why: String },
}

impl UpdateState {
    pub fn line(&self) -> String {
        match self {
            UpdateState::NotInstalled { published: true } => "not installed".into(),
            UpdateState::NotInstalled { published: false } => {
                "not installed, and no desktop build is published yet".into()
            }
            UpdateState::Current { build, .. } => format!("up to date (build {build})"),
            // Same build id, different digest: the title was REPACKAGED, not
            // rebuilt. Real and normal — a publisher can change a launch arg
            // or a metadata field without touching a byte of the binary — and
            // rendering it as "update available: 0.1.0 → 0.1.0" reads as a bug
            // in the launcher. Measured 2026-08-05 doing exactly that.
            UpdateState::Stale {
                from_build,
                to_build,
                plan,
                ..
            } if from_build == to_build => format!(
                "the published depot for build {to_build} changed — {}",
                plan.line()
            ),
            UpdateState::Stale {
                from_build,
                to_build,
                plan,
                ..
            } => format!("update available: {from_build} → {to_build} — {}", plan.line()),
            UpdateState::Unpublished => "no desktop build published yet".into(),
            UpdateState::Unknown { why } => format!("could not check for updates — {why}"),
        }
    }

    pub fn needs_update(&self) -> bool {
        matches!(self, UpdateState::Stale { .. })
    }
}

/// The depot a [`UpdateState::Stale`] or fresh install would fetch, resolved
/// alongside the state so the caller does not have to fetch it twice.
pub struct Checked {
    pub state: UpdateState,
    pub depot: Option<Depot>,
}

/// Every installed build of one title, newest-installed last.
pub fn builds_of(games_root: &Path, slug: &str) -> Vec<Install> {
    installed(games_root)
        .into_iter()
        .filter(|i| i.slug == slug)
        .collect()
}

/// Is a published build newer than what is on disk?
///
/// `platform` is this machine's tag. Only `native` rows are considered — a
/// browser row has nothing to install and nothing to update.
pub fn check(
    manifest: &Manifest,
    platform: &str,
    games_root: &Path,
    source: &dyn JsonSource,
    allow_insecure: bool,
) -> Checked {
    let mine = builds_of(games_root, &manifest.slug);
    let row = manifest.build_for(platform, Some("native"));
    let depot_url = match row.and_then(|r| r.depot_url.clone()) {
        Some(u) => u,
        None => {
            // No published native build. If something is installed anyway it
            // stays installed and playable; the house simply un-listed it.
            return Checked {
                state: if mine.is_empty() {
                    UpdateState::Unpublished
                } else {
                    UpdateState::Current {
                        build: mine.last().map(|i| i.build.clone()).unwrap_or_default(),
                        digest: mine.last().map(|i| i.depot_digest.clone()).unwrap_or_default(),
                    }
                },
                depot: None,
            };
        }
    };
    let raw = match source.get(&depot_url) {
        Ok(v) => v,
        Err(why) => {
            return Checked {
                state: UpdateState::Unknown { why },
                depot: None,
            }
        }
    };
    let depot = match parse_depot(&raw, allow_insecure) {
        Ok(d) => d,
        Err(e) => {
            return Checked {
                state: UpdateState::Unknown {
                    why: format!("the published depot did not parse: {e}"),
                },
                depot: None,
            }
        }
    };
    let want = match depot.digest() {
        Ok(d) => d,
        Err(e) => {
            return Checked {
                state: UpdateState::Unknown {
                    why: format!("the published depot has no computable digest: {e}"),
                },
                depot: None,
            }
        }
    };
    // A title is current if ANY installed build carries the published digest —
    // not merely the newest. Installing 0.2.0 beside 0.1.0 and then rolling
    // back should not read as "an update is available" forever.
    if let Some(hit) = mine.iter().find(|i| i.depot_digest == want) {
        return Checked {
            state: UpdateState::Current {
                build: hit.build.clone(),
                digest: want,
            },
            depot: Some(depot),
        };
    }
    let p = plan(&depot, games_root);
    let state = match mine.last() {
        None => UpdateState::NotInstalled { published: true },
        Some(cur) => UpdateState::Stale {
            from_build: cur.build.clone(),
            to_build: depot.build.clone(),
            from_digest: cur.depot_digest.clone(),
            to_digest: want,
            plan: p,
        },
    };
    Checked {
        state,
        depot: Some(depot),
    }
}

/// Builds of a title other than the one named — what an update leaves behind.
///
/// Removal is a separate, explicit call and never folded into `install`. An
/// update that deletes the old build before the new one is proven is how a
/// failed patch becomes an uninstalled game, which is the one outcome a player
/// never forgives.
pub fn superseded(games_root: &Path, slug: &str, keep_build: &str) -> Vec<Install> {
    builds_of(games_root, slug)
        .into_iter()
        .filter(|i| i.build != keep_build)
        .collect()
}

/// Compare two version strings the way this project writes them: numeric runs
/// compare as numbers, everything else as text, so `0.10.0` sorts above
/// `0.9.0` and `0.2.0-rc1` below `0.2.0`.
///
/// **Display-only, and that is why a second implementation of it is allowed.**
/// `meter/launcher.py` orders the artifacts on the shelf with the same rule to
/// decide which version IS published; this one decides whether to print a
/// nudge. A disagreement between them costs one wrong sentence in `scry check`
/// — no install, no hash, no money — which is a different class of thing from
/// the depot digest, where `CLAUDE.md` invariant 3 bites and the two ports are
/// pinned by shared vectors.
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn runs(s: &str) -> Vec<(i64, String)> {
        let mut out = Vec::new();
        let mut rest = s;
        while !rest.is_empty() {
            let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            if digits > 0 {
                // A run longer than an i64 is not a version, it is a mistake;
                // treating it as text keeps the comparison total either way.
                match rest[..digits].parse::<i64>() {
                    Ok(n) => out.push((n, String::new())),
                    Err(_) => out.push((-1, rest[..digits].to_string())),
                }
                rest = &rest[digits..];
                continue;
            }
            let text = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());
            out.push((-1, rest[..text].to_string()));
            rest = &rest[text..];
        }
        out
    }
    // Semver's prerelease rule, and it has to be explicit: `0.2.0-rc1` is
    // BELOW `0.2.0`, where a plain run-wise compare puts it above (it is the
    // same core plus more runs). Getting this backwards would tell everyone
    // running a release candidate that they are ahead of the shelf, which is
    // the opposite of the one thing this function exists to say.
    // A `fn`, not a closure: a closure returning references borrowed from its
    // argument gets a lifetime the inference cannot make outlive the call.
    fn split(s: &str) -> (&str, Option<&str>) {
        match s.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (s, None),
        }
    }
    let ((a_core, a_pre), (b_core, b_pre)) = (split(a), split(b));
    match runs(a_core).cmp(&runs(b_core)) {
        std::cmp::Ordering::Equal => match (a_pre, b_pre) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(x), Some(y)) => runs(x).cmp(&runs(y)),
        },
        other => other,
    }
}

#[cfg(test)]
mod version_tests {
    use super::version_cmp;
    use std::cmp::Ordering::*;

    #[test]
    fn numeric_runs_compare_as_numbers_not_text() {
        // The whole reason this is not a string compare.
        assert_eq!(version_cmp("0.10.0", "0.9.0"), Greater);
        assert_eq!(version_cmp("0.9.0", "0.10.0"), Less);
    }

    #[test]
    fn equal_is_equal() {
        assert_eq!(version_cmp("0.1.0", "0.1.0"), Equal);
    }

    #[test]
    fn a_prerelease_sorts_below_the_release_it_precedes() {
        assert_eq!(version_cmp("0.2.0-rc1", "0.2.0"), Less);
        assert_eq!(version_cmp("0.2.0", "0.2.0-rc1"), Greater);
    }

    #[test]
    fn the_ordering_is_total_even_for_things_that_are_not_versions() {
        // It must never panic: the string comes off a remote origin.
        let _ = version_cmp("", "0.1.0");
        let _ = version_cmp("nightly", "0.1.0");
        let _ = version_cmp("99999999999999999999999", "0.1.0");
        assert_eq!(version_cmp("nightly", "nightly"), Equal);
    }

    #[test]
    fn a_dev_build_ahead_of_the_shelf_reads_as_ahead() {
        // Not an update — the case that would otherwise nag every developer
        // running a local build.
        assert_eq!(version_cmp("0.1.0", "0.2.0"), Less);
    }
}

#[cfg(test)]
mod version_prerelease_tests {
    use super::version_cmp;
    use std::cmp::Ordering::*;

    #[test]
    fn two_prereleases_of_the_same_core_still_order() {
        assert_eq!(version_cmp("0.2.0-rc1", "0.2.0-rc2"), Less);
        assert_eq!(version_cmp("0.2.0-rc10", "0.2.0-rc2"), Greater);
    }

    #[test]
    fn a_prerelease_of_a_higher_core_still_beats_a_lower_release() {
        assert_eq!(version_cmp("0.3.0-rc1", "0.2.0"), Greater);
    }
}
