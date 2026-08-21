//! Path safety and parse rules — the checks that stand between a stranger's
//! JSON and your filesystem. Ported case-for-case from `test_launcher.py`
//! §"path safety" and §"the schema", because the rules are the asset and the
//! port is only as good as its ability to keep them.

use elo_depot::manifest::parse_manifest;
use elo_depot::path::{resolve_into, safe_relpath};
use elo_depot::{parse_depot, DEPOT_VERSION};
use serde_json::{json, Value};

fn depot_with(overrides: Value) -> Value {
    let mut base = json!({
        "depot_version": DEPOT_VERSION,
        "slug": "g", "build": "1", "root": "https://x/",
        "files": [{"path": "run", "sha256": "0".repeat(64), "bytes": 1, "executable": true}],
        "launch": {"exec": "run"}
    });
    if let (Some(b), Some(o)) = (base.as_object_mut(), overrides.as_object()) {
        for (k, v) in o {
            if v.is_null() {
                b.remove(k);
            } else {
                b.insert(k.clone(), v.clone());
            }
        }
    }
    base
}

#[test]
fn a_good_path_is_accepted() {
    for ok in ["run", "a/b", "data/x.pak", "lib/libgates_sim.so", "a-b_c.1"] {
        assert!(safe_relpath(ok).is_ok(), "should accept {ok:?}");
    }
}

#[test]
fn traversal_and_absolutes_are_refused() {
    for bad in [
        "../etc/passwd",
        "a/../../b",
        "/etc/passwd",
        "..",
        ".",
        "a/",
        "",
        "c:/windows/system32",
        "C:x",
        "a\\..\\..\\b",  // backslash: legal on Linux, traversal on the win build
        "a\0b",
    ] {
        assert!(safe_relpath(bad).is_err(), "should refuse {bad:?}");
    }
}

#[test]
fn a_non_canonical_path_is_refused_as_a_uniqueness_rule() {
    // `a/b` and `./a/b` are two strings for one file. Both normalise to
    // something safe, so this is not traversal — it is that a depot could list
    // both, slip the duplicate check, and let whichever landed last win.
    for bad in ["./a", "a//b", "a/./b", "./run"] {
        let err = safe_relpath(bad).unwrap_err();
        assert!(
            err.0.contains("canonical") || err.0.contains("escapes"),
            "{bad:?} refused for the wrong reason: {}",
            err.0
        );
    }
}

#[test]
fn an_overlong_path_is_refused() {
    assert!(safe_relpath(&"a".repeat(1025)).is_err());
    assert!(safe_relpath(&"a".repeat(1024)).is_ok());
}

#[test]
fn a_symlinked_component_already_on_disk_is_refused() {
    // Not a textual attack, so `safe_relpath` cannot see it. This is the
    // second, filesystem-level check and the reason neither is redundant.
    let tmp = tempfile::tempdir().unwrap();
    let build = tmp.path().join("build");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, build.join("escape")).unwrap();
        let err = resolve_into(&build, "escape/evil.so").unwrap_err();
        assert!(err.0.contains("outside"), "{}", err.0);
    }
    // A path whose parents do not exist yet is fine — that is every file we
    // are about to write.
    assert!(resolve_into(&build, "brand/new/file.pak").is_ok());
}

#[test]
fn a_depot_root_must_be_https() {
    assert!(parse_depot(&depot_with(json!({"root": "http://x/"})), false).is_err());
    assert!(parse_depot(&depot_with(json!({"root": "file:///tmp/"})), false).is_err());
    assert!(parse_depot(&depot_with(json!({"root": ""})), false).is_err());
    // …unless a caller explicitly asked, which is how the local test server in
    // `install.rs` works and is never the default.
    assert!(parse_depot(&depot_with(json!({"root": "http://x/"})), true).is_ok());
}

#[test]
fn duplicate_files_are_refused_case_insensitively() {
    // Two entries differing only in case overwrite each other on mac/win and
    // the second hash wins with no error.
    let sha = "0".repeat(64);
    let d = depot_with(json!({
        "files": [
            {"path": "run", "sha256": sha, "bytes": 1},
            {"path": "RUN", "sha256": sha, "bytes": 1},
        ],
        "launch": {"exec": "run"}
    }));
    let err = parse_depot(&d, false).unwrap_err();
    assert!(err.0.contains("twice"), "{}", err.0);
}

