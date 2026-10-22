//! What the controls in [`crate::windows`] actually do.
//!
//! # Why this is its own module
//!
//! `windows.rs` draws and decides nothing — that rule is what keeps it
//! constructible with no disk, no origin and no display. The behaviour has to
//! live somewhere, and putting it in the binary would have made it the one part
//! of this client with no test at all: an integration test cannot reach into a
//! `[[bin]]`.
//!
//! So it lives here, in the library, and the binary is left as composition.
//!
//! # Asking is injected, and that is the whole reason this is testable
//!
//! A modal password dialog cannot be driven by a test — it blocks on a human.
//! [`Ask`] is therefore a parameter, the same injection the origin's own
//! suites use when they pass `opener=` rather than opening a real browser. In
//! the client it is [`real_ask`]; in `examples/click.rs` it is a script, which
//! lets the REAL button callback run end to end with no hands.
//!
//! [`ask_consent`] is the same shape one layer up: the *decision* is injected
//! into [`crate::consent::Doorbell::pump`], so the queue, the refusals and the
//! thread crossing are unit-tested with no display at all, and only the window
//! itself needs one.
//!
//! ⚠ **A cancelled dialog is not an empty passphrase.** `Ask` returns
//! `Option`, and every caller treats `None` as *the player changed their mind*
//! and returns without touching the keystore. Collapsing those two would make
//! pressing Escape write a key encrypted under the empty string.

use crate::theme;
use crate::windows::{self, AccountWindow, Note, SigningWindow};
use fltk::{app, prelude::*};
use elo_broker::signer::Signer;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Put a window where the player is looking, and say how far it moved.
///
/// **The middle of the screen the mouse is on, a third of the way down.** Every
/// window in this client is constructed at a fixed coordinate — `(100, 100)`
/// for the menu, then a 20px cascade down to the lock screen at `(300, 240)` —
/// which was written against one 1080p desktop and reads as a pile in the top
/// left corner of anything bigger. A modal is the case that actually costs
/// something: the consent prompt and the passphrase window are *answers a
/// player has to give*, and one that opens a monitor away from the pointer
/// gets answered late or not at all.
///
/// A third of the way down rather than halfway, because a window centred on
/// the vertical looks low — the eye reads a window's weight from its title bar.
///
/// ⚠ **Before `show()`, never after.** Moving a window that is already up is a
/// jump on screen, and doing it on every show would drag back any window the
/// player had deliberately put somewhere else.
///
/// Returns the `(dx, dy)` it applied, so a caller centring the main window can
/// carry the rest of the family along by the same amount and keep the cascade
/// it was built with. `(0, 0)` when there is nothing to measure against — a
/// screen the toolkit reports as zero-sized is one this leaves alone.
pub fn centre(win: &mut fltk::window::Window) -> (i32, i32) {
    let (mx, my) = app::get_mouse();
    let (sx, sy, sw, sh) = app::screen_work_area(app::screen_num(mx, my));
    if sw <= 0 || sh <= 0 {
        return (0, 0);
    }
    let x = (sx + (sw - win.width()) / 2).max(sx);
    let y = (sy + (sh - win.height()) / 3).max(sy);
    let delta = (x - win.x(), y - win.y());
    win.set_pos(x, y);
    delta
}

/// The one account, shared by everything that touches it.
///
/// ⚠ **`Arc<Mutex<…>>` and not `Rc<RefCell<…>>`, because the game door is on
/// another thread.** The windows all run on FLTK's thread and `Rc` was right
/// while they were the only readers — then `prove` arrived, and a game asking
/// the launcher to prove an identity is served from the door's thread. An
/// unlocked account has to be visible there or `prove` can only ever answer
/// "locked", which would be a launcher that unlocks in a window nothing else
/// can see.
pub type Shared = Arc<Mutex<elo_vault::LocalSigner>>;

/// How the wiring asks for a secret: `(heading, prompt)`, and `None` means the
/// player cancelled.
///
/// ⚠ **The heading is a separate argument and not decoration.** One of the
/// four things asked for through here is a **private key**, not a passphrase,
/// and a window headed *"Passphrase"* over a field wanting 64 hex characters
/// is a client asking for the wrong secret. Every caller names what it wants.
///
/// The answer is a [`Secret`], so every passphrase and pasted key is wiped
/// when the caller drops it rather than left in freed memory after a lock.
pub type Ask = Rc<dyn Fn(&str, &str) -> Option<Secret>>;

/// A typed passphrase or pasted key: a `String` that zeroes itself on drop.
pub type Secret = elo_vault::Zeroizing<String>;

/// How it reports back — a message, and an alert for a refusal.
pub type Tell = Rc<dyn Fn(Told, &str)>;

/// A finished step that HAS a next step — offered on the notice, not described
/// on it. Returns whether the player took it.
///
/// Operator, 2026-08-23: *"when we install a game it would be nice if it just
/// showed a play button or something. menu navigation is kinda odd after you
/// complete a step."* The install notice already ended *"It is in Games now,
/// with a Play button"* — which is a set of directions, and directions are the
/// thing being complained about. [`windows::notice`]'s own header had the rule
/// written down for restarts since 2026-08-12: a notice that names a next step
/// and leaves the player to go find it "has told them a chore, not offered
/// one." This is that rule with the restart-shaped hole widened.
///
/// Injected like [`Tell`] and [`Ask`] rather than called straight through, so
/// a harness can answer it without a display.
pub type Then = Rc<dyn Fn(&str, &str) -> bool>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Told {
    Done,
    Refused,
}

/// The real prompt. Masked, because two of the three things asked for here are
/// secrets and the third is a private key.
///
/// ⚠ **This was `dialog::password_default` until 2026-08-12**, which draws
/// FLTK's stock dialog: a light grey box with a blue `?` glyph, over a client
/// that is olive everywhere else. Operator: *"the passphrase screen doesnt
/// match anything else."* It could not be themed — the colours and the icon
/// are inside the toolkit's C++ side — so it is `windows::passphrase` now, a
/// real window in the client's own chrome.
pub fn real_ask() -> Ask {
    Rc::new(ask_in_window)
}

/// The real offer: the client's own notice, with the next step on it.
pub fn real_then() -> Then {
    Rc::new(|body, label| show_notice(Note::Done, "Done", body, Some(label)))
}

pub fn real_tell() -> Tell {
    Rc::new(|kind, text| {
        let (note, title) = match kind {
            Told::Done => (Note::Done, "Done"),
            Told::Refused => (Note::Refused, "That did not work"),
        };
        show_notice(note, title, text, None);
    })
}

/// Put up the client's own passphrase window and park on the answer.
///
/// **Runs a nested event loop**, exactly as `dialog::password_default` did and
/// as `ask_consent` does. Every exit that is not Continue is a cancel: the
/// window manager's close, Escape, the app shutting down. That direction costs
/// a retype when it is wrong, where the other direction would write a keystore
/// encrypted under whatever happened to be in the field.
pub fn ask_in_window(title: &str, prompt: &str) -> Option<Secret> {
    let mut win = windows::passphrase(title, prompt);
    let answer: Rc<RefCell<Option<Secret>>> = Rc::new(RefCell::new(None));

    let (a, mut w, f) = (Rc::clone(&answer), win.window.clone(), win.field.clone());
    let mut submit = move || {
        *a.borrow_mut() = Some(Secret::new(f.value()));
        w.hide();
    };
    let mut ok = win.ok.clone();
    let mut on_ok = submit.clone();
    ok.set_callback(move |_| on_ok());
    // Enter in the field is the same act as pressing Continue.
    let mut field = win.field.clone();
    field.set_callback(move |_| submit());

    let mut cancel = win.cancel.clone();
    let mut w2 = win.window.clone();
    cancel.set_callback(move |_| w2.hide());

    centre(&mut win.window);
    win.window.show();
    let _ = win.field.take_focus();
    while win.window.shown() {
        if !app::wait() {
            break;
        }
    }
    win.window.hide();
    // Taken, not cloned: a clone would leave a second copy to be dropped.
    let got = answer.borrow_mut().take();
    got
}

/// Put up the client's own notice window and park on it.
///
/// Returns whether the player asked for a restart, which is only ever `true`
/// when the caller offered one.
pub fn show_notice(kind: Note, title: &str, body: &str, action: Option<&str>) -> bool {
    let mut win = windows::notice(kind, title, body, action);
    let wants = Rc::new(std::cell::Cell::new(false));

    let mut ok = win.ok.clone();
    let mut w = win.window.clone();
    ok.set_callback(move |_| w.hide());

    if let Some(b) = win.action.clone() {
        let mut b = b;
        let (flag, mut w) = (Rc::clone(&wants), win.window.clone());
        b.set_callback(move |_| {
            flag.set(true);
            w.hide();
        });
    }

    centre(&mut win.window);
    win.window.show();
    while win.window.shown() {
        if !app::wait() {
            break;
        }
    }
    win.window.hide();
    wants.get()
}

/// Start this program again and stop being it.
///
/// Operator, 2026-08-12: *"if we need the user to reboot say so and try to
/// self reboot or something."* This is the second half. It re-execs **the same
/// binary with the same arguments** — it is a restart and never an update, and
/// the difference is the whole of why this is allowed to exist: the client
/// still refuses to replace its own bytes, because a binary that rewrites
/// itself is the one part of this program that would need absolute trust
/// (`docs/client/LAUNCHER.md` §8; the update notice says the same thing one
/// screen over).
///
/// ⚠ **`exit` and not a return, and the ordering matters.** The game door is a
/// unix socket the child will unlink and rebind (`transport::Listener::bind`),
/// and this process holds a listener whose `Drop` removes that path — so a
/// tidy shutdown here would delete the socket the child just made.
/// `std::process::exit` runs no destructors, which is what makes handing the
/// door over safe.
pub fn restart_now() -> Result<std::convert::Infallible, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this program: {e}"))?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::Command::new(&exe)
        .args(&args)
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    std::process::exit(0);
}

/// Show a new account's recovery words and wait. `true` means the player
/// says they wrote them down; anything else writes nothing.
///
/// Injected like [`Ask`] so a harness can read the words it was shown and
/// answer the check that follows.
pub type ShowWords = Rc<dyn Fn(&[&str]) -> bool>;

pub fn real_show_words() -> ShowWords {
    Rc::new(show_words_in_window)
}

/// The words window, parked on like [`show_notice`]. Every exit but "I wrote
/// them down" is a cancel.
pub fn show_words_in_window(words: &[&str]) -> bool {
    let mut win = windows::recovery_words(words);
    let wrote = Rc::new(std::cell::Cell::new(false));
    let (flag, mut w) = (Rc::clone(&wrote), win.window.clone());
    win.ok.set_callback(move |_| {
        flag.set(true);
        w.hide();
    });
    let mut w2 = win.window.clone();
    win.cancel.set_callback(move |_| w2.hide());
    centre(&mut win.window);
    win.window.show();
    while win.window.shown() {
        if !app::wait() {
            break;
        }
    }
    // Deleted rather than hidden, so the words do not sit in a live widget
    // tree for the rest of the session.
    fltk::window::Window::delete(win.window);
    wrote.get()
}

/// Make and import — the whole of "how do I get an account".
///
/// **Nothing here touches the network.** The key is generated by the OS pool,
/// encrypted, and written, all on this machine (`CLAUDE.md` invariant 7).
///
/// Making one is words first: shown once, three asked back, and only then a
/// passphrase and a file. The moment before the account exists is the only
/// moment a backup can be insisted on.
pub fn wire_account(
    account_w: &AccountWindow,
    signing_w: &SigningWindow,
    signer: &Shared,
    keystore: &Path,
    ask: Ask,
    tell: Tell,
    show: ShowWords,
) {
    if let Some(b) = account_w.make.clone() {
        let mut b = b;
        let (signer, aw, sw, ks) = (
            Arc::clone(signer),
            account_w.clone(),
            signing_w.clone(),
            keystore.to_path_buf(),
        );
        let (ask, tell) = (Rc::clone(&ask), Rc::clone(&tell));
        b.set_callback(move |_| {
            if elo_depot::streamer::on() {
                return tell(Told::Refused,
                            "Streamer mode is on, and your recovery words would be on stream.\n\n\
                             Turn it off on the main menu, then make the account.");
            }
            let phrase = match elo_vault::Phrase::generate() {
                Ok(p) => p,
                Err(e) => return tell(Told::Refused, &e),
            };
            if !show(&phrase.words()) {
                return;
            }
            let quiz = match phrase.quiz() {
                Ok(q) => q,
                Err(e) => return tell(Told::Refused, &e),
            };
            let Some(answer) = ask("A new account", &format!(
                "Type words {}, {} and {} from your list, with spaces between:",
                quiz[0], quiz[1], quiz[2])) else {
                return;
            };
            if !phrase.check(&quiz, &answer) {
                return tell(Told::Refused,
                            "Those are not the words — nothing was written.\n\n\
                             Press Make an account again for a new set, and write them down.");
            }
            let Some(pass) = ask_twice(&ask, &tell, "A new account", &format!(
                "Choose a passphrase for it (at least {} characters):",
                elo_vault::local::MIN_PASSPHRASE)) else {
                return;
            };
            match elo_vault::LocalSigner::restore(&ks, &pass, &phrase) {
                Ok(s) => adopt(&signer, &aw, &sw, &ks, s, &tell, Arrived::Made),
                Err(e) => tell(Told::Refused, &e),
            }
        });
    }

    if let Some(b) = account_w.import.clone() {
        let mut b = b;
        let (signer, aw, sw, ks) = (
            Arc::clone(signer),
            account_w.clone(),
            signing_w.clone(),
            keystore.to_path_buf(),
        );
        let (ask, tell) = (Rc::clone(&ask), Rc::clone(&tell));
        b.set_callback(move |_| {
            // Masked like a passphrase: this input IS the account, words or
            // key, and it is the one field in this program that must never be
            // painted.
            let Some(raw) = ask("Restore or import",
                                "Your recovery words, or a private key (64 hex):") else {
                return;
            };
            let made = if raw.split_whitespace().count() > 1 {
                let phrase = match elo_vault::Phrase::parse(&raw) {
                    Ok(p) => p,
                    Err(e) => return tell(Told::Refused, &e),
                };
                let Some(pass) = ask_twice(&ask, &tell, "Restore or import",
                                           "Choose a passphrase to encrypt it with:") else {
                    return;
                };
                elo_vault::LocalSigner::restore(&ks, &pass, &phrase).map(|s| (s, Arrived::Restored))
            } else {
                let secret = match parse_secret(&raw) {
                    Ok(s) => s,
                    Err(e) => return tell(Told::Refused, &e),
                };
                let Some(pass) = ask_twice(&ask, &tell, "Restore or import",
                                           "Choose a passphrase to encrypt it with:") else {
                    return;
                };
                elo_vault::LocalSigner::import(&ks, &pass, secret.as_slice())
                    .map(|s| (s, Arrived::Imported))
            };
            match made {
                Ok((s, how)) => adopt(&signer, &aw, &sw, &ks, s, &tell, how),
                Err(e) => tell(Told::Refused, &e),
            }
        });
    }
}

// ── the wallet ──────────────────────────────────────────────────────────────

/// Show a send's review and wait. `true` means Send was pressed; every other
/// exit is a cancel. Injected like [`Ask`] so a harness can press it.
pub type Review = Rc<dyn Fn(&[(String, String)], &[String]) -> bool>;

pub fn real_review() -> Review {
    Rc::new(review_in_window)
}

pub fn review_in_window(lines: &[(String, String)], notes: &[String]) -> bool {
    let mut win = windows::send_review(lines, notes);
    let sent = Rc::new(std::cell::Cell::new(false));
    let (flag, mut w) = (Rc::clone(&sent), win.window.clone());
    win.send.set_callback(move |_| {
        flag.set(true);
        w.hide();
    });
    let mut w2 = win.window.clone();
    win.cancel.set_callback(move |_| w2.hide());
    centre(&mut win.window);
    win.window.show();
    while win.window.shown() {
        if !app::wait() {
            break;
        }
    }
    win.window.hide();
    sent.get()
}

/// What the Wallet window reads and opens, injected so a harness can stand a
/// fake chain behind the real controls.
#[derive(Clone)]
pub struct WalletDesk {
    /// A chain to talk to, made on whichever thread does the reading.
    pub chain: Arc<dyn Fn() -> Box<dyn elo_net::chain::Rpc> + Send + Sync>,
    /// The origin's coin list (`/api/holdings`), or why it did not load.
    pub listed: Arc<dyn Fn() -> Result<serde_json::Value, String> + Send + Sync>,
    /// Where sent-to addresses are remembered (`elo_vault::recipients_path`).
    pub recipients: PathBuf,
    /// Open a url in the browser.
    pub open: Rc<dyn Fn(&str)>,
}

