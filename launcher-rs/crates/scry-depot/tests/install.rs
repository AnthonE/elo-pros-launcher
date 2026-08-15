//! Install, verify, uninstall, and the updater.
//!
//! The install rules are ported from `test_launcher.py` §"install / verify /
//! uninstall". The updater half is new — it is what the operator asked for on
//! 2026-08-05 (*"a game updater like steam has with builds"*) and had no
//! Python equivalent to port.

use scry_depot::install::{human, Plan};
use scry_depot::update::{JsonSource, UpdateState};
use scry_depot::{
    install, installed, launch, parse_depot, parse_manifest, plan, uninstall, verify, Depot,
    DepotError, RECEIPT_NAME,
};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn sha_hex(body: &[u8]) -> String {
    let d = Sha256::digest(body);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// A depot over an in-memory file set, so every rule here is testable offline.
fn depot_for(build: &str, files: &[FileRow]) -> Depot {
    let entries: Vec<Value> = files
        .iter()
        .map(|(path, body, exec)| {
            json!({"path": path, "sha256": sha_hex(body), "bytes": body.len(), "executable": exec})
        })
        .collect();
    let raw = json!({
        "depot_version": 1, "slug": "t", "build": build,
        "platform": "linux-x86_64", "root": "https://example.invalid/d",
        "files": entries,
        "launch": {"exec": files[0].0, "args": [], "cwd": ".", "env": {}}
    });
    parse_depot(&raw, false).expect("fixture depot parses")
}

/// A fetcher serving a fixed file set, optionally corrupting one path.
fn serving(
    files: Vec<Served>,
    corrupt: Option<&str>,
) -> impl Fn(&str, &Path) -> Result<(), DepotError> {
    let corrupt = corrupt.map(str::to_string);
    move |url: &str, dest: &Path| {
        let name = url.rsplit('/').next().unwrap_or("");
        let hit = files
            .iter()
            .find(|(p, _)| p.rsplit('/').next().unwrap_or("") == name || p == name);
        match hit {
            Some((p, body)) => {
                let out = if corrupt.as_deref() == Some(p.as_str()) {
                    b"corrupted".to_vec()
                } else {
                    body.clone()
                };
                std::fs::write(dest, out).map_err(DepotError::from)
            }
            None => Err(DepotError::new(format!("no such file: {url}"))),
        }
    }
}

/// `(path, bytes, executable)` — one row of a depot's file list.
type FileRow = (&'static str, &'static [u8], bool);
/// `(path, bytes)` — what the fake fetcher will hand back for that path.
type Served = (String, Vec<u8>);

fn fixture() -> (Vec<FileRow>, Vec<Served>) {
    let files: Vec<FileRow> = vec![
        ("run", b"#!/bin/sh\necho hi\n" as &[u8], true),
        ("data/x.pak", b"PAK-CONTENT-0001", false),
        ("lib/libt.so", b"ELF-ish", false),
    ];
    let served = files
        .iter()
        .map(|(p, b, _)| (p.to_string(), b.to_vec()))
        .collect();
    (files, served)
}

#[test]
fn a_clean_install_lands_and_writes_a_receipt() {
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    let d = depot_for("1", &files);
    let at = install(&d, tmp.path(), &serving(served, None), None, None).unwrap();

    assert_eq!(at, tmp.path().join("t").join("1"));
    for (p, _, _) in &files {
        assert!(at.join(p).is_file(), "{p} should exist");
    }
    assert!(at.join(RECEIPT_NAME).is_file());
    let rows = installed(tmp.path());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].slug, "t");
    assert_eq!(rows[0].depot_digest, d.digest().unwrap());
    assert!(verify(&at).ok, "{}", verify(&at).line());
}

#[test]
fn a_file_is_executable_iff_the_depot_said_so() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let (files, served) = fixture();
        let at = install(
            &depot_for("1", &files),
            tmp.path(),
            &serving(served, None),
            None,
            None,
        )
        .unwrap();
        let mode = |p: &str| {
            std::fs::metadata(at.join(p)).unwrap().permissions().mode() & 0o111
        };
        assert_ne!(mode("run"), 0, "the depot said executable");
        assert_eq!(mode("data/x.pak"), 0, "the depot did not, whatever the umask gave");
    }
}

