//! The real vendored SDK, against the Rust broker, over a real socket.
//!
//! This is the test that says the port did not break any game. `sdk/PROTOCOL.md`
//! is the contract, `sdk/rust/elo_overlay.rs` is the source a game vendors, and
//! this file compiles that exact source as a module rather than a copy. If the
//! broker's replies drift from what the SDK parses, this fails here instead of
//! in someone's game.
//!
//! ⚠ **This says nothing about what any game actually has.** This header used
//! to call the file *"what Gates has compiled into its binary byte-for-byte
//! under a sha256 pin"*, which was a claim about a file in another repository
//! that nothing checked — and on 2026-08-09 it was false by 326 lines, with
//! this suite green the whole time. A consumer's pin catches the consumer's
//! own edits; nothing on either side sees US move. What we owe instead is a
//! number they can check against: `sdk/SHA256SUMS`, kept honest by
//! `sdk/test_sdk.py` §what a vendoring game checks against. `docs/client/SDK.md` §5a
//! is the whole story.
//!
//! `sdk/test_sdk.py` drives the **Python** client against this same Rust
//! broker (there is one broker now; the Python one was deleted 2026-08-07).
//! Neither suite replaces the other: together they are what makes swapping the
//! server underneath a running ecosystem safe.
//!
//! ⚠ Unix only, and deliberately not faked on Windows. The SDK now HAS a
//! named-pipe half (`docs/client/LAUNCHER.md` §12 row 1), but this suite's own
//! listener is a UNIX socket, so there is still nothing here to run on
//! Windows — and a green test on a platform it never ran on is worse than a
//! missing one. This file compiles to nothing there rather than pretending.

#![cfg(unix)]

use elo_broker::protocol::Refusal;
use elo_broker::{serve_conn, transport::Listener, Host, What};
use serde_json::{json, Value};

// The SDK, compiled from the file Gates vendors. Not a re-implementation and
// not a copy — the path is the source of truth for both.
//
// `#[path]` rather than `include!`: the vendored file carries `//!` module
// docs, and an included file's inner doc comments are not at a module's start,
// so `include!` rejects 33 of them. A path module treats it as what it is.
#[allow(dead_code)]
#[path = "../../../../sdk/rust/elo_overlay.rs"]
mod sdk;

/// The SDK connects to a `&Path` with an explicit timeout. Kept short here so
/// a broker that never answers fails the suite in seconds rather than hanging
/// it — a gate that waits on a clock is not a gate.
fn connect(endpoint: &str, game: &str) -> Result<sdk::Overlay, String> {
    sdk::Overlay::connect_at(
        std::path::Path::new(endpoint),
        game,
        "0.1.0",
        std::time::Duration::from_secs(5),
    )
}

struct RealHost {
    allow: bool,
}

impl Host for RealHost {
    /// ⚠ The SAME account `prove` below signs with, and EIP-55 checksummed.
    ///
    /// It used to be an unrelated hardcoded string, which was invisible while
    /// the proof format did not name the address. SIWE binds it, so a mismatch
    /// now produces a signature that cannot verify against its own message —
    /// exactly what the broker's agreement guard refuses. Deriving it from the
    /// key keeps the stub honest by construction.
    fn address(&self) -> Option<String> {
        elo_vault::Account::from_secret(&[9u8; 32]).ok().map(|a| a.address())
    }
    fn signer_name(&self) -> String { "arca".into() }
    fn host_url(&self) -> String { "https://elopros.com".into() }
    fn ask_consent(&mut self, _g: &str, _f: &str, _t: &str, _w: &str) -> Option<u32> {
        if self.allow { Some(3) } else { None }
    }
    /// A REAL signature from a REAL key — the point of this test.
    ///
    /// Everything else here can be stubbed and still prove the wire. This one
    /// cannot: what a game server does with the reply is `ecrecover`, and a
    /// stub signature would pass every assertion about shape while being
    /// unverifiable by the only party that matters.
    fn prove(&mut self, message: &str) -> Result<Value, Refusal> {
        let account = elo_vault::Account::from_secret(&[9u8; 32])
            .map_err(Refusal::new)?;
        Ok(json!({
            "signature": account.sign_message(message).map_err(Refusal::new)?,
            "address": account.address(),
            "backend": "local",
        }))
    }