impl WalletDesk {
    pub fn real(host: &str, open: Rc<dyn Fn(&str)>) -> WalletDesk {
        let url = format!("{host}/api/holdings");
        WalletDesk {
            chain: Arc::new(|| Box::new(elo_net::chain::HttpRpc::new())),
            listed: Arc::new(move || {
                let got = elo_net::Net::new().get_json(&url);
                got.value.ok_or(got.why)
            }),
            recipients: elo_vault::recipients_path(),
            open,
        }
    }
}

/// Run `work` on a thread and hand its answer to `done` on this one — the
/// shape `show_profile` uses, so a dead RPC never freezes a window.
fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    let mut done = Some(done);
    app::add_timeout3(0.1, move |h| match rx.try_recv() {
        Ok(v) => {
            if let Some(d) = done.take() {
                d(v);
            }
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => app::repeat_timeout3(0.1, h),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
    });
}

/// Paint a read onto the window, and remember its coins for the send row.
pub fn paint_wallet(w: &windows::WalletWindow, snap: &elo_net::wallet::Snapshot,
                    coins: &RefCell<Vec<elo_net::wallet::Coin>>) {
    use elo_net::wallet::format_amount;
    let mut text = String::new();
    for (c, h) in &snap.coins {
        let line = match h {
            Ok(r) => format!("{:>30} {:<6} {}", format_amount(*r, c.decimals), c.sym, c.what),
            Err(e) => format!("{:>30} {:<6} could not read: {e}", "?", c.sym),
        };
        text.push_str(&elo_hive::render::safe_line(&line));
        text.push('\n');
    }
    if let Some(mut b) = w.balances.buffer() {
        b.set_text(&text);
    }
    let mut warning = w.warning.clone();
    let (said, ink) = if let Some(x) = &snap.warning {
        (x.clone(), theme::RED)
    } else if snap.unreachable() {
        ("Could not read the chain, so nothing above is a balance. Refresh to try again.".into(),
         theme::RED)
    } else {
        (snap.note.clone().unwrap_or_default(), theme::GOLD)
    };
    warning.set_label(&crate::chrome::defuse(&said));
    warning.set_label_color(ink);

    let mut choice = w.coin.clone();
    let was = choice.value();
    choice.clear();
    for (c, _) in &snap.coins {
        // A menu label treats `/`, `&` and `_` as structure; a symbol is the
        // contract's own text, so only its letters and digits are drawn.
        let label: String = c.sym.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect();
        choice.add_choice(if label.is_empty() { "?" } else { &label });
    }
    choice.set_value(if was >= 0 && (was as usize) < snap.coins.len() { was } else { 0 });
    *coins.borrow_mut() = snap.coins.iter().map(|(c, _)| c.clone()).collect();
    if let Some(mut win) = w.balances.window() {
        win.redraw();
    }
}

/// The Wallet window's controls. Returns what opening the window should do:
/// read the chain afresh, because a balance is a measurement and an old one
/// shown as current is the thing this client refuses.
pub fn wire_wallet(
    w: &windows::WalletWindow,
    signer: &Shared,
    desk: WalletDesk,
    ask: Ask,
    tell: Tell,
    review: Review,
    then: Then,
) -> Rc<dyn Fn()> {
    let coins: Rc<RefCell<Vec<elo_net::wallet::Coin>>> = Rc::default();
    let who = {
        let signer = Arc::clone(signer);
        move || -> Option<[u8; 20]> {
            let a = signer.lock().ok()?.address()?;
            elo_vault::parse_address(&a).ok()
        }
    };

    let refresh: Rc<dyn Fn()> = {
        let (w, coins, desk, who) = (w.clone(), Rc::clone(&coins), desk.clone(), who.clone());
        Rc::new(move || {
            let Some(me) = who() else { return };
            // An account made in Account after this window was built: show it
            // and turn the controls on in place, with no restart.
            let mut addr = w.address.clone();
            addr.set_label(&elo_depot::streamer::shown(&elo_vault::to_checksum(&me)));
            addr.set_label_color(theme::INK);
            addr.set_label_font(fltk::enums::Font::Courier);
            for b in [&w.copy, &w.explorer, &w.refresh, &w.review] {
                b.clone().activate();
            }
            w.coin.clone().activate();
            w.amount.clone().activate();
            w.to.clone().activate();
            let mut st = w.status.clone();
            st.set_label("reading the chain…");
            let (chain, listed) = (Arc::clone(&desk.chain), Arc::clone(&desk.listed));
            let (w2, coins2) = (w.clone(), Rc::clone(&coins));
            off_thread(
                move || {
                    let rpc = chain();
                    let list = listed();
                    elo_net::wallet::snapshot(&*rpc, list.as_ref().map_err(String::as_str), &me)
                },
                move |snap| {
                    paint_wallet(&w2, &snap, &coins2);
                    let mut st = w2.status.clone();
                    st.set_label("");
                },
            );
        })
    };

    {
        let mut b = w.refresh.clone();
        let r = Rc::clone(&refresh);
        b.set_callback(move |_| r());
    }
    {
        let mut b = w.copy.clone();
        let (who, st) = (who.clone(), w.status.clone());
        b.set_callback(move |_| {
            if let Some(me) = who() {
                app::copy(&elo_vault::to_checksum(&me));
                let mut st = st.clone();
                st.set_label("copied — check it where you paste it");
            }
        });
    }
    {
        let mut b = w.explorer.clone();
        let (who, open) = (who.clone(), Rc::clone(&desk.open));
        b.set_callback(move |_| {
            if let Some(me) = who() {
                open(&elo_net::chain::address_url(&elo_vault::to_checksum(&me)));
            }
        });
    }

    {
        let mut b = w.review.clone();
        let (w, signer, coins, refresh) = (w.clone(), Arc::clone(signer), Rc::clone(&coins), Rc::clone(&refresh));
        b.set_callback(move |btn| {
            let Some(me) = who() else { return };
            let idx = w.coin.value();
            let Some(coin) = (idx >= 0).then(|| coins.borrow().get(idx as usize).cloned()).flatten() else {
                return tell(Told::Refused, "Pick a coin first — Refresh reads them.");
            };
            let (to, amount) = (w.to.value(), w.amount.value());
            let known = elo_net::wallet::known_recipients(&desk.recipients);
            let mut st = w.status.clone();
            st.set_label("checking with the chain…");
            let mut btn = btn.clone();
            btn.deactivate();
            let chain = Arc::clone(&desk.chain);
            let after = {
                let (w, signer, desk, ask, tell, review, then, refresh) =
                    (w.clone(), Arc::clone(&signer), desk.clone(), Rc::clone(&ask), Rc::clone(&tell),
                     Rc::clone(&review), Rc::clone(&then), Rc::clone(&refresh));
                move |planned: Result<elo_net::wallet::Plan, String>| {
                    btn.activate();
                    let mut st = w.status.clone();
                    st.set_label("");
                    let sent = finish_send(planned, &signer, &desk, &ask, &tell, &review, &then);
                    if sent {
                        let (mut a, mut t) = (w.amount.clone(), w.to.clone());
                        a.set_value("");
                        t.set_value("");
                        refresh();
                    }
                }
            };
            off_thread(
                move || {
                    let rpc = chain();
                    elo_net::wallet::plan(&*rpc, &me, &coin, &to, &amount, &known)
                },
                after,
            );
        });
    }
    refresh
}

/// Everything after a plan comes back: the review, the passphrase, the
/// signature and the broadcast. Returns whether a send went out.
fn finish_send(
    planned: Result<elo_net::wallet::Plan, String>,
    signer: &Shared,
    desk: &WalletDesk,
    ask: &Ask,
    tell: &Tell,
    review: &Review,
    then: &Then,
) -> bool {
    let plan = match planned {
        Ok(p) => p,
        Err(e) => {
            tell(Told::Refused, &e);
            return false;
        }
    };
    let lines = match elo_net::wallet::review(&plan) {
        Ok(l) => l,
        Err(e) => {
            tell(Told::Refused, &e);
            return false;
        }
    };
    if !review(&lines, &plan.notes) {
        return false;
    }
    let what = lines[0].1.clone();
    let Some(pass) = ask("Send", &format!("Your passphrase, to send {what}:")) else {
        return false;
    };
    let signed = match signer.lock() {
        Ok(s) => s.sign_transaction(&pass, &plan.tx),
        Err(_) => Err("the account is busy — try again".into()),
    };
    drop(pass);
    let signed = match signed {
        Ok(s) => s,
        Err(e) => {
            tell(Told::Refused, &format!("Nothing was sent.\n\n{e}"));
            return false;
        }
    };
    let rpc = (desk.chain)();
    if let Err(e) = elo_net::chain::send_raw(&*rpc, &signed.raw_hex()) {
        tell(Told::Refused, &format!("The chain did not take it, so nothing was sent.\n\n{e}"));
        return false;
    }
    let _ = elo_net::wallet::remember_recipient(&desk.recipients, &plan.to);
    let hash = signed.hash_hex();
    if then(&format!("Sent {what} to\n{}\n\nIt lands in a few seconds.\n{hash}",
                     elo_vault::to_checksum(&plan.to)),
            "View on explorer") {
        (desk.open)(&elo_net::chain::tx_url(&hash));
    }
    true
}

/// Unlock and its inverse — the two acts that decide whether a key is in
/// memory, and the only ones Signing offers.
pub fn wire_signing(
    signing_w: &SigningWindow,
    signer: &Shared,
    ask: Ask,
    tell: Tell,
) {
    // Lock first, because it must work even if unlock's wiring ever grows a
    // way to fail: forgetting the key is the button that may not be dead.
    // No dialog and no confirmation — locking costs a passphrase to undo,
    // which is exactly the cost it is supposed to impose.
    if let Some(b) = signing_w.lock.clone() {
        let mut b = b;
        let signer = Arc::clone(signer);
        let mut status = signing_w.status.clone();
        b.set_callback(move |_| {
            if let Ok(mut s) = signer.lock() {
                s.lock();
                status.set_label(&s.status());
                app::redraw();
            }
        });
    }

    let Some(b) = signing_w.unlock.clone() else {
        return;
    };
    let mut b = b;
    let signer = Arc::clone(signer);
    let mut status = signing_w.status.clone();
    b.set_callback(move |_| {
        let Some(pass) = ask("Unlock this account", "It stays unlocked until this launcher closes.") else {
            return;
        };
        let mut s = signer.lock().expect("signer lock");
        match s.unlock(&pass) {
            Ok(()) => {
                status.set_label(&s.status());
                app::redraw();
                tell(
                    Told::Done,
                    "Unlocked for this session.\n\nIt is forgotten when this program closes.",
                );
            }
            // The vault draws the line between a wrong passphrase and a corrupt
            // file. Do not blur it back by rewording the message here.
            Err(e) => tell(Told::Refused, &e),
        }
    });
}

// ── the two shelves, and the buttons that used to do nothing ────────────────
//
// Operator, 2026-08-12: *"i litarelly cant click anything to buy the game or
// anything."* They were right twice over. The Games window painted a **Play**
// button per row and dropped the widget on the floor, so the library's one
// verb was a picture. The Store painted no control at all — two labels a row,
// and no way to install, buy, or even read about a title. Both windows hand
// their controls back now (`windows::GamesWindow`, `windows::StoreWindow`) and
// these are what put behaviour on them.

/// What a shelf row needs done, decided here so the window stays free of I/O
/// and the binary stays free of policy.
pub trait Storefront {
    /// Fetch and hash-verify a title's published build. Long — the caller
    /// blocks the UI thread on it, deliberately: an install with a spinner
    /// this client cannot draw is worse than one that plainly holds still.
    fn install(&self, slug: &str) -> Result<String, String>;
    /// Re-hash everything a build's depot names.
    fn verify(&self, slug: &str, build: &str) -> Result<String, String>;
    /// Start what is installed.
    fn play(&self, slug: &str) -> Result<String, String>;
    /// Is a game this launcher started for `slug` still running?
    fn running(&self, _slug: &str) -> bool {
        false
    }
    /// Open a url in the player's browser. The buy step and the store page
    /// both come through here, because this client holds no wallet.
    fn open(&self, url: &str) -> Result<(), String>;
    /// The title's page on the origin — where a buy actually happens.
    fn page_url(&self, slug: &str) -> String;
}

/// Give the Games window's rows their behaviour.
///
/// **Play and Update are the same button and a different verb**, which is why
/// the label is read back off [`crate::windows::Row::action`] rather than
/// guessed: a row saying *"an update is published"* over a Play button is the
/// exact defect `tests/rows.rs` exists for, and wiring it as Play would have
/// re-introduced it one layer down.
pub fn wire_games(
    games_w: &crate::windows::GamesWindow,
    front: Rc<dyn Storefront>,
    tell: Tell,
    then: Then,
) {
    for row in &games_w.rows {
        let (slug, build) = (row.slug.clone(), row.build.clone());
        // Read when the button is pressed, never captured once at wiring. It
        // was a plain `bool`, so after an update the button said Play and kept
        // updating: a press while playing (Space on the focused button counts)
        // re-downloaded the build the game was running from.
        let updating = Rc::new(std::cell::Cell::new(row.act.label() == "Update"));
        let (front_c, tell_c) = (Rc::clone(&front), Rc::clone(&tell));
        let then_c = Rc::clone(&then);
        let mut act = row.act.clone();
        if !updating.get() && front.running(&slug) {
            hold_while_running(&act, Rc::clone(&front), slug.clone());
        }
        act.set_callback(move |b| {
            let update = updating.get();
            b.deactivate();
            b.set_label(if update { "Updating…" } else { "Starting…" });
            app::redraw();
            app::flush();
            let done = if update {
                front_c.install(&slug)
            } else {
                front_c.play(&slug)
            };
            match done {
                Ok(said) if update => {
                    updating.set(false);
                    b.set_label("Play");
                    b.activate();
                    // An update finishes on the row the player is already
                    // looking at, so the offer is the same one the Store makes:
                    // the step after "it updated" is playing it.
                    if then_c(&said, "Play now") {
                        match front_c.play(&slug) {
                            Ok(_) => hold_while_running(b, Rc::clone(&front_c), slug.clone()),
                            Err(e) => tell_c(Told::Refused, &e),
                        }
                    }
                }
                Ok(_) => hold_while_running(b, Rc::clone(&front_c), slug.clone()),
                Err(e) => {
                    // Said on the control the player pressed AND in a window.
                    // A failure reported only to a console nobody is reading
                    // has not been reported.
                    b.set_label("Failed");
                    b.activate();
                    tell_c(Told::Refused, &e);
                }
            }
            app::redraw();
        });

        let (slug, build) = (row.slug.clone(), build);
        let (front_c, tell_c) = (Rc::clone(&front), Rc::clone(&tell));
        let mut verify = row.verify.clone();
        verify.set_callback(move |b| {
            b.deactivate();
            b.set_label("Hashing…");
            app::redraw();
            app::flush();
            match front_c.verify(&slug, &build) {
                Ok(said) => {
                    b.set_label("Verify");
                    tell_c(Told::Done, &said);
                }
                Err(e) => {
                    b.set_label("Verify");
                    tell_c(Told::Refused, &e);
                }
            }
            b.activate();
            app::redraw();
        });
    }
}

/// How often a Running button checks whether its game has exited, in seconds.
const RUNNING_POLL: f64 = 1.0;

/// Show `Running` on a row's button and keep it off until the game exits,
/// then put Play back. While it is off nothing can start a second copy or an
/// install under the running one.
fn hold_while_running(b: &fltk::button::Button, front: Rc<dyn Storefront>, slug: String) {
    let mut b = b.clone();
    b.set_label("Running");
    b.deactivate();
    app::add_timeout3(RUNNING_POLL, move |handle| {
        // A refreshed Games window drops this button; its replacement row
        // checks `running` for itself when it is wired.
        if b.was_deleted() {
            return;
        }
        if front.running(&slug) {
            app::repeat_timeout3(RUNNING_POLL, handle);
        } else {
            b.set_label("Play");
            b.activate();
            app::redraw();
        }
    });
}

