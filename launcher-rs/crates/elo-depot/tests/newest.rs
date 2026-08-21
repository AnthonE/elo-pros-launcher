//! Which build a title actually runs.
//!
//! The whole file is one defect, pinned. A build id is `<version>-g<short
//! sha>`; the sha is hex and carries no order, so sorting ids alphabetically
//! shuffles them into an arbitrary sequence that LOOKS sorted. Three call
//! sites took the last element of that sequence and called it the newest,
//! each with a comment explaining why the first would be wrong. The reasoning
//! was right and the premise was false.
//!
//! Measured 2026-08-15 on a player's machine: six builds of Gates installed,
//! a library row correctly reading *up to date*, Play launching a build four
//! days old, and the sign-in it was supposed to fix still failing. Every
//! surface agreed and every surface was wrong.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Write a build directory with a receipt, and stamp when it was installed.
fn put(root: &Path, slug: &str, build: &str, at: SystemTime) {
    let dir = root.join(slug).join(build);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(elo_depot::install::RECEIPT_NAME),
        serde_json::json!({
            "depot_version": 1,
            "slug": slug,
            "build": build,
            "platform": "linux-x86_64",
            "root": "https://example.invalid/x",
            "depot_digest": format!("0x{}", "0".repeat(64)),
            "files": [],
            "launch": {"exec": "run", "args": [], "cwd": ".", "env": {}},
        })
        .to_string(),
    )
    .unwrap();
    // The mtime is the signal, so it is set explicitly rather than inferred
    // from the order the test happened to create things in.
    let f = fs::File::open(&dir).unwrap();
    f.set_times(fs::FileTimes::new().set_modified(at)).unwrap();
}

/// **The exact ids from the machine this was found on.** `gfebc701e4` sorts
/// last because `f` beats `2`, and it is four days older than the one the
/// player had just installed.
#[test]
fn the_newest_build_is_the_one_installed_last_not_the_last_by_name() {
    let tmp = tempdir();
    let root = tmp.as_path();
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_786_000_000);
    // Deliberately out of both name order and creation order.
    put(root, "gates", "0.2.0-gfebc701e4", base);
    put(root, "gates", "0.2.0-gbed9e02d6", base + Duration::from_secs(3600));
    put(root, "gates", "0.2.0-gbe0220a0a", base - Duration::from_secs(86_400));
    put(root, "gates", "0.2.0-g22574a33b", base + Duration::from_secs(7200));

    let all = elo_depot::builds_of(root, "gates");
    assert_eq!(all.len(), 4, "all four are installed");
    assert_eq!(
        all.last().map(|i| i.build.as_str()),
        Some("0.2.0-gfebc701e4"),
        "name order really does end on the OLD build — if this ever fails, \
         the premise changed and the rest of this file is about nothing"
    );

    let newest = elo_depot::newest_of(root, "gates").expect("something is installed");
    assert_eq!(
        newest.build, "0.2.0-g22574a33b",
        "the build installed most recently, not the last one alphabetically"
    );
}

/// A title with one build is the ordinary case and must not be clever.
#[test]
fn one_installed_build_is_the_newest() {
    let tmp = tempdir();
    let root = tmp.as_path();
    put(root, "gates", "0.2.0-gabc123def", SystemTime::UNIX_EPOCH);
    assert_eq!(
        elo_depot::newest_of(root, "gates").unwrap().build,
        "0.2.0-gabc123def"
    );
}

#[test]
fn a_title_with_nothing_installed_has_no_newest() {
    let tmp = tempdir();
    assert!(elo_depot::newest_of(tmp.as_path(), "gates").is_none());
}

/// `superseded` is what the sweep deletes, so it must be everything the run
/// choice did not keep — otherwise an update either leaves the pile it was
/// meant to clear, or removes the build it was meant to run.
#[test]
fn everything_except_the_newest_is_superseded() {
    let tmp = tempdir();
    let root = tmp.as_path();
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_786_000_000);
    put(root, "gates", "0.2.0-gfebc701e4", base);
    put(root, "gates", "0.2.0-g22574a33b", base + Duration::from_secs(60));

    let newest = elo_depot::newest_of(root, "gates").unwrap();
    let old = elo_depot::superseded(root, "gates", &newest.build);
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].build, "0.2.0-gfebc701e4");
    assert!(
        !old.iter().any(|i| i.build == newest.build),
        "the build we are going to run must never be in the delete list"
    );
}

/// Minimal temp dir — this crate takes no dev-dependency for four lines.
struct Tmp(std::path::PathBuf);
impl Tmp {
    fn as_path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn tempdir() -> Tmp {
    let n = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("elo-newest-{n}"));
    fs::create_dir_all(&p).unwrap();
    Tmp(p)
}
