//! The consent doorbell — how a game's ask for a signature reaches the player.
//!
//! # Why there is a prompt here at all
//!
//! `sign` is the one verb where the **game writes the words**
//! (`sdk/PROTOCOL.md`), so it is the one verb the player has to answer for
//! themselves. Every other signing path in this client either composes its own
//! sentence — `prove`, where the game contributes two opaque fields and cannot
//! say anything with them — or is a human at a terminal typing
//! `elo account sign`. That is why this file exists for exactly one verb and
//! must not grow to cover the others.
//!
//! # The thread boundary, which is the whole problem
//!
//! The game door runs on its own thread (`elo-gui`'s `open_the_door`) and
//! FLTK's widgets belong to the main one. So the ask has to cross, and cross
//! back, with the door thread parked in between:
//!
//! ```text
//!   door thread                       main thread (FLTK)
//!   ───────────                       ──────────────────
//!   ask() ── push ──▶ queue
//!   park on recv()                    pump() ── drains ──▶ shows the window
//!                                     …the player answers…
//!   ◀────────── the answer ────────── reply.send(…)
//! ```
//!
//! **Nothing here draws and nothing here links FLTK.** That is what lets every
//! rule below be tested with no display, on a thread pair a test owns — and
//! this is the piece of the client where an untested rule would be worth the
//! least, because it is the one standing between a hostile game and a
//! signature.
//!
//! # Every failure refuses
//!
//! A poisoned queue, a main loop that has already closed, a window dismissed
//! by the window manager: all of them answer `None`. There is no path through
//! this file that returns a grant nobody typed. The two directions are not
//! symmetric — a refusal the player did not mean costs them a click, and a
//! grant they did not mean costs them a signature.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the door parks on one prompt before answering the game itself.
///
/// **This number is not ours to pick freely — it is the SDK clients'.** Both
/// reference clients read with a 300 second timeout
/// (`sdk/python/elo_overlay.py`, `sdk/rust/elo_overlay.rs`), so past that the
/// game has already given up and hung up. Parking longer would hold a socket
/// nobody is reading and, worse, could produce a signature for a request that
/// no longer exists.
///
/// ⚠ **It is also the backstop against the door hanging forever**, and that is
/// not hypothetical: every clone of a [`Doorbell`] keeps the queue alive, so a
/// pump that stops running does *not* disconnect the reply channel — the
/// asking thread holds it open itself. A bare `recv()` here waits for a sender
/// that can never drop. `sdk/PROTOCOL.md` names the resulting shape as the bad
/// one: *"a launcher that accepts and then never answers hangs the calling
/// game instead of erroring."*
pub const PROMPT_LIMIT: Duration = Duration::from_secs(300);

/// What the player may grant, as data — so the window's buttons, the binary
/// and the tests cannot disagree about what "allow" means.
///
/// **There is no "always", and the absence is the design** (`docs/client/SDK.md` §4).
/// A counted allowance is the whole vocabulary and the largest one is still a
/// number that runs out; every one of them dies when this process does.
pub const ALLOWANCES: [(&str, u32); 3] =
    [("Just this one", 1), ("Allow 25", 25), ("Allow 100", 100)];

/// The longest run of game-supplied text this client will draw on one line.
///
/// Not a formatting preference. `why` is unbounded on the wire and `family` is
/// bounded only in its charset (`protocol::sign_family`), so without a cut a
/// game can push the buttons off the bottom of the prompt — a dialog whose
/// Refuse is somewhere below the screen is one the player answers with the
/// window manager, and hoping that lands on refuse is not a design.
const LINE_LIMIT: usize = 120;

/// One line of game-supplied text, made safe to draw as a **label**.
///
/// ⚠ **This is an injection guard, not tidying.** An FLTK label breaks on
/// `\n`, so a `why` of `"settling round 41\n\n✓ already approved by Elo"`
/// draws two lines the launcher never wrote, in the launcher's own window,
/// directly under the launcher's own words. That is precisely the smuggling
/// `check_prove_field` refuses for `prove`, arriving through the one field a
/// game still gets to fill in on this path.
///
/// Control characters become spaces rather than being dropped, so
/// `"pay\u{8}\u{8}\u{8}free"` cannot reassemble into a different word than the
/// one that was sent — the player sees the gap where the trick was.
pub fn one_line(raw: &str) -> String {
    let flat: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > LINE_LIMIT {
        let kept: String = flat.chars().take(LINE_LIMIT).collect();
        format!("{kept}…")
    } else {
        flat
    }
}