#[test]
fn a_bad_hash_or_size_is_refused() {
    for files in [
        json!([{"path": "run", "sha256": "abc", "bytes": 1}]),
        json!([{"path": "run", "sha256": "z".repeat(64), "bytes": 1}]),
        json!([{"path": "run", "sha256": "0".repeat(64), "bytes": -1}]),
        json!([{"path": "run", "sha256": "0".repeat(64)}]),
        json!([{"path": "run", "sha256": "0".repeat(64), "bytes": 9223372036854775807i64}]),
    ] {
        let d = depot_with(json!({"files": files, "launch": {"exec": "run"}}));
        assert!(parse_depot(&d, false).is_err(), "should refuse {files}");
    }
    // 0x-prefixed and upper-case hashes are normalised, not refused.
    let d = depot_with(json!({
        "files": [{"path": "run", "sha256": format!("0x{}", "A".repeat(64)), "bytes": 1}],
        "launch": {"exec": "run"}
    }));
    assert_eq!(parse_depot(&d, false).unwrap().files[0].sha256, "a".repeat(64));
}

#[test]
fn launch_exec_must_be_one_of_the_depots_own_files() {
    let d = depot_with(json!({"launch": {"exec": "somewhere/else"}}));
    let err = parse_depot(&d, false).unwrap_err();
    assert!(err.0.contains("not one of"), "{}", err.0);
}

#[test]
fn launch_args_with_an_unknown_placeholder_are_refused_at_parse() {
    // The observed failure was a depot shipping `{servers}` for `{server}` —
    // it installed, verified clean, sat in the library as "up to date", and
    // every Play of it died in a dialog. `{servers}` has since become a real
    // placeholder (the title's shard-list url), so the exemplar here is a
    // near-miss that is still wrong; the refusal it earns belongs at parse,
    // while the build is still a download.
    let d = depot_with(json!({
        "launch": {"exec": "run", "args": ["--id", "{wallets}"]}
    }));
    let err = parse_depot(&d, false).unwrap_err();
    assert!(err.0.contains("unknown placeholder"), "{}", err.0);
    // The message does the diagnosis, because the player it lands on cannot:
    // it names what the launcher fills, and the near-miss it suspects.
    assert!(err.0.contains("probably {wallet}"), "{}", err.0);

    let d = depot_with(json!({
        "launch": {"exec": "run",
                   "args": ["--server", "{server}", "--servers", "{servers}", "--id", "{wallet}"]}
    }));
    assert!(parse_depot(&d, false).is_ok());
}

#[test]
fn a_depot_with_no_files_or_no_launch_is_refused() {
    assert!(parse_depot(&depot_with(json!({"files": []})), false).is_err());
    assert!(parse_depot(&depot_with(json!({"launch": null})), false).is_err());
    assert!(parse_depot(&depot_with(json!({"launch": {}})), false).is_err());
}

#[test]
fn a_version_we_do_not_speak_is_refused() {
    assert!(parse_depot(&depot_with(json!({"depot_version": 2})), false).is_err());
    let m = json!({"manifest_version": 99, "slug": "g"});
    assert!(parse_manifest(&m).is_err());
}

#[test]
fn a_build_row_with_no_url_and_no_depot_is_a_slot() {
    // Gates' shape today, and the honest one: it renders as "not published"
    // rather than as a broken download.
    let m = parse_manifest(&json!({
        "slug": "gates", "name": "Gates",
        "builds": [
            {"kind": "browser"},
            {"kind": "native", "platform": "linux-x86_64"}
        ]
    }))
    .unwrap();
    assert!(!m.playable_now());
    assert!(m.builds.iter().all(|b| !b.published()));
    assert!(!m
        .build_for("linux-x86_64", Some("native"))
        .unwrap()
        .published());
}

#[test]
fn a_published_build_beats_a_slot_and_an_exact_platform_beats_any() {
    let m = parse_manifest(&json!({
        "slug": "g", "builds": [
            {"kind": "native", "platform": "linux-x86_64"},
            {"kind": "browser", "platform": "any", "url": "https://x/play"},
        ]
    }))
    .unwrap();
    assert!(m.playable_now());
    assert_eq!(m.build_for("linux-x86_64", None).unwrap().kind, "browser");

    let m2 = parse_manifest(&json!({
        "slug": "g", "builds": [
            {"kind": "native", "platform": "any", "depot": "https://x/any.json"},
            {"kind": "native", "platform": "linux-x86_64", "depot": "https://x/lin.json"},
        ]
    }))
    .unwrap();
    assert_eq!(
        m2.build_for("linux-x86_64", Some("native")).unwrap().platform,
        "linux-x86_64"
    );
}

#[test]
fn an_unknown_kind_or_platform_is_refused() {
    assert!(parse_manifest(&json!({"slug": "g", "builds": [{"kind": "flatpak"}]})).is_err());
    assert!(parse_manifest(&json!({
        "slug": "g", "builds": [{"kind": "native", "platform": "bsd-x86_64"}]
    }))
    .is_err());
}

#[test]
fn a_bad_slug_is_refused() {
    for slug in ["", "../g", "a b", "a/b", "a.b"] {
        assert!(
            parse_manifest(&json!({"slug": slug})).is_err(),
            "should refuse slug {slug:?}"
        );
    }
}

