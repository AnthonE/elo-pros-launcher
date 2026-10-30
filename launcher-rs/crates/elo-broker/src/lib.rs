//! The broker — the door a game knocks on. Steam's pipe, ripped.
//!
//! `steam_api` does not hold your credentials. It talks to the **running Steam
//! client** over a local pipe, and the client does the privileged part. That is
//! why a Steam game never asks for your password, and it is exactly what this
//! platform needs: a desktop build must be able to prove who is playing and
//! sign a game action, and the one thing it must never learn to do is ask for
//! a private key.
//!
//! The wire is `sdk/PROTOCOL.md` —
//! `sdk/PROTOCOL.md` is the contract and the vendored SDK in Gates speaks it
//! byte-for-byte, so this replaces the server without touching a game.
//!
//! # What this is NOT, said before anything else
//!
//! **Against a process running as you, this socket is not a security
//! boundary.** A game you launched runs as your uid; it can already read your
//! files, your browser profile and this socket's path. `arca` can check
//! `SO_PEERCRED` and refuse a same-uid client because the arca is a *different
//! user*; the broker cannot, because it is not.
//!
//! Three things it does provide, and each is real:
//!
//! 1. **A game never needs a key.** Before this, a native build's only path to
//!    a signature was "ask the player to paste a private key" — the thing we
//!    must never teach, and which is unrecoverable once taught. The strongest
//!    ask available to a hostile game is now *a message the signer will sign*:
//!    a bounded, journaled, recognised family with a count.
//! 2. **Consent is per game, per family and action, counted, and dies with
//!    the process.** There is no "always allow" and none is written to disk.
//!    A refusal holds across the game's connections; a grant does not.
//! 3. **The real boundary is the signer behind it.** Point [`Signer`] at an
//!    arca and every request is re-derived by a process running as someone
//!    else, with its own ledger and its own refusals.

pub mod protocol;
pub mod signer;
pub mod transport;

use protocol::{Refusal, VERBS};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};

/// What the broker does not own and will not invent.
///
/// Every one of these is a decision that belongs to the launcher or to the
/// player, so the broker takes them as a trait rather than reaching for them.
/// It is also what makes the whole protocol testable with no keys, no windows
/// and no network.
pub trait Host: Send {
    /// The address the player asked the launcher to watch, or `None`. **An
    /// address, never a key, and never authentication** — a game that needs it
    /// to matter must ask for a signature and verify it.
    ///
    /// ⚠ **It MUST be EIP-55 checksummed**, and that stopped being cosmetic
    /// when `prove` became SIWE: EIP-4361 requires a checksummed address, and
    /// the reference `siwe` parsers reject an all-lowercase one outright — so a
    /// host returning lowercase here makes every proof it issues unverifiable,
    /// with nothing failing on this side. `elo_vault` already returns the
    /// checksummed form (`to_checksum`); this crate cannot re-derive it,
    /// because checksumming needs keccak and `elo-vault` depends on *this*
    /// crate rather than the other way round. So it is a contract, stated here
    /// and held by the implementor.
    fn address(&self) -> Option<String>;

    /// Which backend signs: `local` · `browser` · `arca` · `external` ·
    /// `none`. The shipped client answers `local` once an account exists and
    /// `none` before; the rest are for anyone building on this crate.
    fn signer_name(&self) -> String;

    /// The origin the launcher reads.
    fn host_url(&self) -> String;

    /// Ask the player. Returns how many signatures of this family the game may
    /// take, `None` for refused, or `Some(`[`UNANSWERED`]`)` when nobody
    /// answered in time. **The broker never assumes an answer** — there is no
    /// default-allow path through this trait.
    ///
    /// `text` is the message and must be shown **verbatim**; `why` is the
    /// game's own stated reason and must be shown as a claim, if at all. Both
    /// are untrusted, and the second is unbounded and free-form — it is the
    /// game's sentence, not the launcher's, and a prompt that renders it as
    /// the launcher's own has let a game write the question it is being asked.
    fn ask_consent(&mut self, game: &str, family: &str, text: &str, why: &str) -> Option<u32>;

    /// Sign an EIP-191 personal_sign message, or refuse with a reason a player
    /// can read.
    fn sign(&mut self, text: &str, why: &str) -> Result<Value, Refusal>;

