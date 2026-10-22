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

use elo_hive::voice::{npub_to_hex, UnsignedEvent, Voice};
use elo_hive::relay::note_for;

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
        tags: vec![vec!["t".into(), "elo".into()]],
        content: "hello".into(),
    };
    let want = {
        use sha2::{Digest, Sha256};
        let s = format!(
            r#"[0,"{pubkey}",1700000000,1,[["t","elo"]],"hello"]"#
        );
        elo_hive::voice::hex(&Sha256::digest(s.as_bytes()))
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

/// A link at `voice.nsec` is replaced, never written through: `fs::write`
/// followed it and put the chat key wherever it pointed, at the target's mode.
#[cfg(unix)]
#[test]
fn a_link_at_the_key_path_is_replaced_not_followed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere");
    std::os::unix::fs::symlink(&target, dir.path().join("voice.nsec")).unwrap();
    let v = Voice::generate().unwrap();
    let path = v.save(dir.path()).unwrap();
    assert!(!target.exists(), "the key went where the link pointed");
    let meta = std::fs::symlink_metadata(&path).unwrap();
    assert!(meta.file_type().is_file(), "the key path is a plain file now");
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
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
/// cargo test -p elo-hive --test voice -- --ignored live_relay
/// ```
///
/// It proves the half that cannot be unit-tested: TLS, the WebSocket
/// handshake, and NIP-42 AUTH against buzz's actual author-binding rule. It
/// deliberately stops before `publish` — speaking in a public room is not
/// something a test suite gets to do.
#[test]
#[ignore]
fn live_relay_accepts_a_fresh_voice() {
    use elo_hive::relay::Relay;
    let url = std::env::var("ELO_RELAY")
        .unwrap_or_else(|_| "wss://hive.elopros.com".into());
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
/// cargo test -p elo-hive --test voice -- --ignored live_presence
/// ```
#[test]
#[ignore]
fn live_relay_holds_presence() {
    use elo_hive::relay::{Presence, Status};
    let url = std::env::var("ELO_RELAY")
        .unwrap_or_else(|_| "wss://hive.elopros.com".into());
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

/// Read a room straight off the real relay — the path the Chat window takes
/// when elopros.com is not in the way. **Reads only; says nothing.**
///
/// ```bash
/// cargo test -p elo-hive --test voice -- --ignored live_relay_reads
/// ```
#[test]
#[ignore]
fn live_relay_reads_a_room_directly() {
    use elo_hive::relay::Relay;
    use elo_hive::Hive;
    let host = std::env::var("ELO_HOST").unwrap_or_else(|_| "https://elopros.com".into());
    let net = elo_net::Net::new();
    let rooms = Hive::new(&net, &host).channels().value.expect("the origin's room list");
    let url = rooms.relay.clone().expect("a posted relay");
    let room = rooms.channels.first().expect("at least one room");
    let v = Voice::generate().unwrap();
    let mut relay = Relay::connect(&url, &v)
        .unwrap_or_else(|e| panic!("could not authenticate to {url}: {e}"));
    let backlog = relay
        .subscribe("elo-live-test", &room.filter(20))
        .unwrap_or_else(|e| panic!("#{} would not open on {url}: {e}", room.name));
    relay.close();
    assert_eq!(backlog.dropped, 0, "a stored event failed its signature check");
    println!("#{}: {} stored event(s), all verified", room.name, backlog.events.len());
}

/// The whole engine, against the real origin and relay, until it says `live`.
/// Reads only. A fresh key in a temp dir, so nothing on this machine changes.
///
/// ```bash
/// cargo test -p elo-hive --test voice -- --ignored live_chat_engine
/// ```
#[test]
#[ignore]
fn live_chat_engine_goes_live() {
    use elo_hive::chat::{Chat, Link};
    let host = std::env::var("ELO_HOST").unwrap_or_else(|_| "https://elopros.com".into());
    let keys = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    Voice::generate().unwrap().save(keys.path()).unwrap();
    let chat = Chat::start(&host, keys.path().to_path_buf(), cache.path().to_path_buf());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    loop {
        let v = chat.view();
        if let Link::Relay(url) = &v.link {
            println!("live on {url}: {} room(s), {} message(s), status {:?}",
                     v.rooms.len(), v.messages.len(), v.status);
            assert!(!v.rooms.is_empty());
            assert_eq!(v.dropped, 0);
            break;
        }
        assert!(std::time::Instant::now() < deadline,
                "never went live: {:?} — {} ({})", v.link, v.status, v.why);
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    drop(chat);
    // The room list was cached for the next start without the origin.
    let saved = elo_hive::chat::Cache::load(cache.path());
    assert!(saved.channels.is_some(), "the room list should be cached");
}

/// A face, end to end against the real origin: `/who` names a picture on our
/// own host, the capped fetch returns it, and it reads as an image. Uses the
/// deploy wallet, which has a vow and so a mark. Reads only.
///
/// ```bash
/// cargo test -p elo-hive --test voice -- --ignored live_face
/// ```
#[test]
#[ignore]
fn live_face_of_a_known_wallet_is_an_image_on_our_origin() {
    let host = std::env::var("ELO_HOST").unwrap_or_else(|_| "https://elopros.com".into());
    let net = elo_net::Net::new();
    let id = elo_hive::who::resolve(&net, &host, "0x24445EFddB08d4938E3E3627042B2Cf4063d9092")
        .value
        .expect("the register answers for the deploy wallet");
    let url = id.origin_face(&host).unwrap_or_else(|| panic!("no face on {host}: {:?}", id.face));
    let bytes = net.art(url).value.unwrap_or_else(|| panic!("{url} did not serve"));
    assert!(elo_hive::chat::looks_like_image(&bytes), "{url} is not an image");
    println!("{} — {url} ({} bytes)", id.name, bytes.len());
}

/// **One wallet, one chat key, on every client.** The vector is the server's
/// own reference (`meter/hive.py` `derive_seckey` over an `eth_account`
/// signature, made 2026-10-21 with the meter's venv): the same wallet key,
/// the same text, the same npub. If this fails, the launcher and the website
/// would give one wallet two identities.
#[test]
fn a_wallets_chat_key_is_the_one_the_website_derives() {
    let wallet = "0x3e20ab25eb721dd4b691516238df14f0f5d3f7a3";
    let text = elo_hive::voice::derivation_message(wallet);
    assert_eq!(
        text,
        "scry hive voice v1\nwallet: 0x3e20ab25eb721dd4b691516238df14f0f5d3f7a3\n\
         this signature IS my hive voice key.\n\
         sign it only to derive your scry hive identity — never on any other site."
    );
    // eth_account's signature for the key 0x7a…7a over that text.
    let sig = "85194638f30fce1ede1041f70e85a9cc94841f813babc1e92468dd5ee25cece8\
               48675754662b9a2c319cfc6f2af920106a74547c8920dd5928cc5078578a25d51b";
    let v = Voice::from_wallet_signature(sig).unwrap();
    assert_eq!(v.npub(), "npub1k7dy53fkr25pupyjxqn64aytu8v792v96l9p2f7gl3edk424ljrsnqjp92");
    // A `0x` prefix and a different `v` byte change nothing: only r‖s counts.
    let other_v = format!("0x{}1c", &sig[..128]);
    assert_eq!(Voice::from_wallet_signature(&other_v).unwrap().npub(), v.npub());
    assert!(Voice::from_wallet_signature("0xdead").is_err());
}

#[test]
fn unhex_refuses_what_is_not_hex_without_panicking() {
    use elo_hive::voice::unhex;
    assert_eq!(unhex("00ff").unwrap(), vec![0, 255]);
    assert_eq!(unhex("ABcd").unwrap(), vec![0xab, 0xcd], "wallets write either case");
    // An even number of bytes that is not an even number of characters: a
    // slice through the middle of a char is a panic, and panic = "abort".
    assert!(unhex("a\u{e9}a").is_err());
    assert!(unhex("\u{e9}\u{e9}").is_err());
    // `from_str_radix` takes a sign; hex does not.
    assert!(unhex("+f").is_err());
    assert!(unhex("abc").is_err());
}