/// One game's ask, in the words the player will be shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    /// From the `hello` handshake, never from the `sign` request — a title
    /// cannot ask as another title. `protocol::check_slug` has already bounded
    /// it to a slug, which is why it is the one field drawn unfiltered.
    pub game: String,
    /// Parsed out of the message's first line, so it describes the text that
    /// will actually be signed rather than a field sitting beside it.
    pub family: String,
    /// **The message, verbatim.** Shown in full and never summarised away.
    pub text: String,
    /// The game's own reason. A CLAIM, and drawn as one.
    pub why: String,
}

impl Ask {
    pub fn new(game: &str, family: &str, text: &str, why: &str) -> Ask {
        Ask {
            game: game.to_string(),
            family: family.to_string(),
            text: text.to_string(),
            why: why.to_string(),
        }
    }

    /// The launcher's own sentence, above the message.
    ///
    /// ⚠ **A summary never replaces the text** (`docs/client/SDK.md` §4). A prompt
    /// that showed only this could be lied to by a crafted `detail:` line, so
    /// this says who is asking and for what family — both facts the launcher
    /// established itself — and the message goes underneath in full.
    pub fn summary(&self) -> String {
        format!("{} wants to sign a {} message", one_line(&self.game), one_line(&self.family))
    }

    /// The game's stated reason, attributed. Empty when it gave none — and a
    /// game that gives none is not suspicious, it is just terse.
    pub fn claim(&self) -> Option<String> {
        let why = one_line(&self.why);
        (!why.is_empty()).then(|| format!("it says: {why}"))
    }
}

struct Pending {
    ask: Ask,
    reply: SyncSender<Option<u32>>,
    /// Set by the asking thread when it has stopped waiting.
    ///
    /// Without it, a prompt that timed out would still be sitting in the queue
    /// and would open a window later — asking a player about a game that hung
    /// up minutes ago, whose answer goes nowhere. A stale question is worse
    /// than no question: this is the one window where a player is trained to
    /// read carefully, and spending that on a ghost is how the habit is lost.
    abandoned: Arc<AtomicBool>,
}

/// The queue between the door and the player, and the only way across.
///
/// Cheap to clone — every clone is the same doorbell, which is what lets each
/// connection's `Host` hold one while the main thread rings it out.
#[derive(Clone)]
pub struct Doorbell {
    queue: Arc<Mutex<VecDeque<Pending>>>,
    patience: Duration,
}

impl Default for Doorbell {
    fn default() -> Self {
        Doorbell::new()
    }
}

impl Doorbell {
    pub fn new() -> Doorbell {
        Doorbell::with_patience(PROMPT_LIMIT)
    }

    /// A doorbell that gives up sooner. The client uses [`PROMPT_LIMIT`]
    /// because that is what its own SDK clients wait; a test uses milliseconds
    /// so that the giving-up path can be *checked* rather than described.
    pub fn with_patience(patience: Duration) -> Doorbell {
        Doorbell { queue: Arc::new(Mutex::new(VecDeque::new())), patience }
    }

    /// **Called on the door thread.** Parks until the player answers.
    ///
    /// Blocking is correct and not a compromise: the game is waiting on a
    /// socket read for exactly this answer, and a consent prompt waits for a
    /// person — `sdk/PROTOCOL.md` says so and both reference clients allow 300
    /// seconds for it. What must never happen is this returning a number
    /// because it got bored.
    ///
    /// Returns `None` — refused — when nobody answers in time, which covers
    /// both the player walking away and the launcher closing with a prompt up.
    pub fn ask(&self, ask: Ask) -> Option<u32> {
        // Depth 1: the reply is a single value and the sender never blocks,
        // because it sends once and the receiver is already parked.
        let (reply, answer) = sync_channel(1);
        let abandoned = Arc::new(AtomicBool::new(false));
        match self.queue.lock() {
            Ok(mut q) => {
                q.push_back(Pending { ask, reply, abandoned: Arc::clone(&abandoned) })
            }
            // A panic in the pump poisons the lock. Refusing is the only safe
            // reading of "the part that asks the player is broken".
            Err(_) => return None,
        }
        // Timed, not bare — see `PROMPT_LIMIT`. Both arms of the `Err` are the
        // same answer: whether nobody was listening or nobody decided, no
        // player granted anything.
        match answer.recv_timeout(self.patience) {
            Ok(granted) => granted,
            Err(_) => {
                abandoned.store(true, Ordering::SeqCst);
                None
            }
        }
    }

