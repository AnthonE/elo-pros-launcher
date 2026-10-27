//! What one approval buys, what one refusal stops, and which sign-ins need a
//! person — the consent rules a hostile game pushes on.
//!
//! Each test was written against the unfixed broker and failed there.

use elo_broker::protocol::{check_url, Refusal};
use elo_broker::{Host, Session, What};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use elo_broker::transport;
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::time::Duration;

const W: &str = "0xAbc0000000000000000000000000000000000001";

/// A host that records what the player would be shown and what got signed.
#[derive(Clone, Default)]
struct Seen {
    asked: Arc<Mutex<Vec<(String, String)>>>,
    signed: Arc<Mutex<Vec<String>>>,
}

struct Stub {
    seen: Seen,
    answer: Option<u32>,
    /// The hosts this title publishes as its own servers.
    shards: Vec<&'static str>,
}

impl Host for Stub {
    fn address(&self) -> Option<String> {
        Some(W.into())
    }
    fn signer_name(&self) -> String {
        "local".into()
    }
    fn host_url(&self) -> String {
        "https://elopros.com".into()
    }
    fn ask_consent(&mut self, _g: &str, family: &str, text: &str, _w: &str) -> Option<u32> {
        self.seen.asked.lock().unwrap().push((family.to_string(), text.to_string()));
        self.answer
    }
    fn sign(&mut self, text: &str, _why: &str) -> Result<Value, Refusal> {
        self.seen.signed.lock().unwrap().push(text.to_string());
        Ok(json!({"signature": "0x00"}))
    }
    fn prove(&mut self, message: &str) -> Result<Value, Refusal> {
        self.seen.signed.lock().unwrap().push(message.to_string());
        Ok(json!({"signature": "0x00", "address": W}))
    }
    fn serves(&mut self, _game: &str, server: &str) -> bool {
        self.shards.contains(&server)
    }
    fn title_url(&self, _slug: &str, _what: What) -> Option<String> {
        None
    }
    fn open(&mut self, _url: &str) -> Result<(), Refusal> {
        Ok(())
    }
}

fn stub(answer: Option<u32>) -> (Stub, Seen) {
    let seen = Seen::default();
    (Stub { seen: seen.clone(), answer, shards: vec!["shard-3.gates.example"] }, seen)
}

fn play(action: &str) -> String {
    format!("elo play\naction: {action}\nwallet: {}\nday: 2026-10-22\ndetail: x", W.to_lowercase())
}

// ── l-keys#1: prove for a server the title does not publish asks ───────────

#[test]
fn a_sign_in_to_a_site_the_title_does_not_serve_asks_the_player() {
    let (mut h, seen) = stub(None); // the player refuses
    let mut s = Session::new();
    s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
    let r = s.handle(&mut h, &json!({"op": "prove", "server": "app.thirdparty.example",
                                     "nonce": "Zq8LmN3pXw7Rt2Vb"}));
    assert_ne!(r["ok"], json!(true), "a third-party sign-in must not be signed unasked: {r}");
    assert!(seen.signed.lock().unwrap().is_empty(), "nothing was signed");
    let asked = seen.asked.lock().unwrap();
    assert_eq!(asked.len(), 1, "the player was asked");
    assert!(asked[0].1.contains("app.thirdparty.example wants you to sign in"),
            "the prompt shows the sign-in verbatim: {:?}", asked[0]);
}

