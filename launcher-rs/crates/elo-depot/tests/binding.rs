//! A depot is bound to the manifest that led to it — and the other edges a
//! stranger's document reaches: the http hop, a join link's host, the launch
//! environment.
//!
//! Every test here was written against the unfixed code and failed there.

use elo_depot::update::{check, JsonSource, UpdateState};
use elo_depot::{install, installed, launch, newest_of, parse_depot, parse_manifest, DepotError};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

fn sha_hex(body: &[u8]) -> String {
    Sha256::digest(body).iter().map(|b| format!("{b:02x}")).collect()
}

fn depot_json(slug: &str, build: &str, platform: &str, body: &[u8]) -> Value {
    json!({
        "depot_version": 1, "slug": slug, "build": build,
        "platform": platform, "root": "https://cdn.example/d",
        "files": [{"path": "run", "sha256": sha_hex(body), "bytes": body.len(), "executable": true}],
        "launch": {"exec": "run", "args": ["--server", "{server}"], "cwd": ".", "env": {}}
    })
}

fn manifest(slug: &str, depot_url: &str) -> elo_depot::Manifest {
    parse_manifest(&json!({
        "manifest_version": 1, "slug": slug, "name": slug,
        "builds": [{"kind": "native", "platform": "linux-x86_64", "depot": depot_url}]
    }))
    .unwrap()
}

struct Serve(Value);
impl JsonSource for Serve {
    fn get(&self, _url: &str) -> Result<Value, String> {
        Ok(self.0.clone())
    }
}

fn writes(body: &'static [u8]) -> impl Fn(&str, &Path) -> Result<(), DepotError> {
    move |_u: &str, d: &Path| std::fs::write(d, body).map_err(DepotError::from)
}

// ── l-depot#1 / x-files#1: the depot's slug is the manifest's ──────────────

#[test]
fn a_depot_that_names_another_title_is_refused_by_the_update_check() {
    let games = tempfile::tempdir().unwrap();
    let legit: &[u8] = b"#!/bin/sh\necho gates\n";
    let d = parse_depot(&depot_json("gates", "0.5.0", "linux-x86_64", legit), false).unwrap();
    install(&d, games.path(), &writes(legit), None, None).unwrap();

    // A third-party manifest for `coolgame` whose depot calls itself Gates.
    let m = manifest("coolgame", "https://evil.example/coolgame/depot.json");
    let evil = depot_json("gates", "0.5.0", "linux-x86_64", b"#!/bin/sh\necho evil\n");
    let c = check(&m, "linux-x86_64", games.path(), &Serve(evil), false);
    match &c.state {
        UpdateState::Unknown { why } => assert!(why.contains("gates"), "{why}"),
        other => panic!("a depot for another title must not be installable: {}", other.line()),
    }
    assert!(c.depot.is_none(), "no depot is handed back to install");
    // And Gates' build is untouched.
    let now = newest_of(games.path(), "gates").unwrap();
    assert_eq!(std::fs::read(now.path.join("run")).unwrap(), legit);
}

#[test]
fn a_manifest_from_another_origin_cannot_take_an_installed_titles_name() {
    // The attacker publishes BOTH documents, so they agree: a manifest that
    // calls itself `gates` ("Cool Game") and a gates/0.5.0 depot. Binding the
    // depot to the manifest's slug passes; what differs is where it came from.
    let games = tempfile::tempdir().unwrap();
    let legit: &[u8] = b"#!/bin/sh\necho gates\n";
    let mut house = manifest("gates", "/api/launcher/depot/gates/0.5.0");
    house.rebase("https://elopros.com");
    let c = check(&house, "linux-x86_64", games.path(),
                  &Serve(depot_json("gates", "0.5.0", "linux-x86_64", legit)), false);
    install(&c.depot.unwrap(), games.path(), &writes(legit), None, None).unwrap();

    let mut cool = manifest("gates", "https://attacker.example/gates/depot.json");
    cool.rebase("https://attacker.example");
    let evil = depot_json("gates", "0.5.0", "linux-x86_64", b"#!/bin/sh\necho evil\n");
    let c = check(&cool, "linux-x86_64", games.path(), &Serve(evil), false);
    match &c.state {
        UpdateState::Unknown { why } => assert!(why.contains("elopros.com"), "{why}"),
        other => panic!("another origin's gates must not install over ours: {}", other.line()),
    }
    assert!(c.depot.is_none(), "no depot is handed back to install");
    let now = newest_of(games.path(), "gates").unwrap();
    assert_eq!(std::fs::read(now.path.join("run")).unwrap(), legit);
}

