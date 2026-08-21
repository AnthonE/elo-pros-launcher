//! Two implementations, one set of golden files.
//!
//! `/api/who` is flattened into a game-facing profile, and `prove` composes a
//! SIWE message. Both are pinned here against `sdk/testdata/*.json` rather than
//! against whatever this crate happens to produce today.
//!
//! ⚠ **These files stopped being a parity check and became a regression pin on
//! 2026-08-07**, when the second broker was deleted. That is a weaker guarantee
//! and it is worth naming: a parity check catches one side drifting from
//! another, while a pin only catches drift from a value someone once wrote
//! down. What keeps it honest is that `sdk/test_sdk.py` asserts the same
//! fixture against what actually comes **off the wire** from a running broker —
//! so the composer is checked here and the reply is checked there, and a change
//! that satisfies one without the other fails.
//!
//! The lesson these files exist to carry: `prove` shipped in this broker, in
//! both SDK clients and in `sdk/PROTOCOL.md` while a second broker refused it
//! as an unknown op, and every suite stayed green because each half was only
//! ever driven against its own kind. Deleting that second broker removed the
//! whole bug class; these pins are what is left of the lesson.

use std::path::PathBuf;

fn load(name: &str) -> serde_json::Value {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "..",
        "sdk",
        "testdata",
        name,
    ]
    .iter()
    .collect();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("the fixture is not JSON")
}

fn fixture() -> serde_json::Value {
    load("who_fixture.json")
}

/// The SIWE message, byte for byte, against the same file Python asserts.
///
/// ⚠ **This is the one that a third party's verifier depends on.** A game's
/// backend runs a `siwe` library over these exact bytes; a stray space, a
/// reordered line or a different statement makes a perfectly good signature
/// unverifiable, and the failure surfaces in someone else's server rather than
/// in this repo. EIP-4361's grammar is not advisory — the blank lines around
/// the statement are part of it.
#[test]
fn prove_message_matches_the_golden_siwe_bytes() {
    let fx = load("prove_fixture.json");
    let got = elo_broker::protocol::prove_message(
        fx["domain"].as_str().unwrap(),
        fx["address"].as_str().unwrap(),
        fx["game"].as_str().unwrap(),
        fx["nonce"].as_str().unwrap(),
        fx["issued_at"].as_str().unwrap(),
    );
    assert_eq!(
        got,
        fx["message"].as_str().unwrap(),
        "the two brokers compose different SIWE messages — a game's verifier \
         would reject a signature that is perfectly good over different bytes"
    );
}

/// The address in the fixture is EIP-55 checksummed, and that is load-bearing:
/// the reference `siwe` parsers reject an all-lowercase address outright, so a
/// launcher emitting one issues proofs nobody can verify.
#[test]
fn the_fixture_address_is_checksummed() {
    let fx = load("prove_fixture.json");
    let addr = fx["address"].as_str().unwrap();
    let hex = &addr[2..];
    assert!(
        hex.chars().any(|c| c.is_ascii_uppercase()) && hex.chars().any(|c| c.is_ascii_lowercase()),
        "{addr} does not look EIP-55 checksummed"
    );
}

/// The WEBSITE sign-in message, byte for byte, against the same file Python
/// asserts (`meter/test_signin.py`).
///
/// ⚠ Here the verifier is our own origin rather than a game's backend, which
/// makes the failure QUIETER, not safer: `meter/signin.py` recomposes these
/// bytes to verify a proof, so a drift refuses every launcher sign-in with
/// both suites green — the exact shape `prove` was dark in for a week.
#[test]
fn signin_message_matches_the_golden_siwe_bytes() {
    let fx = load("signin_fixture.json");
    let got = elo_broker::protocol::signin_message(
        fx["domain"].as_str().unwrap(),
        fx["address"].as_str().unwrap(),
        fx["nonce"].as_str().unwrap(),
        fx["issued_at"].as_str().unwrap(),
    );
    assert_eq!(
        got,
        fx["message"].as_str().unwrap(),
        "the launcher and the origin compose different sign-in messages — \
         every proof would be refused with nothing wrong on either end"
    );
}