    /// **Called on the main thread.** Answers everything waiting, and returns
    /// how many that was.
    ///
    /// `decide` is injected rather than being the window, which is the same
    /// shape `wiring::Ask` uses for passphrases and for the same reason: a
    /// modal dialog blocks on a human, so a test that had to open one could
    /// not exist. `examples/click.rs` drives the real window; the tests below
    /// drive the real threading.
    ///
    /// ⚠ **Drained one at a time with the lock released.** `decide` opens a
    /// window and runs a nested event loop, and the door thread pushes into
    /// this same queue — holding the lock across the prompt would deadlock the
    /// second game against the first one's dialog.
    pub fn pump(&self, decide: &mut dyn FnMut(&Ask) -> Option<u32>) -> usize {
        let mut answered = 0;
        loop {
            let Some(next) = self.queue.lock().ok().and_then(|mut q| q.pop_front()) else {
                return answered;
            };
            // A game that gave up while queued is dropped without a window.
            // Counting it would be honest about work done and wrong about work
            // shown, and it is the second that a caller cares about.
            if next.abandoned.load(Ordering::SeqCst) {
                continue;
            }
            let granted = decide(&next.ask);
            // A send that fails means the game hung up mid-prompt. Nothing to
            // do about it and nothing lost: no grant is recorded here, so the
            // next connection asks again.
            let _ = next.reply.send(granted);
            answered += 1;
        }
    }