    /// Sign a **launcher-authored** identity proof — see
    /// [`protocol::prove_message`].
    ///
    /// Separate from [`Host::sign`] on purpose and not a convenience wrapper.
    /// `sign` is gated by consent because the game writes the message; this one
    /// is not, because the game cannot. A host that implemented this by
    /// forwarding to `sign` would keep the prompt and lose nothing, but a host
    /// that implemented `sign` by forwarding *here* would hand a game an
    /// unprompted signature over its own text — so the direction matters.
    ///
    /// The default refuses, so a host that has not thought about it does not
    /// silently acquire the ability to prove anything.
    fn prove(&mut self, _message: &str) -> Result<Value, Refusal> {
        Err(Refusal::new(
            "this launcher cannot prove an identity — no account is configured",
        ))
    }

    /// The name to sign a proof for `server` under, when it is not `server`
    /// itself — [`protocol::prove_message_for`]. `Some` only when a browser
    /// wallet will sign and the server sits under the page that asks it; the
    /// `URI:` line still names the server. The default is the launcher's own
    /// form.
    fn prove_domain(&self, _server: &str) -> Option<String> {
        None
    }

    /// Is `server` (a `prove` domain, its port already dropped) one this
    /// title publishes as its own — a host in its shard list?
    ///
    /// A sign-in to such a server needs no prompt: launching the title was the
    /// decision. **Any other domain asks the player**, because a SIWE
    /// signature for `app.thirdparty.example` logs the player's wallet in
    /// there, and the domain is the one field a hostile game would choose.
    /// The default publishes nothing, so a host that has not thought about it
    /// asks every time.
    fn serves(&mut self, _game: &str, _server: &str) -> bool {
        false
    }

    /// A name and a face for one address — Steam's `GetPlayerSummaries`.
    ///
    /// Reads `/api/who`, the origin's own join on the wallet (its name,
    /// picture, seat and hive link), and flattens it to what a HUD draws. **A read about a stranger,
    /// never authentication**: it describes an address, it does not establish
    /// that the player holds its key. That is [`Host::prove`].
    ///
    /// ⚠ The reply must keep *we could not look* apart from *there is nothing
    /// there* (`CLAUDE.md` §traps). A player with no name set and an origin
    /// that is down are different answers, and a game that draws "no name" for
    /// the second has invented a fact about somebody.
    ///
    /// The default refuses rather than returning an empty profile, which is the
    /// same distinction stated in code: a host that cannot read has not learned
    /// that the address is anonymous.
    fn profile(&mut self, _address: &str) -> Result<Value, Refusal> {
        Err(Refusal::new(
            "this launcher cannot read profiles — it has no connection to the origin. \
             That is not the same as this address having no profile.",
        ))
    }

    /// The manifest url for a slug — `title` and `servers` both answer with a
    /// url and never with the thing behind it. **The launcher does not proxy,
    /// cache or rank a game's shard list**; that list is the game's to serve.
    fn title_url(&self, slug: &str, what: What) -> Option<String>;

    /// Send the player to a page. https only, checked before this is called.
    fn open(&mut self, url: &str) -> Result<(), Refusal>;
}

/// Which url `title`/`servers` is asking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Manifest,
    Servers,
}

/// Who may ask for what, how many times, until the launcher closes.
///
/// In memory only, and that is the design rather than an omission: a grant
/// written to disk is a grant that survives the player forgetting they made
/// it, and a signing permission nobody remembers granting is the shape of
/// every bad outcome this file exists to avoid.
///
/// **Grants live with the connection; refusals live with the door.** A grant
/// shared across connections would let any process that says the same `hello`
/// ride it, so it stays here. A refusal is the other way round: kept per
/// connection, a game hangs up, reconnects and asks again, and a Refuse that
/// never sticks is how a player is worn down into Allow 100. So refusals are a
/// [`Refusals`] the door shares between every connection it serves.
#[derive(Default)]
pub struct Consent {
    grants: HashMap<(String, String), u32>,
    refusals: SharedRefusals,
}

/// The player's refusals for the life of the launcher, shared by every
/// connection the door serves (`transport::serve_forever_counted`).
#[derive(Default)]
pub struct Refusals {
    denied: HashSet<(String, String)>,
    by_game: HashMap<String, u32>,
    /// Prompts a game has open right now, on any connection. They count
    /// against [`REFUSALS_PER_GAME`] until answered, because the lock is let
    /// go across a prompt: without this, N connections asking at once all
    /// pass the gate before the first refusal lands, and N windows queue.
    asking: HashMap<String, u32>,
    pending: HashSet<(String, String)>,
}