#[test]
fn a_sign_in_to_the_titles_own_server_still_needs_no_prompt() {
    let (mut h, seen) = stub(None);
    let mut s = Session::new();
    s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
    let r = s.handle(&mut h, &json!({"op": "prove", "server": "shard-3.gates.example:7777",
                                     "nonce": "Zq8LmN3pXw7Rt2Vb"}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert!(seen.asked.lock().unwrap().is_empty());
}

// ── l-keys#3: a grant covers the action that was shown ─────────────────────

#[test]
fn a_grant_for_one_play_action_does_not_sign_another() {
    let (mut h, seen) = stub(Some(25)); // "Allow 25" on a duel
    let mut s = Session::new();
    s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
    assert_eq!(s.handle(&mut h, &json!({"op": "sign", "text": play("duel")}))["ok"], json!(true));
    assert_eq!(s.handle(&mut h, &json!({"op": "sign", "text": play("duel")}))["ok"], json!(true));
    assert_eq!(seen.asked.lock().unwrap().len(), 1, "the second duel rides the grant");
    for action in ["chatkey", "npub", "vote", "trade"] {
        let t = play(action);
        s.handle(&mut h, &json!({"op": "sign", "text": t}));
        let asked = seen.asked.lock().unwrap();
        assert_eq!(asked.last().unwrap().1, t, "{action} was shown to the player before signing");
    }
}

#[test]
fn an_identity_action_is_one_signature_per_prompt_whatever_was_clicked() {
    let (mut h, seen) = stub(Some(100)); // "Allow 100" on a chat key
    let mut s = Session::new();
    s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
    for _ in 0..3 {
        assert_eq!(s.handle(&mut h, &json!({"op": "sign", "text": play("chatkey")}))["ok"], json!(true));
    }
    assert_eq!(seen.asked.lock().unwrap().len(), 3, "every chatkey signature was asked for");
}

#[test]
fn a_family_with_no_action_is_one_signature_per_prompt_whatever_was_clicked() {
    // `consent_kind` narrows a grant by `action:`, and only `elo play` has
    // one. A counted grant on `elo dev` would be keyed on "dev" alone, so one
    // click would have signed 24 more dev-desk texts the player never saw.
    let (mut h, seen) = stub(Some(25));
    let mut s = Session::new();
    s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
    for (i, act) in ["edit", "post", "delegate"].iter().enumerate() {
        let t = format!("elo dev\nact: {act}\ngame: gates\nbody: {i}");
        assert_eq!(s.handle(&mut h, &json!({"op": "sign", "text": t}))["ok"], json!(true));
        assert_eq!(seen.asked.lock().unwrap().last().unwrap().1, t, "{act} was shown first");
    }
    assert_eq!(seen.asked.lock().unwrap().len(), 3);
    assert!(elo_broker::one_at_a_time("dev"));
    assert!(elo_broker::one_at_a_time("store buy"));
    assert!(!elo_broker::one_at_a_time("play duel"));
}

#[test]
fn an_unanswered_prompt_signs_nothing_and_is_not_remembered_as_a_no() {
    // The player was away when the prompt timed out. Refusing that one is
    // right; refusing every later duel until a restart is not.
    let (mut h, seen) = stub(Some(elo_broker::UNANSWERED));
    let mut s = Session::new();
    s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
    for _ in 0..2 {
        let r = s.handle(&mut h, &json!({"op": "sign", "text": play("duel")}));
        assert_ne!(r["ok"], json!(true), "{r}");
    }
    assert!(seen.signed.lock().unwrap().is_empty(), "nothing unanswered was signed");
    assert_eq!(seen.asked.lock().unwrap().len(), 2, "the second duel was asked again, not refused unseen");
}

// ── x-signed-text#3: every act a meter route verifies is asked for each time ─

#[test]
fn every_platform_act_the_meter_verifies_is_one_signature_per_prompt() {
    // The work board (munus `claim`/`submit`), GatesBench, buddy links, witness
    // pledges and readings share the `elo play` grammar with a game's moves; an
    // "Allow 100" must not let a game sign them on the player's behalf.
    for action in ["bench", "buddy", "claim", "submit", "pledge", "reading"] {
        let (mut h, seen) = stub(Some(100));
        let mut s = Session::new();
        s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
        for _ in 0..2 {
            assert_eq!(s.handle(&mut h, &json!({"op": "sign", "text": play(action)}))["ok"],
                       json!(true));
        }
        assert_eq!(seen.asked.lock().unwrap().len(), 2,
                   "every {action} signature was asked for");
    }
}

/// x-broker-play-family-consent#1: `buddy` carries `gift`, which moves the
/// player's dollar credit to a wallet named in the detail. A grant clicked on
/// one buddy text (a rename) must not sign the next one (a gift) unseen.
#[test]
fn a_buddy_signature_is_shown_every_time_whatever_was_clicked() {
    let (mut h, seen) = stub(Some(25));
    let mut s = Session::new();
    s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
    for _ in 0..3 {
        assert_eq!(s.handle(&mut h, &json!({"op": "sign", "text": play("buddy")}))["ok"], json!(true));
    }
    assert_eq!(seen.asked.lock().unwrap().len(), 3, "every buddy signature was asked for");
}

// ── l-keys#5: a refusal outlives the connection ────────────────────────────

#[cfg(unix)]
fn ask(path: &std::path::Path, game: &str, text: &str) -> Value {
    let s = std::os::unix::net::UnixStream::connect(path).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut w = s.try_clone().unwrap();
    let mut r = BufReader::new(s);
    let mut line = String::new();
    writeln!(w, "{}", json!({"op": "hello", "game": game})).unwrap();
    r.read_line(&mut line).unwrap();
    line.clear();
    writeln!(w, "{}", json!({"op": "sign", "text": text})).unwrap();
    r.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[cfg(unix)]
fn door(seen: Seen) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("launcher.sock");
    let listener = transport::Listener::bind(path.to_str().unwrap()).unwrap();
    std::thread::spawn(move || {
        transport::serve_forever(listener, move || Stub {
            seen: seen.clone(),
            answer: None,
            shards: vec![],
        })
    });
    (dir, path)
}

#[cfg(unix)]
#[test]
fn a_refusal_is_not_forgotten_by_reconnecting() {
    let seen = Seen::default();
    let (_dir, path) = door(seen.clone());
    for _ in 0..3 {
        let r = ask(&path, "gates", &play("duel"));
        assert_eq!(r["refused_by"], json!("the player"), "{r}");
    }
    assert_eq!(seen.asked.lock().unwrap().len(), 1, "the player was asked once, not per connection");
}

#[cfg(unix)]
#[test]
fn a_game_refused_again_and_again_stops_being_able_to_ask() {
    let seen = Seen::default();
    let (_dir, path) = door(seen.clone());
    for (i, action) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
        let r = ask(&path, "gates", &play(action));
        assert_eq!(r["refused_by"], json!("the player"), "{i}: {r}");
    }
    assert!(seen.asked.lock().unwrap().len() <= 3, "prompts stop after a few refusals: {:?}",
            seen.asked.lock().unwrap().len());
}

/// A player who takes a while to answer, and then refuses.
struct Slow {
    stub: Stub,
    open: Arc<std::sync::atomic::AtomicUsize>,
}

impl Host for Slow {
    fn address(&self) -> Option<String> {
        self.stub.address()
    }
    fn signer_name(&self) -> String {
        self.stub.signer_name()
    }
    fn host_url(&self) -> String {
        self.stub.host_url()
    }
    fn ask_consent(&mut self, g: &str, family: &str, text: &str, w: &str) -> Option<u32> {
        self.open.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(300));
        self.stub.ask_consent(g, family, text, w)
    }
    fn sign(&mut self, text: &str, why: &str) -> Result<Value, Refusal> {
        self.stub.sign(text, why)
    }
    fn prove(&mut self, message: &str) -> Result<Value, Refusal> {
        self.stub.prove(message)
    }
    fn serves(&mut self, game: &str, server: &str) -> bool {
        self.stub.serves(game, server)
    }
    fn title_url(&self, slug: &str, what: What) -> Option<String> {
        self.stub.title_url(slug, what)
    }
    fn open(&mut self, url: &str) -> Result<(), Refusal> {
        self.stub.open(url)
    }
}

