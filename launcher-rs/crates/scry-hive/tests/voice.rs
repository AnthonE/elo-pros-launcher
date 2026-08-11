//! The speaking key, and the events it signs.
//!
//! The load-bearing property is that **the relay accepts what we produce**,
//! and the only honest way to check that without a relay is to verify against
//! the same code the meter runs. `tests/parity_verify.py` does exactly that:
//! this suite writes real signed events to `target/nostr_parity.json`, and
//! that script runs `meter/hive.py:verify_event` over every one.
//!
//! Same shape as the depot digest parity, and for the same reason — a second
//! implementation of a cryptographic serialization is only safe if something
//! pins it to the first.

use scry_hive::voice::{npub_to_hex, UnsignedEvent, Voice};
use scry_hive::relay::note_for;

#[test]
fn a_voice_round_trips_through_its_own_nsec() {
    let v = Voice::generate().unwrap();
    let nsec = v.to_nsec();
    assert!(nsec.starts_with("nsec1"), "{nsec}");
    let back = Voice::from_nsec(&nsec).unwrap();
    assert_eq!(back.npub(), v.npub());
    assert_eq!(back.pubkey_hex(), v.pubkey_hex());
}

#[test]
fn an_npub_is_the_bech32_of_the_same_bytes_the_event_carries() {
    let v = Voice::generate().unwrap();
    let npub = v.npub();
    assert!(npub.starts_with("npub1"), "{npub}");
    assert_eq!(npub_to_hex(&npub).unwrap(), v.pubkey_hex());
    assert_eq!(v.pubkey_hex().len(), 64, "x-only keys are 32 bytes");
}

#[test]
fn the_wrong_prefix_is_refused_rather_than_coerced() {
    let v = Voice::generate().unwrap();
    // An npub where an nsec belongs is a mistake that must not silently
    // produce a working-looking key.
    assert!(Voice::from_nsec(&v.npub()).is_err());
    assert!(npub_to_hex(&v.to_nsec()).is_err());
    assert!(Voice::from_nsec("not-bech32-at-all").is_err());
    assert!(Voice::from_nsec("").is_err());
}

#[test]
fn a_key_never_prints_itself() {
    // A secret in a log line is a secret gone. `Debug` shows the npub, which
    // is public, and the test pins that the nsec is not in the output.
    let v = Voice::generate().unwrap();
    let shown = format!("{v:?}");
    assert!(shown.contains(&v.npub()), "{shown}");
    assert!(!shown.contains(&v.to_nsec()), "the secret leaked into Debug");
}

#[test]
fn a_signed_event_verifies_and_a_tampered_one_does_not() {
    let v = Voice::generate().unwrap();
    let ev = v.sign(note_for(None, "hello the town", 1_784_845_562)).unwrap();
    ev.verify().expect("our own signature must check out");

    assert_eq!(ev.pubkey, v.pubkey_hex());
    assert_eq!(ev.id.len(), 64);
    assert_eq!(ev.sig.len(), 128, "a BIP-340 signature is 64 bytes");

    // Every field is inside the id, so changing any of them breaks it.
    for mutate in [0, 1, 2, 3] {
        let mut bad = ev.clone();
        match mutate {
            0 => bad.content.push('!'),
            1 => bad.created_at += 1,
            2 => bad.kind += 1,
            _ => bad.tags.push(vec!["t".into(), "elsewhere".into()]),
        }
        assert!(bad.verify().is_err(), "mutation {mutate} should break the id");
    }

    // …and a swapped signature fails even with the id intact.
    let other = Voice::generate().unwrap();
    let mut forged = ev.clone();
    forged.sig = other
        .sign(note_for(None, "hello the town", 1_784_845_562))
        .unwrap()
        .sig;
    assert!(forged.verify().is_err(), "another key's signature must not pass");
}

#[test]
fn the_event_id_is_the_nip01_serialization() {
    // Pinned by hand so a change to `id()` cannot pass silently: the id is
    // sha256 over the compact array [0, pubkey, created_at, kind, tags,
    // content] with no spaces. A fixed pubkey makes it deterministic.
    let pubkey = "e8d2b1f4c3a5967801234567890abcdef0123456789abcdef0123456789abcde";
    let ev = UnsignedEvent {
        created_at: 1_700_000_000,
        kind: 1,
        tags: vec![vec!["t".into(), "scry".into()]],
        content: "hello".into(),
    };
    let want = {
        use sha2::{Digest, Sha256};
        let s = format!(
            r#"[0,"{pubkey}",1700000000,1,[["t","scry"]],"hello"]"#
        );
        scry_hive::voice::hex(&Sha256::digest(s.as_bytes()))
    };
    assert_eq!(ev.id(pubkey), want);
}

