//! The wire, against `sdk/PROTOCOL.md`.
//!
//! Driven over a pair of in-memory buffers rather than a real socket: the
//! protocol is the thing under test here and the transport has its own file.
//! Every assertion below is a sentence from the spec, and where the spec says
//! two outcomes are *not the same*, this checks they are distinguishable by a
//! machine and not only by a human reading `reason`.

use elo_broker::protocol::Refusal;
use elo_broker::{serve_conn, Host, What};
use serde_json::{json, Value};

/// A host that answers everything, records what it was asked, and never
/// prompts a human. `consent` is what the player would have said.
struct Fake {
    consent: Option<u32>,
    asked: Vec<(String, String)>,
    /// Everything the prompt was handed to show the player, per ask. Recorded
    /// separately from `asked` because these are the untrusted halves — the
    /// message and the game's stated reason — and a prompt that never received
    /// them would still pass every count-based check above.
    shown: Vec<(String, String)>,
    opened: Vec<String>,
    signed: usize,
}

impl Default for Fake {
    fn default() -> Self {
        Fake { consent: Some(1), asked: vec![], shown: vec![], opened: vec![], signed: 0 }
    }
}

impl Host for Fake {
    fn address(&self) -> Option<String> {
        Some("0xabc0000000000000000000000000000000000001".into())
    }
    fn signer_name(&self) -> String { "arca".into() }
    fn host_url(&self) -> String { "https://elopros.com".into() }
    fn ask_consent(&mut self, game: &str, family: &str, text: &str, why: &str) -> Option<u32> {
        self.asked.push((game.into(), family.into()));
        self.shown.push((text.into(), why.into()));
        self.consent
    }
    fn sign(&mut self, _text: &str, _why: &str) -> Result<Value, Refusal> {
        self.signed += 1;
        Ok(json!({"signature": "0xdead", "address": "0xabc", "backend": "arca"}))
    }
    fn title_url(&self, slug: &str, what: What) -> Option<String> {
        match (slug, what) {
            ("gates", What::Manifest) => Some("https://o/api/launcher/manifest/gates".into()),
            ("gates", What::Servers) => None, // Gates publishes no shard list yet
            _ => None,
        }
    }
    fn open(&mut self, url: &str) -> Result<(), Refusal> {
        self.opened.push(url.into());
        Ok(())
    }
}

