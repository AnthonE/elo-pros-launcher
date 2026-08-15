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
use scry_broker::signer::Signer;
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
pub type Shared = Arc<Mutex<scry_vault::LocalSigner>>;

/// How the wiring asks for a secret: `(heading, prompt)`, and `None` means the
/// player cancelled.
///
/// ⚠ **The heading is a separate argument and not decoration.** One of the
/// four things asked for through here is a **private key**, not a passphrase,
/// and a window headed *"Passphrase"* over a field wanting 64 hex characters
/// is a client asking for the wrong secret. Every caller names what it wants.
pub type Ask = Rc<dyn Fn(&str, &str) -> Option<String>>;

/// How it reports back — a message, and an alert for a refusal.
pub type Tell = Rc<dyn Fn(Told, &str)>;

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

pub fn real_tell() -> Tell {
    Rc::new(|kind, text| {
        let (note, title) = match kind {
            Told::Done => (Note::Done, "Done"),
            Told::Refused => (Note::Refused, "That did not work"),
        };
        show_notice(note, title, text, false);
    })
}

/// Put up the client's own passphrase window and park on the answer.
///
/// **Runs a nested event loop**, exactly as `dialog::password_default` did and
/// as `ask_consent` does. Every exit that is not Continue is a cancel: the
/// window manager's close, Escape, the app shutting down. That direction costs
/// a retype when it is wrong, where the other direction would write a keystore
/// encrypted under whatever happened to be in the field.
pub fn ask_in_window(title: &str, prompt: &str) -> Option<String> {
    let mut win = windows::passphrase(title, prompt);
    let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let (a, mut w, f) = (Rc::clone(&answer), win.window.clone(), win.field.clone());
    let mut submit = move || {
        *a.borrow_mut() = Some(f.value());
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
    let got = answer.borrow().clone();
    got
}

/// Put up the client's own notice window and park on it.
///
/// Returns whether the player asked for a restart, which is only ever `true`
/// when the caller offered one.
pub fn show_notice(kind: Note, title: &str, body: &str, offer_restart: bool) -> bool {
    let mut win = windows::notice(kind, title, body, offer_restart);
    let wants = Rc::new(std::cell::Cell::new(false));

    let mut ok = win.ok.clone();
    let mut w = win.window.clone();
    ok.set_callback(move |_| w.hide());

    if let Some(b) = win.restart.clone() {
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

/// Make and import — the whole of "how do I get an account".
///
/// **Nothing here touches the network.** The key is generated by the OS pool,
/// encrypted, and written, all on this machine (`CLAUDE.md` invariant 7).
pub fn wire_account(
    account_w: &AccountWindow,
    signing_w: &SigningWindow,
    signer: &Shared,
    keystore: &Path,
    ask: Ask,
    tell: Tell,
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
            let Some(pass) = ask_twice(&ask, &tell, "A new account",
                                       "Choose a passphrase for it:") else {
                return;
            };
            match scry_vault::LocalSigner::create(&ks, &pass) {
                Ok(s) => adopt(&signer, &aw, &sw, &ks, s, &tell),
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
            // Masked like a passphrase: this input IS a private key, and it is
            // the one field in this program that must never be painted.
            let Some(raw) = ask("Import a key",
                                "The private key to adopt — 64 hex characters:") else {
                return;
            };
            let secret = match parse_secret(&raw) {
                Ok(s) => s,
                Err(e) => return tell(Told::Refused, &e),
            };
            let Some(pass) = ask_twice(&ask, &tell, "Import a key",
                                       "Choose a passphrase to encrypt it with:") else {
                return;
            };
            match scry_vault::LocalSigner::import(&ks, &pass, &secret) {
                Ok(s) => adopt(&signer, &aw, &sw, &ks, s, &tell),
                Err(e) => tell(Told::Refused, &e),
            }
        });
    }
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
///
/// ⚠ **The verb is live state, not a wiring-time constant.** Until 2026-08-15
/// it was read once, here, and moved into the callback — so a finished update
/// relabelled its button **Play** while the closure went on holding *update*.
/// Operator: *"after i update and hit play it just updates again."* Every
/// press of that Play button re-downloaded the build it had just installed,
/// and nothing ever launched. The verb lives in a cell now: a landed update
/// flips it, and a press reads it as of the press. A FAILED act deliberately
/// does not flip it — **Failed** retries the verb that failed.
///
/// `after_update` is called once an update lands, after the player dismisses
/// the notice. The binary re-reads the library with it — the same shape as
/// `wire_store`'s `after_install`, for the same reason: the row under the
/// button still says *"an update is published"* over the old build id, and a
/// row that keeps saying that about a build it no longer means is this bug's
/// sentence-vs-control disagreement all over again.
pub fn wire_games(
    games_w: &crate::windows::GamesWindow,
    front: Rc<dyn Storefront>,
    tell: Tell,
    after_update: Rc<dyn Fn()>,
) {
    for row in &games_w.rows {
        let (slug, build) = (row.slug.clone(), row.build.clone());
        let updating = Rc::new(std::cell::Cell::new(row.act.label() == "Update"));
        let (front_c, tell_c) = (Rc::clone(&front), Rc::clone(&tell));
        let after = Rc::clone(&after_update);
        let mut act = row.act.clone();
        act.set_callback(move |b| {
            let is_update = updating.get();
            b.deactivate();
            b.set_label(if is_update { "Updating…" } else { "Starting…" });
            app::redraw();
            app::flush();
            let done = if is_update {
                front_c.install(&slug)
            } else {
                front_c.play(&slug)
            };
            match done {
                Ok(said) => {
                    b.set_label(if is_update { "Play" } else { "Running" });
                    b.activate();
                    if is_update {
                        // Flipped BEFORE the notice goes up: the notice runs a
                        // nested event loop, and this button — already saying
                        // Play — is still pressable under it.
                        updating.set(false);
                        tell_c(Told::Done, &said);
                        // And the rebuild is asked for AFTER the notice comes
                        // down, never before: `after_update` defers a rebuild
                        // that drops the window this button lives in, and the
                        // nested loop above would dispatch that drop with the
                        // button's own callback still on the stack.
                        after();
                    }
                }
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
    after_install: Rc<dyn Fn()>,
) {
    use crate::windows::Act;
    for row in &store_w.rows {
        let slug = row.slug.clone();
        let act = row.act;
        let (front_c, tell_c) = (Rc::clone(&front), Rc::clone(&tell));
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
                        tell_c(Told::Done, &said);
                        after();
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
            Act::Installed => tell_c(
                Told::Done,
                "You already have this one — it is in Games, with a Play button.",
            ),
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
    fn challenge(&self, host: &str, code: &str) -> scry_net::Fetched<serde_json::Value>;
    /// `POST {host}/api/signin/{code}/proof` — deliver the signed message.
    fn deliver(&self, host: &str, code: &str, body: &serde_json::Value)
        -> scry_net::Fetched<serde_json::Value>;
}

pub struct RealSigninHttp;

impl SigninHttp for RealSigninHttp {
    fn challenge(&self, host: &str, code: &str) -> scry_net::Fetched<serde_json::Value> {
        scry_net::Net::new().get_json(&format!("{host}/api/signin/{code}/challenge"))
    }
    fn deliver(&self, host: &str, code: &str, body: &serde_json::Value)
        -> scry_net::Fetched<serde_json::Value> {
        scry_net::Net::new().post_json_told(&format!("{host}/api/signin/{code}/proof"), body)
    }
}

/// Pair a browser with this account — the GUI half of `scry signin`
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
/// (`scry signin`, attached in a terminal) is the surface built for that. A
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
    scry_broker::protocol::check_nonce(&code).map_err(|r| {
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
    let message = scry_broker::protocol::signin_message(
        &domain,
        &address,
        &code,
        &scry_broker::protocol::now_iso8601(),
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
         requests, pair a fresh code from a terminal: scry signin <code>"
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

/// The published client version NEWER than `mine`, off the origin's
/// `/api/launcher` card — or `None`, which deliberately covers four different
/// states: current, ahead (a dev build is not nagged), no client published,
/// and a card we could not read. The caller must therefore never render
/// `None` as "up to date" — the corner stays a bare local version, which
/// claims nothing about currency. Same three-answer discipline as `scry
/// check`, whose comment also carries the ruling this UI half inherits: **a
/// notice, never a self-update** — a binary that replaces itself must be
/// trusted absolutely, and the package manager or the download page does the
/// swap in the open.
pub fn newer_client(card: Option<&serde_json::Value>, mine: &str) -> Option<String> {
    let there = card?.get("client")?.get("version")?.as_str()?;
    (scry_depot::update::version_cmp(there, mine) == std::cmp::Ordering::Greater)
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
/// bounded version: every signature refreshes the account's `last_used`, so an
/// afternoon of play never relocks mid-session, and an unlock nobody has used
/// since lunch stops being one.
///
/// `after` of zero disables it — that is the operator's own machine saying so
/// (`SCRY_RELOCK_MINUTES=0`), not a default.
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
) {
    if after.is_zero() {
        return;
    }
    let mut status = signing_w.status.clone();
    let busy = Rc::new(std::cell::Cell::new(false));
    app::add_timeout3(RELOCK_POLL, move |handle| {
        // The same re-entrancy guard `serve_consent` carries, for the same
        // reason: the gate below runs a nested event loop, and FLTK dispatches
        // timeouts inside one. Without this, a player who walks away from the
        // lock screen gets a second one stacked on it every thirty seconds.
        if !busy.get() {
            busy.set(true);
            let relocked = match signer.lock() {
                Ok(mut s) if s.idle().is_some_and(|idle| idle >= after) => {
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
                lock_screen(
                    &signer,
                    "it locked itself while you were away — every signature resets that clock",
                );
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

/// Whether the lock screen is armed at all. `SCRY_LOCK_SCREEN=0` is the posted
/// door out (`CLAUDE.md` invariant 10), for the person who keeps this client
/// open on their own machine and does not want a gate in front of the store.
pub fn lock_screen_armed() -> bool {
    !matches!(
        std::env::var("SCRY_LOCK_SCREEN").as_deref().map(str::trim),
        Ok("0") | Ok("off") | Ok("no")
    )
}

/// Hold the client behind a passphrase until it opens — or the player quits.
///
/// Returns when the account is unlocked. **It does not return any other way**,
/// which is the ask: *"prompt for my passphrase and not go pass that."* Quit
/// ends the process rather than returning a `false` some caller might ignore,
/// because a gate whose refusal path is a boolean is one bad `if` away from
/// being no gate at all.
///
/// Three states walk straight through and none of them is a bypass:
///
/// * **no keystore** — there is nothing to unlock. Playing with no account is
///   a normal state and always has been (`docs/client/LAUNCHER.md` §1).
/// * **already unlocked** — the gate is for a locked key.
/// * **`SCRY_LOCK_SCREEN=0`** — the operator's own machine saying so.
pub fn lock_screen(signer: &Shared, why: &str) {
    if !lock_screen_armed() {
        return;
    }
    let (exists, unlocked, address) = match signer.lock() {
        Ok(s) => (s.exists(), s.is_unlocked(), s.address().unwrap_or_default()),
        // A poisoned lock means another thread panicked holding the signer.
        // Gating on it would strand the player in front of a window that can
        // never open, so this walks through — the account stays locked and
        // every act that needs it still refuses with its own reason.
        Err(_) => return,
    };
    if !exists || unlocked {
        return;
    }

    let mut win = crate::windows::lock(&address, why);
    let done = Rc::new(std::cell::Cell::new(false));

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
        let typed = field.value();
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
}

fn ask_twice(ask: &Ask, tell: &Tell, heading: &str, prompt: &str) -> Option<String> {
    let first = ask(heading, prompt)?;
    let again = ask(heading, "Type it again — it cannot be reset later:")?;
    if first != again {
        tell(Told::Refused, "Those two did not match — nothing was written.");
        return None;
    }
    Some(first)
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
    new_signer: scry_vault::LocalSigner,
    tell: &Tell,
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
    // vault would refuse anyway. Deactivating says why before it is pressed.
    for b in [account_w.make.clone(), account_w.import.clone()].into_iter().flatten() {
        let mut b = b;
        b.deactivate();
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

    tell(
        Told::Done,
        &format!(
            "Account created.\n\n{address}\n\n\
             The keystore is the only copy of this key:\n{}\n\n\
             Back it up. The passphrase cannot be reset — nobody else has it,\n\
             including scry.\n\n\
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
pub fn parse_secret(raw: &str) -> Result<[u8; 32], String> {
    let hex = raw.trim();
    let hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    if hex.len() != 64 {
        return Err(format!(
            "A private key is 64 hex characters (32 bytes); that was {}.",
            hex.len()
        ));
    }
    let mut out = [0u8; 32];
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
    Rc::new(move |_heading, _prompt| queue.borrow_mut().next().unwrap_or(None))
}

/// Somewhere for a test to read what the player was told.
pub fn recording_tell(log: Rc<RefCell<Vec<(Told, String)>>>) -> Tell {
    Rc::new(move |kind, text| log.borrow_mut().push((kind, text.to_string())))
}

/// Where the keystore lives, for a caller that wants to show it.
pub fn keystore_path() -> PathBuf {
    scry_vault::keystore_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_parsed_with_or_without_the_prefix() {
        let bare = "01".repeat(32);
        let prefixed = format!("0x{bare}");
        assert_eq!(parse_secret(&bare).unwrap(), [1u8; 32]);
        assert_eq!(parse_secret(&prefixed).unwrap(), [1u8; 32]);
        // Whitespace from a paste is the common case, not an exotic one.
        assert_eq!(parse_secret(&format!("  {prefixed}\n")).unwrap(), [1u8; 32]);
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
        fn challenge(&self, _: &str, _: &str) -> scry_net::Fetched<serde_json::Value> {
            panic!("the origin was consulted before the local refusal")
        }
        fn deliver(&self, _: &str, _: &str, _: &serde_json::Value)
            -> scry_net::Fetched<serde_json::Value> {
            panic!("a proof was delivered with nothing signed")
        }
    }

    /// A challenge that answers 404, for the expired-code path.
    struct Gone;
    impl SigninHttp for Gone {
        fn challenge(&self, _: &str, _: &str) -> scry_net::Fetched<serde_json::Value> {
            scry_net::Fetched::status(404)
        }
        fn deliver(&self, _: &str, _: &str, _: &serde_json::Value)
            -> scry_net::Fetched<serde_json::Value> {
            panic!("delivered after a refused challenge")
        }
    }

    fn no_signer() -> Shared {
        Arc::new(Mutex::new(scry_vault::LocalSigner::at("/nonexistent/keystore.json")))
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
            "scry-signin-test-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"address": "24445efddb08d4938e3e3627042b2cf4063d9092"}"#,
        )
        .expect("write test keystore");
        let signer: Shared =
            Arc::new(Mutex::new(scry_vault::LocalSigner::at(&path)));
        let ask: Ask = Rc::new(|_, _| panic!("asked for a passphrase over a dead code"));
        let e = pair_browser(&signer, "https://x.example", "abcd-2345", &ask, &Gone)
            .unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(e.contains("not waiting to sign in"), "{e}");
        // The lowercased, dashed input was normalised into the refusal's code.
        assert!(e.contains("ABCD2345"), "{e}");
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