#[test]
fn a_hash_mismatch_aborts_and_leaves_nothing_behind() {
    // A failed install must be indistinguishable from no install — otherwise
    // the library counts a half-build as real.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    let err = install(
        &depot_for("1", &files),
        tmp.path(),
        &serving(served, Some("data/x.pak")),
        None,
        None,
    )
    .unwrap_err();
    assert!(err.0.contains("hash mismatch"), "{}", err.0);
    assert!(err.0.contains("Nothing was installed"), "{}", err.0);
    assert!(installed(tmp.path()).is_empty());
    let leftovers: Vec<PathBuf> = std::fs::read_dir(tmp.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

#[test]
fn verify_notices_missing_changed_and_extra_files() {
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    let at = install(
        &depot_for("1", &files),
        tmp.path(),
        &serving(served, None),
        None,
        None,
    )
    .unwrap();

    std::fs::write(at.join("data/x.pak"), b"tampered").unwrap();
    std::fs::remove_file(at.join("lib/libt.so")).unwrap();
    std::fs::write(at.join("stowaway.dll"), b"not in the depot").unwrap();

    let v = verify(&at);
    assert!(!v.ok);
    assert_eq!(v.changed, ["data/x.pak"]);
    assert_eq!(v.missing, ["lib/libt.so"]);
    assert_eq!(v.extra, ["stowaway.dll"]);
    assert!(v.line().contains("1 missing"), "{}", v.line());
}

#[test]
fn a_directory_without_a_receipt_is_not_an_install() {
    // It is someone else's folder, and the launcher does not claim it, count
    // it, or delete it — which is what stops a mistyped install root from
    // becoming `rm -rf` on a home directory.
    let tmp = tempfile::tempdir().unwrap();
    let stray = tmp.path().join("t").join("not-ours");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("precious.txt"), b"mine").unwrap();

    assert!(installed(tmp.path()).is_empty());
    let err = uninstall(&stray).unwrap_err();
    assert!(err.0.contains("no install receipt"), "{}", err.0);
    assert!(stray.join("precious.txt").is_file(), "must not be deleted");
}

#[test]
fn uninstall_removes_a_real_build() {
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    let at = install(
        &depot_for("1", &files),
        tmp.path(),
        &serving(served, None),
        None,
        None,
    )
    .unwrap();
    uninstall(&at).unwrap();
    assert!(!at.exists());
    assert!(installed(tmp.path()).is_empty());
}

// ── the updater ─────────────────────────────────────────────────────────────

#[test]
fn an_update_reuses_unchanged_files_instead_of_refetching_them() {
    // This is the delta step, and it needs no delta format: every depot file
    // is content-addressed, so a file that did not change between builds is
    // already on disk in the prior build.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(
        &depot_for("1", &files),
        tmp.path(),
        &serving(served, None),
        None,
        None,
    )
    .unwrap();

    // Build 2 rewrites one file and leaves the other two byte-identical.
    let v2: Vec<FileRow> = vec![
        ("run", b"#!/bin/sh\necho hi\n" as &[u8], true),
        ("data/x.pak", b"PAK-CONTENT-0002-BIGGER", false),
        ("lib/libt.so", b"ELF-ish", false),
    ];
    let d2 = depot_for("2", &v2);
    let p = plan(&d2, tmp.path());
    assert_eq!(p.reuse.len(), 2, "run and libt.so are unchanged");
    assert!(p.reuse.contains_key("run"));
    assert!(p.reuse.contains_key("lib/libt.so"));
    assert_eq!(p.fetch_bytes, 23, "only the changed pak is fetched");
    assert!(p.line().contains("reused"), "{}", p.line());

    // The fetcher below serves ONLY the changed file. If the plan were wrong
    // the install would fail, which is a stronger assertion than counting.
    let only_changed = vec![("data/x.pak".to_string(), b"PAK-CONTENT-0002-BIGGER".to_vec())];
    let at2 = install(
        &d2,
        tmp.path(),
        &serving(only_changed, None),
        Some(&p),
        None,
    )
    .unwrap();
    assert!(verify(&at2).ok, "{}", verify(&at2).line());
    assert_eq!(installed(tmp.path()).len(), 2, "builds live side by side");
}