/// Drive a scripted conversation and return one reply per request.
fn talk(host: &mut Fake, reqs: &[Value]) -> Vec<Value> {
    let mut input = String::new();
    for r in reqs {
        input.push_str(&r.to_string());
        input.push('\n');
    }
    let stream = Duplex { input: std::io::Cursor::new(input.into_bytes()), output: Vec::new() };
    let mut stream = stream;
    serve_conn(&mut stream, host).expect("serve");
    String::from_utf8(stream.output)
        .expect("utf8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("reply is JSON"))
        .collect()
}

struct Duplex {
    input: std::io::Cursor<Vec<u8>>,
    output: Vec<u8>,
}
impl std::io::Read for Duplex {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> { self.input.read(b) }
}
impl std::io::Write for Duplex {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.output.write(b) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

fn hello() -> Value {
    json!({"op": "hello", "game": "gates", "protocol": 1, "version": "0.1.0"})
}

#[test]
fn hello_answers_with_the_vocabulary_and_the_signer() {
    let mut h = Fake::default();
    let out = talk(&mut h, &[hello()]);
    assert_eq!(out[0]["ok"], json!(true));
    assert_eq!(out[0]["protocol"], json!(1));
    assert_eq!(out[0]["signer"], json!("arca"));
    let caps = out[0]["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "sign"), "capabilities must list the verbs");
}

/// The rule that keeps a consent prompt meaningful.
#[test]
fn every_verb_refuses_until_hello_has_named_the_game() {
    let mut h = Fake::default();
    for op in ["identity", "sign", "title", "servers", "open", "overlay"] {
        let out = talk(&mut h, &[json!({"op": op})]);
        assert_eq!(out[0]["ok"], json!(false), "{op} answered before hello");
        assert!(
            out[0]["reason"].as_str().unwrap().contains("hello"),
            "{op}'s refusal should say to say hello: {}", out[0]["reason"]
        );
    }
}

/// An unknown field is a REFUSAL, never an ignore — the caller that thinks it
/// set something and was quietly dropped has been misled in the dangerous
/// direction.
#[test]
fn an_unknown_field_is_refused_and_named() {
    let mut h = Fake::default();
    let out = talk(&mut h, &[
        hello(),
        json!({"op": "sign", "text": "elo play\nx", "budget": 999}),
    ]);
    assert_eq!(out[1]["ok"], json!(false));
    let reason = out[1]["reason"].as_str().unwrap();
    assert!(reason.contains("budget"), "the refusal must name the field: {reason}");
    assert_eq!(h.signed, 0, "nothing may be signed for a refused request");
}

#[test]
fn an_unknown_op_lists_what_the_door_speaks() {
    let mut h = Fake::default();
    let out = talk(&mut h, &[hello(), json!({"op": "install", "slug": "gates"})]);
    assert_eq!(out[1]["ok"], json!(false));
    let reason = out[1]["reason"].as_str().unwrap();
    assert!(reason.contains("install"), "name what was asked: {reason}");
    assert!(reason.contains("sign"), "and list what is available: {reason}");
}

/// Nothing in the vocabulary operates the launcher. This is asserted as an
/// absence because that is the property: a verb that installs, uninstalls or
/// picks a signer must never appear, and a test that only checks the verbs
/// that DO exist would not notice one arriving.
#[test]
fn the_vocabulary_cannot_operate_the_launcher() {
    let caps = elo_broker::capabilities();
    for forbidden in ["install", "uninstall", "update", "play", "launch",
                      "settings", "signer", "read", "write", "prune"] {
        assert!(
            !caps.contains(&forbidden),
            "{forbidden:?} is in the vocabulary. A game may ask about itself and \
             ask for a signature; it may not operate the launcher."
        );
    }
    // ⚠ **A budget, not a fact.** 6 → 7 bought the overlay; 7 → 8 bought
    // `prove` (2026-08-06); **8 → 9 buys `profile` (2026-08-07)**, and it is
    // spent in the commit that adds the verb, the way the dependency cap is.
    //
    // What `prove` bought: a signature proving who is playing WITHOUT a consent
    // prompt, because the launcher composes the message and the game
    // contributes only two inert fields. Adding it to `sign` instead would have
    // been free here and wrong there — it would have meant auto-approving text
    // a game wrote.
    //
    // What `profile` buys: a name and a face for an address, so a game can draw
    // a scoreboard without every title reimplementing the join across sworn
    // name, npub, vow and wallet. **It is a READ and authorises nothing** — no
    // signature, no consent, no key. It could have been left to the game (fetch
    // `/api/who` yourself, as `servers` leaves the shard list), and the reason
    // it is not is that the vendored Rust SDK is `std`-only with no HTTP
    // client: a game linking it has no way to make the call. The launcher
    // already carries a reader that keeps *unreachable* apart from *empty*, and
    // that distinction is the easiest thing in this system to get wrong.
    //
    // The wire is the whole of a game's authority, so the next verb costs the
    // same deliberate edit and the same paragraph.
    assert_eq!(caps.len(), 9, "the vocabulary grew — was that deliberate? {caps:?}");
}

#[test]
fn a_signable_message_must_declare_its_family_on_the_first_line() {
    let mut h = Fake::default();
    let out = talk(&mut h, &[
        hello(),
        json!({"op": "sign", "text": "just sign this please", "why": "trust me"}),
    ]);
    assert_eq!(out[1]["ok"], json!(false));
    assert_eq!(h.signed, 0);
    assert!(out[1]["reason"].as_str().unwrap().contains("elo <family>"));
}

#[test]
fn consent_is_counted_and_a_second_ask_reprompts() {
    let mut h = Fake::default();
    h.consent = Some(1); // the player allows exactly one
    let sign = json!({"op": "sign", "text": "elo play\nround 41", "why": "settling"});
    let out = talk(&mut h, &[hello(), sign.clone(), sign.clone()]);
    assert_eq!(out[1]["ok"], json!(true), "the granted one signs");
    assert_eq!(out[1]["family"], json!("play"));
    assert_eq!(out[2]["ok"], json!(true), "the second re-asks and is allowed again");
    assert_eq!(h.asked.len(), 2, "the grant was spent, so the player was asked twice");
}

/// The prompt is handed the whole message and the game's own reason.
///
/// **Both are needed and neither is decoration.** The message is the thing
/// being signed, so a prompt without it is asking a player to approve a
/// description (`docs/client/SDK.md` §4 — the summary never replaces the text); the
/// `why` is the only place a game says what it is *for*, and it reached
/// nothing but a browser handoff url until this was wired.
///
/// ⚠ The test asserts they arrive **unaltered**. Flattening `why` for display
/// is the launcher's job and is done where it is drawn
/// (`elo_ui::consent::one_line`); a broker that sanitised on the way through
/// would hand every future host a value it could no longer tell had been
/// tampered with.
#[test]
fn the_prompt_is_given_the_message_and_the_games_reason() {
    let mut h = Fake::default();
    let text = "elo play\naction: duel\nvow: vow_x\ndetail: ETH up 5";
    let out = talk(&mut h, &[hello(), json!({
        "op": "sign", "text": text, "why": "settling round 41",
    })]);
    assert_eq!(out[1]["ok"], json!(true));
    let (shown, why) = h.shown.first().expect("the player must have been asked");
    assert_eq!(shown, text, "the prompt must get the message verbatim");
    assert_eq!(why, "settling round 41", "the game's reason must reach the player");
}

/// A game that gives no reason is not refused — it is just terse, and the
/// prompt says so rather than inventing one.
#[test]
fn a_sign_with_no_why_still_asks() {
    let mut h = Fake::default();
    let out = talk(&mut h, &[hello(), json!({"op": "sign", "text": "elo play\nx"})]);
    assert_eq!(out[1]["ok"], json!(true));
    assert_eq!(h.shown[0].1, "", "an absent reason arrives as empty, not as a default");
}

/// A refusal by the player is remembered for the session, so a game cannot
/// re-prompt in a loop. That is a different outcome from a spent grant and the
/// test asserts the ask count, not just the reply.
#[test]
fn a_players_refusal_stops_the_asking() {
    let mut h = Fake::default();
    h.consent = None; // refused
    let sign = json!({"op": "sign", "text": "elo play\nx", "why": "y"});
    let out = talk(&mut h, &[hello(), sign.clone(), sign.clone(), sign.clone()]);
    for r in &out[1..] {
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["refused_by"], json!("the player"),
                   "a player refusal is machine-distinguishable, not just prose");
    }
    assert_eq!(h.asked.len(), 1, "asked once, then remembered — never a prompt loop");
    assert_eq!(h.signed, 0);
}

#[test]
fn open_is_https_only() {
    let mut h = Fake::default();
    let out = talk(&mut h, &[
        hello(),
        json!({"op": "open", "url": "file:///etc/passwd"}),
        json!({"op": "open", "url": "javascript:alert(1)"}),
        json!({"op": "open", "url": "http://elopros.com"}),
        json!({"op": "open", "url": "https://elopros.com/wallet.html"}),
    ]);
    for (i, r) in out[1..4].iter().enumerate() {
        assert_eq!(r["ok"], json!(false), "reply {i} should be refused");
    }
    assert_eq!(out[4]["ok"], json!(true));
    assert_eq!(h.opened, vec!["https://elopros.com/wallet.html"],
               "only the https one was ever opened");
}

/// `servers` answers with a URL and never with the list. Gates publishes none
/// yet, so the honest answer is a refusal that says so — not an empty array,
/// which would read as "there are no shards".
#[test]
fn servers_answers_a_url_and_says_so_when_there_is_none() {
    let mut h = Fake::default();
    let out = talk(&mut h, &[
        hello(),
        json!({"op": "title", "slug": "gates"}),
        json!({"op": "servers", "slug": "gates"}),
    ]);
    assert_eq!(out[1]["ok"], json!(true));
    assert!(out[1]["url"].as_str().unwrap().contains("/manifest/gates"));
    assert!(out[1]["note"].as_str().unwrap().contains("does not proxy"));

    assert_eq!(out[2]["ok"], json!(false), "no shard list is published");
    assert!(out[2]["reason"].as_str().unwrap().contains("shard list"));
    assert!(out[2].get("servers").is_none(), "never an empty list — that is a claim");
}

#[test]
fn a_bad_line_is_answered_rather_than_disconnected() {
    let mut h = Fake::default();
    let mut stream = Duplex {
        input: std::io::Cursor::new(b"not json at all\n{\"op\":\"hello\",\"game\":\"gates\"}\n".to_vec()),
        output: Vec::new(),
    };
    serve_conn(&mut stream, &mut h).expect("a framing bug must not kill the connection");
    let lines: Vec<&str> = std::str::from_utf8(&stream.output).unwrap().lines().collect();
    assert_eq!(lines.len(), 2, "both lines answered; the socket stayed open");
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["ok"], json!(false));
    let second: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(second["ok"], json!(true), "and hello still worked afterwards");
}

#[test]
fn a_hostile_game_name_is_refused_before_it_reaches_a_path() {
    let mut h = Fake::default();
    for bad in ["../../etc", "a/b", "", "x".repeat(65).as_str()] {
        let out = talk(&mut h, &[json!({"op": "hello", "game": bad, "protocol": 1})]);
        assert_eq!(out[0]["ok"], json!(false), "{bad:?} should be refused as a slug");
    }
}

// ── prove — the verb whose safety is that the game cannot write the message ──

/// A host that proves with a fixed signature and records what it was asked to
/// sign, so a test can assert on the BYTES rather than on the reply.
struct Prover {
    seen: std::cell::RefCell<Vec<String>>,
    consents: std::cell::RefCell<u32>,
}
impl Default for Prover {
    fn default() -> Self {
        Prover { seen: Default::default(), consents: Default::default() }
    }
}
impl elo_broker::Host for Prover {
    // EIP-55 checksummed, because SIWE binds this into the signed bytes and a
    // lowercase address makes the proof unverifiable (see `Host::address`).
    fn address(&self) -> Option<String> {
        Some("0x24445EFddB08d4938E3E3627042B2Cf4063d9092".into())
    }
    fn signer_name(&self) -> String { "local".into() }
    fn host_url(&self) -> String { "https://elo.example".into() }
    fn ask_consent(&mut self, _g: &str, _f: &str, _t: &str, _w: &str) -> Option<u32> {
        *self.consents.borrow_mut() += 1;
        Some(1)
    }
    fn sign(&mut self, _t: &str, _w: &str) -> Result<serde_json::Value, elo_broker::protocol::Refusal> {
        Err(elo_broker::protocol::Refusal::new("no"))
    }
    fn prove(&mut self, message: &str) -> Result<serde_json::Value, elo_broker::protocol::Refusal> {
        self.seen.borrow_mut().push(message.to_string());
        Ok(serde_json::json!({
            "signature": "0xsig",
            // Must match `address()` or the broker refuses — a proof signed by
            // an account other than the one named in the message can never
            // verify.
            "address": "0x24445EFddB08d4938E3E3627042B2Cf4063d9092",
        }))
    }
    fn title_url(&self, _s: &str, _w: elo_broker::What) -> Option<String> { None }
    fn open(&mut self, _u: &str) -> Result<(), elo_broker::protocol::Refusal> { Ok(()) }
}

fn prove_call(host: &mut Prover, nonce: &str, server: &str) -> serde_json::Value {
    let mut s = elo_broker::Session::default();
    s.handle(host, &serde_json::json!({"op":"hello","game":"gates","protocol":1}));
    s.handle(host, &serde_json::json!({"op":"prove","nonce":nonce,"server":server}))
}

fn prove_call_at(host: &mut Prover, nonce: &str, server: &str, at: serde_json::Value)
    -> serde_json::Value
{
    let mut s = elo_broker::Session::default();
    s.handle(host, &serde_json::json!({"op":"hello","game":"gates","protocol":1}));
    s.handle(
        host,
        &serde_json::json!({"op":"prove","nonce":nonce,"server":server,"issued_at":at}),
    )
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// **The reason `issued_at` exists.** A shard that stamps its own challenge
/// cannot verify a `prove` whose timestamp came off the launcher's clock — it
/// can rebuild every other field and then has to guess a string. Guessing is
/// not verification, so those servers reached for `sign` instead and were
/// refused, because `sign_family` wants `elo <family>` and a SIWE message
/// does not start that way. That was every Gates login, refused as a guest.
#[test]
fn prove_takes_the_callers_issued_at_so_a_server_can_recompute() {
    let mut host = Prover::default();
    let at = now_secs() - 5;
    let out = prove_call_at(&mut host, "8f14e45fceea167a", "gates.example", serde_json::json!(at));
    assert_eq!(out["ok"], serde_json::json!(true), "{out}");

    let signed = host.seen.borrow()[0].clone();
    assert!(
        signed.ends_with(&format!("Issued At: {}", elo_broker::protocol::iso8601(at))),
        "the launcher stamps the second the caller chose, formatted its own way: {signed}"
    );
    // The whole message, rebuilt from what a server already knows. This is the
    // claim the feature makes, so it is asserted as a whole rather than by line.
    assert_eq!(
        signed,
        elo_broker::protocol::prove_message(
            "gates.example",
            "0x24445EFddB08d4938E3E3627042B2Cf4063d9092",
            "gates",
            "8f14e45fceea167a",
            &elo_broker::protocol::iso8601(at),
        ),
        "a verifier that knows domain, address, game, nonce and second rebuilds it exactly"
    );
}

/// Absent is the old behaviour, because a launcher serves games it was not
/// built alongside and an added field must not change what they get.
#[test]
fn prove_without_an_issued_at_still_uses_the_launchers_clock() {
    let mut host = Prover::default();
    let out = prove_call(&mut host, "8f14e45fceea167a", "gates.example");
    assert_eq!(out["ok"], serde_json::json!(true), "{out}");
    let signed = host.seen.borrow()[0].clone();
    let stamped = signed
        .lines()
        .find_map(|l| l.strip_prefix("Issued At: "))
        .expect("EIP-4361 requires Issued At");
    assert!(
        (now_secs() - 5..=now_secs() + 5)
            .map(elo_broker::protocol::iso8601)
            .any(|s| s == stamped),
        "an omitted issued_at is now, not zero and not an error: {stamped}"
    );
}

/// The bound is the player's protection, not the caller's convenience: a
/// message dated last year is a replay wearing a fresh coat, and one dated
/// next year outlives the moment the player agreed to.
#[test]
fn an_issued_at_far_from_this_clock_is_refused() {
    for skew in [-86_400i64, 86_400] {
        let mut host = Prover::default();
        let out = prove_call_at(
            &mut host,
            "8f14e45fceea167a",
            "gates.example",
            serde_json::json!(now_secs() + skew),
        );
        assert_ne!(out["ok"], serde_json::json!(true), "skew {skew} must not sign");
        assert!(
            host.seen.borrow().is_empty(),
            "refused BEFORE the signer was asked — skew {skew}"
        );
    }
}

/// A string is not a second. Refused rather than parsed, because a launcher
/// that guesses at a timestamp format signs a message nobody can rebuild.
#[test]
fn an_issued_at_that_is_not_a_number_is_refused() {
    let mut host = Prover::default();
    let out = prove_call_at(
        &mut host,
        "8f14e45fceea167a",
        "gates.example",
        serde_json::json!("2026-08-15T00:00:00Z"),
    );
    assert_ne!(out["ok"], serde_json::json!(true), "{out}");
    assert!(host.seen.borrow().is_empty(), "nothing was signed");
}

#[test]
fn prove_signs_a_message_the_launcher_wrote() {
    let mut host = Prover::default();
    let out = prove_call(&mut host, "8f14e45fceea167a", "shard-3.gates.example");
    assert_eq!(out["ok"], serde_json::json!(true), "{out}");
    let signed = host.seen.borrow()[0].clone();

    // A SIWE (EIP-4361) message. `Issued At` is the launcher's clock, so the
    // fixed part is asserted line by line rather than byte for byte.
    let issued_at = signed
        .lines()
        .find_map(|l| l.strip_prefix("Issued At: "))
        .expect("EIP-4361 requires Issued At");
    assert_eq!(
        signed,
        elo_broker::protocol::prove_message(
            "shard-3.gates.example",
            "0x24445EFddB08d4938E3E3627042B2Cf4063d9092",
            "gates",
            "8f14e45fceea167a",
            issued_at
        ),
        "the launcher composes every word of the message"
    );
    // The domain leads, which is what binds a proof to one server.
    assert!(signed.starts_with(
        "shard-3.gates.example wants you to sign in with your Ethereum account:\n\
         0x24445EFddB08d4938E3E3627042B2Cf4063d9092\n"));
    // The game names itself in the statement and nowhere a game controls.
    assert!(signed.contains("\nProve your identity to play gates."));
    assert!(signed.contains("\nURI: https://shard-3.gates.example\nVersion: 1\n"));
    assert!(signed.contains("\nChain ID: 4663\nNonce: 8f14e45fceea167a\n"));

    // The reply echoes it, names the scheme, and says not to trust the echo.
    assert_eq!(out["message"], serde_json::json!(signed));
    assert_eq!(out["scheme"], serde_json::json!("eip4361"));
    assert!(out["verify"].as_str().unwrap().contains("siwe"));
}

/// ⚠ A proof composed for one account and signed by another can never verify,
/// because SIWE names the address inside the bytes. Refusing beats handing the
/// game something no verifier will accept for a reason it cannot see.
#[test]
fn a_proof_signed_by_the_wrong_account_is_refused() {
    struct Mismatched;
    impl elo_broker::Host for Mismatched {
        fn address(&self) -> Option<String> {
            Some("0x24445EFddB08d4938E3E3627042B2Cf4063d9092".into())
        }
        fn signer_name(&self) -> String { "local".into() }
        fn host_url(&self) -> String { "https://x.example".into() }
        fn ask_consent(&mut self, _g: &str, _f: &str, _t: &str, _w: &str) -> Option<u32> { None }
        fn sign(&mut self, _t: &str, _w: &str)
            -> Result<serde_json::Value, elo_broker::protocol::Refusal> {
            Err(elo_broker::protocol::Refusal::new("no"))
        }
        fn prove(&mut self, _m: &str)
            -> Result<serde_json::Value, elo_broker::protocol::Refusal> {
            Ok(serde_json::json!({
                "signature": "0xsig",
                "address": "0x000000000000000000000000000000000000dEaD",
            }))
        }
        fn title_url(&self, _s: &str, _w: elo_broker::What) -> Option<String> { None }
        fn open(&mut self, _u: &str) -> Result<(), elo_broker::protocol::Refusal> { Ok(()) }
    }
    let mut s = elo_broker::Session::default();
    let mut h = Mismatched;
    s.handle(&mut h, &serde_json::json!({"op":"hello","game":"gates","protocol":1}));
    let out = s.handle(&mut h, &serde_json::json!({
        "op":"prove","nonce":"8f14e45fceea167a","server":"s.example"}));
    assert_eq!(out["ok"], serde_json::json!(false));
    assert!(out["reason"].as_str().unwrap().contains("cannot verify against its own message"),
            "{out}");
}

/// **No prompt, and that is the whole point of the verb.**
#[test]
fn prove_never_asks_for_consent() {
    let mut host = Prover::default();
    prove_call(&mut host, "abc", "s.example");
    assert_eq!(*host.consents.borrow(), 0,
               "prove authorises nothing, so it must not spend a consent grant");
}

/// The game cannot prove as another title: `game` comes from `hello`, and a
/// `game` field on the request is not in the verb's allowed set.
#[test]
fn a_game_cannot_prove_as_someone_else() {
    let mut host = Prover::default();
    let mut s = elo_broker::Session::default();
    s.handle(&mut host, &serde_json::json!({"op":"hello","game":"gates","protocol":1}));
    let out = s.handle(&mut host, &serde_json::json!({
        "op":"prove","nonce":"n","server":"s","game":"some-other-title"}));
    assert_eq!(out["ok"], serde_json::json!(false),
               "an unknown field is a refusal, never an ignore: {out}");
}

/// ⚠ **The injection this verb would otherwise have.** A nonce carrying a
/// newline would let the game append lines to a message the launcher believes
/// it authored — smuggling its own sentence into an unprompted signature,
/// which is exactly what `prove` exists to prevent.
#[test]
fn a_field_that_could_add_a_line_is_refused() {
    for bad in [
        "x\nplay:transfer everything",
        "x\r\nserver:evil.example",
        "x\nnonce:0",
        "ok but then\na line",
    ] {
        let mut host = Prover::default();
        let out = prove_call(&mut host, bad, "s.example");
        assert_eq!(out["ok"], serde_json::json!(false),
                   "a nonce containing a newline must be refused: {bad:?} -> {out}");
        assert!(host.seen.borrow().is_empty(),
                "and nothing may reach the signer: {bad:?}");
    }
    // The server field is interpolated too, and gets the same guard.
    let mut host = Prover::default();
    let out = prove_call(&mut host, "n", "evil.example\nnonce:attacker-chosen");
    assert_eq!(out["ok"], serde_json::json!(false), "{out}");
    assert!(host.seen.borrow().is_empty());
}

#[test]
fn empty_and_oversized_fields_are_refused() {
    let mut host = Prover::default();
    assert_eq!(prove_call(&mut host, "", "s.example")["ok"], serde_json::json!(false));
    assert_eq!(prove_call(&mut host, "n", "")["ok"], serde_json::json!(false));
    let long = "a".repeat(129);
    assert_eq!(prove_call(&mut host, &long, "s.example")["ok"], serde_json::json!(false));
    assert!(host.seen.borrow().is_empty(), "nothing reached the signer");
}

/// The shapes a real server issues, under EIP-4361's alphabet.
///
/// ⚠ **This test used to assert the opposite for two of these, and the change
/// is a real cost to game developers rather than a tightening we chose.** The
/// old free-text guard took a dash-separated uuid and a base64url string;
/// EIP-4361 requires the nonce to be **alphanumeric, at least 8 characters**,
/// so those must now be normalised — a uuid with its dashes stripped is 32 hex
/// characters and fine. That is the price of a message any `siwe` library can
/// parse, and `sdk/PROTOCOL.md` says it in the place a developer will hit it.
#[test]
fn the_nonces_a_server_would_really_issue_are_accepted() {
    for good in [
        "8f14e45fceea167a5a36dedd4bea2543",   // hex, the common case
        "550e8400e29b41d4a716446655440000",   // a uuid with dashes stripped
        "abc123XYZ",                           // base62, 9 chars
        "aB3dE6gH",                            // the 8-character floor exactly
    ] {
        let mut host = Prover::default();
        let out = prove_call(&mut host, good, "shard-3.gates.example");
        assert_eq!(out["ok"], serde_json::json!(true), "{good:?} should be fine: {out}");
    }
}

/// What the EIP-4361 alphabet now refuses, and each one names its own fix.
#[test]
fn nonces_the_standard_does_not_allow_are_refused_with_a_reason() {
    for (bad, expect) in [
        ("550e8400-e29b-41d4-a716-446655440000", "alphanumeric"),  // strip the dashes
        ("abc_-123+/=", "alphanumeric"),                            // base64url
        ("short7", "at least 8"),                                   // under the floor
        ("nonce\nNonce: other", "alphanumeric"),                    // the injection
    ] {
        let mut host = Prover::default();
        let out = prove_call(&mut host, bad, "shard-3.gates.example");
        assert_eq!(out["ok"], serde_json::json!(false), "{bad:?} must be refused");
        assert!(out["reason"].as_str().unwrap().contains(expect),
                "{bad:?} should say {expect:?}: {out}");
        assert!(host.seen.borrow().is_empty(), "nothing reached the signer");
    }
}

/// The `server` field is a DOMAIN now, not free text — it lands in the signed
/// bytes twice, and EIP-4361 binds it as an authority.
#[test]
fn the_domain_is_an_authority_and_not_a_url() {
    for (bad, expect) in [
        ("https://shard-3.gates.example", "not a url"),
        ("shard-3.gates.example/play", "not a url"),
        ("shard-3.gates.example:notaport", "port"),
        ("bad host.example", "not a hostname"),
        ("a\nb", "not a hostname"),
    ] {
        let mut host = Prover::default();
        let out = prove_call(&mut host, "8f14e45fceea167a", bad);
        assert_eq!(out["ok"], serde_json::json!(false), "{bad:?} must be refused");
        assert!(out["reason"].as_str().unwrap().contains(expect),
                "{bad:?} should say {expect:?}: {out}");
    }
    // A port is legitimate — a shard on 7777 is an ordinary thing.
    let mut host = Prover::default();
    assert_eq!(prove_call(&mut host, "8f14e45fceea167a", "shard-3.gates.example:7777")["ok"],
               serde_json::json!(true));
}

/// A host that has not implemented `prove` refuses rather than silently
/// acquiring the ability — the trait's default.
#[test]
fn a_host_that_did_not_implement_prove_refuses() {
    struct Bare;
    impl elo_broker::Host for Bare {
        fn address(&self) -> Option<String> { None }
        fn signer_name(&self) -> String { "none".into() }
        fn host_url(&self) -> String { "https://x.example".into() }
        fn ask_consent(&mut self, _g: &str, _f: &str, _t: &str, _w: &str) -> Option<u32> { None }
        fn sign(&mut self, _t: &str, _w: &str) -> Result<serde_json::Value, elo_broker::protocol::Refusal> {
            Err(elo_broker::protocol::Refusal::new("no"))
        }
        fn title_url(&self, _s: &str, _w: elo_broker::What) -> Option<String> { None }
        fn open(&mut self, _u: &str) -> Result<(), elo_broker::protocol::Refusal> { Ok(()) }
    }
    let mut host = Bare;
    let mut s = elo_broker::Session::default();
    s.handle(&mut host, &serde_json::json!({"op":"hello","game":"gates","protocol":1}));
    let out = s.handle(&mut host, &serde_json::json!({"op":"prove","nonce":"n","server":"s"}));
    assert_eq!(out["ok"], serde_json::json!(false), "the default must refuse: {out}");
}

/// **The bytes another repo recomputes.** Gates' shard verifies a `prove` by
/// rebuilding this message from what it already knows; its own golden pins the
/// same literal (`crates/protocol/src/auth.rs`, `siwe_message`). Two crates in
/// two repos agreeing is a claim — this is the half of the evidence that lives
/// here, so a change to the format fails on this side too instead of silently
/// making every Gates login unverifiable.
#[test]
fn prove_message_is_pinned_for_the_servers_that_recompute_it() {
    let nonce: String = (0u8..32).map(|b| format!("{b:02x}")).collect();
    let got = elo_broker::protocol::prove_message(
        "gates.example",
        "0x7E5F4552091A69125d5DfcB7b8C2659029395Bdf",
        "gates",
        &nonce,
        &elo_broker::protocol::iso8601(1_770_000_000),
    );
    assert_eq!(
        got,
        "gates.example wants you to sign in with your Ethereum account:\n\
         0x7E5F4552091A69125d5DfcB7b8C2659029395Bdf\n\n\
         Prove your identity to play gates. This signature authorises \
         nothing and moves no funds.\n\n\
         URI: https://gates.example\nVersion: 1\nChain ID: 4663\n\
         Nonce: 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\n\
         Issued At: 2026-02-02T02:40:00Z"
    );
}