/// Give the Store's rows their behaviour.
///
/// **Buy opens a browser and this is not a limitation to route around.** A
/// purchase is a wallet signing a transaction; this client holds no wallet by
/// design and the whole buy box already exists as a page that has been driven
/// end to end (`TICKET.md` §8a). What the launcher owns is the half after the
/// money: install, verify, play.
///
/// `after_install` is called once an install lands. The binary re-reads both
/// shelves with it — the Store row becomes *Installed* and the game appears in
/// the library — because the alternative is a message telling a player their
/// game is in a window that will not show it until the next launch.
pub fn wire_store(
    store_w: &crate::windows::StoreWindow,
    front: Rc<dyn Storefront>,
    tell: Tell,
    then: Then,
    after_install: Rc<dyn Fn()>,
) {
    use crate::windows::Act;
    for row in &store_w.rows {
        let slug = row.slug.clone();
        let act = row.act;
        let (front_c, tell_c) = (Rc::clone(&front), Rc::clone(&tell));
        let then_c = Rc::clone(&then);
        let after = Rc::clone(&after_install);
        let mut button = row.button.clone();
        button.set_callback(move |b| match act {
            Act::Install => {
                b.deactivate();
                b.set_label("Installing…");
                app::redraw();
                app::flush();
                match front_c.install(&slug) {
                    Ok(said) => {
                        b.set_label("Installed");
                        // The shelves are re-read BEFORE the notice, not after:
                        // if the player takes the Play offer, the library it
                        // starts from has to be the one holding this install.
                        after();
                        if then_c(&said, "Play now") {
                            if let Err(e) = front_c.play(&slug) {
                                tell_c(Told::Refused, &e);
                            }
                        }
                    }
                    Err(e) => {
                        b.set_label("Install");
                        b.activate();
                        tell_c(Told::Refused, &e);
                    }
                }
                app::redraw();
            }
            Act::Buy | Act::Page => {
                let url = front_c.page_url(&slug);
                if let Err(e) = front_c.open(&url) {
                    tell_c(
                        Told::Refused,
                        &format!("{e}\n\nOpen it yourself:\n{url}"),
                    );
                }
            }
            // Already yours — so the next step is playing it, not being told
            // which window to go and look in.
            Act::Installed => {
                if then_c("You already have this one.", "Play now") {
                    if let Err(e) = front_c.play(&slug) {
                        tell_c(Told::Refused, &e);
                    }
                }
            }
            // Not pressable, so this is unreachable by a click. Kept as a real
            // arm rather than a catch-all so adding a sixth `Act` fails to
            // compile instead of silently doing nothing.
            Act::Unpublished => {}
        });

        let slug = row.slug.clone();
        let (front_c, tell_c) = (Rc::clone(&front), Rc::clone(&tell));
        let mut page = row.page.clone();
        page.set_callback(move |_| {
            let url = front_c.page_url(&slug);
            if let Err(e) = front_c.open(&url) {
                tell_c(Told::Refused, &format!("{e}\n\nOpen it yourself:\n{url}"));
            }
        });
    }
}

/// How often the main thread looks for a game waiting at the door.
///
/// A poll rather than `app::awake`, and the reason is soundness: waking FLTK
/// from the door thread means handing it a closure built over there, and the
/// closure this one needs opens windows. 150ms is under what anyone perceives
/// as lag on a prompt a human is about to read, and an idle launcher spends
/// nothing measurable on it.
const DOORBELL_POLL: f64 = 0.15;

/// Show the real consent prompt and wait for the player.
///
/// **Runs a nested event loop**, which is what every modal in this toolkit
/// does — `dialog::password_default` above is the same shape. The door thread
/// is parked on the answer this returns, so the loop ends only when a button
/// is pressed or the window goes away.
///
/// ⚠ **Every exit that is not a button is a refusal.** The window manager's
/// close, the app quitting out from under the prompt, a main loop that stops
/// dispatching — all of them leave `answer` untouched at `None`. That
/// direction is not arbitrary: a wrong refusal costs a click, and a wrong
/// grant costs a signature the player never approved.
pub fn ask_consent(signer: &Shared, ask: &crate::consent::Ask) -> Option<u32> {
    // Read the account state BEFORE the window exists, and drop the lock
    // before the nested loop starts. Holding it across the prompt would freeze
    // the Signing window's Unlock button for exactly as long as the prompt is
    // up — which is the one moment a player is most likely to press it.
    let (address, unlocked) = match signer.lock() {
        Ok(s) => (s.address(), s.is_unlocked()),
        Err(_) => (None, false),
    };

    let mut win = crate::windows::consent(ask, address.as_deref(), unlocked);
    let answer = wire_consent(&win);

    centre(&mut win.window);
    win.window.show();
    while win.window.shown() {
        // `wait` returning false is the app shutting down. Breaking here is
        // what stops a closing launcher from parking the door thread forever
        // on a window that will never be answered.
        if !app::wait() {
            break;
        }
    }
    win.window.hide();
    answer.get()
}

/// Give a consent window's buttons their meaning, and hand back the cell they
/// write into.
///
/// Split out of [`ask_consent`] so `examples/click.rs` can press the **real**
/// buttons: a nested event loop is the one part of a modal a test cannot
/// drive, so it is the only part left outside this function. Everything that
/// decides what a press means is in here, where a test can reach it.
pub fn wire_consent(win: &crate::windows::ConsentWindow) -> Rc<std::cell::Cell<Option<u32>>> {
    let answer = Rc::new(std::cell::Cell::new(None::<u32>));

    for (i, b) in win.allow.iter().enumerate() {
        // Paired by index with the table the window drew from, rather than by
        // reading the label back off the button. A grant is a number and the
        // label is decoration; recovering one from the other would put "how
        // many signatures" behind a string comparison.
        let Some((_, grant)) = crate::consent::ALLOWANCES.get(i).copied() else {
            continue;
        };
        let (mut b, answer, mut w) = (b.clone(), Rc::clone(&answer), win.window.clone());
        b.set_callback(move |_| {
            answer.set(Some(grant));
            w.hide();
        });
    }

    let (refused, mut w) = (Rc::clone(&answer), win.window.clone());
    let mut refuse = win.refuse.clone();
    refuse.set_callback(move |_| {
        // Deliberately writes no value: `None` is already what the cell holds,
        // and Refuse only ends the wait. Anything else here would mean the
        // refusal path had a value that could be got wrong.
        refused.set(None);
        w.hide();
    });

    answer
}

/// Answer the door for as long as the launcher is open.
///
/// Registers a repeating timeout on FLTK's thread — the one thread allowed to
/// open a window — and drains [`crate::consent::Doorbell`] on each tick.
///
/// ⚠ **The re-entrancy guard is defensive, and the ordering below is what
/// actually holds today.** `ask_consent` runs a nested event loop and FLTK
/// dispatches timeouts inside one — but this timeout is not re-armed until
/// `pump` returns, so it cannot fire while a prompt is up. The flag exists
/// because that safety is a property of *where* `repeat_timeout3` sits, and
/// moving it to the top of the callback is the most natural refactor in the
/// world. Without the flag, that edit silently starts stacking a second game's
/// prompt over the one the player is reading — in the one window where their
/// answer authorises something.
pub fn serve_consent(bell: crate::consent::Doorbell, signer: Shared) {
    let busy = Rc::new(std::cell::Cell::new(false));
    app::add_timeout3(DOORBELL_POLL, move |handle| {
        if !busy.get() {
            busy.set(true);
            bell.pump(&mut |ask| ask_consent(&signer, ask));
            busy.set(false);
        }
        app::repeat_timeout3(DOORBELL_POLL, handle);
    });
}

/// The two origin calls the website pairing makes, injectable so the flow is
/// testable with no network — the same seam [`Ask`] is for dialogs.
pub trait SigninHttp {
    /// `GET {host}/api/signin/{code}/challenge` — is this code real and waiting?
    fn challenge(&self, host: &str, code: &str) -> elo_net::Fetched<serde_json::Value>;
    /// `POST {host}/api/signin/{code}/proof` — deliver the signed message.
    fn deliver(&self, host: &str, code: &str, body: &serde_json::Value)
        -> elo_net::Fetched<serde_json::Value>;
}

pub struct RealSigninHttp;

impl SigninHttp for RealSigninHttp {
    fn challenge(&self, host: &str, code: &str) -> elo_net::Fetched<serde_json::Value> {
        elo_net::Net::new().get_json(&format!("{host}/api/signin/{code}/challenge"))
    }
    fn deliver(&self, host: &str, code: &str, body: &serde_json::Value)
        -> elo_net::Fetched<serde_json::Value> {
        elo_net::Net::new().post_json_told(&format!("{host}/api/signin/{code}/proof"), body)
    }
}

/// Pair a browser with this account — the GUI half of `elo signin`
/// (`docs/client/SIGN-IN.md`; the origin is `meter/signin.py`).
///
/// Returns the sentence to show the player, or the refusal.
///
/// Two properties are the point, and both are for the machine someone else
/// can sit down at:
///
/// * **The passphrase is asked EVERY time, unlocked or not.** A pairing signs
///   a browser in *as this account*, so it is an act of the
///   passphrase-holder — an already-unlocked launcher must not let whoever is
///   at the keyboard pair their own browser. `unlock` re-derives from the
///   file, so answering it IS proving the passphrase.
/// * **A transient unlock stays transient.** If the account was locked before
///   the pairing, it is locked again after — signing a browser in leaves no
///   live signer behind.
///
/// What the GUI pairing deliberately does not do: watch the relay. Approving
/// a page's signature requests is a stream of prompts, and the CLI
/// (`elo signin`, attached in a terminal) is the surface built for that. A
/// code works once, so the watch wants its own code, typed there.
pub fn pair_browser(
    signer: &Shared,
    host: &str,
    raw_code: &str,
    ask: &Ask,
    http: &dyn SigninHttp,
) -> Result<String, String> {
    let code: String = raw_code
        .trim()
        .to_uppercase()
        .chars()
        .filter(|c| *c != '-')
        .collect();
    elo_broker::protocol::check_nonce(&code).map_err(|r| {
        format!(
            "That is not a sign-in code — {}. The page shows one like ABCD-2345.",
            r.reason
        )
    })?;

    let (exists, address, was_unlocked) = {
        let s = signer.lock().map_err(|_| "the signer is busy".to_string())?;
        (s.exists(), s.address(), s.is_unlocked())
    };
    if !exists {
        return Err(
            "No account on this machine — make one first. The pairing is this \
             account proving itself to a browser, so there is nothing to pair yet."
                .into(),
        );
    }
    let address = address.ok_or_else(|| "the keystore names no address".to_string())?;

    // Look before the passphrase: a typo costs one request, never a dialog.
    let asked = http.challenge(host, &code);
    if asked.value.is_none() {
        return Err(match asked.status {
            Some(404) => format!(
                "{code} is not waiting to sign in — it expired or was mistyped. \
                 The browser page shows a fresh one."
            ),
            Some(409) => "That code was already used — a sign-in code works once.".into(),
            _ if !asked.reachable => format!("could not reach {host} — {}", asked.why),
            _ => format!("the origin refused the code — {}", asked.why),
        });
    }

    let domain = host
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(host)
        .trim_end_matches('/')
        .to_string();
    let message = elo_broker::protocol::signin_message(
        &domain,
        &address,
        &code,
        &elo_broker::protocol::now_iso8601(),
    );

    // The full message rides in the prompt: the player reads what is signed
    // in the same dialog that asks for the passphrase to sign it.
    let prompt = format!("About to sign exactly this, and nothing else:\n\n{message}");
    let Some(pass) = ask("Sign the browser in", &prompt) else {
        return Err("Cancelled — nothing was signed.".into());
    };

    let out = {
        let mut s = signer.lock().map_err(|_| "the signer is busy".to_string())?;
        s.unlock(&pass)?;
        let signed = s.sign(&message, "sign the website in").map_err(|r| r.reason);
        if !was_unlocked {
            s.lock();
        }
        signed?
    };
    let sig = out
        .get("signature")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    if sig.is_empty() {
        return Err("the signer returned no signature".into());
    }

    let resp = http.deliver(
        host,
        &code,
        &serde_json::json!({ "message": message, "signature": sig }),
    );
    let Some(v) = resp.value else {
        return Err(format!("could not deliver the proof — {}", resp.why));
    };
    if v.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(format!(
            "the origin refused the proof — {}",
            v.get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&resp.why)
        ));
    }
    Ok(format!(
        "Signed in — the browser showing {code} now reads as\n{address}\n\n\
         No session was created anywhere: the page holds a proof, and every act\n\
         still asks for its own signature. To also approve that page's signature\n\
         requests, pair a fresh code from a terminal: elo signin <code>"
    ))
}

/// Give the Account window's pairing row its meaning.
///
/// ⚠ The two origin calls run on the UI thread and block it — the same trade
/// the binary's startup reads make, and short on a healthy origin. If it ever
/// grows past two calls it wants the doorbell's thread shape instead.
pub fn wire_signin(
    account_w: &AccountWindow,
    signer: &Shared,
    host: &str,
    ask: Ask,
    tell: Tell,
    http: Rc<dyn SigninHttp>,
) {
    let (Some(b), Some(code_in)) = (account_w.signin.clone(), account_w.signin_code.clone())
    else {
        return;
    };
    let mut b = b;
    let signer = Arc::clone(signer);
    let host = host.to_string();
    b.set_callback(move |_| {
        let raw = code_in.value();
        match pair_browser(&signer, &host, &raw, &ask, http.as_ref()) {
            Ok(msg) => {
                let mut ci = code_in.clone();
                ci.set_value("");
                tell(Told::Done, &msg);
            }
            Err(e) => tell(Told::Refused, &e),
        }
    });
}

/// How often the pairing poll looks. Slower than a spinner and faster than a
/// person: the visitor is switching to a browser, connecting a wallet and
/// reading a message, so a two-second beat is well inside their own tempo and
/// costs the origin one small GET.
const PAIR_POLL: f64 = 2.0;