#[test]
fn a_depot_for_another_platform_is_refused_by_the_update_check() {
    let games = tempfile::tempdir().unwrap();
    let m = manifest("t", "https://cdn.example/t/depot.json");
    let other = depot_json("t", "1", "win-x86_64", b"MZ");
    let c = check(&m, "linux-x86_64", games.path(), &Serve(other), false);
    assert!(matches!(c.state, UpdateState::Unknown { .. }), "{}", c.state.line());
    // `any` is a depot that runs everywhere, and stays installable.
    let any = depot_json("t", "1", "any", b"#!/bin/sh\n");
    let c = check(&m, "linux-x86_64", games.path(), &Serve(any), false);
    assert!(matches!(c.state, UpdateState::NotInstalled { .. }), "{}", c.state.line());
}

#[test]
fn a_build_or_slug_a_windows_disk_would_rename_never_installs() {
    // `gates.` is `gates` on NTFS, and `0.5.0 ` is `0.5.0`. The same rule the
    // origin files depots under (`meter/launcher.py` read_depot), plus the
    // two Windows trims.
    let games = tempfile::tempdir().unwrap();
    for (slug, build) in [("gates.", "1"), ("gates", "1."), ("gates", "1 "), ("gates", ".1"),
                          ("gates", "a/b"), ("gat es", "1")] {
        let raw = depot_json(slug, build, "linux-x86_64", b"x");
        let Ok(d) = parse_depot(&raw, false) else { continue };
        let err = install(&d, games.path(), &writes(b"x"), None, None);
        assert!(err.is_err(), "{slug:?}/{build:?} must not install");
    }
    assert!(installed(games.path()).is_empty());
}

// ── l-depot#3: the document is held to https too ───────────────────────────

#[test]
fn an_http_depot_document_is_refused_without_allow_insecure() {
    let games = tempfile::tempdir().unwrap();
    let m = manifest("t", "http://cdn.example/t/depot.json");
    // The document a MITM on that hop substitutes: an https root, its own hashes.
    let doc = depot_json("t", "1", "linux-x86_64", b"#!/bin/sh\necho mitm\n");
    let c = check(&m, "linux-x86_64", games.path(), &Serve(doc.clone()), false);
    match &c.state {
        UpdateState::Unknown { why } => assert!(why.contains("https"), "{why}"),
        other => panic!("an http depot document must not be trusted: {}", other.line()),
    }
    assert!(c.depot.is_none());
    // The flag still means what it says.
    let c = check(&m, "linux-x86_64", games.path(), &Serve(doc), true);
    assert!(matches!(c.state, UpdateState::NotInstalled { .. }), "{}", c.state.line());
}

// ── l-depot#5: a join link's host is a host ────────────────────────────────

#[test]
fn a_join_address_is_a_dns_name_or_an_ip_literal() {
    use elo_depot::deeplink::check_addr;
    for bad in [
        "--identity=0xVictim:1",
        "-x.example:1",
        "a.-x.example:1",
        "game.elopros.com@evil.example:61234",
        "moc.elpmaxe-live\u{202e}game.elopros.com:1",
        "gämé.example:1",
        "[not-v6]:1",
    ] {
        assert!(check_addr(bad).is_err(), "{bad:?} should be refused");
    }
    for good in ["game.elopros.com:61234", "127.0.0.1:4433", "[::1]:4433", "eu-1.example.test:61234"] {
        check_addr(good).unwrap_or_else(|e| panic!("{good:?}: {e}"));
    }
    // Through the link, after percent-decoding.
    assert!(elo_depot::deeplink::parse("elo://join/gates/game.elopros.com%40evil.example:61234").is_err());
    assert!(elo_depot::deeplink::parse("elo://join/gates/--identity=0x1:1").is_err());
}