#[test]
fn a_reused_file_is_hashed_like_a_fetched_one() {
    // The prior build's directory is writable by whoever runs this, so a
    // reused file gets no more trust than one off the wire.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(
        &depot_for("1", &files),
        tmp.path(),
        &serving(served, None),
        None,
        None,
    )
    .unwrap();

    let d2 = depot_for("2", &files);
    let mut p = plan(&d2, tmp.path());
    // Tamper with the source AFTER planning — the plan hashed it, the install
    // hashes it again, and only the second check can catch this.
    let victim = p.reuse.get("lib/libt.so").cloned().unwrap();
    std::fs::write(&victim, b"swapped-in-payload").unwrap();
    p.reuse.retain(|k, _| k == "lib/libt.so");

    let err = install(&d2, tmp.path(), &serving(vec![], None), Some(&p), None).unwrap_err();
    assert!(
        err.0.contains("hash mismatch") || err.0.contains("no such file"),
        "{}",
        err.0
    );
}

fn manifest_pointing_at(depot_url: &str) -> scry_depot::Manifest {
    parse_manifest(&json!({
        "slug": "t", "name": "T",
        "builds": [{"kind": "native", "platform": "linux-x86_64", "depot": depot_url}]
    }))
    .unwrap()
}

struct Source(Result<Value, String>);
impl JsonSource for Source {
    fn get(&self, _url: &str) -> Result<Value, String> {
        self.0.clone()
    }
}

#[test]
fn the_updater_reports_current_stale_and_not_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    let d1 = depot_for("1", &files);
    let m = manifest_pointing_at("https://example.invalid/d/depot.json");

    // nothing installed yet
    let c = scry_depot::check(&m, "linux-x86_64", tmp.path(), &Source(Ok(d1.raw.clone())), false);
    assert!(matches!(c.state, UpdateState::NotInstalled { published: true }));

    install(&d1, tmp.path(), &serving(served, None), None, None).unwrap();

    // installed and current
    let c = scry_depot::check(&m, "linux-x86_64", tmp.path(), &Source(Ok(d1.raw.clone())), false);
    assert!(matches!(c.state, UpdateState::Current { .. }), "{}", c.state.line());
    assert!(!c.state.needs_update());

    // a newer build is published
    let v2: Vec<FileRow> = vec![
        ("run", b"#!/bin/sh\necho hi\n" as &[u8], true),
        ("data/x.pak", b"PAK-CONTENT-0002", false),
        ("lib/libt.so", b"ELF-ish", false),
    ];
    let d2 = depot_for("2", &v2);
    let c = scry_depot::check(&m, "linux-x86_64", tmp.path(), &Source(Ok(d2.raw.clone())), false);
    match &c.state {
        UpdateState::Stale { from_build, to_build, plan, .. } => {
            assert_eq!(from_build, "1");
            assert_eq!(to_build, "2");
            assert_eq!(plan.reuse.len(), 2);
        }
        other => panic!("expected stale, got {}", other.line()),
    }
    assert!(c.state.needs_update());

    // A REPACKAGE: same build id, different depot document. Real and normal —
    // a publisher can change a launch arg or a metadata field without touching
    // a byte of the binary. It is still stale (the digest is what decides),
    // but it must not render as "update available: 1 → 1", which reads as a
    // bug in the launcher rather than a fact about the title. Measured doing
    // exactly that on 2026-08-05.
    let repack = depot_for("1", &v2);
    let c = scry_depot::check(&m, "linux-x86_64", tmp.path(), &Source(Ok(repack.raw)), false);
    assert!(c.state.needs_update(), "a changed depot is still an update");
    let line = c.state.line();
    assert!(
        !line.contains("1 → 1"),
        "a repackage must not read as a version bump to itself: {line}"
    );
    assert!(
        line.contains("changed"),
        "...it should say the published depot changed: {line}"
    );
}

#[test]
fn a_failed_check_is_unknown_and_never_up_to_date() {
    // The repo's own trap: a never-raises reader returns `[]` for both
    // "nothing there" and "we could not look." An updater that renders a dead
    // network as "up to date" is that trap with a patch schedule attached.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(
        &depot_for("1", &files),
        tmp.path(),
        &serving(served, None),
        None,
        None,
    )
    .unwrap();
    let m = manifest_pointing_at("https://example.invalid/d/depot.json");

    let c = scry_depot::check(
        &m,
        "linux-x86_64",
        tmp.path(),
        &Source(Err("connection refused".into())),
        false,
    );
    match &c.state {
        UpdateState::Unknown { why } => assert!(why.contains("refused")),
        other => panic!("a dead network must not read as {}", other.line()),
    }
    assert!(!c.state.needs_update());
    assert!(c.state.line().contains("could not check"), "{}", c.state.line());

    // Garbage that parses as JSON but not as a depot is also unknown, not current.
    let c = scry_depot::check(
        &m,
        "linux-x86_64",
        tmp.path(),
        &Source(Ok(json!({"depot_version": 1, "files": []}))),
        false,
    );
    assert!(matches!(c.state, UpdateState::Unknown { .. }), "{}", c.state.line());
}