pub type SharedRefusals = Arc<Mutex<Refusals>>;

/// How many prompts one game may have refused before it cannot open another.
/// Without it a game walks the families — `elo a`, `elo b`, … — and every one
/// is a fresh modal window.
pub const REFUSALS_PER_GAME: u32 = 3;

/// `elo play` actions that bind or move a public identity, vote, or link an
/// account (`meter/playauth.py` `WALLET_ONLY`, plus the actions that do the
/// same job under another name), `buddy`, whose `gift` moves the player's
/// credit to a wallet the detail names, and every other act a meter route verifies
/// for the platform rather than a game: the work board (`claim`, `submit`),
/// GatesBench, buddy links, witness pledges and readings, and a trade on a
/// prediction (`predict`), a lobby act (`lobby`), reading out or deleting
/// cloud saves (`cloud`), a Workshop subscription (`workshop`) and being
/// listed live on the site while streaming (`stream`), and the daily
/// check-in, which can name a friend's link (`checkin`). A grant
/// never covers the next one: each is shown, each time, whatever count was
/// clicked.
/// `meter/test_playauth_platform.py` fails on a meter action missing here.
pub const ONE_AT_A_TIME: [&str; 26] = [
    "npub", "listing", "report", "face", "herald", "profile", "chatkey", "vote", "drops",
    "friends", "guilds", "grant", "revoke", "swear", "claim", "submit", "bench", "buddy",
    "pledge", "reading", "predict", "lobby", "cloud", "workshop", "stream", "checkin",
];

/// What [`Host::ask_consent`] returns when the prompt went unanswered: the
/// player was away, not saying no. It signs nothing, like a refusal, but it is
/// not remembered as one — otherwise a game asked while its player was making
/// tea is refused that kind of message until the launcher restarts.
pub const UNANSWERED: u32 = 0;

/// What a consent grant is keyed on: the family, and for a text whose second
/// line is `action: …` (`elo play`), the action too. A player who allowed a
/// duel allowed duels — not every message whose first line reads the same.
pub fn consent_kind(family: &str, text: &str) -> String {
    match text.lines().nth(1).and_then(|l| l.strip_prefix("action: ")).map(str::trim) {
        Some(action) if !action.is_empty() => format!("{family} {action}"),
        _ => family.to_string(),
    }
}

/// Does a grant for this kind of message run out after one signature?
///
/// Only an `elo play` action can be granted a count. Every other family — the
/// dev desk, the store, partners, pairing, account links — is the platform's
/// authority rather than a game's, and its kind has no action to narrow it,
/// so a counted grant on one would cover any later text that starts the same
/// way. Those are asked about every time.
pub fn one_at_a_time(kind: &str) -> bool {
    match kind.split_once(' ') {
        Some(("play", action)) => ONE_AT_A_TIME.contains(&action),
        _ => true,
    }
}

impl Consent {
    /// A consent table whose refusals are the door's, not this connection's.
    pub fn sharing(refusals: SharedRefusals) -> Consent {
        Consent { grants: HashMap::new(), refusals }
    }