#[test]
fn flatten_who_matches_the_golden_shape() {
    let fx = fixture();
    let address = fx["address"].as_str().expect("fixture has an address");
    let got = elo_broker::protocol::flatten_who(&fx["who"], address);
    let want = &fx["flat"];

    // Field by field first — a whole-value assert on a 14-key object prints a
    // wall of JSON and names nothing.
    let want_obj = want.as_object().expect("fixture `flat` is an object");
    let got_obj = got.as_object().expect("flatten_who returned an object");
    for (k, w) in want_obj {
        assert_eq!(
            got_obj.get(k),
            Some(w),
            "field {k:?} differs — the fixture says {w}, this build says {:?}",
            got_obj.get(k)
        );
    }
    let extra: Vec<_> = got_obj.keys().filter(|k| !want_obj.contains_key(*k)).collect();
    assert!(extra.is_empty(), "this build invented fields the fixture does not carry: {extra:?}");
    assert_eq!(got, *want);
}

#[test]
fn the_sworn_and_the_self_declared_never_merge() {
    // The one distinction a profile may not blur. A face is typed by its owner;
    // a handle is in the locked index and cost SCRY to change. A surface that
    // shows the first in the clothes of the second has lied for free.
    let who = serde_json::json!({
        "found": {"agent": "impostor", "nip05": null, "sandbox": false,
                  "wallet": "0x1111111111111111111111111111111111111111"},
    });
    let flat = elo_broker::protocol::flatten_who(&who, "0x1111111111111111111111111111111111111111");
    assert_eq!(flat["display_name"], "impostor");
    assert!(flat["handle"].is_null(), "a typed name must never land in `handle`");
    assert_eq!(flat["sworn"], false);
}

#[test]
fn a_sandbox_identity_is_never_sworn() {
    // A sandbox vow has a name but no wallet behind it. Reporting it as sworn
    // would let a free identity wear a paid one's clothes.
    let who = serde_json::json!({
        "found": {"nip05": "moss", "sandbox": true,
                  "wallet": "0x1111111111111111111111111111111111111111"},
    });
    let flat = elo_broker::protocol::flatten_who(&who, "0x1111111111111111111111111111111111111111");
    assert_eq!(flat["handle"], "moss");
    assert_eq!(flat["sworn"], false, "a sandbox identity is not sworn");
}

#[test]
fn every_link_carries_the_api_prefix_the_edge_strips() {
    // `CLAUDE.md` §traps: nginx serves the meter under `location ^~ /api/` with
    // a trailing-slash proxy_pass, which REPLACES the matched prefix. Routes are
    // declared bare and urls written into replies must carry `/api/`. A path
    // emitted without it is a link to nothing, and the game finds out, not us.
    let fx = fixture();
    let address = fx["address"].as_str().unwrap();
    let flat = elo_broker::protocol::flatten_who(&fx["who"], address);
    for (name, v) in flat["links"].as_object().expect("links is an object") {
        if let Some(s) = v.as_str() {
            assert!(s.starts_with("/api/"), "link {name:?} is {s:?} — missing the /api prefix");
        }
    }
    if let Some(a) = flat["avatar"].as_str() {
        assert!(a.starts_with("/api/"), "avatar is {a:?} — missing the /api prefix");
    }
}

#[test]
fn a_wallet_with_several_vows_says_so() {
    // One party, several identities — not several claimants. A game that reads
    // this as "1" would present one vow as the whole of somebody.
    let fx = fixture();
    let address = fx["address"].as_str().unwrap();
    let flat = elo_broker::protocol::flatten_who(&fx["who"], address);
    assert!(
        flat["identities"].as_i64().unwrap_or(0) >= 1,
        "identities must carry the count through"
    );
}