#[test]
fn the_shipped_gates_example_parses() {
    // The example depot with every field filled. It is parsed here so the
    // shape cannot rot before the first real build exists. It moved beside this
    // test on 2026-08-07 — it used to live in the deleted Python client, which
    // made a documentation example a build dependency of another tree.
    let raw = include_str!("gates.depot.example.json");
    let value: Value = serde_json::from_str(raw).expect("the example is JSON");
    let d = parse_depot(&value, false).expect("the example parses");
    assert_eq!(d.slug, "gates");
    assert!(d.files.iter().any(|f| f.path == d.launch_exec));
    assert!(d.digest().unwrap().starts_with("0x"));

    // Every placeholder it uses is one a launcher actually fills. Parsing does
    // not check this — `fill` refuses at LAUNCH time, which is a player's
    // machine — so a typo here would ship a build that installs perfectly and
    // never starts, and the example is what a title copies from.
    for a in &d.launch_args {
        let Some(name) = a.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else { continue };
        assert!(
            elo_depot::launch::ARG_VARS.contains(&name),
            "the example uses {{{name}}}, which no launcher fills"
        );
    }
}

// ── rebasing an origin-served manifest ───────────────────────────────────────
//
// An origin serving a manifest writes `"depot": "/api/launcher/depot/…"` rather
// than a full url, because it does not reliably know its own public name —
// behind nginx and a CDN the only candidate is the `Host` header, which the
// caller controls. The client knows where it asked; the server does not. These
// pin what completing that path may and may not do.

fn manifest_with_depot(depot: Value) -> elo_depot::Manifest {
    parse_manifest(&json!({
        "slug": "gates", "name": "Gates",
        "builds": [{"kind": "native", "platform": "linux-x86_64", "depot": depot}],
        "servers": {"url": "/api/launcher/servers/gates"}
    }))
    .expect("parses")
}

#[test]
fn a_path_absolute_depot_is_completed_against_the_origin_we_asked() {
    let mut m = manifest_with_depot(json!("/api/launcher/depot/gates/0.1.0"));
    m.rebase("https://elopros.com");
    assert_eq!(
        m.builds[0].depot_url.as_deref(),
        Some("https://elopros.com/api/launcher/depot/gates/0.1.0")
    );
    assert_eq!(
        m.servers_url.as_deref(),
        Some("https://elopros.com/api/launcher/servers/gates")
    );
    assert!(m.builds[0].published());
}

#[test]
fn an_absolute_url_is_left_exactly_alone() {
    // A title may host its depot anywhere and most will. Rebasing one that is
    // already complete would point every third-party build at our origin.
    let mut m = manifest_with_depot(json!("https://cdn.example.com/gates/0.1.0.json"));
    m.rebase("https://elopros.com");
    assert_eq!(
        m.builds[0].depot_url.as_deref(),
        Some("https://cdn.example.com/gates/0.1.0.json")
    );
}

#[test]
fn a_protocol_relative_url_is_blanked_rather_than_rebased() {
    // `//evil.example/x` LOOKS relative and is not — prefixing a scheme would
    // silently send the download to another host while the manifest read as
    // same-origin. Losing a build row is recoverable; fetching from the wrong
    // host is not, so the row becomes a slot.
    for hostile in ["//evil.example/gates.json", "..%2f", "../up", "sneaky/rel"] {
        let mut m = manifest_with_depot(json!(hostile));
        m.rebase("https://elopros.com");
        assert_eq!(
            m.builds[0].depot_url, None,
            "should have blanked {hostile:?}, not rebased it"
        );
        assert!(
            !m.builds[0].published(),
            "{hostile:?} must render as a slot, never as a download"
        );
    }
}

#[test]
fn rebasing_is_idempotent_and_survives_a_trailing_slash() {
    let mut m = manifest_with_depot(json!("/api/launcher/depot/gates/0.1.0"));
    m.rebase("https://elopros.com/");
    let once = m.builds[0].depot_url.clone();
    m.rebase("https://elopros.com/");
    assert_eq!(m.builds[0].depot_url, once);
    assert_eq!(
        once.as_deref(),
        Some("https://elopros.com/api/launcher/depot/gates/0.1.0")
    );
}

#[test]
fn an_empty_slot_stays_an_empty_slot() {
    // Gates' shape before anything is published: rebasing must not invent a
    // url for a row that has none.
    let mut m = parse_manifest(&json!({
        "slug": "gates",
        "builds": [{"kind": "native", "platform": "linux-x86_64", "depot": null}]
    }))
    .expect("parses");
    m.rebase("https://elopros.com");
    assert_eq!(m.builds[0].depot_url, None);
    assert!(!m.builds[0].published());
}