    /// How many games are parked. For a caller that wants to know whether to
    /// bother pumping.
    pub fn waiting(&self) -> usize {
        self.queue.lock().map(|q| q.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// The round trip, on two real threads — the arrangement the client runs.
    #[test]
    fn an_answer_crosses_back_to_the_thread_that_asked() {
        let bell = Doorbell::new();
        let door = bell.clone();
        let asked = std::thread::spawn(move || {
            door.ask(Ask::new("gates", "play", "elo play\naction: duel", "round 41"))
        });

        // Park until the door thread has actually queued, the way the pump's
        // timeout does.
        while bell.waiting() == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        let answered = bell.pump(&mut |ask| {
            assert_eq!(ask.game, "gates");
            assert_eq!(ask.family, "play");
            Some(25)
        });

        assert_eq!(answered, 1);
        assert_eq!(asked.join().unwrap(), Some(25), "the grant must reach the door");
    }

    /// A refusal is a refusal, not a zero grant the caller might round up.
    #[test]
    fn a_refusal_crosses_back_as_none() {
        let bell = Doorbell::new();
        let door = bell.clone();
        let asked = std::thread::spawn(move || door.ask(Ask::new("gates", "play", "elo play", "")));
        while bell.waiting() == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        bell.pump(&mut |_| None);
        assert_eq!(asked.join().unwrap(), None);
    }

    /// A pump that stops running must not park the door thread forever.
    ///
    /// ⚠ **This is the test that found the real bug.** The first version of
    /// `ask` used a bare `recv()`, reasoning that a vanished pump would drop
    /// the sender and disconnect the channel. It does not: every `Doorbell`
    /// clone keeps the queue alive, and the asking thread is holding one, so
    /// the `Pending` — and the sender inside it — outlives the pump by exactly
    /// as long as the caller waits. Dropping every *other* clone changes
    /// nothing. Only the timeout ends it.
    #[test]
    fn a_pump_that_stops_refuses_rather_than_hanging_forever() {
        let bell = Doorbell::with_patience(Duration::from_millis(150));
        let door = bell.clone();
        let asked =
            std::thread::spawn(move || door.ask(Ask::new("gates", "play", "elo play", "")));
        while bell.waiting() == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        // What shutdown looks like from here: nobody ever pumps again.
        drop(bell);
        assert_eq!(asked.join().unwrap(), None, "an unanswered ask must refuse");
    }

    /// A game that gave up is not shown to the player later.
    ///
    /// Without this the queue keeps a timed-out ask and opens a window for it
    /// on the next tick — a question about a game that hung up, whose answer
    /// goes nowhere, in the one window a player is supposed to read carefully.
    #[test]
    fn a_game_that_gave_up_is_never_drawn() {
        let bell = Doorbell::with_patience(Duration::from_millis(80));
        let door = bell.clone();
        let asked = std::thread::spawn(move || door.ask(Ask::new("gates", "play", "t", "")));
        assert_eq!(asked.join().unwrap(), None, "it must give up on its own");

        // The entry is still queued — nothing has drained it — and the pump
        // must skip it rather than prompt.
        assert_eq!(bell.waiting(), 1, "the abandoned ask is still in the queue");
        let mut prompted = 0;
        let answered = bell.pump(&mut |_| {
            prompted += 1;
            Some(100)
        });
        assert_eq!(prompted, 0, "a window was opened for a game that had left");
        assert_eq!(answered, 0, "and it must not be counted as answered");
        assert_eq!(bell.waiting(), 0, "but it is drained, not left to accumulate");
    }

    /// Nothing is answered by accident — an empty queue must not call `decide`
    /// at all, or a pump on a timer would prompt on its own schedule.
    #[test]
    fn an_idle_pump_asks_the_player_nothing() {
        let calls = AtomicUsize::new(0);
        let bell = Doorbell::new();
        let answered = bell.pump(&mut |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(1)
        });
        assert_eq!(answered, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "an idle pump must not draw a window");
    }

    /// Two games parked at once are answered separately — the second is not
    /// covered by the first's grant, which is the per-game half of the rule.
    #[test]
    fn two_games_are_asked_about_one_at_a_time() {
        let bell = Doorbell::new();
        let (a, b) = (bell.clone(), bell.clone());
        let one = std::thread::spawn(move || a.ask(Ask::new("gates", "play", "t", "")));
        let two = std::thread::spawn(move || b.ask(Ask::new("barrow", "store", "t", "")));
        while bell.waiting() < 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
        let seen = Mutex::new(Vec::new());
        let answered = bell.pump(&mut |ask| {
            seen.lock().unwrap().push(ask.game.clone());
            // Grant only to gates: a pump that answered both the same way
            // would not prove they are separate decisions.
            (ask.game == "gates").then_some(1)
        });
        assert_eq!(answered, 2);
        assert_eq!(one.join().unwrap(), Some(1));
        assert_eq!(two.join().unwrap(), None);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    /// The guard that keeps a game from drawing its own lines in the
    /// launcher's window.
    #[test]
    fn a_game_cannot_add_lines_to_the_prompt() {
        let ask = Ask::new(
            "gates",
            "play",
            "elo play\naction: duel",
            "settling\n\n✓ already approved by Elo — no signature needed",
        );
        let claim = ask.claim().expect("a why must still be shown");
        assert!(!claim.contains('\n'), "a newline reached the label: {claim:?}");
        assert!(claim.starts_with("it says:"), "the claim must be attributed: {claim:?}");
        // Flattened, not dropped: the player still sees what was sent.
        assert!(claim.contains("already approved"), "the text itself is not censored");
    }

    /// A long field is cut rather than allowed to push the buttons away.
    #[test]
    fn an_enormous_field_cannot_push_the_buttons_off_the_window() {
        let ask = Ask::new("gates", "play", "elo play", &"a".repeat(10_000));
        let claim = ask.claim().unwrap();
        assert!(claim.chars().count() < LINE_LIMIT + 32, "not cut: {} chars", claim.chars().count());
        assert!(claim.ends_with('…'), "a cut must be visible to the reader");

        // The same cut protects the summary, whose family is only
        // charset-bounded on the wire.
        let wide = Ask::new("gates", &"x".repeat(10_000), "elo x", "");
        assert!(wide.summary().chars().count() < LINE_LIMIT + 48);
    }

    /// The message itself is NOT cut — it is the thing being signed, and a
    /// prompt showing two thirds of it would be worse than no prompt.
    #[test]
    fn the_message_itself_is_never_truncated_by_this_module() {
        let long = format!("elo play\n{}", "detail: x\n".repeat(500));
        let ask = Ask::new("gates", "play", &long, "");
        assert_eq!(ask.text, long, "the signed text must survive verbatim");
    }

    /// The launcher must never park a game's socket longer than the game's own
    /// client will wait, or it holds a connection nobody is reading and can
    /// produce a signature for a request that no longer exists.
    #[test]
    fn the_wait_is_bounded_by_what_the_sdk_clients_allow() {
        // 300s in `sdk/python/elo_overlay.py` and `sdk/rust/elo_overlay.rs`.
        assert!(
            PROMPT_LIMIT <= Duration::from_secs(300),
            "the door would outwait its own SDK clients"
        );
        // And not so short that reading a message counts as refusing.
        assert!(PROMPT_LIMIT >= Duration::from_secs(60), "a human needs time to read");
    }

    /// No allowance is unbounded, and none of them is "always".
    #[test]
    fn every_allowance_is_a_number_that_runs_out() {
        assert!(!ALLOWANCES.is_empty());
        for (label, n) in ALLOWANCES {
            assert!(n > 0, "{label} grants nothing");
            assert!(n <= 100, "{label} is effectively always");
            assert!(
                !label.to_ascii_lowercase().contains("always"),
                "{label} promises what this cannot do"
            );
        }
        assert_eq!(ALLOWANCES[0].1, 1, "the first offer must be the smallest");
    }
}