#[test]
fn a_slot_reads_as_unpublished_not_as_an_update() {
    let tmp = tempfile::tempdir().unwrap();
    let m = parse_manifest(&json!({
        "slug": "gates", "name": "Gates",
        "builds": [{"kind": "native", "platform": "linux-x86_64"}]
    }))
    .unwrap();
    let c = scry_depot::check(&m, "linux-x86_64", tmp.path(), &Source(Ok(json!({}))), false);
    assert!(matches!(c.state, UpdateState::Unpublished));
    assert!(c.state.line().contains("no desktop build published"), "{}", c.state.line());
}

#[test]
fn rolling_back_to_an_older_build_does_not_read_as_stale_forever() {
    // A title is current if ANY installed build carries the published digest.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    let d1 = depot_for("1", &files);
    let v2: Vec<FileRow> = vec![
        ("run", b"#!/bin/sh\necho hi\n" as &[u8], true),
        ("data/x.pak", b"PAK-CONTENT-0002", false),
        ("lib/libt.so", b"ELF-ish", false),
    ];
    let d2 = depot_for("2", &v2);
    install(&d1, tmp.path(), &serving(served.clone(), None), None, None).unwrap();
    let served2 = v2.iter().map(|(p, b, _)| (p.to_string(), b.to_vec())).collect();
    install(&d2, tmp.path(), &serving(served2, None), None, None).unwrap();

    // The house re-lists build 1. Build 2 is newer on disk but 1 is published.
    let m = manifest_pointing_at("https://example.invalid/d/depot.json");
    let c = scry_depot::check(&m, "linux-x86_64", tmp.path(), &Source(Ok(d1.raw.clone())), false);
    match &c.state {
        UpdateState::Current { build, .. } => assert_eq!(build, "1"),
        other => panic!("expected current, got {}", other.line()),
    }
}

#[test]
fn superseded_lists_the_old_builds_and_never_the_kept_one() {
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(&depot_for("1", &files), tmp.path(), &serving(served.clone(), None), None, None).unwrap();
    install(&depot_for("2", &files), tmp.path(), &serving(served, None), None, None).unwrap();
    let old = scry_depot::superseded(tmp.path(), "t", "2");
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].build, "1");
}

// ── launching ───────────────────────────────────────────────────────────────

#[test]
fn a_depot_may_not_redirect_the_dynamic_loader() {
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(&depot_for("1", &files), tmp.path(), &serving(served, None), None, None).unwrap();
    let mut i = installed(tmp.path()).remove(0);

    for bad in ["LD_PRELOAD", "LD_AUDIT", "PATH", "PYTHONPATH", "DYLD_INSERT_LIBRARIES"] {
        i.launch_env = BTreeMap::from([(bad.to_string(), "/tmp/evil".to_string())]);
        let err = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap_err();
        assert!(err.0.contains("may not set"), "{bad}: {}", err.0);
    }

    // LD_LIBRARY_PATH is allowed — games need it — and must resolve INSIDE
    // the build, or the depot picks which libc you run.
    i.launch_env = BTreeMap::from([("LD_LIBRARY_PATH".into(), "lib".into())]);
    let l = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap();
    assert!(l.env["LD_LIBRARY_PATH"].starts_with(i.path.to_str().unwrap()));

    i.launch_env = BTreeMap::from([("LD_LIBRARY_PATH".into(), "/usr/lib".into())]);
    assert!(launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).is_err());
}