    /// Spend one signature of this kind, asking the player if there is no
    /// live grant. A denial is remembered for the life of the launcher so a
    /// game cannot re-prompt in a loop — which is the whole reason `denied` is
    /// separate from a zero grant.
    pub fn spend(&mut self, host: &mut dyn Host, game: &str, kind: &str, text: &str,
                 why: &str)
        -> Result<(), Refusal>
    {
        let key = (game.to_string(), kind.to_string());
        {
            let r = self.refusals.lock().map_err(|_| Refusal::new("consent is busy"))?;
            if r.denied.contains(&key) {
                return Err(Refusal::by_player(format!(
                    "you refused {kind:?} for {game} this session"
                )));
            }
            if r.by_game.get(game).copied().unwrap_or(0) >= REFUSALS_PER_GAME {
                return Err(Refusal::by_player(format!(
                    "you refused {game} {REFUSALS_PER_GAME} times this session, so it \
                     may not ask again until the launcher restarts"
                )));
            }
        }
        if let Some(left) = self.grants.get_mut(&key) {
            if *left > 0 {
                *left -= 1;
                return Ok(());
            }
        }
        {
            // Reserve the prompt under the same lock that counts refusals, so
            // what is open plus what was refused never passes the limit.
            let mut r = self.refusals.lock().map_err(|_| Refusal::new("consent is busy"))?;
            let open = r.asking.get(game).copied().unwrap_or(0);
            if r.pending.contains(&key)
                || r.by_game.get(game).copied().unwrap_or(0) + open >= REFUSALS_PER_GAME
            {
                return Err(Refusal::new(format!(
                    "{game} is already waiting on the player — answer that first"
                )));
            }
            r.pending.insert(key.clone());
            *r.asking.entry(game.to_string()).or_insert(0) += 1;
        }
        // The lock is not held across the prompt: a person can take minutes,
        // and every other connection's refusal check would wait behind them.
        let answer = host.ask_consent(game, kind, text, why);
        let mut r = self.refusals.lock().map_err(|_| Refusal::new("consent is busy"))?;
        r.pending.remove(&key);
        if let Some(open) = r.asking.get_mut(game) {
            *open = open.saturating_sub(1);
        }
        match answer {
            Some(n) if n > 0 => {
                drop(r);
                if !one_at_a_time(kind) {
                    self.grants.insert(key, n - 1);
                }
                Ok(())
            }
            Some(_) => Err(Refusal::new(format!(
                "nobody answered the prompt for {game}'s {kind:?} message in time — nothing was signed"
            ))),
            None => {
                r.denied.insert(key);
                *r.by_game.entry(game.to_string()).or_insert(0) += 1;
                Err(Refusal::by_player(format!(
                    "you refused to let {game} sign a {kind:?} message"
                )))
            }
        }
    }
}

/// One connected game, and the state that makes `hello` mean something.
pub struct Session {
    game: Option<String>,
    consent: Consent,
}

impl Default for Session {
    fn default() -> Self {
        Session { game: None, consent: Consent::default() }
    }
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// A session whose refusals are the door's — what every connection the
    /// launcher serves gets, so a refusal outlives the connection it was
    /// made on.
    pub fn sharing(refusals: SharedRefusals) -> Session {
        Session { game: None, consent: Consent::sharing(refusals) }
    }

    /// The game that said hello, if one has.
    pub fn game(&self) -> Option<&str> {
        self.game.as_deref()
    }