    fn sign(&mut self, _text: &str, _why: &str) -> Result<Value, Refusal> {
        Ok(json!({
            "signature": "0x".to_string() + &"ab".repeat(65),
            "address": "0x24445efddb08d4938e3e3627042b2cf4063d9092",
            "backend": "arca",
        }))
    }
    fn title_url(&self, slug: &str, what: What) -> Option<String> {
        match what {
            What::Manifest => Some(format!("https://o/api/launcher/manifest/{slug}")),
            What::Servers => None,
        }
    }
    fn open(&mut self, _url: &str) -> Result<(), Refusal> { Ok(()) }
}

/// Stand the broker up on a real socket in a temp dir and run `body` against
/// its path. One connection is served, then the thread ends.
fn with_broker<T>(allow: bool, body: impl FnOnce(&str) -> T) -> T {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("launcher.sock");
    let endpoint = path.display().to_string();
    let listener = Listener::bind(&endpoint).expect("bind");

    let served = std::thread::spawn(move || {
        let mut host = RealHost { allow };
        if let Ok(conn) = listener.accept() {
            let _ = serve_conn(conn, &mut host);
        }
    });

    let out = body(&endpoint);
    let _ = served.join();
    out
}

#[test]
fn the_vendored_sdk_completes_a_handshake_and_reads_identity() {
    with_broker(true, |endpoint| {
        let mut ov = connect(endpoint, "gates")
            .expect("the SDK should connect to the Rust broker");
        assert_eq!(ov.signer(), "arca", "hello's signer field must survive the port");
        let addr = ov.address().expect("identity should answer an address");
        assert!(addr.starts_with("0x"), "got {addr:?}");
    });
}

#[test]
fn the_vendored_sdk_gets_a_signature_back_in_the_shape_it_parses() {
    with_broker(true, |endpoint| {
        let mut ov = connect(endpoint, "gates").expect("connect");
        let sig = ov
            .sign("elo play\naction: duel\nday: 2026-08-06", "settling round 41")
            .expect("the broker should sign for a consenting player");
        assert!(sig.signature.starts_with("0x"), "signature: {:?}", sig.signature);
        assert!(sig.address.as_deref().unwrap_or("").starts_with("0x"),
                "address: {:?}", sig.address);
    });
}

/// A refusal must arrive as a refusal the SDK can tell apart from a transport
/// failure — `sdk/PROTOCOL.md` lists three non-signature outcomes and calls
/// them "not the same outcome".
#[test]
fn a_players_refusal_reaches_the_sdk_as_a_refusal_not_an_error() {
    with_broker(false, |endpoint| {
        let mut ov = connect(endpoint, "gates").expect("connect");
        let err = ov
            .sign("elo play\nx", "y")
            .expect_err("a refused player must not produce a signature");
        let text = format!("{err:?}");
        assert!(
            text.to_lowercase().contains("refus"),
            "the SDK should see this as a refusal, got {text}"
        );
    });
}

/// No launcher is a NORMAL state and the SDK must say so rather than erroring
/// in a way a game would render as broken. This is the one every game hits
/// first — someone who launched the binary directly.
#[test]
fn no_launcher_is_a_normal_state() {
    let dir = tempfile::tempdir().expect("tmp");
    let missing = dir.path().join("nothing-here.sock");
    let r = connect(&missing.display().to_string(), "gates");
    assert!(r.is_err(), "there is no launcher, so connecting must not succeed");
}