#[test]
fn prompts_asked_at_once_on_many_connections_still_stop_at_the_limit() {
    // Six connections ask at the same moment, each for a different action.
    // The refusal count is zero for all of them when they pass the gate, so
    // without counting open prompts six windows queue for the player.
    let refusals = elo_broker::SharedRefusals::default();
    let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let start = Arc::new(std::sync::Barrier::new(6));
    let threads: Vec<_> = (0..6)
        .map(|i| {
            let (refusals, open, start) = (refusals.clone(), open.clone(), start.clone());
            std::thread::spawn(move || {
                let (stub, _seen) = stub(None);
                let mut host = Slow { stub, open };
                let mut c = elo_broker::Consent::sharing(refusals);
                let text = play(&format!("a{i}"));
                start.wait();
                c.spend(&mut host, "gates", &format!("elo play a{i}"), &text, "")
            })
        })
        .collect();
    for t in threads {
        assert!(t.join().unwrap().is_err(), "nothing was allowed");
    }
    let asked = open.load(std::sync::atomic::Ordering::SeqCst);
    assert!(asked <= elo_broker::REFUSALS_PER_GAME as usize,
            "{asked} prompts reached the player at once");
}

#[test]
fn a_connection_cannot_rename_its_game() {
    let (mut h, _seen) = stub(Some(1));
    let mut s = Session::new();
    s.handle(&mut h, &json!({"op": "hello", "game": "gates"}));
    let r = s.handle(&mut h, &json!({"op": "hello", "game": "other"}));
    assert_ne!(r["ok"], json!(true), "{r}");
    assert_eq!(s.game(), Some("gates"));
}

// ── l-keys#4: a url a shell could read as a command is refused ─────────────

#[test]
fn open_refuses_a_url_carrying_shell_metacharacters() {
    for bad in [
        r"https://x.example/&\\attacker.example@SSL\DavWWWRoot\p.exe",
        "https://x.example/\"&calc",
        "https://x.example/^&calc",
        "https://x.example/a|b",
        "https://x.example/a b",
        "https://x.example/<a>",
        "https://x.example/\u{202e}",
    ] {
        assert!(check_url(&json!({"url": bad})).is_err(), "{bad:?} should be refused");
    }
    for good in ["https://elopros.com/game.html?slug=gates&x=1", "https://x.example/a%20b#frag"] {
        check_url(&json!({"url": good})).unwrap_or_else(|e| panic!("{good:?}: {e:?}"));
    }
}