/// Lend the launcher the wallet in a browser — the GUI half of `elo pair`
/// (`docs/client/SIGN-IN.md` §8; the origin is `meter/browser_signer.py`).
///
/// The mirror of [`pair_browser`] above, and the differences are the
/// interesting part:
///
/// * **No passphrase, because there is no key here to prove.** `pair_browser`
///   asks for one every time precisely because it acts AS this machine's
///   account; this acts as nothing — it asks a browser to start answering, and
///   the browser's own wallet is the thing that consents.
/// * **This one polls, and that is why it cannot be a straight call.** The
///   code has to be shown here and typed there, so the wait is a human's, not
///   a request's. A blocking loop would freeze the window for up to ten
///   minutes; `add_timeout3` is the same non-blocking beat [`serve_relock`]
///   runs on.
/// * **The code is SHOWN here and TYPED there** — §1's rule reflected, and it
///   does not weaken in the mirror: a code a visitor read off their own
///   launcher cannot be a stranger's. There is no deep link on purpose.
///
/// What lands on disk when it works is a code, a secret and an address, and
/// **none of the three can sign**. The browser's key stays in the browser.
pub fn wire_pair(account_w: &AccountWindow, host: &str, tell: Tell) {
    let Some(b) = account_w.pair.clone() else { return };
    let mut b = b;
    let host = host.trim_end_matches('/').to_string();
    let note = account_w.pair_note.clone();
    let row = account_w.pair_row.clone();
    let shown = account_w.clone();

    // Copy puts the code on the clipboard and SAYS it did. A button that
    // silently succeeds is one a player presses three times and then retypes
    // the code anyway, which is the state this whole row was added to end.
    if let Some(r) = row.clone() {
        let (c, mut cb) = (r.code.clone(), r.copy.clone());
        cb.set_callback(move |btn| {
            let v = c.value();
            if v.is_empty() {
                return;
            }
            app::copy(&v);
            btn.set_label("Copied");
            app::redraw();
        });
    }
    // The PAGE, never the code. `{host}/pair.html` carries nothing, so what
    // reaches the form is still only what the player carried there themselves
    // — `SIGN-IN.md` §1 intact.
    if let Some(r) = row.clone() {
        let mut ob = r.open.clone();
        let (host_o, tell_o) = (host.clone(), Rc::clone(&tell));
        ob.set_callback(move |_| {
            let url = format!("{host_o}/pair.html");
            if elo_depot::launch::open_page(&url).is_err() {
                tell_o(Told::Refused, &format!("Could not open a browser.\n\nGo to:\n{url}"));
            }
        });
    }
    // One at a time. Without this, a second press mints a second code and the
    // first poller keeps running against a pairing nobody will ever claim —
    // two timeouts, two codes, and a window that reports whichever finishes.
    let waiting = Rc::new(std::cell::Cell::new(false));
    b.set_callback(move |btn| {
        if waiting.get() {
            return;
        }
        let net = elo_net::Net::new();
        let (code, secret, _) = match elo_net::pairing::start(&net, &host) {
            Ok(got) => got,
            Err(e) => return tell(Told::Refused, &e),
        };
        let show = if code.len() == 8 {
            format!("{}-{}", &code[..4], &code[4..])
        } else {
            code.clone()
        };
        let mut note = note.clone();
        // Both the code AND its warning belong to the row now, so this label
        // is status only and stays empty until there is something to report.
        note.set_label("");
        if let Some(mut r) = row.clone() {
            r.code.set_value(&show);
            // A second press mints a second code, so the button goes back to
            // "Copy" — leaving "Copied" there would name the old one.
            r.copy.set_label("Copy");
            r.show();
        }
        let mut btn = btn.clone();
        btn.deactivate();
        waiting.set(true);
        app::redraw();

        let (host, waiting) = (host.clone(), Rc::clone(&waiting));
        let tell = Rc::clone(&tell);
        // The poller outlives this press, so it takes its own handles to the
        // three widgets rather than borrowing the ones this closure keeps for
        // the NEXT press.
        let row = row.clone();
        let shown = shown.clone();
        let began = std::time::Instant::now();
        app::add_timeout3(PAIR_POLL, move |handle| {
            match elo_net::pairing::poll_claim(&net, &host, &code, &secret) {
                elo_net::pairing::Claim::Paired(p) => {
                    let path = elo_vault::pairing_path();
                    match elo_net::pairing::save(&path, &p) {
                        Ok(()) => {
                            show_pairing(&shown, &p.address);
                            // The code is spent. Take the row away rather than
                            // leave a stale code sitting under a Copy button
                            // that would hand someone eight dead characters.
                            if let Some(mut r) = row.clone() {
                                r.code.set_value("");
                                r.hide();
                            }
                            tell(Told::Done, &format!(
                                "Paired with {}.\n\nThe launcher will ask that browser \
                                 to sign, one message at a time. Nothing that can sign \
                                 was written to this machine.",
                                p.address
                            ));
                        }
                        // The pairing is real at the origin and we could not
                        // remember it. Say exactly that — "pairing failed"
                        // would send someone to redo a thing that worked.
                        Err(e) => tell(Told::Refused, &format!(
                            "The browser claimed the code, but the pairing could not be \
                             written to disk: {e}"
                        )),
                    }
                    btn.activate();
                    waiting.set(false);
                    app::redraw();
                }
                elo_net::pairing::Claim::Gone(why) => {
                    note.set_label("");
                    tell(Told::Refused, &why);
                    btn.activate();
                    waiting.set(false);
                    app::redraw();
                }
                elo_net::pairing::Claim::Pending => {
                    if began.elapsed() >= elo_net::pairing::CLAIM_TIMEOUT {
                        note.set_label("");
                        tell(Told::Refused, "Nobody claimed the code in time. Press again \
                                             for a fresh one.");
                        btn.activate();
                        waiting.set(false);
                        app::redraw();
                    } else {
                        app::repeat_timeout3(PAIR_POLL, handle);
                    }
                }
            }
        });
    });
}

/// Say in Account that a browser wallet is paired.
///
/// Called when a pairing lands and at startup when one is already live. With
/// no account on this machine the paired wallet IS who this launcher is, so it
/// goes in the address well; before this the well said *"no address set —
/// games play anonymously"* over a launcher that was handing games the paired
/// address.
pub fn show_pairing(account_w: &AccountWindow, address: &str) {
    let safe = crate::chrome::defuse(address);
    if account_w.make.is_some() {
        let (mut a, mut n) = (account_w.address.clone(), account_w.note.clone());
        a.set_label(&safe);
        a.set_label_color(theme::INK);
        n.set_label("your browser wallet — approve each message in the pair tab");
    }
    let mut note = account_w.pair_note.clone();
    // Short enough to fit the window with a full address in it; the longer
    // version ran off the edge at "close the tab to end…".
    note.set_label(&format!("paired with {safe} — keep the pair tab open"));
    app::redraw();
}

/// The published client version NEWER than `mine`, off the origin's
/// `/api/launcher` card — or `None`, which deliberately covers four different
/// states: current, ahead (a dev build is not nagged), no client published,
/// and a card we could not read. The caller must therefore never render
/// `None` as "up to date" — the corner stays a bare local version, which
/// claims nothing about currency. Same three-answer discipline as `elo
/// check`, whose comment also carries the ruling this UI half inherits: **a
/// notice, never a self-update** — a binary that replaces itself must be
/// trusted absolutely, and the package manager or the download page does the
/// swap in the open.
pub fn newer_client(card: Option<&serde_json::Value>, mine: &str) -> Option<String> {
    let there = card?.get("client")?.get("version")?.as_str()?;
    (elo_depot::update::version_cmp(there, mine) == std::cmp::Ordering::Greater)
        .then(|| there.to_string())
}

/// How often the idle relock looks at the clock. Coarse on purpose: the
/// threshold is minutes, and a check that ran hot would be spending wakeups
/// to gain seconds of precision on a lock nobody is racing.
const RELOCK_POLL: f64 = 30.0;

/// Relock the unlocked key after it sits idle.
///
/// "Unlocked for this session" was the whole story, and for a machine that is
/// only ever the player's own it is a fine one — but a launcher left open on a
/// shared desk holds a live signer for anyone who sits down next. This is the
/// bounded version: an unlock nobody has used since lunch stops being one.
///
/// **A running game counts as use.** `playing` answers whether one is, and
/// while it says yes the clock does not run. Signatures alone were never the
/// clock of play: a game proves once when it joins and then plays for hours
/// without signing, so this fired half an hour into a session and threw the
/// passphrase window up over the game (operator, 2026-10-17: *"launcher is
/// popping up while i play games"*). The clock starts again when the game
/// exits, so leaving the launcher open after playing still relocks.
///
/// `on_browser` runs when the player answers the gate by choosing the wallet in
/// their browser instead (see [`lock_screen`]).
///
/// `after` of zero disables it — that is the operator's own machine saying so
/// (`ELO_RELOCK_MINUTES=0`), not a default.
///
/// What this is NOT: a security boundary against a process running as you.
/// `local.rs` says it first — the process that draws the window holds the key,
/// and rung 2's answer is the arca, not a timer. This narrows the WINDOW a
/// walk-by has, which is real, and nothing more.
pub fn serve_relock(
    signer: Shared,
    signing_w: &SigningWindow,
    after: std::time::Duration,
    gate: bool,
    playing: Rc<dyn Fn() -> bool>,
    on_browser: Rc<dyn Fn()>,
) {
    if after.is_zero() {
        return;
    }
    let mut status = signing_w.status.clone();
    let busy = Rc::new(std::cell::Cell::new(false));
    let last_play = std::cell::Cell::new(None::<std::time::Instant>);
    app::add_timeout3(RELOCK_POLL, move |handle| {
        // The same re-entrancy guard `serve_consent` carries, for the same
        // reason: the gate below runs a nested event loop, and FLTK dispatches
        // timeouts inside one. Without this, a player who walks away from the
        // lock screen gets a second one stacked on it every thirty seconds.
        if !busy.get() {
            busy.set(true);
            if playing() {
                last_play.set(Some(std::time::Instant::now()));
            }
            let since_play = last_play.get().map(|t| t.elapsed());
            let relocked = match signer.lock() {
                Ok(mut s) if relock_due(s.idle(), since_play, after) => {
                    s.lock();
                    status.set_label(&s.status());
                    app::redraw();
                    true
                }
                _ => false,
            };
            // **The relock puts the gate back up**, which is the whole of
            // *"when i return to the app it should prompt for my passphrase"*
            // (operator, 2026-08-12): the case that ask is really about is
            // walking away and coming back, and that is exactly when this
            // fires. Locking silently — what it did before — meant the player
            // came back to a client that looked unlocked and a game that would
            // be told "locked" the next time it asked.
            if relocked && gate {
                let answered = lock_screen(
                    &signer,
                    "it locked itself while you were away — playing or signing resets that clock",
                );
                if answered == Gate::Browser {
                    on_browser();
                }
                let mut st = status.clone();
                if let Ok(s) = signer.lock() {
                    st.set_label(&s.status());
                }
                app::redraw();
            }
            busy.set(false);
        }
        app::repeat_timeout3(RELOCK_POLL, handle);
    });
}

/// Whether the idle relock is due: the key has sat unused for `after`, and no
/// game has been running for at least as long. `since_play` is `None` when no
/// game has run this session.
fn relock_due(
    idle: Option<std::time::Duration>,
    since_play: Option<std::time::Duration>,
    after: std::time::Duration,
) -> bool {
    idle.is_some_and(|i| i >= after) && since_play.is_none_or(|p| p >= after)
}

/// Whether the lock screen is armed at all. `ELO_LOCK_SCREEN=0` is the posted
/// door out (`CLAUDE.md` invariant 10), for the person who keeps this client
/// open on their own machine and does not want a gate in front of the store.
pub fn lock_screen_armed() -> bool {
    !matches!(
        std::env::var("ELO_LOCK_SCREEN").as_deref().map(str::trim),
        Ok("0") | Ok("off") | Ok("no")
    )
}

/// How the player got past [`lock_screen`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// There was nothing to gate: no keystore, already unlocked, the screen is
    /// off, or the program is shutting down.
    Open,
    /// They typed the passphrase.
    Unlocked,
    /// They chose the wallet in their browser. **The key on this machine stays
    /// locked**; the caller shows them where to pair it.
    Browser,
}

/// Hold the client behind a passphrase until it opens, the player picks their
/// browser wallet instead, or they quit.
///
/// The ask was *"prompt for my passphrase and not go pass that"* (2026-08-12),
/// and a passphrase is still the only thing that UNLOCKS. But a gate with only
/// Unlock and Quit stranded a player whose wallet lives in their browser:
/// with an account on this machine they could not reach the pairing at all
/// without the local passphrase (operator, 2026-10-17: *"i dont see how i can
/// use my metamask chrome to sign into the launcher anymore"*). Choosing the
/// browser leaves the key locked, so nothing the gate protects is exposed.
/// Quit ends the process rather than returning a value some caller might
/// ignore.
///
/// Three states walk straight through and none of them is a bypass:
///
/// * **no keystore** — there is nothing to unlock. Playing with no account is
///   a normal state and always has been (`docs/client/LAUNCHER.md` §1).
/// * **already unlocked** — the gate is for a locked key.
/// * **`ELO_LOCK_SCREEN=0`** — the operator's own machine saying so.
pub fn lock_screen(signer: &Shared, why: &str) -> Gate {
    if !lock_screen_armed() {
        return Gate::Open;
    }
    let (exists, unlocked, address) = match signer.lock() {
        Ok(s) => (s.exists(), s.is_unlocked(), s.address().unwrap_or_default()),
        // A poisoned lock means another thread panicked holding the signer.
        // Gating on it would strand the player in front of a window that can
        // never open, so this walks through — the account stays locked and
        // every act that needs it still refuses with its own reason.
        Err(_) => return Gate::Open,
    };
    if !exists || unlocked {
        return Gate::Open;
    }

    let mut win = crate::windows::lock(&address, why);
    let done = Rc::new(std::cell::Cell::new(false));
    let browser = Rc::new(std::cell::Cell::new(false));
    {
        let (flag, mut w) = (Rc::clone(&browser), win.window.clone());
        win.browser.set_callback(move |_| {
            flag.set(true);
            w.hide();
        });
    }

    // Quit is a real quit. It is the only other way out of this window, and
    // the window manager's close box does the same thing rather than dropping
    // the player into an unlocked-looking client.
    let mut quit = win.quit.clone();
    quit.set_callback(|_| std::process::exit(0));
    win.window.set_callback(|_| std::process::exit(0));

    let signer_c = Arc::clone(signer);
    let (flag, field, mut status, mut w) = (
        Rc::clone(&done),
        win.field.clone(),
        win.status.clone(),
        win.window.clone(),
    );
    let mut try_unlock = move || {
        let typed = Secret::new(field.value());
        let mut s = match signer_c.lock() {
            Ok(s) => s,
            Err(_) => {
                status.set_label("the account is busy — try again");
                return;
            }
        };
        match s.unlock(&typed) {
            Ok(()) => {
                flag.set(true);
                w.hide();
            }
            // The vault draws the line between a wrong passphrase and a
            // corrupt file, and it is said in place rather than in a second
            // window — a modal over a modal is how a player loses the field
            // they were typing in.
            Err(e) => {
                let mut f = field.clone();
                f.set_value("");
                let _ = f.take_focus();
                status.set_label(&e);
                app::redraw();
            }
        }
    };
    let mut unlock = win.unlock.clone();
    let mut on_click = try_unlock.clone();
    unlock.set_callback(move |_| on_click());
    let mut field = win.field.clone();
    field.set_callback(move |_| try_unlock());

    centre(&mut win.window);
    win.window.show();
    let _ = win.field.take_focus();
    while win.window.shown() && !done.get() {
        // `wait` returning false is the app shutting down. Breaking on it is
        // what stops a quitting launcher from spinning here forever.
        if !app::wait() {
            break;
        }
    }
    win.window.hide();
    if browser.get() {
        Gate::Browser
    } else if done.get() {
        Gate::Unlocked
    } else {
        Gate::Open
    }
}

fn ask_twice(ask: &Ask, tell: &Tell, heading: &str, prompt: &str) -> Option<Secret> {
    let first = ask(heading, prompt)?;
    let again = ask(heading, "Type it again — it cannot be reset later:")?;
    if first != again {
        tell(Told::Refused, "Those two did not match — nothing was written.");
        return None;
    }
    Some(first)
}

/// How an account arrived: what [`adopt`] says about its backup, and whether
/// it may already have a profile (one made here has none yet).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Arrived {
    Made,
    Restored,
    Imported,
}

/// Take a freshly made account as this launcher's, and say so on screen.
///
/// Updates in place rather than rebuilding: a rebuilt window loses where the
/// player put it and reads as a flicker.
fn adopt(
    signer: &Shared,
    account_w: &AccountWindow,
    signing_w: &SigningWindow,
    keystore: &Path,
    new_signer: elo_vault::LocalSigner,
    tell: &Tell,
    how: Arrived,
) {
    let address = new_signer.address().unwrap_or_default();
    let status = new_signer.status();
    *signer.lock().expect("signer lock") = new_signer;

    let mut addr = account_w.address.clone();
    let mut note = account_w.note.clone();
    addr.set_label(&address);
    addr.set_label_color(theme::INK);
    note.set_label("made on this machine, and transmitted nowhere");
    // Both doors close: this window never offers to replace a key, and the
    // vault would refuse anyway. Their place goes to the profile, which an
    // account has from its first moment — a made key has none set yet, an
    // imported one may, so that one is read.
    for b in [account_w.make.clone(), account_w.import.clone()].into_iter().flatten() {
        let mut b = b;
        b.deactivate();
        b.hide();
    }
    if let Some(mut e) = account_w.edit_profile.clone() {
        e.show();
    }
    if let (Some(mut face), Some(mut name)) = (account_w.face.clone(), account_w.profile_name.clone()) {
        face.show();
        name.show();
        if how == Arrived::Made {
            paint_profile(&face, &name, &Ok(crate::profile::Shown::default()));
        } else {
            show_profile(account_w, &account_w.host, &address);
        }
    }
    let mut st = signing_w.status.clone();
    st.set_label(&status);
    // ⚠ **The restart that used to be asked for happens here instead.** This
    // message ended *"Restart the launcher to unlock it in Signing"* until
    // 2026-08-12, because `signing()` decided whether to build an Unlock
    // button from a keystore that did not exist when the window was built. It
    // builds them deactivated now, so a fresh account turns them on in place
    // and nobody restarts a program to gain a button (operator, 2026-08-12).
    for b in [signing_w.unlock.clone(), signing_w.lock.clone()].into_iter().flatten() {
        let mut b = b;
        b.activate();
    }
    app::redraw();

    let (heading, backup) = match how {
        Arrived::Made => ("Account created.",
                          "Your recovery words are its backup: they bring it back in any wallet.\n\
                           It is empty until something is sent to it."),
        Arrived::Restored => ("Account restored.",
                              "Your recovery words stay its backup."),
        Arrived::Imported => ("Key imported.",
                              "A bare key has no recovery words: keep the key, or the keystore\n\
                               file below, backed up."),
    };
    tell(
        Told::Done,
        &format!(
            "{heading}\n\n{address}\n\n{backup}\n\
             The keystore is at {}.\n\n\
             The passphrase cannot be reset — nobody else has it, including elo.\n\n\
             Signing can unlock it now; no restart is needed.",
            keystore.display()
        ),
    );
}

