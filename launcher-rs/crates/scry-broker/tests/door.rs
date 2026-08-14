//! The door's serving discipline: **two connections at once, both answered.**
//!
//! This is not a hypothetical. A game holds the connection it made at boot —
//! that is what the SDK's `Overlay` is for, and Gates keeps one for the life of
//! the process — and then opens a SECOND one when it needs a signature, because
//! `sign_siwe` calls `Scry::discover` again. A door that accepts one connection
//! at a time is parked inside the first one forever, so the second never gets
//! accepted, and on unix the game sits on a 300-second read timeout before
//! deciding there is no launcher.
//!
//! What that looks like from the outside is the part worth writing down: the
//! player's identity line prints fine — that is connection one, answered at
//! boot — and every signature afterwards fails as though no launcher were
//! running. `sdk/rust/scry_overlay.rs` already names it: *"a launcher that
//! accepts a connection and then never answers will hang the calling game …
//! That is a launcher bug in both cases."*

use scry_broker::protocol::Refusal;
use scry_broker::{transport, Host, What};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

struct Quiet;

impl Host for Quiet {
    fn address(&self) -> Option<String> {
        Some("0xabc0000000000000000000000000000000000001".into())
    }
    fn signer_name(&self) -> String {
        "local".into()
    }
    fn host_url(&self) -> String {
        "https://scry.moreright.xyz".into()
    }
    fn ask_consent(&mut self, _g: &str, _f: &str, _t: &str, _w: &str) -> Option<u32> {
        Some(1)
    }
    fn sign(&mut self, _text: &str, _why: &str) -> Result<Value, Refusal> {
        Ok(json!({"signature": "0x00"}))
    }
    fn title_url(&self, _slug: &str, _what: What) -> Option<String> {
        None
    }
    fn open(&mut self, _url: &str) -> Result<(), Refusal> {
        Ok(())
    }
}

/// One `hello`, with a read deadline so a door that never answers fails the
/// test instead of hanging it.
fn hello(path: &std::path::Path, game: &str) -> Option<(std::os::unix::net::UnixStream, Value)> {
    let s = std::os::unix::net::UnixStream::connect(path).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut w = s.try_clone().ok()?;
    let mut r = BufReader::new(s.try_clone().ok()?);
    writeln!(
        w,
        "{}",
        json!({"op": "hello", "game": game, "version": "0.0.0", "protocol": 1})
    )
    .ok()?;
    w.flush().ok()?;
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok().map(|v| (s, v))
}

/// **A second connection is answered while the first is still open.**
///
/// The first stream is deliberately still in scope and never closed when the
/// second one is made — that is precisely the state a running game leaves the
/// door in, and the state in which this used to deadlock.
#[test]
fn a_game_holding_its_boot_connection_can_still_open_a_second_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("launcher.sock");
    let listener = transport::Listener::bind(path.to_str().unwrap()).expect("bind");
    std::thread::spawn(move || transport::serve_forever(listener, || Quiet));

    let (boot, first) = hello(&path, "gates").expect("the boot connection is answered");
    assert!(first.get("ok").is_some() || first.get("hello").is_some(), "{first}");

    // Held open, exactly as a game holds its overlay for the life of the
    // process. If the door serves one connection at a time, nothing below
    // this line ever completes.
    let (_second, reply) = hello(&path, "gates")
        .expect("a second connection must be answered while the first is open — \
                 this is how every signature after boot reaches the launcher");
    assert!(reply.get("ok").is_some() || reply.get("hello").is_some(), "{reply}");

    drop(boot);
}

/// The door outlives a connection that dies mid-request.
#[test]
fn a_hung_up_connection_does_not_take_the_door_with_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("launcher.sock");
    let listener = transport::Listener::bind(path.to_str().unwrap()).expect("bind");
    std::thread::spawn(move || transport::serve_forever(listener, || Quiet));

    {
        let s = std::os::unix::net::UnixStream::connect(&path).expect("connect");
        drop(s); // gone without saying anything
    }

    let (_s, reply) = hello(&path, "gates").expect("the door still answers");
    assert!(reply.get("ok").is_some() || reply.get("hello").is_some(), "{reply}");
}