#[test]
fn unicode_and_escapes_survive_the_id_intact() {
    // The id is over UTF-8 with JSON escaping, and getting this wrong yields
    // a signature every relay rejects — for messages containing exactly the
    // characters people actually type.
    let v = Voice::generate().unwrap();
    for content in [
        "the record anchored — root 0x2b88…",
        "emoji 🐝 and CJK 龍",
        "quotes \" and backslash \\ and newline \n inside",
        "",
    ] {
        let ev = v.sign(note_for(None, content, 1_700_000_000)).unwrap();
        ev.verify().unwrap_or_else(|e| panic!("{content:?}: {e}"));
        assert_eq!(ev.content, content);
    }
}

#[test]
fn a_key_is_stored_at_0600_and_reloads() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!Voice::exists(dir.path()));
    let v = Voice::generate().unwrap();
    let path = v.save(dir.path()).unwrap();
    assert!(Voice::exists(dir.path()));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "a key file is not world-readable");
        let dmode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700, "nor is its directory");
    }
    assert_eq!(Voice::load(dir.path()).unwrap().npub(), v.npub());
}

#[test]
fn a_missing_key_says_how_to_make_one() {
    let dir = tempfile::tempdir().unwrap();
    let err = Voice::load(dir.path()).unwrap_err();
    assert!(err.0.contains("voice-new"), "should point at the fix: {}", err.0);
}

/// Writes real signed events for `tests/parity_verify.py` to check against
/// `meter/hive.py:verify_event` — the same code the server runs.
#[test]
fn emit_events_for_the_python_verifier() {
    let v = Voice::generate().unwrap();
    let mut out = Vec::new();
    let cases: [(Option<&str>, &str); 5] = [
        (None, "a plain town note"),
        (None, "unicode — 🐝 龍 éàü"),
        (None, "quotes \" backslash \\ newline \n tab \t"),
        (Some("70e0e2b7-3d6d-4e44-8459-a433be1df670"), "a room note"),
        (None, ""),
    ];
    for (room, content) in cases {
        let ev = v.sign(note_for(room, content, 1_784_845_562)).unwrap();
        ev.verify().expect("self-check");
        out.push(ev.to_json());
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("nostr_parity.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "npub": v.npub(), "pubkey": v.pubkey_hex(), "events": out
        }))
        .unwrap(),
    )
    .unwrap();
}

/// Connect to the real relay and authenticate — **without publishing**.
///
/// `#[ignore]` because it needs the network and because it touches a live
/// relay. Run it explicitly:
///
/// ```bash
/// cargo test -p scry-hive --test voice -- --ignored live_relay
/// ```
///
/// It proves the half that cannot be unit-tested: TLS, the WebSocket
/// handshake, and NIP-42 AUTH against buzz's actual author-binding rule. It
/// deliberately stops before `publish` — speaking in a public room is not
/// something a test suite gets to do.
#[test]
#[ignore]
fn live_relay_accepts_a_fresh_voice() {
    use scry_hive::relay::Relay;
    let url = std::env::var("SCRY_RELAY")
        .unwrap_or_else(|_| "wss://hive.scry.moreright.xyz".into());
    let v = Voice::generate().unwrap();
    let relay = Relay::connect(&url, &v)
        .unwrap_or_else(|e| panic!("could not authenticate to {url}: {e}"));
    relay.close();
    println!("authenticated to {url} as {}", v.npub());
}

/// Hold presence on the real relay, beat, and release — **without speaking**.
///
/// `#[ignore]`: needs the network. Presence is ephemeral and carries no
/// content anyone reads as a message, so unlike `publish` this is safe to
/// exercise against the live relay.
///
/// ```bash
/// cargo test -p scry-hive --test voice -- --ignored live_presence
/// ```
#[test]
#[ignore]
fn live_relay_holds_presence() {
    use scry_hive::relay::{Presence, Status};
    let url = std::env::var("SCRY_RELAY")
        .unwrap_or_else(|_| "wss://hive.scry.moreright.xyz".into());
    let v = Voice::generate().unwrap();
    let npub = v.npub();
    let mut held = Presence::hold(&url, v, Status::Online)
        .unwrap_or_else(|e| panic!("could not hold presence on {url}: {e}"));
    assert_eq!(held.beats(), 1, "hold() sends the first beat");
    held.tick().expect("a second beat on the same socket");
    assert_eq!(held.beats(), 2);
    held.set_status(Status::Away).expect("away is a beat too");
    assert_eq!(held.beats(), 3);
    println!("held presence for {npub} across {} beats", held.beats());
    held.release();
}