/// 32 bytes from hex.
///
/// ⚠ **No input ever reaches the error.** The input is a private key, and a
/// message that echoed a mistyped one would carry it into a screenshot, a
/// terminal scrollback or a bug report. Say the shape, never the value.
pub fn parse_secret(raw: &str) -> Result<elo_vault::Zeroizing<[u8; 32]>, String> {
    let hex = raw.trim();
    let hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    if hex.len() != 64 {
        return Err(format!(
            "A private key is 64 hex characters (32 bytes); that was {}.",
            hex.len()
        ));
    }
    // Checked as bytes before any slicing: 64 BYTES of a paste with "é" in it
    // pass the length check, and a two-byte slice inside a char panics.
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("That is not hexadecimal — nothing was written.".into());
    }
    let mut out = elo_vault::Zeroizing::new([0u8; 32]);
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| "That is not hexadecimal — nothing was written.".to_string())?;
    }
    Ok(out)
}

/// A scripted [`Ask`] — hands back the given answers in order, then `None`.
///
/// Lives beside the thing it doubles rather than in a test file, because
/// `examples/click.rs` uses it too and a second copy would drift.
pub fn scripted_ask(answers: Vec<Option<String>>) -> Ask {
    let queue = RefCell::new(answers.into_iter());
    Rc::new(move |_heading, _prompt| queue.borrow_mut().next().flatten().map(Secret::new))
}

/// A scripted [`ShowWords`]: keeps what it was shown, and says they were
/// written down.
pub fn recording_show(seen: Rc<RefCell<Vec<String>>>) -> ShowWords {
    Rc::new(move |words| {
        *seen.borrow_mut() = words.iter().map(|w| w.to_string()).collect();
        true
    })
}

/// An [`Ask`] that answers the words check from what [`recording_show`] saw,
/// and hands back `rest` in order for everything else.
pub fn quiz_answering_ask(seen: Rc<RefCell<Vec<String>>>, rest: Vec<Option<String>>) -> Ask {
    let rest = scripted_ask(rest);
    Rc::new(move |heading, prompt| {
        let Some(tail) = prompt.strip_prefix("Type words ") else {
            return rest(heading, prompt);
        };
        let words = seen.borrow();
        let picks: Vec<String> = tail
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|n| n.parse::<usize>().ok())
            .filter_map(|n| words.get(n.checked_sub(1)?).cloned())
            .collect();
        Some(Secret::new(picks.join(" ")))
    })
}

/// Somewhere for a test to read what the player was told.
pub fn recording_tell(log: Rc<RefCell<Vec<(Told, String)>>>) -> Tell {
    Rc::new(move |kind, text| log.borrow_mut().push((kind, text.to_string())))
}

/// Where the keystore lives, for a caller that wants to show it.
pub fn keystore_path() -> PathBuf {
    elo_vault::keystore_path()
}

/// Change this account's profile: one signature over the text the origin
/// prepares, then the change itself.
///
/// The order is the point, as it is in [`pair_browser`]: every refusal that
/// can happen without the passphrase happens first — no account, nothing to
/// change, a name or picture the origin would refuse, an origin that will not
/// prepare — so a player only ever types the passphrase over a change that
/// will land. The prompt carries the exact text being signed.
pub fn set_profile(
    signer: &Shared,
    host: &str,
    change: &crate::profile::Change,
    ask: &Ask,
    http: &dyn crate::profile::FaceHttp,
) -> Result<String, String> {
    use crate::profile::{b64, name_error, png_error};
    use sha2::{Digest, Sha256};

    let (exists, address, was_unlocked) = {
        let s = signer.lock().map_err(|_| "the signer is busy".to_string())?;
        (s.exists(), s.address(), s.is_unlocked())
    };
    if !exists {
        return Err("No account on this machine — a profile belongs to an account, \
                    so make one first.".into());
    }
    let wallet = address.ok_or_else(|| "the keystore names no address".to_string())?;
    if change.is_empty() {
        return Err("Nothing to change — type a name or choose a picture.".into());
    }
    let name = change.name.as_deref().map(str::trim);
    if let Some(e) = name.and_then(name_error) {
        return Err(e);
    }
    if let Some(e) = change.picture.as_deref().and_then(png_error) {
        return Err(e);
    }
    let sha = change.picture.as_deref().map(|p| elo_depot::digest::hex_lower(&Sha256::digest(p)));

    let mut ask_body = serde_json::json!({ "wallet": wallet });
    if let Some(n) = name {
        ask_body["name"] = serde_json::json!(n);
    }
    if let Some(h) = &sha {
        ask_body["picture_sha256"] = serde_json::json!(h);
    }
    let prepared = http.prepare(host, &ask_body);
    let Some(p) = prepared.value else {
        return Err(if prepared.reachable {
            format!("the origin would not prepare the change — {}", prepared.why)
        } else {
            format!("could not reach {host} — {}", prepared.why)
        });
    };
    let text = p.get("sign_this").and_then(serde_json::Value::as_str).unwrap_or("");
    let at = p.get("at").and_then(serde_json::Value::as_i64);
    let (Some(at), false) = (at, text.is_empty()) else {
        return Err("the origin's reply named nothing to sign".into());
    };

    let prompt = format!("About to sign exactly this, and nothing else:\n\n{text}");
    let Some(pass) = ask("Change your profile", &prompt) else {
        return Err("Cancelled — nothing was signed.".into());
    };
    let signed = {
        let mut s = signer.lock().map_err(|_| "the signer is busy".to_string())?;
        s.unlock(&pass)?;
        let out = s.sign(text, "change this account's profile").map_err(|r| r.reason);
        if !was_unlocked {
            s.lock();
        }
        out?
    };
    let sig = signed.get("signature").and_then(serde_json::Value::as_str).unwrap_or("");
    if sig.is_empty() {
        return Err("the signer returned no signature".into());
    }

    let mut body = serde_json::json!({ "wallet": wallet, "at": at, "signature": sig });
    if let Some(n) = name {
        body["name"] = serde_json::json!(n);
    }
    if let Some(pic) = &change.picture {
        body["picture_b64"] = serde_json::json!(b64(pic));
    }
    let saved = http.save(host, &body);
    let Some(v) = saved.value else {
        return Err(format!("could not save the change — {}", saved.why));
    };
    if let Some(e) = v.get("error").and_then(serde_json::Value::as_str) {
        return Err(format!("the origin refused the change — {e}"));
    }
    let shown = v.get("name").and_then(serde_json::Value::as_str).unwrap_or("no name");
    Ok(format!("Saved. The store, the games and chat now show {shown} for\n{wallet}."))
}

/// Fill the Account window's face and name from `/api/face`, off the UI
/// thread. A read that could not be made says so on the name line; it never
/// reads as "no profile".
pub fn show_profile(account_w: &AccountWindow, host: &str, wallet: &str) {
    let (Some(face), Some(name)) = (account_w.face.clone(), account_w.profile_name.clone()) else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let (host, wallet) = (host.to_string(), wallet.to_string());
        std::thread::spawn(move || {
            let _ = tx.send(crate::profile::read(&crate::profile::RealFaceHttp, &host, &wallet));
        });
    }
    let shown: Rc<RefCell<Option<crate::profile::Shown>>> = Rc::default();
    let keep = Rc::clone(&shown);
    app::add_timeout3(0.2, move |h| match rx.try_recv() {
        Ok(got) => {
            paint_profile(&face, &name, &got);
            *keep.borrow_mut() = got.ok();
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => app::repeat_timeout3(0.2, h),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
    });
}

/// Put a read profile on the Account window.
pub fn paint_profile(face: &fltk::frame::Frame, name: &fltk::frame::Frame,
                     got: &Result<crate::profile::Shown, String>) {
    let (mut face, mut name) = (face.clone(), name.clone());
    match got {
        Ok(s) => {
            let drawn = s
                .picture
                .clone()
                .and_then(crate::art::Icon::decode)
                .is_some_and(|i| i.draw_into(&mut face, windows::PROFILE_FACE));
            if drawn {
                face.set_label("");
            } else {
                face.set_image(None::<fltk::image::PngImage>);
                tile(&mut face, s.name.as_deref(), 28);
            }
            let line = match &s.name {
                Some(n) => format!("profile name: {}", elo_hive::render::safe_line(n)),
                None => "no profile name or picture yet — Edit profile sets them".into(),
            };
            name.set_label(&crate::chrome::defuse(&line));
        }
        Err(why) => {
            let line = format!("could not read the profile — {}", elo_hive::render::safe_line(why));
            name.set_label(&crate::chrome::defuse(&line));
        }
    }
    if let Some(mut w) = face.window() {
        w.redraw();
    }
}

/// A frame with no picture wears the name's letter, as a chat face does —
/// drawn as a label through the escape, because the name is the origin's.
fn tile(f: &mut fltk::frame::Frame, name: Option<&str>, size: i32) {
    let letter = crate::chatpane::initial_of(name.unwrap_or(""));
    f.set_label(&crate::chrome::defuse(&letter));
    f.set_label_color(theme::DIM);
    f.set_label_size(size);
    f.set_align(fltk::enums::Align::Center | fltk::enums::Align::Inside);
}

/// Give **Edit profile…** its behaviour: the editor, then the signed change.
pub fn wire_profile(account_w: &AccountWindow, signer: &Shared, host: &str, ask: Ask,
                    tell: Tell) {
    let Some(mut edit) = account_w.edit_profile.clone() else { return };
    let (signer, host, acct) = (Arc::clone(signer), host.to_string(), account_w.clone());
    edit.set_callback(move |_| {
        let wallet = signer.lock().ok().and_then(|s| s.address());
        let Some(wallet) = wallet else { return };
        let current = crate::profile::read(&crate::profile::RealFaceHttp, &host, &wallet).ok();
        let Some(change) = edit_profile_in_window(current.as_ref()) else { return };
        match set_profile(&signer, &host, &change, &ask, &crate::profile::RealFaceHttp) {
            Ok(done) => {
                tell(Told::Done, &done);
                show_profile(&acct, &host, &wallet);
            }
            Err(e) => tell(Told::Refused, &e),
        }
    });
}

/// Run the profile editor and hand back what the player asked to change, or
/// `None` for Cancel. Checks a picked file against the picture rules as soon
/// as it is picked, so Save is never pressed over a file that cannot go.
fn edit_profile_in_window(current: Option<&crate::profile::Shown>) -> Option<crate::profile::Change> {
    let mut win = windows::profile_editor(current.and_then(|c| c.name.as_deref()));
    let had = current
        .and_then(|c| c.picture.clone())
        .and_then(crate::art::Icon::decode)
        .is_some_and(|p| p.draw_into(&mut win.preview, 96));
    if !had {
        tile(&mut win.preview, current.and_then(|c| c.name.as_deref()), 44);
    }
    let picture: Rc<RefCell<Option<Vec<u8>>>> = Rc::default();
    let answer: Rc<RefCell<Option<crate::profile::Change>>> = Rc::default();

    {
        let (picture, mut preview, mut picked, mut status) =
            (Rc::clone(&picture), win.preview.clone(), win.picked.clone(), win.status.clone());
        win.pick.clone().set_callback(move |_| {
            let mut chooser = fltk::dialog::NativeFileChooser::new(
                fltk::dialog::NativeFileChooserType::BrowseFile);
            chooser.set_filter("PNG\t*.png");
            if chooser.try_show().is_err() {
                return;
            }
            let path = chooser.filename();
            if path.as_os_str().is_empty() {
                return;
            }
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    status.set_label(&crate::chrome::defuse(&format!("could not read it — {e}")));
                    return;
                }
            };
            if let Some(e) = crate::profile::png_error(&bytes) {
                status.set_label(&crate::chrome::defuse(&e));
                return;
            }
            status.set_label("");
            let shown = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            picked.set_label(&crate::chrome::defuse(&shown));
            if crate::art::Icon::decode(bytes.clone()).is_some_and(|i| i.draw_into(&mut preview, 96)) {
                preview.set_label("");
            }
            *picture.borrow_mut() = Some(bytes);
            if let Some(mut w) = preview.window() {
                w.redraw();
            }
        });
    }
    {
        let before = current.and_then(|c| c.name.clone()).unwrap_or_default();
        let (picture, answer, name, mut status, mut w) = (
            Rc::clone(&picture), Rc::clone(&answer), win.name.clone(), win.status.clone(),
            win.window.clone());
        win.save.clone().set_callback(move |_| {
            let typed = name.value().trim().to_string();
            let change = crate::profile::Change {
                name: (typed != before).then_some(typed),
                picture: picture.borrow().clone(),
            };
            if change.is_empty() {
                status.set_label("nothing to change yet");
                return;
            }
            if let Some(e) = change.name.as_deref().and_then(crate::profile::name_error) {
                status.set_label(&crate::chrome::defuse(&e));
                return;
            }
            *answer.borrow_mut() = Some(change);
            w.hide();
        });
    }
    {
        let mut w = win.window.clone();
        win.cancel.clone().set_callback(move |_| w.hide());
    }

    centre(&mut win.window);
    win.window.show();
    while win.window.shown() {
        if !app::wait() {
            break;
        }
    }
    win.window.hide();
    let got = answer.borrow_mut().take();
    got
}

// ── friends ──────────────────────────────────────────────────────────────────

/// Who signs a friends change made in the window.
pub enum FriendSigner {
    /// The account on this machine: a passphrase prompt that carries the
    /// exact text, then the keystore signs it.
    Local(Shared),
    /// The wallet in a paired browser: its holder reads and approves each
    /// text there, so nothing is asked here.
    Paired(Box<dyn Signer>),
    /// Neither — the change is finished on the website, where the wallet is.
    Neither,
}

impl FriendSigner {
    /// The signer for the wallet the Friends window lists, reached in the
    /// order its read uses (`elo-gui`'s `known_address`): an unlocked account,
    /// then a live browser pairing, then a locked account. One order, so a
    /// button on a row of one wallet's list never signs as another.
    pub fn pick(signer: &Shared, pairing: Option<elo_net::pairing::Pairing>) -> FriendSigner {
        let (exists, unlocked) = signer
            .lock()
            .map(|s| (s.exists(), s.is_unlocked()))
            .unwrap_or((false, false));
        if exists && unlocked {
            return FriendSigner::Local(Arc::clone(signer));
        }
        if let Some(p) = pairing {
            return FriendSigner::Paired(Box::new(elo_net::pairing::PairedBrowserSigner::new(p)));
        }
        if exists {
            return FriendSigner::Local(Arc::clone(signer));
        }
        FriendSigner::Neither
    }

    /// The wallet it signs as, lowercased. Reading it costs no passphrase.
    pub fn address(&self) -> Option<String> {
        let a = match self {
            FriendSigner::Local(s) => s.lock().ok().filter(|s| s.exists()).and_then(|s| s.address()),
            FriendSigner::Paired(p) => p.address(),
            FriendSigner::Neither => None,
        };
        a.map(|w| w.trim().to_ascii_lowercase())
    }
}

/// What a friends change came to.
#[derive(Debug)]
pub enum Changed {
    /// It landed. The list as the origin answered it after the change, so the
    /// window repaints from it with no second read.
    Done(elo_net::friends::View),
    /// No signer here: the change is finished at this url, in the browser.
    HandedOff(String),
}