#[test]
fn an_unknown_placeholder_is_refused_never_passed_through() {
    // A literal `{token}` reaching a game's argv is a bug that shows up as a
    // mystery once, in production, on someone else's machine.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(&depot_for("1", &files), tmp.path(), &serving(served, None), None, None).unwrap();
    let mut i = installed(tmp.path()).remove(0);

    i.launch_args = vec!["--token".into(), "{secret}".into()];
    let err = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap_err();
    assert!(err.0.contains("unknown placeholder"), "{}", err.0);
    // The message names what the launcher DOES fill — the player reading the
    // dialog cannot diagnose, so the error has to.
    assert!(err.0.contains("{server}") && err.0.contains("{wallet}"), "{}", err.0);

    // The one that actually happened — a build published with `{servers}`
    // meaning `{server}` — can no longer be refused by name: `{servers}` is a
    // real placeholder now, the title's shard-list url (`title_values`). What
    // survives of that incident is the dialog's name-the-fix hint, which fires
    // for any OTHER singular/plural near-miss, and that is what this asserts.
    i.launch_args = vec!["--id".into(), "{wallets}".into()];
    let err = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap_err();
    assert!(err.0.contains("probably {wallet}"), "{}", err.0);

    // `{servers}` unvalued fills as an empty string — not a literal
    // `{servers}` in a game's argv, and not a refusal: a title with no
    // published shard list still launches, and the SDK backstop is the door
    // that answers (`title_url`).
    i.launch_args = vec!["--servers".into(), "{servers}".into()];
    let l = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap();
    assert_eq!(&l.argv[1..], &["--servers", ""]);

    i.launch_args = vec!["--server".into(), "{server}".into(), "--id".into(), "{wallet}".into()];
    let vals = BTreeMap::from([
        ("server".to_string(), "eu-1:7777".to_string()),
        ("wallet".to_string(), "0xabc".to_string()),
    ]);
    let l = launch::resolve(&i, &vals, &BTreeMap::new()).unwrap();
    assert_eq!(&l.argv[1..], &["--server", "eu-1:7777", "--id", "0xabc"]);
}

#[test]
fn a_game_can_ask_where_its_shard_list_is() {
    // The hole this closes: Gates draws its own shard browser and had no way
    // to be told the url, so every launch that did not come through the
    // Servers window drew an empty list over a live shard. `{servers}` is the
    // manifest's `servers.url` — the same document the launcher lists.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(&depot_for("1", &files), tmp.path(), &serving(served, None), None, None).unwrap();
    let mut i = installed(tmp.path()).remove(0);
    i.launch_args = vec!["--servers".into(), "{servers}".into()];

    let vals = BTreeMap::from([(
        "servers".to_string(),
        "https://scry.example/api/launcher/servers/gates".to_string(),
    )]);
    let l = launch::resolve(&i, &vals, &BTreeMap::new()).unwrap();
    assert_eq!(
        &l.argv[1..],
        &["--servers", "https://scry.example/api/launcher/servers/gates"]
    );

    // A title that publishes no list fills EMPTY rather than refusing — the
    // same shape `{wallet}` already had, and what a game reads as absence.
    // Refusing here would make a title's optional field able to stop a launch.
    let l = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap();
    assert_eq!(&l.argv[1..], &["--servers", ""]);

    // ⚠ AND THE TWO ASSERTIONS ABOVE COEXISTED WITH A BROKEN LAUNCHER FOR
    // THREE DAYS. Both were green while the GUI's Play button passed an empty
    // map, because they pin what `resolve` does with a map and say nothing
    // about who builds one. `title_values` is the answer to the second
    // question, and `scry-gui.rs`'s own source scan is what holds the call
    // sites to it — a rule with no owner is a rule each new door re-derives.
    assert_eq!(
        launch::title_values(Some("https://scry.example/api/launcher/servers/gates")),
        BTreeMap::from([(
            "servers".to_string(),
            "https://scry.example/api/launcher/servers/gates".to_string()
        )]),
        "the title's shard list is the one value a launch gets for free"
    );
    assert!(
        launch::title_values(None).is_empty(),
        "a title that publishes no list contributes nothing — absent, never an empty string, \
         because `fill` is the only place that decides what absence renders as"
    );
}

#[test]
fn the_launcher_env_wins_over_the_depots() {
    // So a depot cannot shadow the broker socket with one of its own and stand
    // between the game and the player's signer.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(&depot_for("1", &files), tmp.path(), &serving(served, None), None, None).unwrap();
    let mut i = installed(tmp.path()).remove(0);
    i.launch_env = BTreeMap::from([("SCRY_LAUNCHER_SOCKET".into(), "/tmp/evil.sock".into())]);
    let ours = BTreeMap::from([("SCRY_LAUNCHER_SOCKET".to_string(), "/run/real.sock".to_string())]);
    let l = launch::resolve(&i, &BTreeMap::new(), &ours).unwrap();
    assert_eq!(l.env["SCRY_LAUNCHER_SOCKET"], "/run/real.sock");
}