    /// Handle one request line and produce one reply. Pure with respect to the
    /// transport, which is what lets the whole protocol be tested over a pair
    /// of in-memory buffers.
    pub fn handle(&mut self, host: &mut dyn Host, req: &Value) -> Value {
        let op = match protocol::validate(req) {
            Ok(op) => op,
            Err(r) => return r.to_json(),
        };

        if op == "hello" {
            return match protocol::check_hello(req) {
                // One name per connection. A second hello under another name
                // would start that name's consent from nothing, mid-stream.
                Ok(game) if self.game.as_ref().is_some_and(|g| *g != game) => Refusal::new(
                    format!(
                        "this connection already said hello as {:?} — a game's name \
                         is fixed for the connection",
                        self.game.as_deref().unwrap_or("")
                    ),
                )
                .to_json(),
                Ok(game) => {
                    self.game = Some(game);
                    protocol::hello_reply(&host.host_url(), &host.signer_name())
                }
                Err(r) => r.to_json(),
            };
        }

        // Everything else waits for a name. `sdk/PROTOCOL.md`: a consent
        // prompt with no name in it is not consent.
        let Some(game) = self.game.clone() else {
            return Refusal::new(format!(
                "{op}: say hello first — every verb needs the game's name"
            ))
            .to_json();
        };

        match op {
            "identity" => json!({
                "ok": true,
                "address": host.address(),
                "host": host.host_url(),
                "signer": host.signer_name(),
            }),
            "sign" => {
                let Some(text) = req.get("text").and_then(Value::as_str) else {
                    return Refusal::new("sign needs a text").to_json();
                };
                let why = req.get("why").and_then(Value::as_str).unwrap_or("");
                let family = match protocol::sign_family(text) {
                    Ok(f) => f,
                    Err(r) => return r.to_json(),
                };
                // Before the prompt: a byte the prompt cannot draw is a byte
                // the player would approve without seeing.
                if let Err(r) = protocol::check_signable(text) {
                    return r.to_json();
                }
                let kind = consent_kind(&family, text);
                if let Err(r) = self.consent.spend(host, &game, &kind, text, why) {
                    return r.to_json();
                }
                match host.sign(text, why) {
                    Ok(mut v) => {
                        if let Some(m) = v.as_object_mut() {
                            m.insert("ok".into(), json!(true));
                            m.insert("family".into(), json!(family));
                        }
                        v
                    }
                    Err(r) => r.to_json(),
                }
            }
            // ── prove — identity, and nothing else ───────────────────────────
            //
            // **No consent check for the title's own servers, and that is
            // deliberate.** Every other signing path in this file spends a
            // consent grant because the game chose the words. Here the launcher
            // writes them: the game contributes a nonce and a server, both
            // refused unless they are inert, and the sentence is
            // `protocol::prove_message`. There is no prompt because there is no
            // decision — the player already made it by launching this game.
            //
            // The server is the one word the game picks, though, and a SIWE
            // message for a site the title does not run is a login there. So a
            // domain the title does not publish (`Host::serves`) asks first.
            //
            // A signature over this message authorises nothing. It says "the
            // holder of this address was here, for this server, on this
            // challenge", which is exactly what a server needs to check a
            // ticket and exactly nothing more.
            "prove" => {
                let nonce = req.get("nonce").and_then(Value::as_str).unwrap_or("");
                let server = req.get("server").and_then(Value::as_str).unwrap_or("");
                if let Err(r) = protocol::check_nonce(nonce) {
                    return r.to_json();
                }
                if let Err(r) = protocol::check_domain(server) {
                    return r.to_json();
                }
                // SIWE binds the address INTO the signed bytes, so a launcher
                // with no account cannot compose a valid message at all. Saying
                // so here is better than composing a message with an empty
                // address line and letting the signer refuse it for a different
                // reason.
                let Some(address) = host.address() else {
                    return Refusal::new(
                        "no account on this machine — make one in Account, or run \
                         `elo account new`. Playing without one is a normal state; \
                         a server that wants proof of who you are will not get it.",
                    )
                    .to_json();
                };
                // **`issued_at` may be the SERVER's, and that is the point.**
                //
                // A verifier has to recompute these bytes to check them, and it
                // can recompute every field except a timestamp this launcher
                // picked out of its own clock. That gap is why Gates could not
                // use `prove` at all: its shard issues a challenge carrying the
                // instant it chose, and no amount of care here lets it guess the
                // string we would have written. So a caller may hand us the
                // second, in the same UNIX form its challenge already speaks,
                // and we format it with the SAME `iso8601` we would have used —
                // one formatter, so the two sides cannot disagree about padding
                // or the trailing `Z`.
                //
                // Absent, it stays `now`, which is what every existing caller
                // gets. The field is additive on purpose: a launcher is used by
                // games it was not built alongside.
                let issued_at = match req.get("issued_at") {
                    None | Some(Value::Null) => protocol::now_iso8601(),
                    Some(v) => {
                        let Some(secs) = v.as_i64() else {
                            return Refusal::new(
                                "prove: issued_at is UNIX seconds as a number, or absent",
                            )
                            .to_json();
                        };
                        match protocol::check_issued_at(secs) {
                            Ok(()) => protocol::iso8601(secs),
                            Err(r) => return r.to_json(),
                        }
                    }
                };
                // `game` is the name from `hello`, never a field on this
                // request — a game cannot prove as somebody else's title.
                let message = match host.prove_domain(server) {
                    Some(page) => {
                        if let Err(r) = protocol::check_domain(&page) {
                            return r.to_json();
                        }
                        // A paired browser wallet signs this one, and the
                        // wallet shows its holder the sign-in itself.
                        protocol::prove_message_for(&page, server, &address, &game, nonce, &issued_at)
                    }
                    None => {
                        let message =
                            protocol::prove_message(server, &address, &game, nonce, &issued_at);
                        // **A sign-in anywhere but the title's own servers asks.**
                        // The game names the domain, and a SIWE signature for a
                        // site the title does not run is a login there as this
                        // wallet. The prompt shows the message verbatim, and its
                        // first line names the site.
                        let site = server.split(':').next().unwrap_or(server).to_ascii_lowercase();
                        if !host.serves(&game, &site) {
                            let kind = format!("sign-in to {site}");
                            if let Err(r) = self.consent.spend(host, &game, &kind, &message,
                                                               "sign in with this wallet")
                            {
                                return r.to_json();
                            }
                        }
                        message
                    }
                };
                match host.prove(&message) {
                    Ok(mut v) => {
                        // ⚠ SIWE binds the address into the signed bytes, so the
                        // account that SIGNED must be the account named in the
                        // message. If a host composes with one and signs with
                        // another, every field still looks right and no verifier
                        // on earth will accept it: `siwe` checks the recovered
                        // signer against `message.address` and rejects. Handing
                        // back a proof that cannot verify is worse than refusing,
                        // because the game learns nothing about why.
                        let signed_by = v.get("address").and_then(Value::as_str).unwrap_or("");
                        if !signed_by.is_empty()
                            && !signed_by.eq_ignore_ascii_case(&address)
                        {
                            return Refusal::new(format!(
                                "this launcher composed a proof for {address} and signed \
                                 it with {signed_by} — the signature cannot verify against \
                                 its own message. That is a launcher bug, not yours."
                            ))
                            .to_json();
                        }
                        if let Some(m) = v.as_object_mut() {
                            m.insert("ok".into(), json!(true));
                            // The message is echoed so a game can see what it
                            // got, and a verifier must still RECOMPUTE it from
                            // what it knows rather than trusting this copy.
                            m.insert("message".into(), json!(message));
                            m.insert(
                                "verify".into(),
                                json!("this is a SIWE (EIP-4361) message — hand it to any \
                                       siwe library and check it against the nonce YOU \
                                       issued and the domain YOU serve, then look the \
                                       recovered address up. Never trust `message` as sent"),
                            );
                            m.insert("scheme".into(), json!("eip4361"));
                        }
                        v
                    }
                    Err(r) => r.to_json(),
                }
            }

            // A read about an address. No consent, because nothing is signed
            // and nothing is authorised — but also no invention: a host that
            // cannot reach the origin refuses rather than reporting an empty
            // profile, so a game can tell "anonymous" from "offline".
            "profile" => {
                let watched = host.address().unwrap_or_default();
                let address = req
                    .get("address")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let address = if address.is_empty() { watched } else { address };
                if address.is_empty() {
                    json!({
                        "ok": true,
                        "reachable": true,
                        "address": Value::Null,
                        "profile": Value::Null,
                        "note": "no address to look up — this launcher watches none and \
                                 the request named none. Anonymous is a normal state.",
                    })
                } else if !protocol::looks_like_address(&address) {
                    Refusal::new("profile: address must be 0x followed by 40 hex digits")
                        .to_json()
                } else {
                    match host.profile(&address) {
                        Ok(mut v) => {
                            if let Some(m) = v.as_object_mut() {
                                m.insert("ok".into(), json!(true));
                                m.insert("address".into(), json!(address));
                                m.insert(
                                    "note".into(),
                                    json!("what the origin says about this address. \
                                           `handle` is a held hive handle; \
                                           `display_name` is self-declared — draw them differently or do \
                                           not draw the second."),
                                );
                            }
                            v
                        }
                        Err(r) => r.to_json(),
                    }
                }
            }

            "title" | "servers" => {
                let slug = req.get("slug").and_then(Value::as_str).unwrap_or(&game);
                let what = if op == "title" { What::Manifest } else { What::Servers };
                match host.title_url(slug, what) {
                    Some(url) => json!({
                        "ok": true,
                        "url": url,
                        "note": "fetch it yourself; the launcher does not proxy it",
                    }),
                    None => Refusal::new(format!(
                        "no {} url is published for {slug:?}",
                        if what == What::Manifest { "manifest" } else { "shard list" }
                    ))
                    .to_json(),
                }
            }
            "open" => match protocol::check_url(req) {
                Ok(url) => match host.open(&url) {
                    Ok(()) => json!({"ok": true}),
                    Err(r) => r.to_json(),
                },
                Err(r) => r.to_json(),
            },
            // One snapshot a game can DRAW. Steam injects a library and hooks
            // the graphics API because Steam does not own the games; this
            // platform does, so the overlay is the game rendering its own HUD
            // from this read — no injection, nothing to break on a driver
            // update, nothing that reads as malware to an anti-cheat.
            //
            // ⚠ **Display freely; never approve.** A game controls its own
            // pixels, so it can draw a convincing fake of any dialog. There is
            // no consent token in this reply and no way to mint one, no
            // pending-request queue to render as a prompt, and no
            // already-allowed flag. Every field here could already have been
            // asked for one at a time; bundling is a convenience for a HUD
            // drawn each frame, never new authority.
            //
            // ⚠ This used to return `{slug, note}` and nothing else, while
            // `sdk/PROTOCOL.md` and BOTH SDK clients described an `overlay`
            // object — so `Overlay::overlay()` returned None against this
            // server for every game, and no test caught it because each half
            // was only ever driven against its own kind. Same shape as `prove`
            // being dark; found the day the second broker was deleted.
            "overlay" => {
                let slug = req.get("slug").and_then(Value::as_str).unwrap_or(&game);
                json!({
                    "ok": true,
                    "overlay": {
                        "host": host.host_url(),
                        "signer": host.signer_name(),
                        "address": host.address(),
                        "title": {
                            "slug": slug,
                            // A url, because the launcher does not proxy a
                            // game's documents — the same rule `title` states.
                            "manifest": host.title_url(slug, What::Manifest),
                            "servers": host.title_url(slug, What::Servers),
                        },
                        "consent": {
                            "drawn_by": "the launcher",
                            "note": "this process asks the player, in its own window. \
                                     A game that draws its own approval dialog is \
                                     drawing a forgery — display freely, never approve.",
                        },
                    },
                })
            }
            _ => Refusal::new(format!("{op} is known but not handled")).to_json(),
        }
    }
}