/// One friends change from the window — `op` about `who`, a wallet or a name.
///
/// [`set_profile`]'s order: everything that can be refused without the
/// passphrase is refused first — not a change, nothing typed, a name nobody
/// goes by, your own wallet, an origin that will not prepare it or prepares a
/// different text ([`crate::friendsop::checked_text`]) — and only then is the
/// player asked, with the exact text in the prompt. A paired browser signs
/// with no prompt here: its holder approves the text in the browser. With
/// neither signer, the change is handed to the website.
///
/// `ask` is a plain `Fn` rather than an [`Ask`] so the flow can run off the
/// UI thread, with the prompt carried back to it (`elo-gui`'s `wire_friends`).
pub fn change_friend(
    signer: &mut FriendSigner,
    host: &str,
    op: &str,
    who: &str,
    ask: &dyn Fn(&str, &str) -> Option<Secret>,
    http: &dyn crate::friendsop::FriendsHttp,
) -> Result<Changed, String> {
    use elo_net::friends::{is_wallet, op_key, short};
    if op_key(op).is_none() {
        return Err(format!("{op:?} is not a friends change"));
    }
    let handle = who.trim();
    if handle.is_empty() {
        return Err("Type a wallet (0x…) or a name first.".into());
    }
    if !is_wallet(handle) {
        elo_hive::who::check_handle(handle).map_err(|e| e.to_string())?;
    }
    let handoff = || Changed::HandedOff(format!("{}/profile.html", host.trim_end_matches('/')));
    if matches!(signer, FriendSigner::Neither) {
        return Ok(handoff());
    }
    let me = signer
        .address()
        .filter(|w| is_wallet(w))
        .ok_or("the signer names no address")?;

    let other = if is_wallet(handle) {
        handle.to_ascii_lowercase()
    } else {
        let got = http.who(host, handle);
        let Some(id) = got.value else {
            return Err(if got.reachable {
                got.why
            } else {
                format!("could not reach the register — {}", got.why)
            });
        };
        id.wallet
            .filter(|w| is_wallet(w))
            .map(|w| w.trim().to_ascii_lowercase())
            .ok_or_else(|| {
                format!("no wallet goes by {} — use their 0x address",
                        elo_hive::render::safe_line(handle))
            })?
    };
    if other == me {
        return Err("That is your own wallet.".into());
    }

    let why = format!("friends: {op} {}", short(&other));
    let heading = crate::friendsop::heading(op);
    let signature_of = |out: serde_json::Value| {
        out.get("signature")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "the signer returned no signature".to_string())
    };
    let view = match signer {
        FriendSigner::Local(shared) => {
            let was_unlocked = shared
                .lock()
                .map_err(|_| "the signer is busy".to_string())?
                .is_unlocked();
            let mut sign = |text: &str| -> Result<String, String> {
                let prompt = format!("About to sign exactly this, and nothing else:\n\n{text}");
                let Some(pass) = ask(heading, &prompt) else {
                    return Err("Cancelled — nothing was signed.".into());
                };
                let mut s = shared.lock().map_err(|_| "the signer is busy".to_string())?;
                s.unlock(&pass)?;
                let out = s.sign(text, &why).map_err(|r| r.reason);
                if !was_unlocked {
                    s.lock();
                }
                signature_of(out?)
            };
            crate::friendsop::submit(http, host, &me, op, &other, &mut sign)?
        }
        FriendSigner::Paired(p) => {
            let mut sign = |text: &str| signature_of(p.sign(text, &why).map_err(|r| r.reason)?);
            crate::friendsop::submit(http, host, &me, op, &other, &mut sign)?
        }
        FriendSigner::Neither => return Ok(handoff()),
    };
    Ok(Changed::Done(view))
}

/// Ask the player to approve one more act, showing what it is: `(title, body,
/// button)` → pressed. The real one is a notice with an action button.
pub type Confirm = Rc<dyn Fn(&str, &str, &str) -> bool>;

pub fn real_confirm() -> Confirm {
    Rc::new(|title, body, button| show_notice(Note::Done, title, body, Some(button)))
}

/// What the Chat window needs to offer **Link to my wallet** and **Unlink**.
#[derive(Clone)]
pub struct ChatLink {
    pub signer: Shared,
    pub ask: Ask,
    pub confirm: Confirm,
    pub tell: Tell,
}

/// The account, and whether it was already unlocked — or why there is none.
fn chat_account(signer: &Shared, what: &str) -> Result<(String, bool), String> {
    let s = signer.lock().map_err(|_| "the signer is busy".to_string())?;
    if !s.exists() {
        return Err(format!("No account on this machine — {what}, so make one in Account first."));
    }
    let wallet = s.address().ok_or("the keystore names no address")?.to_lowercase();
    Ok((wallet, s.is_unlocked()))
}

fn relock(signer: &Shared, was_unlocked: bool) {
    if !was_unlocked {
        if let Ok(mut s) = signer.lock() {
            s.lock();
        }
    }
}

/// The account as `elo_hive::link` asks for signatures: **every text shown
/// before it is signed.** Locked, the passphrase prompt shows the text, and
/// typing the passphrase is the consent; unlocked, a confirmation shows it.
/// It unlocks at most once, and the caller relocks.
fn showing_signer<'a>(signer: &'a Shared, ask: &'a Ask, confirm: &'a Confirm, title: &'a str)
    -> impl FnMut(&elo_hive::link::Request) -> Result<String, String> + 'a {
    const CANCELLED: &str = "Cancelled — nothing was signed.";
    move |r| {
        let unlocked = signer.lock().map_err(|_| "the signer is busy".to_string())?.is_unlocked();
        if unlocked {
            if !confirm(title, &format!("{}\n\n{}", r.about, r.text), r.button) {
                return Err(CANCELLED.into());
            }
        } else {
            let prompt = format!("About to sign exactly this, and nothing else:\n\n{}\n\n{}",
                                 r.text, r.about);
            let Some(pass) = ask(title, &prompt) else { return Err(CANCELLED.into()) };
            signer.lock().map_err(|_| "the signer is busy".to_string())?.unlock(&pass)?;
        }
        let signed = signer.lock().map_err(|_| "the signer is busy".to_string())?
            .sign(r.text, r.why).map_err(|refused| refused.reason)?;
        signed.get("signature").and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty()).map(str::to_string)
            .ok_or_else(|| "the account returned no signature".to_string())
    }
}

/// Link this machine's chat key to the account's wallet (`elo_hive::link`).
///
/// **The key linked is the wallet's own**: the account signs the text its chat
/// key is derived from, so the key is the one elopros.com makes from the same
/// wallet and one link covers both. One passphrase, and every text shown
/// before it is signed: the derivation text in the passphrase prompt, the link
/// text in a confirmation after it. A key that was on this machine before is
/// kept beside the new one, never deleted.
pub fn link_chat_key(
    signer: &Shared,
    host: &str,
    voice_dir: &Path,
    ask: &Ask,
    confirm: &Confirm,
    http: &dyn elo_hive::link::LinkHttp,
) -> Result<String, String> {
    let (wallet, was_unlocked) =
        chat_account(signer, "linking is the account signing for its chat key")?;
    let outcome = {
        let mut sign = showing_signer(signer, ask, confirm, "Link your chat key");
        elo_hive::link::link(http, host, &wallet, voice_dir, &mut sign)
    };
    relock(signer, was_unlocked);
    let done = outcome?;
    let kept = done.kept
        .map(|p| format!("\n\nThe key that was here is kept at\n{}", p.display()))
        .unwrap_or_default();
    Ok(format!(
        "Linked. Your chat key is now your wallet's — {} — the same one\n\
         elopros.com uses, and rooms show your profile name and picture.{kept}",
        elo_hive::who::short(&done.npub)))
}

/// Unlink the account's chat key: one signature, shown first. The key stays on
/// this machine and keeps working in chat; rooms just stop naming it as the
/// wallet, and **Link to my wallet** links it again.
pub fn unlink_chat_key(
    signer: &Shared,
    host: &str,
    ask: &Ask,
    confirm: &Confirm,
    http: &dyn elo_hive::link::LinkHttp,
) -> Result<String, String> {
    let (wallet, was_unlocked) = chat_account(signer, "unlinking is the account's signature")?;
    let outcome = {
        let mut sign = showing_signer(signer, ask, confirm, "Unlink your chat key");
        elo_hive::link::unlink(http, host, &wallet, &mut sign)
    };
    relock(signer, was_unlocked);
    let was = outcome?;
    Ok(format!(
        "Unlinked. Rooms stop showing your profile name and picture beside\n\
         {}. The key stays on this machine; Link to my wallet links it again.",
        elo_hive::who::short(&was)))
}

/// How often the Chat window looks for news from its engine.
const CHAT_POLL: f64 = 0.25;
/// How often the feed is redrawn with nothing new, so *"3m ago"* stays true.
const CHAT_AGES: std::time::Duration = std::time::Duration::from_secs(30);

/// The window's side of the chat: what it last drew, and the engine while the
/// window is open.
#[derive(Default)]
struct ChatState {
    engine: Option<elo_hive::chat::Chat>,
    seen: u64,
    drawn: Option<std::time::Instant>,
    rooms: Vec<String>,
    feed_len: usize,
    feed: String,
    /// The text of the note in flight, so a sent note clears the box and a
    /// refused one stays in it to be fixed and sent again.
    pending: Option<String>,
    said: Option<Result<String, String>>,
    /// The account this machine holds, when the link door is wired — the
    /// wallet a chat key would be linked to.
    account: Option<String>,
}

/// Give the Chat window its behaviour, and return the way to open it.
///
/// **The engine runs only while the window is open.** Opening starts it;
/// closing the window drops it, which closes the socket, which is what takes
/// the player offline — buzz reads presence off the held connection. A player
/// who never opens Chat never connects to the relay at all.
///
/// Every redraw happens on a timer, never inside a widget's own callback, so a
/// room list being rebuilt can never free the line the player just clicked.
pub fn wire_chat(w: &windows::ChatWindow, host: &str, voice_dir: PathBuf,
                 cache_dir: PathBuf, link: Option<ChatLink>) -> Rc<dyn Fn()> {
    use elo_hive::chat::{Ask, Chat};
    let state = Rc::new(RefCell::new(ChatState::default()));
    let host = host.trim_end_matches('/').to_string();
    let link_signer = link.as_ref().map(|l| Arc::clone(&l.signer));

    let ask = {
        let state = Rc::clone(&state);
        move |a: Ask| {
            if let Ok(s) = state.try_borrow() {
                if let Some(e) = &s.engine {
                    e.ask(a);
                }
            }
        }
    };

    {
        let state = Rc::clone(&state);
        let ask = ask.clone();
        let mut rooms = w.rooms.clone();
        rooms.set_callback(move |b| {
            let i = b.value();
            let id = state.try_borrow().ok().and_then(|s| {
                usize::try_from(i - 1).ok().and_then(|i| s.rooms.get(i).cloned())
            });
            if let Some(id) = id {
                ask(Ask::Open(id));
            }
        });
    }

    let send_now: Rc<dyn Fn()> = {
        let state = Rc::clone(&state);
        let ask = ask.clone();
        let input = w.input.clone();
        Rc::new(move || {
            let text = input.value();
            if text.trim().is_empty() {
                return;
            }
            if let Ok(mut s) = state.try_borrow_mut() {
                s.pending = Some(text.clone());
            }
            ask(Ask::Say(text));
        })
    };
    {
        let send_now = Rc::clone(&send_now);
        w.input.clone().set_callback(move |_| send_now());
    }
    {
        let send_now = Rc::clone(&send_now);
        w.send.clone().set_callback(move |_| send_now());
    }
    {
        let ask = ask.clone();
        w.make_key.clone().set_callback(move |_| ask(Ask::MakeVoice));
    }
    // **Link to my wallet**: the account signs for its own chat key, and the
    // engine reloads it — the socket the old key held is dropped and the new
    // one connects, already linked. **Unlink** reloads too, so the window's
    // own name is looked up again rather than remembered.
    if let Some(l) = link {
        {
            let (ask, l) = (ask.clone(), l.clone());
            let (host, voice_dir) = (host.clone(), voice_dir.clone());
            w.link.clone().set_callback(move |_| {
                match link_chat_key(&l.signer, &host, &voice_dir, &l.ask, &l.confirm,
                                    &elo_hive::link::RealLinkHttp) {
                    Ok(done) => {
                        ask(Ask::ReloadVoice);
                        (l.tell)(Told::Done, &done);
                    }
                    Err(e) => (l.tell)(Told::Refused, &e),
                }
            });
        }
        let (ask, host) = (ask.clone(), host.clone());
        w.unlink.clone().set_callback(move |_| {
            match unlink_chat_key(&l.signer, &host, &l.ask, &l.confirm,
                                  &elo_hive::link::RealLinkHttp) {
                Ok(done) => {
                    ask(Ask::ReloadVoice);
                    (l.tell)(Told::Done, &done);
                }
                Err(e) => (l.tell)(Told::Refused, &e),
            }
        });
    }

    // ⚠ Escape must not close it: FLTK ends a window on Escape, and Escape is
    // what a player presses to clear a half-typed line. The window manager's
    // close box is `Event::Close`, and that one means it.
    {
        let state = Rc::clone(&state);
        w.window.clone().set_callback(move |win| {
            if app::event() == fltk::enums::Event::Close {
                win.hide();
                if let Ok(mut s) = state.try_borrow_mut() {
                    *s = ChatState::default();
                }
            }
        });
    }

    {
        let state = Rc::clone(&state);
        let w = w.clone();
        let host = host.clone();
        app::add_timeout3(CHAT_POLL, move |handle| {
            if let Ok(mut s) = state.try_borrow_mut() {
                let due = s.drawn.is_none_or(|t| t.elapsed() >= CHAT_AGES);
                let fresh = s.engine.as_ref().map(|e| (e.version(), e.view()));
                if let Some((version, view)) = fresh {
                    if version != s.seen || due {
                        s.seen = version;
                        s.account = link_signer
                            .as_ref()
                            .and_then(|sg| sg.lock().ok().and_then(|g| g.address()))
                            .map(|a| a.to_lowercase());
                        draw_chat(&w, &view, &host, &mut s);
                    }
                }
            }
            app::repeat_timeout3(CHAT_POLL, handle);
        });
    }

    let w = w.clone();
    Rc::new(move || {
        if let Ok(mut s) = state.try_borrow_mut() {
            if s.engine.is_none() {
                s.engine = Some(Chat::start(&host, voice_dir.clone(), cache_dir.clone()));
            }
        }
        let mut win = w.window.clone();
        win.show();
    })
}

/// Draw a view with no engine behind it — for a capture, which has to show
/// the window's states without a relay or an origin to put it in them.
pub fn paint_chat(w: &windows::ChatWindow, view: &elo_hive::chat::View, host: &str) {
    draw_chat(w, view, host, &mut ChatState::default());
}

/// [`paint_chat`] as a machine with this account would draw it — the link
/// door shows or hides by whether the view's key is linked to it.
pub fn paint_chat_as(w: &windows::ChatWindow, view: &elo_hive::chat::View, host: &str,
                     account: &str) {
    draw_chat(w, view, host, &mut ChatState {
        account: Some(account.to_lowercase()),
        ..ChatState::default()
    });
}