#[test]
fn launching_re_checks_what_the_installer_already_checked() {
    // A receipt is a file on disk and a build directory can be edited after it
    // is written. Trusting our own past output is how a verified install
    // becomes an unverified launch.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(&depot_for("1", &files), tmp.path(), &serving(served, None), None, None).unwrap();
    let mut i = installed(tmp.path()).remove(0);

    i.launch_exec = "data/../run".into();
    assert!(launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).is_err());

    i.launch_exec = "lib/libt.so".into(); // declared, but not executable
    let err = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap_err();
    assert!(err.0.contains("not executable"), "{}", err.0);

    i.launch_exec = "run".into();
    std::fs::remove_file(i.path.join(RECEIPT_NAME)).unwrap();
    let err = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap_err();
    assert!(err.0.contains("no install receipt"), "{}", err.0);
}

#[test]
fn a_browser_row_opens_only_http_and_supports_app_mode() {
    for bad in ["file:///etc/passwd", "steam://run/440", "javascript:alert(1)", "play"] {
        assert!(scry_depot::browser_argv(bad, "").is_err(), "should refuse {bad}");
    }
    assert_eq!(
        scry_depot::browser_argv("https://x/play", "").unwrap(),
        ["xdg-open", "https://x/play"]
    );
    // `{url}` is what app mode needs — `--app=URL` must be ONE argument, which
    // appending the url never produces.
    assert_eq!(
        scry_depot::browser_argv("https://x/play", "chromium --app={url}").unwrap(),
        ["chromium", "--app=https://x/play"]
    );
    assert_eq!(
        scry_depot::browser_argv("https://x/play", "firefox --kiosk").unwrap(),
        ["firefox", "--kiosk", "https://x/play"]
    );
}

#[test]
fn human_bytes_reads_like_a_download() {
    assert_eq!(human(0), "0 B");
    assert_eq!(human(999), "999 B");
    assert_eq!(human(1024), "1.0 KB");
    assert_eq!(human(1536 * 1024 * 1024), "1.5 GB");
}

#[test]
fn an_empty_plan_fetches_everything() {
    let p = Plan::default();
    assert!(p.reuse.is_empty());
    assert_eq!(p.total_bytes(), 0);
    assert!(p.line().contains("to fetch"));
}

#[test]
fn the_newest_installed_build_is_the_one_that_plays() {
    // Caught by the CLI smoke test, not by a unit test: `play` used the FIRST
    // install of a title, and builds sort by name, so a player who had just
    // updated started the build they updated away from — a patch that reads as
    // having done nothing.
    let tmp = tempfile::tempdir().unwrap();
    let (files, served) = fixture();
    install(&depot_for("0.1.0", &files), tmp.path(), &serving(served.clone(), None), None, None).unwrap();
    install(&depot_for("0.2.0", &files), tmp.path(), &serving(served, None), None, None).unwrap();

    let newest = installed(tmp.path())
        .into_iter().rfind(|i| i.slug == "t")
        .expect("something is installed");
    assert_eq!(newest.build, "0.2.0");
}

#[test]
fn a_supervised_launch_measures_real_elapsed_time() {
    // The unit tests cover the arithmetic; this covers the claim underneath
    // it — that `spawn_supervised` actually waits, which is the only reason
    // any playtime number in this client is real rather than guessed.
    use scry_depot::session::{self, Ending, Session};

    let tmp = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let sleeper: &[u8] = b"#!/bin/sh\nsleep 1.2\n";
    let files: Vec<FileRow> = vec![("run", sleeper, true)];
    let served: Vec<Served> = vec![("run".into(), sleeper.to_vec())];
    install(&depot_for("1", &files), tmp.path(), &serving(served, None), None, None).unwrap();
    let i = installed(tmp.path()).remove(0);

    let l = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap();
    let began = std::time::Instant::now();
    let started_at = 1_000_000i64;
    let mut child = launch::spawn_supervised(&l).unwrap();
    session::append(state.path(), Session {
        slug: i.slug.clone(), build: i.build.clone(), started_at,
        ending: Ending::Running { pid: child.id() },
    })
    .unwrap();
    // While it runs, it reads as running — and the pid is genuinely alive.
    assert!(session::load(state.path())[0].is_running());
    assert!(session::is_alive(child.id()));

    let status = child.wait().unwrap();
    let elapsed = began.elapsed();
    assert!(elapsed.as_millis() >= 1000, "did not actually wait: {elapsed:?}");
    assert!(status.success());

    session::close(state.path(), &i.slug, &i.build,
                   started_at + elapsed.as_secs() as i64, status.code()).unwrap();
    let done = &session::load(state.path())[0];
    assert!(!done.is_running());
    assert_eq!(done.playtime(), Some(elapsed.as_secs() as i64));
}