/// **The whole chain a game server will run, end to end.**
///
/// The vendored SDK, over a real unix socket, to the real broker, to a host
/// holding a real key — and then the check Gates does: recompute the message
/// from what the server already knows, `ecrecover`, compare. If this passes,
/// a ticket check works.
#[test]
fn the_vendored_sdk_proves_an_identity_a_server_can_verify() {
    with_broker(true, |endpoint| {
        let mut o = connect(endpoint, "gates").expect("connect");
        let proof = match o.prove("shard-3.gates.example", "8f14e45fceea167a") {
            Ok(p) => p,
            Err(e) => panic!("prove refused: {e:?}"),
        };

        // ⚠ 1 · **The check changed shape when this became SIWE, and getting
        //        it wrong is the whole risk.** The old format had every field
        //        server-known, so a server recomputed the string and compared.
        //        EIP-4361 carries `Issued At`, which the LAUNCHER chooses — so
        //        a server cannot rebuild the bytes and must instead PARSE the
        //        message and check the fields it cares about (domain, nonce,
        //        address, chain), then verify the signature over the bytes as
        //        received. That is what every siwe library does, and the
        //        security is identical: the signature covers exactly these
        //        bytes, and every field a server relies on is one it checked
        //        against its own state rather than accepted.
        let issued_at = proof.message
            .lines()
            .find_map(|l| l.strip_prefix("Issued At: "))
            .expect("a SIWE message carries Issued At")
            .to_string();
        let account = elo_vault::Account::from_secret(&[9u8; 32]).expect("account");
        let expected = elo_broker::protocol::prove_message(
            "shard-3.gates.example", &account.address(), "gates",
            "8f14e45fceea167a", &issued_at);
        assert_eq!(proof.message, expected,
                   "every field but the timestamp must be derivable by the server");

        // The fields a server actually gates on, checked the way it would.
        assert!(proof.message.starts_with(
            "shard-3.gates.example wants you to sign in with your Ethereum account:"),
            "the domain is bound into the first line");
        assert!(proof.message.contains("\nNonce: 8f14e45fceea167a\n"),
                "the nonce the server issued is in the signed bytes");
        assert!(proof.message.contains("\nChain ID: 4663\n"), "chain is bound");
        assert!(proof.message.contains("\nURI: https://shard-3.gates.example\n"),
                "the URI is built by the launcher from the domain");
        assert!(proof.message.contains("\nVersion: 1\n"), "EIP-4361 version line");

        // 2 · Recover the signer.
        let recovered = elo_vault::recover_signer(&proof.message, &proof.signature)
            .expect("a server must be able to ecrecover this");

        // 3 · It is the account that signed, and the SDK reported the same one.
        assert_eq!(recovered, account.address(),
                   "the signature must recover to the player's address");
        assert_eq!(proof.address.as_deref(), Some(account.address().as_str()),
                   "and the SDK must report the address that actually signed");
        // SIWE binds the address into the message, so the `address` field and
        // the recovered signer must agree — a server that trusted the field
        // without recovering would be reading a client's claim.
        assert!(proof.message.contains(&account.address()),
                "the address is bound into the signed bytes");

        // ⚠ 4 · A DIFFERENT nonce must not verify against this signature —
        //        otherwise a captured proof is a permanent skeleton key.
        let other = elo_broker::protocol::prove_message(
            "shard-3.gates.example", &account.address(), "gates",
            "aDifferentNonce1", &issued_at);
        let elsewhere = elo_vault::recover_signer(&other, &proof.signature)
            .expect("recovery still yields some address");
        assert_ne!(elsewhere, account.address(),
                   "a proof must not verify against a nonce it did not sign");

        // ⚠ 5 · Nor may it verify for a DIFFERENT game, which is what stops one
        //        title's proof admitting a player to another. The slug is in the
        //        statement line, so it is inside the signed bytes.
        let other_game = elo_broker::protocol::prove_message(
            "shard-3.gates.example", &account.address(), "some-other-title",
            "8f14e45fceea167a", &issued_at);
        let cross = elo_vault::recover_signer(&other_game, &proof.signature)
            .expect("recovery still yields some address");
        assert_ne!(cross, account.address(), "a proof is scoped to its game");

        // ⚠ 6 · Nor for a different DOMAIN — the property the old format did
        //        not have at all. A proof issued for one shard must not admit a
        //        player to another operator's server.
        let other_domain = elo_broker::protocol::prove_message(
            "evil.example", &account.address(), "gates",
            "8f14e45fceea167a", &issued_at);
        let elsewhere2 = elo_vault::recover_signer(&other_domain, &proof.signature)
            .expect("recovery still yields some address");
        assert_ne!(elsewhere2, account.address(),
                   "a proof is scoped to the domain it was issued for");
    });
}

/// `prove` asks no consent, so it works on a host that refuses everything —
/// the property the verb exists for, asserted through the real SDK.
#[test]
fn the_sdk_can_prove_even_when_the_player_refuses_signatures() {
    with_broker(false, |endpoint| {
        let mut o = connect(endpoint, "gates").expect("connect");
        assert!(o.sign("elo play\nx", "y").is_err(),
                "sign must still be refused by this host");
        // The nonce is 16 alphanumeric characters because EIP-4361 requires at
        // least 8 — the old free-text field took anything.
        assert!(o.prove("s.example", "n0nce4d8f1e2b7c").is_ok(),
                "prove authorises nothing, so a consent refusal must not block it");
    });
}