/// Serve one connection to completion: read lines, reply in order, stop at EOF.
///
/// A line that is not JSON gets a refusal rather than a closed socket — a game
/// with a framing bug should be told, not disconnected, because a disconnect
/// reads to the player as the launcher crashing.
pub fn serve_conn<S>(stream: S, host: &mut dyn Host) -> std::io::Result<()>
where
    S: std::io::Read + Write,
{
    serve_conn_sharing(stream, host, SharedRefusals::default())
}

/// [`serve_conn`], with the player's refusals held by the door rather than by
/// this connection — what `transport::serve_forever_counted` serves with.
pub fn serve_conn_sharing<S>(stream: S, host: &mut dyn Host, refusals: SharedRefusals)
    -> std::io::Result<()>
where
    S: std::io::Read + Write,
{
    let mut session = Session::sharing(refusals);
    let mut reader = BufReader::new(stream);
    let mut raw = Vec::new();
    loop {
        raw.clear();
        // Bounded: any local process can connect, and an unbounded read_line
        // lets one grow the process that holds the unlocked key until the OS
        // kills it.
        let n = std::io::Read::take(&mut reader, MAX_LINE as u64 + 1).read_until(b'\n', &mut raw)?;
        if n == 0 {
            return Ok(());
        }
        let too_long = raw.len() > MAX_LINE && !raw.ends_with(b"\n");
        let reply = if too_long {
            Refusal::new(format!("that line is over {MAX_LINE} bytes")).to_json()
        } else {
            match std::str::from_utf8(&raw) {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => match serde_json::from_str::<Value>(line.trim()) {
                    Ok(req) => session.handle(host, &req),
                    Err(e) => Refusal::new(format!("that line is not JSON: {e}")).to_json(),
                },
                Err(_) => Refusal::new("that line is not UTF-8").to_json(),
            }
        };
        let out = reply.to_string();
        let w = reader.get_mut();
        w.write_all(out.as_bytes())?;
        w.write_all(b"\n")?;
        w.flush()?;
        if too_long {
            // The rest of that line would be read as the next request, so
            // the conversation ends here, after saying why.
            return Ok(());
        }
    }
}

/// The longest request line the door reads. Far beyond any real request — a
/// message to sign is a few hundred bytes — and small enough that a client
/// cannot use the read itself to exhaust memory.
pub const MAX_LINE: usize = 1 << 20;

/// Every verb this build speaks — for a capabilities line and for tests that
/// assert the vocabulary did not grow by accident.
pub fn capabilities() -> Vec<&'static str> {
    VERBS.iter().map(|(v, _)| *v).collect()
}
