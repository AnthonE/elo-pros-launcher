//! The page serves a build of this crate, not this crate. `build_web.sh`
//! records what it built in `web.sha256`; this holds the record against the
//! source and the served files, so a change to a .rs file that never reached
//! `watchtower/animate/elo_rig.wasm` fails here instead of on the site.

use sha2::{Digest, Sha256};
use std::path::Path;

fn hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn record() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("web.sha256")).unwrap();
    text.lines()
        .filter_map(|l| l.split_once("  ").map(|(h, n)| (n.to_string(), h.to_string())))
        .collect()
}

#[test]
fn the_wasm_was_built_from_this_source() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<_> = std::fs::read_dir(here.join("src"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    let mut all = std::fs::read(here.join("Cargo.toml")).unwrap();
    for f in files {
        all.extend(std::fs::read(f).unwrap());
    }
    let src = record().into_iter().find(|(n, _)| n == "src").unwrap().1;
    assert_eq!(hex(&all), src, "crates/elo-rig changed since the web build: run crates/elo-rig/build_web.sh");
}

#[test]
fn the_served_files_are_the_recorded_ones() {
    let site = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../watchtower/animate");
    let rec = record();
    assert!(rec.len() >= 5, "{rec:?}");
    for (name, hash) in rec.iter().filter(|(n, _)| n != "src") {
        let bytes = std::fs::read(site.join(name)).unwrap_or_else(|_| panic!("watchtower/animate/{name} is missing"));
        assert_eq!(&hex(&bytes), hash, "watchtower/animate/{name} is not the recorded build");
    }
    // The page asks for this build by its hash, so no cache serves an older one.
    let wasm = &rec.iter().find(|(n, _)| n == "elo_rig.wasm").unwrap().1[..8];
    let js = std::fs::read_to_string(site.join("../js/animate.js")).unwrap();
    assert!(js.contains(&format!("elo_rig.wasm?v={wasm}")), "js/animate.js must ask for elo_rig.wasm?v={wasm}");
}
