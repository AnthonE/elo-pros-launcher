//! The door's serving discipline: **two connections at once, both answered.**
//!
//! This is not a hypothetical. A game holds the connection it made at boot —
//! that is what the SDK's `Overlay` is for, and Gates keeps one for the life of
//! the process — and then opens a SECOND one when it needs a signature, because
//! `sign_siwe` calls `Elo::discover` again. A door that accepts one connection
//! at a time is parked inside the first one forever, so the second never gets
//! accepted, and on unix the game sits on a 300-second read timeout before
//! deciding there is no launcher.
//!
//! What that looks like from the outside is the part worth writing down: the
//! player's identity line prints fine — that is connection one, answered at
//! boot — and every signature afterwards fails as though no launcher were
//! running. `sdk/rust/elo_overlay.rs` already names it: *"a launcher that
//! accepts a connection and then never answers will hang the calling game …
//! That is a launcher bug in both cases."*

use elo_broker::protocol::Refusal;
use elo_broker::{transport, Host, What};
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
        "https://elopros.com".into()
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

/// **The open count says a game is running for as long as it holds the door.**
///
/// The window reads it so the idle relock does not put the passphrase gate up
/// over a game in play (`elo_ui::wiring::serve_relock`). It must rise while
/// the boot connection is held and fall back to zero when the game leaves,
/// or the relock either fires mid-game or never fires again.
#[test]
fn the_open_count_follows_a_game_holding_the_door() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("launcher.sock");
    let listener = transport::Listener::bind(path.to_str().unwrap()).expect("bind");
    let open = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&open);
    std::thread::spawn(move || transport::serve_forever_counted(listener, counted, || Quiet));

    assert_eq!(open.load(Ordering::Relaxed), 0, "nobody is playing yet");
    let (boot, _) = hello(&path, "gates").expect("the boot connection is answered");
    assert_eq!(open.load(Ordering::Relaxed), 1, "a game holding the door is counted");

    drop(boot);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while open.load(Ordering::Relaxed) != 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(open.load(Ordering::Relaxed), 0, "a game that left is not counted");
}

/// **A live door is refused, a dead one is replaced.**
///
/// The second launcher used to unlink the first one's socket and bind over
/// it, and when the first quit its `Drop` deleted the second's — after which
/// no game could find a launcher that was plainly open.
#[test]
fn a_second_launcher_cannot_take_a_live_door_and_a_stale_one_is_replaced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("launcher.sock");
    let first = transport::Listener::bind(path.to_str().unwrap()).expect("bind");
    std::thread::spawn(move || transport::serve_forever(first, || Quiet));
    hello(&path, "gates").expect("the first door answers");

    let err = transport::Listener::bind(path.to_str().unwrap())
        .err()
        .expect("a second bind on a live door must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse, "{err}");
    hello(&path, "gates").expect("and the first door still answers");

    // A socket file nobody is listening on is a crashed launcher's leftover.
    let stale = dir.path().join("stale.sock");
    drop(std::os::unix::net::UnixListener::bind(&stale).expect("make a stale socket"));
    assert!(stale.exists());
    let fresh = transport::Listener::bind(stale.to_str().unwrap()).expect("a stale door is replaced");

    // A listener whose path now names someone else's socket leaves it alone.
    std::fs::remove_file(&stale).unwrap();
    let other = std::os::unix::net::UnixListener::bind(&stale).expect("another socket at the path");
    drop(fresh);
    assert!(stale.exists(), "dropping a listener deleted a socket it did not make");
    drop(other);
}