/// Put one view on screen. Only what changed is touched, so a redraw does not
/// throw the player's scroll position away for an unchanged feed.
fn draw_chat(w: &windows::ChatWindow, view: &elo_hive::chat::View, host: &str,
             s: &mut ChatState) {
    use crate::chatpane;
    use crate::chrome::defuse;
    use elo_hive::chat::Link;

    let mut w = w.clone();
    let ids: Vec<String> = view.rooms.iter().map(|c| c.id.clone()).collect();
    if ids != s.rooms {
        w.rooms.clear();
        for c in &view.rooms {
            w.rooms.add(&windows::room_line(&chatpane::room_label(c)));
        }
        s.rooms = ids;
    }
    let open = view.room.as_ref().and_then(|r| view.rooms.iter().position(|c| &c.id == r));
    if let Some(i) = open {
        if w.rooms.value() != i as i32 + 1 {
            w.rooms.select(i as i32 + 1);
        }
        let c = &view.rooms[i];
        w.room_name.set_label(&defuse(&chatpane::room_label(c)));
        let about = elo_hive::render::safe_line(&c.about);
        w.room_about.set_label(&defuse(&about));
        w.room_about.set_tooltip(&defuse(&about));
    }

    let lines = chatpane::lines(&view.messages, &view.people, host, elo_hive::clock::now());
    let feed = chatpane::feed_text(&lines, chatpane::quiet_line(view));
    {
        // The faces first, so the redraw below paints them with the text.
        let mut f = w.faces.borrow_mut();
        f.rows = feed
            .starts
            .iter()
            .zip(&lines)
            .map(|(pos, l)| windows::FaceRow {
                pos: *pos,
                key: l.key.clone(),
                house: l.house,
                initial: l.initial.clone(),
                tint: l.tint,
            })
            .collect();
        for (npub, p) in &view.people {
            let Some(bytes) = &p.picture else { continue };
            if f.decoded.get(npub).is_some_and(|had| std::sync::Arc::ptr_eq(had, bytes)) {
                continue;
            }
            // Sniffed again here, by the same rule as a title's icon, and a
            // picture that will not decode leaves the tile — never a hole.
            match crate::art::Icon::decode(bytes.to_vec()).and_then(|i| i.picture(windows::FACE)) {
                Some(pic) => {
                    f.pictures.insert(npub.clone(), pic);
                }
                None => {
                    f.pictures.remove(npub);
                }
            }
            f.decoded.insert(npub.clone(), std::sync::Arc::clone(bytes));
        }
    }
    let text = feed.text;
    if text != s.feed {
        w.feed_text.set_text(&text);
        w.feed_style.set_text(&feed.style);
        // Follow the newest message when one arrives; leave the view alone
        // when only the ages changed. Only on a window that is up — a hidden
        // one has nothing to scroll, and a capture paints before it shows.
        if lines.len() != s.feed_len && w.window.shown() {
            let end = w.feed_text.length();
            w.feed.set_insert_position(end);
            w.feed.show_insert_position();
        }
        s.feed = text;
        s.feed_len = lines.len();
    }

    w.status.set_label(&defuse(&chatpane::status_text(view)));
    w.status.set_label_color(match view.link {
        Link::Relay(_) => theme::INK,
        Link::Mirror => theme::MUTED,
        Link::Starting | Link::Offline => theme::GOLD,
    });
    let why = elo_hive::render::safe_line(&view.why);
    w.status.set_tooltip(&defuse(&why));

    // One slot, four states: make a key (no account, no key), link the
    // wallet's key (an account, and a key not linked to it), unlink (linked),
    // or nothing (a key and no account to link it to).
    let linked = chatpane::linked_to(view, s.account.as_deref());
    let mut slot = [w.make_key.clone(), w.link.clone(), w.unlink.clone()];
    let shown = match (&s.account, linked) {
        (Some(_), false) => Some(1),
        (Some(_), true) => Some(2),
        (None, _) if view.npub.is_none() => Some(0),
        _ => None,
    };
    for (i, b) in slot.iter_mut().enumerate() {
        if Some(i) == shown {
            b.show();
        } else {
            b.hide();
        }
    }
    if view.npub.is_some() {
        w.input.activate();
        w.send.activate();
    } else {
        w.input.deactivate();
        w.send.deactivate();
    }
    w.you.set_label(&defuse(&chatpane::you_line(view)));
    if let Some(n) = &view.npub {
        w.you.set_tooltip(n);
    }

    if view.said != s.said {
        if let Some(Ok(_)) = &view.said {
            if s.pending.as_deref() == Some(w.input.value().as_str()) {
                w.input.set_value("");
            }
            s.pending = None;
        }
        s.said = view.said.clone();
    }
    w.said.set_label(&defuse(&chatpane::said_line(view)));

    s.drawn = Some(std::time::Instant::now());
    w.window.redraw();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A running game holds the relock off**, and the clock starts again
    /// when it exits. A game proves once at join and then plays for hours
    /// without signing, so a relock that counted only signatures put the
    /// passphrase gate up over the game half an hour in.
    #[test]
    fn the_idle_relock_waits_while_a_game_is_running() {
        let min = std::time::Duration::from_secs(60);
        let after = 30 * min;
        // Nobody played: the key's own idle time decides.
        assert!(relock_due(Some(31 * min), None, after));
        assert!(!relock_due(Some(29 * min), None, after));
        // Locked already: nothing to relock.
        assert!(!relock_due(None, None, after));
        // Idle an hour, but a game was running moments ago: not yet.
        assert!(!relock_due(Some(60 * min), Some(std::time::Duration::ZERO), after));
        assert!(!relock_due(Some(60 * min), Some(10 * min), after));
        // The game has been gone for the full interval: now it locks.
        assert!(relock_due(Some(60 * min), Some(30 * min), after));
    }

    #[test]
    fn a_key_is_parsed_with_or_without_the_prefix() {
        let bare = "01".repeat(32);
        let prefixed = format!("0x{bare}");
        assert_eq!(*parse_secret(&bare).unwrap(), [1u8; 32]);
        assert_eq!(*parse_secret(&prefixed).unwrap(), [1u8; 32]);
        // Whitespace from a paste is the common case, not an exotic one.
        assert_eq!(*parse_secret(&format!("  {prefixed}\n")).unwrap(), [1u8; 32]);
    }

    /// The refusal names the SHAPE and never the value. This is the test that
    /// keeps a private key out of a bug report.
    #[test]
    fn a_bad_key_never_appears_in_its_own_error() {
        let secret = "d3adbeef".repeat(9); // 72 chars — wrong length
        let e = parse_secret(&secret).expect_err("must refuse");
        assert!(!e.contains("d3adbeef"), "the error echoed the key: {e}");
        assert!(e.contains("72"), "say the shape: {e}");

        let not_hex = "z".repeat(64);
        let e = parse_secret(&not_hex).expect_err("must refuse");
        assert!(!e.contains("zzz"), "the error echoed the input: {e}");
    }

    /// **A multibyte paste is refused, not a crash.** 64 bytes passes the
    /// length check, and the byte slice then lands inside the two-byte "é":
    /// that panicked, and `panic = "abort"` took the whole launcher with it.
    #[test]
    fn a_multibyte_paste_is_refused_not_a_crash() {
        let pasted = format!("a\u{e9}{}", "a".repeat(61));
        assert_eq!(pasted.len(), 64);
        let got = std::panic::catch_unwind(|| parse_secret(&pasted).map(|_| ()));
        let e = got.expect("parse_secret panicked on a multibyte paste").expect_err("must refuse");
        assert!(e.contains("nothing was written"), "{e}");
    }

    /// **The pasted key and every typed passphrase come back wiped on drop.**
    /// The types carry it, so a caller cannot get either back as a plain
    /// `[u8; 32]` or `String` that outlives the lock.
    #[test]
    fn typed_secrets_come_back_zeroizing() {
        let key: elo_vault::Zeroizing<[u8; 32]> = parse_secret(&"01".repeat(32)).unwrap();
        assert_eq!(*key, [1u8; 32]);
        let ask = scripted_ask(vec![Some("same".into()), Some("same".into())]);
        let pass: Option<Secret> = ask_twice(&ask, &recording_tell(Rc::new(RefCell::new(Vec::new()))),
                                             "h", "p");
        assert_eq!(pass.as_deref().map(String::as_str), Some("same"));
    }

    /// One function decides whether the update notice fires, so its edges
    /// live here: only a MEASURED newer version speaks, and every other
    /// state — current, ahead, unpublished, unreadable — stays silent
    /// rather than nagging or, worse, reassuring.
    #[test]
    fn only_a_measured_newer_client_raises_the_notice() {
        let card = serde_json::json!({"client": {"version": "0.2.0"}});
        assert_eq!(newer_client(Some(&card), "0.1.0"), Some("0.2.0".into()));
        assert_eq!(newer_client(Some(&card), "0.2.0"), None, "current is quiet");
        assert_eq!(newer_client(Some(&card), "0.3.0"), None,
                   "a dev build ahead of the shelf is not asked to downgrade");
        assert_eq!(newer_client(Some(&serde_json::json!({})), "0.1.0"), None,
                   "an origin that publishes no client says nothing");
        assert_eq!(newer_client(None, "0.1.0"), None,
                   "could not look is not an update, and not up-to-date either");
    }

    // ── the pairing's refusal order, with no network and no keystore ────────

    /// Panics if consulted: for refusals that must happen before the origin.
    struct NoHttp;
    impl SigninHttp for NoHttp {
        fn challenge(&self, _: &str, _: &str) -> elo_net::Fetched<serde_json::Value> {
            panic!("the origin was consulted before the local refusal")
        }
        fn deliver(&self, _: &str, _: &str, _: &serde_json::Value)
            -> elo_net::Fetched<serde_json::Value> {
            panic!("a proof was delivered with nothing signed")
        }
    }

    /// A challenge that answers 404, for the expired-code path.
    struct Gone;
    impl SigninHttp for Gone {
        fn challenge(&self, _: &str, _: &str) -> elo_net::Fetched<serde_json::Value> {
            elo_net::Fetched::status(404)
        }
        fn deliver(&self, _: &str, _: &str, _: &serde_json::Value)
            -> elo_net::Fetched<serde_json::Value> {
            panic!("delivered after a refused challenge")
        }
    }

    fn no_signer() -> Shared {
        Arc::new(Mutex::new(elo_vault::LocalSigner::at("/nonexistent/keystore.json")))
    }

    #[test]
    fn a_bad_code_is_refused_before_a_dialog_or_the_origin() {
        let asked = Rc::new(RefCell::new(0u32));
        let a2 = Rc::clone(&asked);
        let ask: Ask = Rc::new(move |_, _| {
            *a2.borrow_mut() += 1;
            None
        });
        let e = pair_browser(&no_signer(), "https://x.example", "nope", &ask, &NoHttp)
            .unwrap_err();
        assert!(e.contains("not a sign-in code"), "{e}");
        assert_eq!(*asked.borrow(), 0, "no dialog may open before the shape check");
    }

    #[test]
    fn no_account_refuses_before_a_dialog_or_the_origin() {
        let ask: Ask = Rc::new(|_, _| panic!("asked for a passphrase with no account"));
        let e = pair_browser(&no_signer(), "https://x.example", "ABCD-2345", &ask, &NoHttp)
            .unwrap_err();
        assert!(e.contains("No account"), "{e}");
    }

    /// An expired code costs one request and NO passphrase — the dialog only
    /// ever opens over a code the origin says is waiting.
    #[test]
    fn an_expired_code_is_refused_before_the_passphrase() {
        // A keystore FILE is enough: `at` reads only the public address, and
        // this path never unlocks. No slow scrypt in this test on purpose.
        let path = std::env::temp_dir().join(format!(
            "elo-signin-test-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"address": "24445efddb08d4938e3e3627042b2cf4063d9092"}"#,
        )
        .expect("write test keystore");
        let signer: Shared =
            Arc::new(Mutex::new(elo_vault::LocalSigner::at(&path)));
        let ask: Ask = Rc::new(|_, _| panic!("asked for a passphrase over a dead code"));
        let e = pair_browser(&signer, "https://x.example", "abcd-2345", &ask, &Gone)
            .unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(e.contains("not waiting to sign in"), "{e}");
        // The lowercased, dashed input was normalised into the refusal's code.
        assert!(e.contains("ABCD2345"), "{e}");
    }

    // ── the profile change's refusal order, with no network and no scrypt ───

    /// Records what was asked of the origin; `prepare` answers from a script.
    struct FaceDesk {
        prepared: Option<serde_json::Value>,
        asked: RefCell<Vec<serde_json::Value>>,
    }
    impl crate::profile::FaceHttp for FaceDesk {
        fn face(&self, _: &str, _: &str) -> elo_net::Fetched<serde_json::Value> {
            unreachable!("a change never reads the profile")
        }
        fn picture(&self, _: &str, _: &str) -> elo_net::Fetched<Vec<u8>> {
            unreachable!()
        }
        fn prepare(&self, _: &str, body: &serde_json::Value) -> elo_net::Fetched<serde_json::Value> {
            self.asked.borrow_mut().push(body.clone());
            match &self.prepared {
                Some(v) => elo_net::Fetched::good(v.clone()),
                None => elo_net::Fetched::unreachable("dns error"),
            }
        }
        fn save(&self, _: &str, _: &serde_json::Value) -> elo_net::Fetched<serde_json::Value> {
            panic!("a change was saved with nothing signed")
        }
    }

    fn desk(prepared: Option<serde_json::Value>) -> FaceDesk {
        FaceDesk { prepared, asked: RefCell::new(Vec::new()) }
    }

    /// A keystore FILE with only an address — enough to say who the account
    /// is, and it can never unlock, so no test here runs scrypt or signs.
    fn address_only() -> (Shared, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "elo-profile-test-{}-{:?}.json", std::process::id(), std::thread::current().id()));
        std::fs::write(&path, r#"{"address": "24445efddb08d4938e3e3627042b2cf4063d9092"}"#)
            .expect("write test keystore");
        (Arc::new(Mutex::new(elo_vault::LocalSigner::at(&path))), path)
    }

    fn named(n: &str) -> crate::profile::Change {
        crate::profile::Change { name: Some(n.into()), picture: None }
    }

    #[test]
    fn a_profile_needs_an_account_before_anything_is_asked() {
        let ask: Ask = Rc::new(|_, _| panic!("asked for a passphrase with no account"));
        let e = set_profile(&no_signer(), "https://x", &named("Mara"), &ask, &desk(None))
            .unwrap_err();
        assert!(e.contains("No account"), "{e}");
    }

    #[test]
    fn a_change_the_origin_would_refuse_is_refused_before_the_origin_or_a_prompt() {
        let (signer, path) = address_only();
        let ask: Ask = Rc::new(|_, _| panic!("prompted over a change that cannot land"));
        let d = desk(None);
        for (change, says) in [
            (crate::profile::Change::default(), "Nothing to change"),
            (named("0xdeadbeef"), "0x"),
            (named("much too long a name"), "at most"),
            (crate::profile::Change { name: None, picture: Some(b"not a png".to_vec()) }, "PNG"),
        ] {
            let e = set_profile(&signer, "https://x", &change, &ask, &d).unwrap_err();
            assert!(e.contains(says), "{e}");
        }
        assert!(d.asked.borrow().is_empty(), "the origin was asked about a refused change");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_origin_that_will_not_prepare_costs_no_passphrase() {
        let (signer, path) = address_only();
        let ask: Ask = Rc::new(|_, _| panic!("prompted with nothing to sign"));
        let e = set_profile(&signer, "https://x", &named("Mara"), &ask, &desk(None)).unwrap_err();
        assert!(e.contains("could not reach"), "{e}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_prompt_shows_the_exact_text_and_a_cancel_signs_nothing() {
        let (signer, path) = address_only();
        let text = "elo play\naction: profile\nwallet: 0x2444…\ndetail: {\"at\":1,\"name\":\"Mara\"}";
        let d = desk(Some(serde_json::json!({"sign_this": text, "at": 1})));
        let seen = Rc::new(RefCell::new(String::new()));
        let s2 = Rc::clone(&seen);
        let ask: Ask = Rc::new(move |_, prompt| {
            *s2.borrow_mut() = prompt.to_string();
            None
        });
        let mut pic = crate::profile::tests_png(128);
        pic.extend(b"rest of the file");
        let change = crate::profile::Change { name: Some("  Mara ".into()), picture: Some(pic.clone()) };
        let e = set_profile(&signer, "https://x", &change, &ask, &d).unwrap_err();
        assert!(e.contains("Cancelled"), "{e}");
        assert!(seen.borrow().ends_with(text), "the prompt carries the signed text verbatim");
        // What was prepared: the trimmed name and the picture's own sha256.
        use sha2::{Digest, Sha256};
        let asked = d.asked.borrow()[0].clone();
        assert_eq!(asked["name"], "Mara");
        assert_eq!(asked["picture_sha256"], elo_depot::digest::hex_lower(&Sha256::digest(&pic)));
        assert_eq!(asked["wallet"].as_str().unwrap().to_lowercase(),
                   "0x24445efddb08d4938e3e3627042b2cf4063d9092");
        let _ = std::fs::remove_file(path);
    }

    // ── a friends change: what is refused before what ────────────────────────

    const ACCOUNT: &str = "0x24445efddb08d4938e3e3627042b2cf4063d9092";
    const ROOK: &str = "0x6666666666666666666666666666666666666666";

    /// Answers `who` and `prepare` from a script and records them; `act`
    /// answers the fixture list only when the test allows a change to land.
    struct FriendDesk {
        who: Option<&'static str>,
        prepared: Option<serde_json::Value>,
        may_act: bool,
        sent: RefCell<Vec<(&'static str, serde_json::Value)>>,
    }
    impl crate::friendsop::FriendsHttp for FriendDesk {
        fn who(&self, host: &str, handle: &str) -> elo_net::Fetched<elo_hive::who::Identity> {
            self.sent.borrow_mut().push(("who", serde_json::json!(handle)));
            match self.who {
                Some(w) => elo_net::Fetched::good(elo_hive::who::Identity::bare(w, host)),
                None => elo_net::Fetched::unreachable("dns error"),
            }
        }
        fn prepare(&self, _: &str, body: &serde_json::Value) -> elo_net::Fetched<serde_json::Value> {
            self.sent.borrow_mut().push(("prepare", body.clone()));
            match &self.prepared {
                Some(v) => elo_net::Fetched::good(v.clone()),
                None => elo_net::Fetched::unreachable("dns error"),
            }
        }
        fn act(&self, _: &str, body: &serde_json::Value) -> elo_net::Fetched<serde_json::Value> {
            assert!(self.may_act, "a change was sent with nothing signed");
            self.sent.borrow_mut().push(("act", body.clone()));
            let view: serde_json::Value = serde_json::from_str(include_str!(
                "../../../../sdk/testdata/friends_view.json")).unwrap();
            elo_net::Fetched::good(serde_json::json!({"ok": true, "op": "ask", "view": view}))
        }
    }

    fn friend_desk(who: Option<&'static str>, prepared: Option<serde_json::Value>) -> FriendDesk {
        FriendDesk { who, prepared, may_act: false, sent: RefCell::new(Vec::new()) }
    }

    fn never_ask() -> Ask {
        Rc::new(|_, _| panic!("asked for a passphrase before every local refusal"))
    }

    fn the_text(op: &str, other: &str, at: u64) -> String {
        elo_net::friends::sign_text(ACCOUNT, op, elo_net::friends::op_key(op).unwrap(), other, at)
            .unwrap()
    }

    #[test]
    fn with_no_signer_a_change_is_handed_to_the_website() {
        let d = friend_desk(None, None);
        let mut via = FriendSigner::pick(&no_signer(), None);
        assert!(matches!(via, FriendSigner::Neither));
        match change_friend(&mut via, "https://elo.test/", "ask", ROOK, &*never_ask(), &d) {
            Ok(Changed::HandedOff(url)) => assert_eq!(url, "https://elo.test/profile.html"),
            other => panic!("expected a handoff: {other:?}"),
        }
        assert!(d.sent.borrow().is_empty(), "a handoff asks the origin nothing");
        // A change that is not one is refused even with nowhere to send it.
        assert!(change_friend(&mut via, "https://x", "befriend", ROOK, &*never_ask(), &d).is_err());
        assert!(change_friend(&mut via, "https://x", "ask", "  ", &*never_ask(), &d).is_err());
    }

    #[test]
    fn a_friends_change_is_refused_locally_before_the_origin_or_a_prompt() {
        let (signer, path) = address_only();
        let mut via = FriendSigner::pick(&signer, None);
        assert_eq!(via.address().as_deref(), Some(ACCOUNT), "a locked account still says who");
        let d = friend_desk(None, None);
        let shouted = ACCOUNT.to_uppercase().replacen("0X", "0x", 1);
        for (op, who, says) in [
            ("befriend", ROOK, "not a friends change"),
            ("ask", "", "Type a wallet"),
            ("ask", ACCOUNT, "your own wallet"),
            ("remove", shouted.as_str(), "your own wallet"),
        ] {
            let e = change_friend(&mut via, "https://x", op, who, &*never_ask(), &d).unwrap_err();
            assert!(e.contains(says), "{op} {who}: {e}");
        }
        assert!(d.sent.borrow().is_empty(), "the origin was asked about a refused change");

        // A name goes through the register; one with no wallet stops there.
        let d = friend_desk(Some("rook"), None);
        let e = change_friend(&mut via, "https://x", "ask", "rook", &*never_ask(), &d).unwrap_err();
        assert!(e.contains("no wallet goes by rook"), "{e}");
        assert_eq!(d.sent.borrow().len(), 1, "looked up, and nothing prepared");
        let d = friend_desk(None, None);
        let e = change_friend(&mut via, "https://x", "ask", "rook", &*never_ask(), &d).unwrap_err();
        assert!(e.contains("could not reach the register"), "{e}");

        // An origin that will not prepare, or prepares another change, costs
        // no passphrase.
        let e = change_friend(&mut via, "https://x", "ask", ROOK, &*never_ask(),
                              &friend_desk(None, None)).unwrap_err();
        assert!(e.contains("could not reach"), "{e}");
        let wrong = serde_json::json!({"sign_this": the_text("block", ROOK, 9), "at": 9});
        let d = friend_desk(None, Some(wrong));
        let e = change_friend(&mut via, "https://x", "ask", ROOK, &*never_ask(), &d).unwrap_err();
        assert_eq!(e, crate::friendsop::MISMATCH);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_friends_prompt_carries_the_exact_text_and_a_cancel_sends_nothing() {
        let (signer, path) = address_only();
        let mut via = FriendSigner::pick(&signer, None);
        // A name that resolves: what is prepared and shown is the wallet's.
        let text = the_text("ask", ROOK, 1_761_003_600);
        let d = friend_desk(Some(ROOK), Some(serde_json::json!({"sign_this": text, "at": 1_761_003_600})));
        let seen = Rc::new(RefCell::new((String::new(), String::new())));
        let s2 = Rc::clone(&seen);
        let ask: Ask = Rc::new(move |heading, prompt| {
            *s2.borrow_mut() = (heading.to_string(), prompt.to_string());
            None
        });
        let e = change_friend(&mut via, "https://x", "ask", " Rook ", &*ask, &d).unwrap_err();
        assert!(e.contains("Cancelled"), "{e}");
        assert_eq!(seen.borrow().0, "Ask to be friends");
        assert!(seen.borrow().1.ends_with(&text), "the prompt carries the text verbatim");
        let sent = d.sent.borrow();
        assert_eq!(sent[0], ("who", serde_json::json!("Rook")));
        assert_eq!(sent[1].1, serde_json::json!({"wallet": ACCOUNT, "op": "ask",
                                                 "args": {"to": ROOK}}));
        assert_eq!(sent.len(), 2, "nothing was sent after the cancel");

        // A wrong passphrase stops before the act, and leaves it locked.
        let d = friend_desk(None, Some(serde_json::json!({"sign_this": text, "at": 1_761_003_600})));
        let ask: Ask = Rc::new(|_, _| Some(Secret::new("not it".into())));
        assert!(change_friend(&mut via, "https://x", "ask", ROOK, &*ask, &d).is_err());
        assert!(!signer.lock().unwrap().is_unlocked());
        let _ = std::fs::remove_file(path);
    }

    /// A paired browser signs with no prompt here, and the list that comes
    /// back is the one the window repaints from.
    #[test]
    fn a_paired_browser_signs_with_no_passphrase_and_the_list_comes_back() {
        struct Browser(Arc<Mutex<Vec<(String, String)>>>);
        impl Signer for Browser {
            fn kind(&self) -> &'static str { "browser-paired" }
            fn status(&self) -> String { String::new() }
            fn address(&self) -> Option<String> { Some(ACCOUNT.to_uppercase().replacen("0X", "0x", 1)) }
            fn sign(&mut self, text: &str, why: &str)
                -> Result<serde_json::Value, elo_broker::protocol::Refusal> {
                self.0.lock().unwrap().push((text.into(), why.into()));
                Ok(serde_json::json!({"signature": format!("0x{}", "cd".repeat(65))}))
            }
        }
        let asked: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let mut via = FriendSigner::Paired(Box::new(Browser(Arc::clone(&asked))));
        let text = the_text("accept", ROOK, 1_761_003_601);
        let mut d = friend_desk(None, Some(serde_json::json!({"sign_this": text, "at": 1_761_003_601})));
        d.may_act = true;
        let got = change_friend(&mut via, "https://x", "accept", ROOK, &*never_ask(), &d).unwrap();
        let Changed::Done(view) = got else { panic!("expected the list back") };
        assert_eq!(view.buddies[0].label(), "Pip");
        let sent = d.sent.borrow();
        assert_eq!(sent[1].0, "act");
        assert_eq!(sent[1].1["wallet"], ACCOUNT, "made as the signer's wallet, lowercased");
        assert_eq!(sent[1].1["signature"], format!("0x{}", "cd".repeat(65)));
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1, "one signature, in the browser");
        assert_eq!(asked[0].0, text, "the text checked is the text signed");
        assert!(asked[0].1.starts_with("friends: accept"), "the browser is told why: {}", asked[0].1);
    }

    // ── linking the chat key: what is refused before what ────────────────────

    /// Answers the quote from a script; refuses to be asked to link unless told.
    struct LinkDesk {
        voice_text: Option<String>,
        linked: RefCell<u32>,
    }
    impl elo_hive::link::LinkHttp for LinkDesk {
        fn quote(&self, _: &str, wallet: &str, npub: Option<&str>)
            -> elo_net::Fetched<serde_json::Value> {
            match (&self.voice_text, npub) {
                (None, _) => elo_net::Fetched::unreachable("dns error"),
                (Some(t), None) => elo_net::Fetched::good(serde_json::json!({
                    "wallet": wallet, "linked": null, "voice_key": {"sign_this": t}})),
                (Some(_), Some(n)) => elo_net::Fetched::good(serde_json::json!({
                    "npub": n, "at": 1, "sign_this": "elo play\naction: chatkey",
                    "event_content": elo_hive::voice::link_content(wallet)})),
            }
        }
        fn link(&self, _: &str, _: &serde_json::Value) -> elo_net::Fetched<serde_json::Value> {
            *self.linked.borrow_mut() += 1;
            panic!("a link was posted in a test that should have refused first")
        }
    }

    fn link_desk(text: Option<&str>) -> LinkDesk {
        LinkDesk { voice_text: text.map(str::to_string), linked: RefCell::new(0) }
    }

    fn never_confirm() -> Confirm {
        Rc::new(|_, _, _| panic!("confirmed before the wallet signed anything"))
    }

    #[test]
    fn linking_needs_an_account_before_anything_is_asked() {
        let ask: Ask = Rc::new(|_, _| panic!("asked for a passphrase with no account"));
        let dir = std::env::temp_dir();
        let e = link_chat_key(&no_signer(), "https://x", &dir, &ask, &never_confirm(),
                              &link_desk(None)).unwrap_err();
        assert!(e.contains("No account"), "{e}");
    }

    #[test]
    fn a_key_text_that_differs_from_the_websites_is_refused_before_the_passphrase() {
        let (signer, path) = address_only();
        let ask: Ask = Rc::new(|_, _| panic!("prompted over a text that would make a different key"));
        let dir = std::env::temp_dir();
        let e = link_chat_key(&signer, "https://x", &dir, &ask, &never_confirm(),
                              &link_desk(Some("scry hive voice v2 — something else"))).unwrap_err();
        assert!(e.contains("different key text"), "{e}");
        let e = link_chat_key(&signer, "https://x", &dir, &ask, &never_confirm(),
                              &link_desk(None)).unwrap_err();
        assert!(e.contains("could not reach"), "{e}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_prompt_shows_the_key_text_and_a_cancel_signs_and_saves_nothing() {
        let (signer, path) = address_only();
        let wallet = "0x24445efddb08d4938e3e3627042b2cf4063d9092";
        let text = elo_hive::voice::derivation_message(wallet);
        let seen = Rc::new(RefCell::new(String::new()));
        let s2 = Rc::clone(&seen);
        let ask: Ask = Rc::new(move |_, prompt| {
            *s2.borrow_mut() = prompt.to_string();
            None
        });
        let dir = std::env::temp_dir().join(format!("elo-link-test-{}", std::process::id()));
        let d = link_desk(Some(&text));
        let e = link_chat_key(&signer, "https://x", &dir, &ask, &never_confirm(), &d).unwrap_err();
        assert!(e.contains("Cancelled"), "{e}");
        assert!(seen.borrow().contains(&text), "the prompt carries the key text verbatim");
        assert_eq!(*d.linked.borrow(), 0);
        assert!(!dir.join(elo_hive::voice::KEY_NAME).exists(), "no key was saved");
        let _ = std::fs::remove_file(path);
    }

    /// A keystore that cannot unlock refuses after the passphrase and before
    /// the confirmation — and leaves the account locked.
    #[test]
    fn a_wrong_passphrase_stops_before_the_link_and_relocks() {
        let (signer, path) = address_only();
        let text = elo_hive::voice::derivation_message("0x24445efddb08d4938e3e3627042b2cf4063d9092");
        let ask: Ask = Rc::new(|_, _| Some(Secret::new("not it".into())));
        let dir = std::env::temp_dir();
        let d = link_desk(Some(&text));
        assert!(link_chat_key(&signer, "https://x", &dir, &ask, &never_confirm(), &d).is_err());
        assert!(!signer.lock().unwrap().is_unlocked());
        let _ = std::fs::remove_file(path);
    }

    /// An origin with a key linked to the wallet, which must never be posted
    /// to by a test that refuses first.
    struct LinkedDesk;
    impl elo_hive::link::LinkHttp for LinkedDesk {
        fn quote(&self, _: &str, wallet: &str, npub: Option<&str>)
            -> elo_net::Fetched<serde_json::Value> {
            let at = 1_790_000_000;
            elo_net::Fetched::good(serde_json::json!({
                "wallet": wallet, "linked": "npub1k7dy53fkr25pupyjxqn64aytu8v792v96l9p2f7gl3edk424ljrsnqjp92",
                "npub": npub, "at": at,
                "sign_this": elo_hive::link::sign_text(wallet, at, npub.unwrap_or("-"))}))
        }
        fn link(&self, _: &str, _: &serde_json::Value) -> elo_net::Fetched<serde_json::Value> {
            panic!("an unlink was posted in a test that should have refused first")
        }
    }

    #[test]
    fn unlinking_needs_an_account_and_shows_its_one_text_before_the_passphrase() {
        let ask: Ask = Rc::new(|_, _| panic!("asked for a passphrase with no account"));
        let e = unlink_chat_key(&no_signer(), "https://x", &ask, &never_confirm(), &LinkedDesk)
            .unwrap_err();
        assert!(e.contains("No account"), "{e}");

        let (signer, path) = address_only();
        let wallet = "0x24445efddb08d4938e3e3627042b2cf4063d9092";
        let seen = Rc::new(RefCell::new(String::new()));
        let s2 = Rc::clone(&seen);
        let ask: Ask = Rc::new(move |_, prompt| {
            *s2.borrow_mut() = prompt.to_string();
            None
        });
        let e = unlink_chat_key(&signer, "https://x", &ask, &never_confirm(), &LinkedDesk)
            .unwrap_err();
        assert!(e.contains("Cancelled"), "{e}");
        assert!(seen.borrow().contains(&elo_hive::link::sign_text(wallet, 1_790_000_000, "-")),
                "the prompt carries the unlink text verbatim: {}", seen.borrow());
        assert!(!signer.lock().unwrap().is_unlocked());
        let _ = std::fs::remove_file(path);
    }

    /// The vault's signature is `eth_account`'s, byte for byte — the vector the
    /// derivation test in `elo-hive` uses, made by the meter's own reference.
    /// Together they prove a wallet in this client and a wallet in a browser
    /// give the same chat key.
    #[test]
    fn the_vault_signs_the_key_text_exactly_as_a_browser_wallet_does() {
        let acct = elo_vault::Account::from_secret(&[0x7a; 32]).unwrap();
        let wallet = acct.address().to_lowercase();
        assert_eq!(wallet, "0x3e20ab25eb721dd4b691516238df14f0f5d3f7a3");
        let sig = acct.sign_message(&elo_hive::voice::derivation_message(&wallet)).unwrap();
        assert_eq!(sig, "0x85194638f30fce1ede1041f70e85a9cc94841f813babc1e92468dd5ee25cece8\
                         48675754662b9a2c319cfc6f2af920106a74547c8920dd5928cc5078578a25d51b");
        let v = elo_hive::voice::Voice::from_wallet_signature(&sig).unwrap();
        assert_eq!(v.npub(), "npub1k7dy53fkr25pupyjxqn64aytu8v792v96l9p2f7gl3edk424ljrsnqjp92");
    }

    #[test]
    fn a_cancelled_prompt_is_not_an_empty_answer() {
        let ask = scripted_ask(vec![None]);
        assert_eq!(ask("a heading", "anything"), None, "cancel must stay cancel");
    }

    #[test]
    fn mismatched_passphrases_are_refused_and_say_so() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let ask = scripted_ask(vec![Some("one".into()), Some("two".into())]);
        let got = ask_twice(&ask, &recording_tell(Rc::clone(&log)), "h", "p");
        assert!(got.is_none(), "a mismatch must not yield a passphrase");
        assert_eq!(log.borrow()[0].0, Told::Refused);
        assert!(log.borrow()[0].1.contains("nothing was written"));
    }
}