// ── l-depot#6: the loader cannot be redirected by another spelling ─────────

#[test]
fn a_depot_may_not_set_any_loader_variable_in_any_case() {
    let games = tempfile::tempdir().unwrap();
    let d = parse_depot(&depot_json("t", "1", "linux-x86_64", b"x"), false).unwrap();
    install(&d, games.path(), &writes(b"x"), None, None).unwrap();
    let mut i = installed(games.path()).remove(0);
    for bad in [
        "Path", "path", "ld_preload", "DYLD_FALLBACK_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH",
        "GCONV_PATH", "VK_ICD_FILENAMES", "VK_LAYER_PATH", "LIBGL_DRIVERS_PATH",
        "__EGL_VENDOR_LIBRARY_FILENAMES", "GIO_EXTRA_MODULES", "QT_PLUGIN_PATH", "GTK_PATH",
        "NODE_OPTIONS", "PERL5LIB", "RUBYLIB", "LD_DEBUG_OUTPUT",
        // Each names code an interpreter or plugin loader the game starts runs.
        "GST_PLUGIN_PATH", "GST_PLUGIN_SYSTEM_PATH_1_0", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS",
        "JDK_JAVA_OPTIONS", "BASH_ENV", "ENV", "PYTHONSTARTUP", "LUA_PATH", "LUA_CPATH_5_4",
    ] {
        i.launch_env = BTreeMap::from([(bad.to_string(), "/tmp/evil".to_string())]);
        let err = launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new());
        assert!(err.is_err(), "{bad} must be refused");
    }
    // A game's own settings still pass.
    i.launch_env = BTreeMap::from([("SDL_VIDEODRIVER".into(), "x11".into())]);
    launch::resolve(&i, &BTreeMap::new(), &BTreeMap::new()).unwrap();
}

// ── l-keys#1: which sign-in domains a title publishes as its own ───────────

#[test]
fn a_title_serves_the_hosts_its_shard_list_names_and_nothing_else() {
    use elo_depot::shardlist::{parse, serves};
    let rows = parse(json!({
        "kind": "elo-shardlist-v1",
        "servers": [
            {"id": "eu-1", "name": "EU", "addr": "shard-3.gates.example:61234"},
            {"id": "v6", "name": "V6", "addr": "[2001:db8::1]:443"},
        ]
    }).to_string().as_bytes()).unwrap();
    let list = "https://gates.example/servers.json";
    for yes in ["shard-3.gates.example", "SHARD-3.gates.example", "gates.example", "2001:db8::1"] {
        assert!(serves(&rows, list, yes), "{yes}");
    }
    for no in ["app.thirdparty.example", "evil-shard-3.gates.example", "example", ""] {
        assert!(!serves(&rows, list, no), "{no}");
    }
}

// ── l-keys#4: opening a page never goes through a shell ────────────────────

#[test]
fn opening_a_page_on_windows_is_one_argument_to_the_url_handler_never_cmd() {
    use elo_depot::launch::open_argv_for;
    let url = "https://elopros.com/game.html?slug=gates&x=1";
    let argv = open_argv_for(true, url).unwrap();
    assert!(!argv[0].to_ascii_lowercase().starts_with("cmd"), "{argv:?}");
    assert_eq!(argv.last().unwrap(), url, "the url is one argv entry, untouched");
    assert_eq!(open_argv_for(false, url).unwrap(), vec!["xdg-open".to_string(), url.to_string()]);
    for bad in [
        r"https://x.example/&\\attacker.example@SSL\DavWWWRoot\p.exe",
        "https://x.example/\"&calc",
        "https://x.example/^&calc",
        "https://x.example/a b",
        "file:///etc/passwd",
    ] {
        assert!(open_argv_for(true, bad).is_err(), "{bad:?} should be refused");
    }
}
