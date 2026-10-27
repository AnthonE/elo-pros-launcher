//! `elo-gui` — the launcher a player actually runs.
//!
//! Until this file existed there was a CLI and a window *library* and no
//! launcher. This is the program: it opens the menu, reads what is really
//! installed through `elo-depot`, and serves the game door through
//! `elo-broker` for as long as it is open.
//!
//! **The CLI stays separate and does not link FLTK.** `elo.exe` is 1.9 MB
//! because a headless box, a CI job and an RL harness have no use for a
//! window, and a launcher operable only through one would be the wrong shape
//! for half its users. Both binaries read the same crates, so neither is the
//! "real" one.
//!
//! What this deliberately does NOT do is written where it is missing rather
//! than in a TODO list — anything unbuilt renders as unbuilt and says so.
//! **A launcher that says "not built" is honest; one that shows a dead button
//! is not**, and `windows::MENU` carries that state per entry rather than a
//! list here going stale.
//!
//! The door signs now. `sign` refused for want of a window to ask in until
//! 2026-08-08; the consent prompt is `elo_ui::consent` plus
//! `wiring::serve_consent`, and what is left of that refusal is two honest
//! answers about the account — no account, and locked.
//!
//! ⚠ **The Windows subsystem is set here, and it is not cosmetic.** A PE built
//! with the default console subsystem opens a black terminal behind the window
//! every time a player double-clicks it — the first thing anyone would see of
//! this client on Windows, and it reads as something crashing. `windows` is the
//! subsystem a GUI program wants. What that costs is the two `println!`s below,
//! which go to a console that no longer exists; `attach_parent_console()` buys
//! them back for the case that actually wants them, someone running the binary
//! from a terminal on purpose. Console-subsystem-plus-FreeConsole is the other
//! way round and is worse: the window flashes before it is hidden.
#![cfg_attr(windows, windows_subsystem = "windows")]

use fltk::{app, prelude::*};
use elo_broker::signer::Signer;
use elo_broker::{protocol::Refusal, transport, Host, What};
use elo_ui::art;
use elo_ui::consent;
use elo_ui::windows::{self, Row};
use elo_ui::wiring;
use serde_json::{json, Value};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// The launcher's answers to the questions a game may ask.
///
/// Every method here is deliberately the *honest unbuilt* answer where the
/// thing behind it is unbuilt. `sign` refuses with a reason a player can read
/// rather than returning a fake signature, because a game that receives a
/// forged signature will send it somewhere that checks it.
struct Launcher {
    host: String,
    /// Kept for the windows this Host will answer for once they exist —
    /// `title` already answers by slug and the shard list is next.
    #[allow(dead_code)]
    games_root: std::path::PathBuf,
    /// The SAME account the windows hold. Unlocking in Signing has to be
    /// visible here, or `prove` could only ever answer "locked" — a launcher
    /// that unlocks in a window nothing else can see.
    signer: wiring::Shared,
    /// The origin reader, for `profile`. It carries the reachable/empty
    /// distinction the whole verb depends on, so this is deliberately not a
    /// bare `ureq` call.
    net: elo_net::Net,
    /// The way to the player. Every connection holds a clone of the same
    /// doorbell; the main thread is the only thing that answers it.
    consent: consent::Doorbell,
}

/// The browser pairing this machine currently reaches, or None.
///
/// Read from disk per call rather than held in the struct, and that is on
/// purpose: the Account window can make one while the door is already serving,
/// and a cached handle would mean a player pairs, presses Sign, and is told
/// there is no signer — with a live pairing sitting in a file two inches away.
/// It is one small read on `hello` and on a signature, neither of which is hot.
///
/// ⚠ **An expired pairing reads as None** (`pairing::load`), so no branch
/// below has to remember to check.
fn paired() -> Option<elo_net::pairing::Pairing> {
    elo_net::pairing::load(&elo_vault::pairing_path())
}

/// The wallet this launcher answers as: an unlocked account, then a live
/// browser pairing, then a locked account — the order `Launcher::sign`
/// reaches them in. One function, so the door and the Friends window cannot
/// name two different players.
fn known_address(signer: &wiring::Shared) -> Option<String> {
    let (local, unlocked) = signer
        .lock()
        .map(|s| (s.address(), s.is_unlocked()))
        .unwrap_or((None, false));
    if unlocked && local.is_some() {
        return local;
    }
    paired().map(|p| p.address).or(local)
}

impl Host for Launcher {
    fn address(&self) -> Option<String> {
        // The real one now. Reading it needs no passphrase — `LocalSigner::at`
        // takes the public address out of the keystore in the clear, which is
        // why a LOCKED account can still say who it is. `None` stays a real
        // answer: "someone browsing without an address set is a normal player"
        // (sdk/PROTOCOL.md).
        //
        // ⚠ **The same order `sign` below reaches them in**: an unlocked
        // account, then a live browser pairing, then a locked account. This
        // named the keystore first even when it was locked, while `sign` was
        // already falling through to the browser, so a game could be told one
        // address and handed a signature from another. A player who chose
        // their browser wallet at the lock screen is that wallet here.
        known_address(&self.signer)
    }

    fn signer_name(&self) -> String {
        // `local` once an account exists, because that is what is true: the
        // vault IS the backend, and `prove` works through it.
        //
        // This said `none` until 2026-08-06 and the reason has changed twice.
        // First it was that no Rust signer existed. Then it was that `sign`
        // had no consent window to ask in, and reporting `local` would make a
        // game offer a Sign button that always failed. `prove` settles it:
        // there is now a signing verb that works with no prompt, so reporting
        // `none` would hide the capability a game actually wants. `sign` still
        // refuses — with a reason, below — and that is the honest split.
        //
        // ⚠ **An UNLOCKED account wins, and a locked one loses to a live
        // pairing** — the order `address` and `sign` use, so the three never
        // disagree. The CLI prefers the pairing outright because it unlocks
        // per command, so a local signature always costs a prompt there; here
        // an unlocked account answers a game with no interruption at all. A
        // locked one cannot, and a pairing made by hand in the last twelve
        // hours is the newer deliberate act.
        let (exists, unlocked) = self
            .signer
            .lock()
            .map(|s| (s.exists(), s.is_unlocked()))
            .unwrap_or((false, false));
        if exists && unlocked {
            "local".into()
        } else if paired().is_some() {
            "browser-paired".into()
        } else if exists {
            "local".into()
        } else {
            "none".into()
        }
    }

    fn host_url(&self) -> String {
        self.host.clone()
    }

    /// Ask the player, in the launcher's own window.
    ///
    /// This runs on the **door thread** and the window belongs to the main
    /// one, so it rings a doorbell and parks — `elo_ui::consent` is that
    /// crossing, and every way it can fail answers refused.
    ///
    /// ⚠ **The account state is deliberately not checked here.** A locked
    /// account is not a refusal by the player, and returning `None` for one
    /// would both tell the game *"you refused"* — a sentence about a person,
    /// and untrue — and record a denial that sticks for the rest of the
    /// session (`Consent::denied`). So the prompt fires either way, says on
    /// its face that a signature cannot be produced yet, and `sign` below
    /// gives the game the actionable reason.
    fn ask_consent(&mut self, game: &str, family: &str, text: &str, why: &str) -> Option<u32> {
        self.consent.ask(consent::Ask::new(game, family, text, why))
    }

    /// Sign an EIP-191 message the game wrote, **after** the player allowed it.
    ///
    /// Consent is already spent by the time this is called — the broker asks
    /// first and signs second — so the only questions left are about the
    /// account, and they are the same two `prove` answers, for the same reason:
    /// no account and a locked account are different problems with different
    /// fixes, and one message covering both sends a player to make a second
    /// account they already have.
    fn sign(&mut self, text: &str, why: &str) -> Result<Value, Refusal> {
        // ⚠ **The unlocked local account answers first, and a browser pairing
        // is the fallback rather than the winner.** This is the reverse of
        // `elo-launcher/src/signer.rs`'s precedence and it is the SAME rule:
        // prefer the signer that can answer without interrupting. In the CLI
        // nothing is ever unlocked, so both doors cost an interruption and the
        // newer deliberate act breaks the tie; here an unlocked key answers a
        // game mid-play with no round trip at all, and bouncing that player to
        // a browser tab would be strictly worse for them.
        {
            let mut signer = self
                .signer
                .lock()
                .map_err(|_| Refusal::new("the account is busy — try again"))?;
            if signer.exists() && signer.is_unlocked() {
                return signer.sign(text, why);
            }
        }
        // The lock is dropped before this: the browser round trip can take a
        // person's whole attention span, and holding the account's mutex
        // across it would make Unlock in the Signing window hang behind a
        // stranger's tab.
        if let Some(p) = paired() {
            let mut s = elo_net::pairing::PairedBrowserSigner::new(p);
            return s.sign(text, why);
        }
        let signer = self
            .signer
            .lock()
            .map_err(|_| Refusal::new("the account is busy — try again"))?;
        if !signer.exists() {
            return Err(Refusal::new(
                "no signer on this machine — make an account in Account, or \
                 pair the wallet in your browser there instead. Playing \
                 without either is a normal state.",
            ));
        }
        Err(Refusal::new(
            "the account is locked — unlock it in Signing. It stays \
             unlocked until this launcher closes.",
        ))
    }

    /// The identity proof. **No consent prompt here** — the launcher wrote
    /// this message (`protocol::prove_message`), the game only handed over a
    /// nonce and a server name, and a signature over it authorises nothing.
    /// A server the title does not publish was already put to the player by
    /// the broker before this is called (`serves` below).
    ///
    /// An unlocked account on this machine signs at once. Otherwise a paired
    /// browser wallet signs, when the server is on the pairing's own site: the
    /// broker has already composed the message under that site's name
    /// (`prove_domain`), so the wallet's sign-in check passes, and its holder
    /// approves it there. Every other state refuses and says which it is.
    fn prove(&mut self, message: &str) -> Result<Value, Refusal> {
        let (exists, unlocked) = self
            .signer
            .lock()
            .map(|s| (s.exists(), s.is_unlocked()))
            .map_err(|_| Refusal::new("the account is busy — try again"))?;
        if exists && unlocked {
            let mut signer = self
                .signer
                .lock()
                .map_err(|_| Refusal::new("the account is busy — try again"))?;
            return signer.sign(message, "prove identity");
        }
        // Unlocked above and dropped here, for `sign`'s reason: the browser
        // round trip can take a person's whole attention span.
        if let Some(p) = paired() {
            let domain = message
                .split(" wants you to sign in")
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if p.signs_for(&domain).as_deref() == Some(domain.as_str()) {
                let mut s = elo_net::pairing::PairedBrowserSigner::new(p);
                return s.sign(message, "sign in to the game server");
            }
            return Err(Refusal::new(format!(
                "{domain} is not on {}, so your browser wallet can't sign in \
                 there from the launcher — the wallet would warn that it is a \
                 different site. An account on this machine can: make or \
                 import one in Account.",
                p.site()
            )));
        }
        if !exists {
            return Err(Refusal::new(
                "no account on this machine — make one in Account, pair your \
                 browser wallet there, or run `elo account new`. Playing \
                 without one is a normal state; a server that wants proof of \
                 who you are will not get it.",
            ));
        }
        Err(Refusal::new(
            "the account is locked — unlock it in Signing, or use your browser \
             wallet from Account. It stays unlocked until this launcher closes.",
        ))
    }

    /// Sign as the pairing's site when the paired browser wallet will be the
    /// one signing (`Host::prove_domain`): no unlocked account here, and the
    /// server is on that site. `None` keeps the launcher's own form.
    fn prove_domain(&self, server: &str) -> Option<String> {
        let unlocked = self
            .signer
            .lock()
            .map(|s| s.exists() && s.is_unlocked())
            .unwrap_or(false);
        if unlocked {
            return None;
        }
        paired()?.signs_for(server)
    }

    /// A name and a face for one address. **A read, never authentication** —
    /// `prove` is the verb that establishes who is holding a key.
    ///
    /// ⚠ **The three states are kept apart, and that is the whole care here.**
    /// A 404 from `/api/who` is a real ANSWER — nobody answers to this
    /// handle — and comes back as `ok` with a null profile. Anything else that
    /// failed refuses with `reachable` saying whether we ever got there. The
    /// bug this shape prevents is `CLAUDE.md` §traps: a reader that returns
    /// nothing for both "no profile" and "the origin is down" lets a game draw
    /// a confident "anonymous" over a network outage.
    fn profile(&mut self, address: &str) -> Result<Value, Refusal> {
        let url = format!("{}/api/who?handle={address}", self.host);
        let got = self.net.get_json(&url);
        if let Some(body) = got.value {
            return Ok(json!({
                "reachable": true,
                "profile": elo_broker::protocol::flatten_who(&body, address),
                "source": url,
            }));
        }
        if got.status == Some(404) {
            return Ok(json!({
                "reachable": true,
                "profile": Value::Null,
                "note": "nothing set for that address. Playing with a bare \
                         wallet is a normal state, and this is a real answer \
                         rather than a failed read.",
            }));
        }
        Err(Refusal::new(if got.reachable {
            format!("the origin answered, but not with a profile — {}", got.why)
        } else {
            format!(
                "could not reach the origin, so this launcher cannot say whether \
                 that address has a profile or not — {}. That is not the same as \
                 having none.",
                got.why
            )
        }))
    }

    fn title_url(&self, slug: &str, what: What) -> Option<String> {
        match what {
            // The origin serves manifests by slug; the client rebases.
            What::Manifest => Some(format!("{}/api/launcher/manifest/{slug}", self.host)),
            // The shard list is the GAME's to serve and this launcher still
            // does not invent one — it reads the url the title published and
            // answers `None` when there is none, which is the honest answer
            // for a title that has not published a list.
            //
            // ⚠ This returned a bare `None` until 2026-08-14, which is not the
            // same sentence. Gates published `servers.url` on 2026-08-11, and
            // this is the ONLY `Host` in the client that ships — every other
            // implementation of this trait is a test stub. So the SDK's
            // `Overlay::servers_url` backstop, which a game asks when no
            // `--servers` reached its argv, could never answer for any title
            // however many lists were live. A refusal written as a placeholder
            // reads exactly like a refusal that was decided.
            What::Servers => published_servers_url(&self.net, &self.host, slug),
        }
    }

    /// A sign-in to one of the title's own servers — a host in the shard list
    /// it publishes — signs with no prompt. Anything else, or a list that
    /// cannot be read, answers `false`, and the broker asks the player.
    fn serves(&mut self, game: &str, server: &str) -> bool {
        let Some(url) = published_servers_url(&self.net, &self.host, game) else {
            return false;
        };
        match self.net.shards(&url).value {
            Some(rows) => elo_depot::shardlist::serves(&rows, &url, server),
            None => false,
        }
    }

    fn open(&mut self, url: &str) -> Result<(), Refusal> {
        // https-only was already checked by the protocol layer. Opening a
        // browser is the one thing this does to the player's machine, so it
        // goes through the platform's own handler and never a shell string —
        // `cmd /C start` was one, and read `&` in a url as a second command.
        elo_depot::launch::open_page(url).map_err(|e| Refusal::new(e.to_string()))
    }
}

// Where the keystore lives is `elo_vault::keystore_path()` and is defined
// there ONLY. This file used to carry its own copy, which was fine while the
// GUI was the only thing that could hold a key — the moment `elo account new`
// existed, two copies of that logic became two places for the CLI and the
// window to disagree about which file is the account.

fn games_root() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("ELO_GAMES") {
        return std::path::PathBuf::from(p);
    }
    // The CLI's own rule (`elo_depot::dirs::home_based`), so the two cannot
    // disagree about where games live — and an empty or relative
    // `XDG_DATA_HOME` is not a folder under wherever the launcher started.
    elo_depot::dirs::home_based("XDG_DATA_HOME", ".local/share").join("elo").join("games")
}

/// Serve the game door for as long as the launcher is open.
///
/// Spawned detached: if the door cannot be opened the launcher still runs,
/// because being unable to serve games is not a reason to refuse to show a
/// player their library.
///
/// ⚠ **This read "one thread, one connection at a time — which is enough,
/// because a player runs one game" and that reasoning was wrong.** The count
/// that matters is connections per *game*, not games: a game holds its boot
/// connection for the life of the process and opens a second one every time it
/// wants a signature. `transport::serve_forever` is one thread per connection
/// and `elo-broker/tests/door.rs` is the failing case it was written against.
fn open_the_door(host: String, games_root: std::path::PathBuf,
                 signer: wiring::Shared, consent: consent::Doorbell,
                 held: Arc<std::sync::atomic::AtomicUsize>) -> Option<String> {
    let endpoint = transport::default_endpoint();
    let listener = match transport::Listener::bind(&endpoint) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("elo-gui: the game door did not open ({e}) — games will \
                       still install and start, but they will play anonymously");
            return None;
        }
    };
    let shown = listener.endpoint();
    std::thread::spawn(move || {
        transport::serve_forever_counted(listener, held, move || Launcher {
            host: host.clone(),
            games_root: games_root.clone(),
            signer: Arc::clone(&signer),
            net: elo_net::Net::new(),
            consent: consent.clone(),
        })
    });
    Some(shown)
}

/// Re-attach to the terminal that started us, when there is one.
///
/// A `windows` subsystem binary is given no console, so `println!` writes into
/// a closed handle and a player who ran `elo-gui.exe` from PowerShell on
/// purpose sees nothing at all. `ATTACH_PARENT_PROCESS` (`-1`) borrows the
/// parent's console if it has one and fails harmlessly if it does not — which
/// is the double-click case, and the case we wanted no console for. Nothing is
/// created, so this can never be the thing that opens a black window.
///
/// The three standard handles are re-opened onto it, because attaching a
/// console does not redirect the streams Rust already captured at startup.
/// Win32 straight through, and deliberately NOT the CRT's `freopen`: that
/// route needs `__acrt_iob_func` on the UCRT and `__iob_func` on mingw's
/// msvcrt, so it would build under one Windows toolchain and fail to link
/// under the other. `SetStdHandle` is the same call on both, and Rust's
/// `println!` re-reads the std handle on each write, so it is enough.
#[cfg(windows)]
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX; // (DWORD)-1
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE_VALUE: *mut core::ffi::c_void = usize::MAX as *mut _;
    // -11 and -12 as DWORDs; std's own names for stdout and stderr.
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
    const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;

    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn CreateFileW(
            name: *const u16, access: u32, share: u32, security: *mut core::ffi::c_void,
            disposition: u32, flags: u32, template: *mut core::ffi::c_void,
        ) -> *mut core::ffi::c_void;
        fn SetStdHandle(which: u32, handle: *mut core::ffi::c_void) -> i32;
    }

    // SAFETY: one Win32 call taking an integer. Its failure is a return code we
    // deliberately ignore — "the parent has no console" is the double-click
    // case, which is the case this whole subsystem change exists to serve.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    // SAFETY: `name` is a NUL-terminated UTF-16 buffer that outlives the call,
    // and the two null pointers are the documented "no security descriptor, no
    // template" arguments. A failed open leaves the handles as they were.
    let conout = unsafe {
        CreateFileW(name.as_ptr(), GENERIC_WRITE, FILE_SHARE_WRITE,
                    core::ptr::null_mut(), OPEN_EXISTING, 0, core::ptr::null_mut())
    };
    if conout != INVALID_HANDLE_VALUE && !conout.is_null() {
        // SAFETY: a live console handle, published under the two std slots.
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, conout);
            SetStdHandle(STD_ERROR_HANDLE, conout);
        }
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn main() {
    attach_parent_console();
    // Before any key can be in memory: no core file, no same-user attach
    // (`elo_vault::harden`).
    for e in elo_vault::harden_process() {
        eprintln!("elo-gui: {e}");
    }
    let host = std::env::var("ELO_HOST")
        .unwrap_or_else(|_| "https://elopros.com".into());
    let root = games_root();
    // Streamer mode before any window is drawn: the windows read the switch.
    elo_depot::streamer::load(&elo_depot::dirs::state_root());

    // The account first, because the door needs it: a game may ask this
    // launcher to prove who is playing, and that is served on the door's
    // thread from the same account the windows unlock.
    let keystore = elo_vault::keystore_path();
    let signer: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&keystore)));

    // The doorbell, which both threads hold: the door rings it, and the main
    // loop below is the only thing that ever answers. It is made here rather
    // than inside `open_the_door` because the answering half outlives any one
    // connection — a launcher whose door failed to open still runs, and a
    // pump with nothing to pump costs nothing.
    let doorbell = consent::Doorbell::new();

    // Then the door, so its path is in the environment before any game starts.
    // `held` counts the connections games hold open on it: a running game
    // keeps one for its whole life, so above zero means someone is playing.
    let held = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let door = open_the_door(host.clone(), root.clone(), Arc::clone(&signer), doorbell.clone(),
                             Arc::clone(&held));
    if let Some(endpoint) = &door {
        std::env::set_var(transport::SOCKET_ENV, endpoint);
        println!("elo-gui: the game door is open at {endpoint}");
    }

    let a = app::App::default();
    let has_account = signer.lock().map(|s| s.exists()).unwrap_or(false);

    // ── the gate ────────────────────────────────────────────────────────────
    //
    // Operator, 2026-08-12: *"when i return to the app it should prompt for my
    // passphrase and not go pass that."* So the client opens locked when there
    // is an account to unlock, and nothing else is drawn until it opens. With
    // no account this is a no-op — playing without one stays a normal state —
    // and `ELO_LOCK_SCREEN=0` turns it off (`wiring::lock_screen_armed`).
    //
    // It runs BEFORE the windows are built rather than over them, so a player
    // does not watch the client assemble itself behind a modal they cannot
    // dismiss.
    let gate = wiring::lock_screen(
        &signer,
        "unlock the account on this machine, or use the wallet in your browser",
    );

    let mut menu = windows::main_menu(has_account, env!("CARGO_PKG_VERSION"));

    // Where the client opens. Every window is constructed at a hard-coded
    // coordinate — the menu at `(100, 100)`, then a 20px cascade — so the
    // whole family is put back where the player is looking by centring the
    // menu and carrying everything else by the same amount. That keeps the
    // cascade the windows were built with instead of stacking six windows on
    // one spot (`wiring::centre`).
    let drift = wiring::centre(&mut menu.window);

    // The streamer switch. Saved at once, so the next game started gets it;
    // windows already drawn keep what they drew, and the line beside it says so.
    {
        let mut line = menu.streamer_line.clone();
        menu.streamer.set_callback(move |b| {
            let want = b.is_checked();
            match elo_depot::streamer::save(&elo_depot::dirs::state_root(), want) {
                Ok(()) => {
                    line.set_label(windows::streamer_note(want, true));
                    line.set_label_color(if want { elo_ui::theme::GOLD } else { elo_ui::theme::MUTED });
                }
                Err(e) => {
                    b.set_checked(!want);
                    line.set_label(&format!("could not save it: {e}"));
                    line.set_label_color(elo_ui::theme::RED);
                }
            }
            line.redraw_label();
            if let Some(mut p) = line.parent() {
                p.redraw();
            }
        });
    }

    // ⚠ **Escape must not quit the launcher.** FLTK ends a window on Escape by
    // calling its callback (`Fl.cxx`: *"make Escape key close windows"*), and
    // for the main menu that is the whole program: the game door closes, every
    // other window goes with it, and the player pressed one key. It is exactly
    // the key they press to dismiss the passphrase prompt, so it is exactly
    // the key that lands on this window a moment later. `Event::Close` is the
    // window manager's own close box and nothing else, which is the one that
    // means it.
    //
    // Closing the menu DOES quit, deliberately: it is the way back to every
    // other window, so a client with the menu shut and Games still open is a
    // program the player cannot navigate but also cannot see they still have.
    menu.window.set_callback(|_| {
        if app::event() == fltk::enums::Event::Close {
            app::quit();
        }
    });
    menu.window.show();
    // Paint it NOW. Everything below reads the origin — the shelf, the shard
    // list, the version card — before `a.run()` starts the event loop, so
    // without this the menu stayed an empty frame until the last read came
    // back: up to a minute per read with the origin down, which a player reads
    // as a launcher that did not start. A few turns of the loop is enough for
    // the first expose.
    app::flush();
    for _ in 0..5 {
        if !app::wait_for(0.02).unwrap_or(false) {
            break;
        }
    }

    // ── the one place a restart is genuinely the fix ────────────────────────
    //
    // The game door is a socket bound once at startup. If the bind failed —
    // another launcher already running, a stale path this process cannot
    // remove, a full or read-only runtime dir — nothing later in this process
    // retries it, so every game started from here plays anonymously for the
    // rest of the session. There is no button that repairs that; a fresh
    // process is the repair.
    //
    // Operator, 2026-08-12: *"if we need the user to reboot say so and try to
    // self reboot or something."* Both halves, here: it says so, in a window
    // rather than on a console nobody is reading, and it offers to do it.
    if door.is_none() {
        let restart = wiring::show_notice(
            windows::Note::Refused,
            "Games can't reach your account",
            "Games will still install and start, but they will play anonymously:\n\
             they can't ask elo who is playing or for a signature.\n\n\
             The usual cause is another copy of elo already running. Close it,\n\
             then restart this one.",
            Some("Restart elo now"),
        );
        if restart {
            // `restart_now` never returns on success — it re-execs and exits —
            // so the only value it can hand back is the reason it could not.
            let Err(e) = wiring::restart_now();
            wiring::show_notice(
                windows::Note::Refused,
                "Could not restart",
                &format!("{e}\n\nClose this program and start it again yourself."),
                None,
            );
        }
    }

    // Every window is built once and shown on demand — the original client's
    // shape, and it means a player can leave Games open while reading Signing.
    //
    // `LocalSigner::at` reads only the public address, so a LOCKED account
    // still shows who it is — what a player expects, and it costs no
    // passphrase. Unlocking is a deliberate act in Signing, never on startup.
    //
    // ⚠ **Shared, because two windows act on one account.** Account makes it
    // and Signing unlocks it; if each held its own copy, a key unlocked in one
    // would be invisible to the other and the player would be told to unlock
    // something they just unlocked. `Rc<RefCell<…>>` and not `Arc<Mutex<…>>`:
    // every one of these callbacks runs on FLTK's own thread. The door's
    // thread deliberately does not share this — see `Launcher::sign`.

    // Everything the two shelves' buttons do. One object, shared by both
    // windows, so `Install` in the Store and `Update` in Games are the same
    // code path with the same rules.
    let shop = Rc::new(Front {
        host: host.clone(),
        root: root.clone(),
        started: Default::default(),
        installing: Installing::default(),
    });
    let front: Rc<dyn wiring::Storefront> = shop.clone();


    let account_w = {
        let s = signer.lock().expect("signer lock");
        windows::account(s.address().as_deref(), &host)
    };
    let signing_w = {
        let s = signer.lock().expect("signer lock");
        windows::signing(s.kind(), &wiring::status_shown(&s.status(), s.address().as_deref()),
                         s.address().as_deref(), s.exists())
    };

    // The behaviour lives in `elo_ui::wiring` so it can be driven by a test
    // and by `examples/click.rs`; a callback defined in this binary would be
    // the one path in the client nothing could reach.
    wiring::wire_account(&account_w, &signing_w, &signer, &keystore,
                         wiring::real_ask(), wiring::real_tell(), wiring::real_show_words());
    wiring::wire_signing(&signing_w, &signer, wiring::real_ask(), wiring::real_tell());
    // The website pairing (`docs/client/SIGN-IN.md`): the code field in
    // Account. Always asks the passphrase — an unlocked launcher must not let
    // whoever is at the keyboard sign their own browser in — and a transient
    // unlock relocks when the pairing is done.
    wiring::wire_signin(&account_w, &signer, &host, wiring::real_ask(),
                        wiring::real_tell(), std::rc::Rc::new(wiring::RealSigninHttp));
    // The OTHER pairing (`SIGN-IN.md` §8): lend this launcher the wallet in a
    // browser. No passphrase, because there is no key here to prove — the
    // browser's own wallet is the thing that consents, one message at a time.
    // Note it takes no `signer`: that is the point, and the button is offered
    // whether or not this machine has an account.
    wiring::wire_pair(&account_w, &host, wiring::real_tell());
    // Your profile: the face and name in the account well, read off the UI
    // thread, and **Edit profile…** — one signature by this account.
    wiring::wire_profile(&account_w, &signer, &host, wiring::real_ask(), wiring::real_tell());
    if let Some(addr) = signer.lock().ok().and_then(|s| s.address()) {
        wiring::show_profile(&account_w, &host, &addr);
    }
    if let Some(p) = paired() {
        wiring::show_pairing(&account_w, &p.address);
    }

    // Start answering the door. This must be registered before `a.run()` and
    // never from the door's own thread — FLTK's widgets belong to this one,
    // and the whole shape of `elo_ui::consent` exists to keep that true.
    wiring::serve_consent(doorbell, Arc::clone(&signer));

    // Relock an unlocked key that sits idle — every signature refreshes the
    // clock, so play never trips it. Minutes; 0 disables (the operator's own
    // machine saying so, not a default).
    let relock_min = std::env::var("ELO_RELOCK_MINUTES")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(30);
    //
    // Playing counts as use: a game holding the door, or one Play started.
    // Without this the gate came up over the game half an hour in, because a
    // game signs when it joins and then not again (`wiring::serve_relock`).
    let playing: Rc<dyn Fn() -> bool> = {
        let (shop, held) = (Rc::clone(&shop), Arc::clone(&held));
        Rc::new(move || {
            held.load(std::sync::atomic::Ordering::Relaxed) > 0 || shop.any_running()
        })
    };
    let open_account: Rc<dyn Fn()> = {
        let w = account_w.window.clone();
        Rc::new(move || {
            let mut w = w.clone();
            w.show();
        })
    };
    wiring::serve_relock(
        Arc::clone(&signer),
        &signing_w,
        std::time::Duration::from_secs(relock_min * 60),
        wiring::lock_screen_armed(),
        playing,
        Rc::clone(&open_account),
    );

    // The two shelves. The Store is read first because the library borrows its
    // names and its art — one read, and the two windows cannot then disagree
    // about what a title is called (`Ui`).
    //
    // Until 2026-08-12 the Games window was built here and its controls thrown
    // on the floor, so every Play button in the library was a painting.
    let ui = Rc::new(Ui {
        host: host.clone(),
        root: root.clone(),
        front: Rc::clone(&front),
        drift,
        shelf: std::cell::RefCell::new(Vec::new()),
        store: std::cell::RefCell::new(None),
        games: std::cell::RefCell::new(None),
        friends: std::cell::RefCell::new(None),
        want: std::cell::RefCell::new(std::collections::BTreeSet::new()),
        servers: std::cell::RefCell::new(None),
        pairs: std::cell::RefCell::new(None),
        orders: std::cell::RefCell::new(None),
        orders_run: std::cell::RefCell::new(None),
        orders_said: std::cell::RefCell::new(None),
        skins: std::cell::RefCell::new(None),
        skins_key: std::cell::Cell::new(0),
        skins_said: std::cell::RefCell::new(None),
        signer: Arc::clone(&signer),
        changing: std::cell::Cell::new(false),
    });
    // Read off this thread: the menu is up and answering while the catalog
    // and the library load, and Store or Games pressed before they land opens
    // as soon as they do (`wanted`).
    refresh_shelves(&ui);

    // The wallet: balances read off the chain each time it opens, and a send
    // that is checked, shown, and then asks the passphrase (`wiring::wire_wallet`).
    let wallet_w = {
        let s = signer.lock().expect("signer lock");
        windows::wallet(s.address().as_deref())
    };
    let open_url: Rc<dyn Fn(&str)> = {
        let f = Rc::clone(&front);
        Rc::new(move |u| {
            if let Err(e) = f.open(u) {
                eprintln!("elo-gui: {e}");
            }
        })
    };
    let read_wallet = wiring::wire_wallet(&wallet_w, &signer, wiring::WalletDesk::real(&host, open_url),
                                          wiring::real_ask(), wiring::real_tell(),
                                          wiring::real_review(), wiring::real_then());

    let windows_by_label: Vec<(&str, fltk::window::Window)> = vec![
        ("Account", account_w.window.clone()),
        ("Signing", signing_w.window.clone()),
        ("About", windows::about()),
    ];
    // Chat is built now and connects only when opened: a player who never
    // presses Chat never touches the relay (`wiring::wire_chat`). The chat key
    // and the cache are `elo-hive`'s paths, the same ones `elo chat` uses.
    let chat_w = windows::chat();
    let open_chat = wiring::wire_chat(&chat_w, &host, elo_hive::voice::default_dir(),
                                      elo_hive::chat::default_cache_dir(),
                                      Some(wiring::ChatLink {
                                          signer: Arc::clone(&signer),
                                          ask: wiring::real_ask(),
                                          confirm: wiring::real_confirm(),
                                          tell: wiring::real_tell(),
                                      }));
    // Carried by the same amount the menu was, so the cascade they were built
    // with survives the move. Done before any of them is shown — see
    // `wiring::centre`.
    for w in windows_by_label.iter().map(|(_, w)| w).chain([&chat_w.window, &wallet_w.window]) {
        let mut w = w.clone();
        w.set_pos(w.x() + drift.0, w.y() + drift.1);
    }
    for i in 0..menu.window.children() {
        if let Some(mut child) = menu.window.child(i) {
            let label = child.label();
            if let Some((_, w)) = windows_by_label.iter().find(|(n, _)| *n == label) {
                let mut w = w.clone();
                child.set_callback(move |_| {
                    w.show();
                });
                continue;
            }
            // Store and Games are the two whose window is REPLACED on a
            // refresh, so the menu asks the cell for the current one rather
            // than holding a handle a rebuild would strand.
            let ui_c = Rc::clone(&ui);
            match label.as_str() {
                "Store" => child.set_callback(move |_| {
                    match ui_c.store.borrow().as_ref() {
                        Some(s) => s.window.clone().show(),
                        None => {
                            ui_c.want.borrow_mut().insert("Store");
                        }
                    }
                }),
                "Games" => child.set_callback(move |_| {
                    match ui_c.games.borrow().as_ref() {
                        Some(g) => g.window.clone().show(),
                        None => {
                            ui_c.want.borrow_mut().insert("Games");
                        }
                    }
                }),
                "Chat" => {
                    let open = Rc::clone(&open_chat);
                    child.set_callback(move |_| open());
                }
                "Wallet" => {
                    let (w, read) = (wallet_w.window.clone(), Rc::clone(&read_wallet));
                    child.set_callback(move |_| {
                        let mut w = w.clone();
                        w.show();
                        read();
                    });
                }
                // Read every time it is opened, not once at startup: where a
                // friend is playing is a measurement, and an hour-old one
                // shown as current is the cached balance wall 3 refuses.
                "Friends" => child.set_callback(move |_| defer(&ui_c, refresh_friends)),
                // The same for player counts: read when the window is asked
                // for, so a count is never a startup number shown all session.
                "Servers" => child.set_callback(move |_| defer(&ui_c, refresh_servers)),
                "Pairs" => child.set_callback(move |_| defer(&ui_c, refresh_pairs)),
                "Orders" => child.set_callback(move |_| defer(&ui_c, refresh_orders)),
                "Skins" => child.set_callback(move |_| defer(&ui_c, refresh_skins)),
                _ => {}
            }
        }
    }

    // The first-run push, measured against a real player: "idk how to make a
    // new key" (operator, 2026-08-11). With no account on this machine the
    // Account window opens ITSELF, Make/Import front and center, instead of
    // waiting to be found behind a menu button. Closable, once per launch,
    // and never again once a keystore exists — an account stays optional
    // (games play anonymously and say so), it just stops being hidden.
    //
    // Choosing the browser wallet at the gate lands here too: Account is where
    // the pairing starts, and the key on this machine stays locked.
    if !has_account || gate == wiring::Gate::Browser {
        open_account();
    }

    // Is this build current? Same read and same three answers as `elo
    // check`, whose comment carries the ruling: **a notice, never a
    // self-update** — a binary that replaces itself is the one part of this
    // client that would need absolute trust, so the swap stays with the
    // package manager or the download page, in the open. Only a MEASURED
    // newer version speaks; current, ahead, unpublished and unreadable all
    // leave the corner as built (`newer_client`'s own test pins that), so
    // "could not look" never wears "up to date".
    //
    // Read off this thread like every other origin read (`load`), so a slow
    // origin delays the notice and never the menu.
    let card_url = format!("{host}/api/launcher");
    let mut version_line = menu.version_line.clone();
    let host_c = host.clone();
    load(&ui, false, move || elo_net::Net::new().get_json(&card_url), move |ui, card, _| {
        let host = host_c;
        if let Some(newer) = wiring::newer_client(card.value.as_ref(), env!("CARGO_PKG_VERSION")) {
            let mine = env!("CARGO_PKG_VERSION");
            // The origin's version string, so drawn through the escape like
            // any other text this program did not write.
            version_line.set_label(&elo_ui::chrome::defuse(&format!("v{mine} — {newer} is out")));
            version_line.set_label_color(elo_ui::theme::GOLD);
            version_line.redraw_label();
            println!("elo-gui: client {mine} — {newer} is published at {host}/download.html");
            // One dialog, once per launch, dismissible — a nudge with the reason
            // in it, never a nag loop and never a download this program performs.
            //
            // ⚠ **This was the last `dialog::message_default` in the client**, and
            // it outlived the pass that removed the other three (`windows.rs`, the
            // three-stock-dialogs note). It drew FLTK's own grey box with a blue
            // `?` over an olive client — the operator's *"the passphrase screen
            // doesnt match anything else"*, surviving in the one window a player
            // sees before they have touched anything. `Note::Done` and not
            // `Refused`: a published update is news, not a fault.
            if wiring::show_notice(
                windows::Note::Done,
                "A newer elo is published",
                &format!(
                    "{newer} is out — this is {mine}.\n\n\
                     Get it at {host}/download.html, or from your package manager\n\
                     if you installed it there.\n\n\
                     This program does not replace itself: a binary that rewrites\n\
                     itself would need absolute trust, so the swap happens in the\n\
                     open, where you can see it."
                ),
                // Same rule as the restart notice one screen up: the step after
                // "a newer one is out" is going to get it, so offer that rather
                // than print an address and leave the player to carry it. The
                // program still does not replace itself — this opens a page.
                Some("Open the download page"),
            ) {
                let url = format!("{host}/download.html");
                if let Err(e) = ui.front.open(&url) {
                    wiring::show_notice(
                        windows::Note::Refused,
                        "Could not open a browser",
                        &format!("{e}\n\nOpen it yourself:\n{url}"),
                        None,
                    );
                }
            }
        }
    });

    a.run().unwrap();
}

/// The origin's catalog, as `(rows, reachable)`.
///
/// Reads the same route the CLI's `elo games` reads, so the two halves of the
/// client cannot disagree about what is listed.
/// The title whose shards the Servers window shows.
///
/// One title, because there is one game. When there are two this becomes a
/// picker rather than a constant — and that is a real change to the window,
/// not a loop over this line, which is why it is named here rather than
/// hidden in a call.
const SERVERS_SLUG: &str = "gates";

/// A title's shard list, with a live count polled from each shard.
///
/// **Three states out, because there are three.** The launcher does not
/// invent, cache or rank a list (`docs/client/LAUNCHER.md` §6), and it must not
/// collapse "we could not look" into "nothing is up" — a player on a train
/// would be told the game is dead.
/// The shard-list url a title publishes, or `None` when it publishes none.
///
/// Two things needed this and neither had it, which is why one title could be
/// launched with a shard list through one door and without it through the
/// other: **Play** passed an empty values map, so `{servers}` filled as the
/// empty string, and the **game door** answered `None` to
/// `Overlay::servers_url` no matter what the manifest said. A game that came in
/// either way found nothing and fell back to its loopback default.
///
/// It fetches per call rather than caching. A manifest read is one request
/// against the origin the client is already talking to, and a cache here would
/// be a launcher holding a stale answer about a document that is the title's to
/// change — `read_shards` below makes the same read for the same reason.
///
/// ⚠ **On the SHORT deadline, and that is not a detail.** Both callers are
/// holding up something a player is waiting for — the Play button on the UI
/// thread, and a game's own `servers_url` call on the door thread. At
/// `elo_net::TIMEOUT` an unreachable origin would freeze the launcher for a
/// minute before starting a game that is installed and ready, which is a worse
/// bug than the empty shard list this function exists to prevent. A miss
/// returns `None` and the launch carries on without the list.
///
/// `read_shards` below deliberately keeps the long deadline: a player who
/// opened the Servers window IS waiting for that list, and there is nothing
/// else for that window to draw.
fn published_servers_url(net: &elo_net::Net, host: &str, slug: &str) -> Option<String> {
    let v = net.get_json_quick(&format!("{host}/api/launcher/manifest/{slug}")).value?;
    elo_depot::parse_manifest(&v).ok()?.servers_url
}

fn read_shards(host: &str, slug: &str) -> windows::Shards {
    let net = elo_net::Net::new();
    let url = {
        let got = net.get_json(&format!("{host}/api/launcher/manifest/{slug}"));
        let Some(v) = got.value else {
            return windows::Shards::Unreadable {
                url: format!("{host}/api/launcher/manifest/{slug}"),
                why: got.why,
                reachable: got.reachable,
            };
        };
        match elo_depot::parse_manifest(&v) {
            // A title with no `servers.url` is dark BY DESIGN — the list is
            // its to serve and it has not. Not an error.
            Ok(m) => match m.servers_url {
                Some(u) => u,
                None => return windows::Shards::Unpublished,
            },
            Err(e) => {
                return windows::Shards::Unreadable {
                    url: format!("{host}/api/launcher/manifest/{slug}"),
                    why: e.to_string(),
                    reachable: true,
                }
            }
        }
    };

    let got = net.shards(&url);
    let Some(mut rows) = got.value else {
        return windows::Shards::Unreadable { url, why: got.why, reachable: got.reachable };
    };
    // Counts are polled from each shard, never read out of the document — see
    // `shardlist.rs`. A shard that does not answer keeps whatever the row
    // said, which is usually nothing, and draws `?`.
    for r in rows.iter_mut() {
        let Some(su) = r.status_url_to_poll(&url).map(str::to_string) else { continue };
        if let Some(s) = net.status(&su, &url).value {
            r.apply_status(&s);
        }
    }
    println!("elo-gui: {} shard(s) listed for {slug}", rows.len());
    windows::Shards::Listed { url, rows }
}

/// Give the Servers window's controls their behaviour.
///
/// **Join starts the installed build with `{server}` filled in** — the same
/// substitution `elo play --server` performs, through the same
/// `launch::resolve`, because a second launch path is a second place for the
/// argv rules to be got wrong.
///
/// `{servers}` rides along with it, and for the same reason: a player who came
/// in through this window and then backs out to the game's own browser must
/// not find it empty. `list_url` is whatever `read_shards` actually read, so
/// the two surfaces are drawing one document.
///
/// It deliberately does NOT install. A player who has not got the game should
/// be told so, not have a download begin because they clicked Join on a row
/// they were browsing; installing is the Store's verb and it asks first.
fn wire_servers(
    w: &windows::ServersWindow,
    slug: &str,
    host: &str,
    games_root: &std::path::Path,
    list_url: Option<&str>,
) {
    for row in &w.rows {
        let addr = row.addr.clone();
        let slug = slug.to_string();
        let host = host.to_string();
        let root = games_root.to_path_buf();
        let list_url = list_url.map(str::to_string);
        let mut join = row.join.clone();
        join.set_callback(move |b| {
            // The most recently INSTALLED build, by directory mtime. Not by
            // name: a build id ends in a hex sha and hex does not sort, which
            // is how a row reading "up to date" launched a four-day-old client.
            // And one the host installed: a slug is anyone's to write.
            let Some(i) = elo_depot::newest_from(&root, &slug, &host) else {
                b.set_label("Not installed");
                b.deactivate();
                return;
            };
            let mut values =
                std::collections::BTreeMap::from([("server".to_string(), addr.clone())]);
            if let Some(u) = list_url.clone() {
                values.insert("servers".to_string(), u);
            }
            match elo_depot::launch::resolve(&i, &values, &std::collections::BTreeMap::new())
                .and_then(|l| elo_depot::launch::spawn(&l))
            {
                Ok(pid) => println!("elo-gui: joined {addr} (pid {pid})"),
                Err(e) => {
                    // Said on the control the player pressed. A launcher that
                    // logs a failure to a console nobody is reading has not
                    // reported it.
                    eprintln!("elo-gui: could not join {addr}: {e}");
                    b.set_label("Failed");
                    b.deactivate();
                }
            }
        });

        let link = row.link.clone();
        let mut copy = row.copy.clone();
        if link.is_empty() {
            // No link means the writer refused this address, and a button
            // that copies nothing is worse than one that is plainly off.
            copy.deactivate();
        } else {
            copy.set_callback(move |b| {
                fltk::app::copy(&link);
                // The only feedback a clipboard can give. It stays changed,
                // because the next thing the player does is leave.
                b.set_label("Copied");
            });
        }
    }
}

/// The two windows that are REBUILT rather than built once, and what they need.
///
/// Every other window in this client is constructed at startup and shown on
/// demand. These two cannot be: their height, their rows and their controls
/// are all functions of a read, so re-rendering means rebuilding the widgets.
///
/// The state exists because of the operator's *"if we need the user to reboot"*
/// (2026-08-12). Both windows had a restart baked into them: a catalog fetched
/// once at startup meant a player whose wifi was down when the client opened
/// saw *"the catalog could not be read"* until they restarted the program, and
/// a game installed from the Store did not appear in Games until the next
/// launch. Both are now a read away.
struct Ui {
    host: String,
    root: std::path::PathBuf,
    front: Rc<dyn wiring::Storefront>,
    /// How far the menu moved when it was centred, for the FIRST build of
    /// these two. Every later one inherits the position of the window it
    /// replaces instead — see [`place`].
    drift: (i32, i32),
    /// The last shelf that was read. The library borrows it for names and
    /// icons, so the two windows cannot disagree about what a title is called.
    shelf: std::cell::RefCell<Vec<windows::Shelf>>,
    store: std::cell::RefCell<Option<windows::StoreWindow>>,
    games: std::cell::RefCell<Option<windows::GamesWindow>>,
    /// Built on first open and rebuilt on every open after: presence moves,
    /// so the window is read when it is asked for rather than at startup.
    friends: std::cell::RefCell<Option<windows::FriendsWindow>>,
    /// Windows asked for before their first read landed — see [`wanted`].
    want: std::cell::RefCell<std::collections::BTreeSet<&'static str>>,
    /// Servers, read each time it is opened: a player count is a measurement.
    servers: std::cell::RefCell<Option<windows::ServersWindow>>,
    /// Pairs, read each time it is opened: a price is a measurement.
    pairs: std::cell::RefCell<Option<windows::PairsWindow>>,
    /// Orders, rebuilt from the book on disk whenever it changes.
    orders: std::cell::RefCell<Option<windows::OrdersWindow>>,
    /// The runner's stop flag while it runs, and what it has said since the
    /// window last looked.
    orders_run: std::cell::RefCell<Option<Arc<std::sync::atomic::AtomicBool>>>,
    orders_said: std::cell::RefCell<Option<std::sync::mpsc::Receiver<String>>>,
    /// Skins, read each time it is opened; which collection it shows; and the
    /// answer of the one sale, cancel or offer in flight.
    skins: std::cell::RefCell<Option<windows::SkinsWindow>>,
    skins_key: std::cell::Cell<usize>,
    skins_said: std::cell::RefCell<Option<std::sync::mpsc::Receiver<Result<String, String>>>>,
    /// Whose list Friends reads — see [`known_address`].
    signer: wiring::Shared,
    /// A friends change is in flight. One at a time: a second press while the
    /// first waits on a passphrase or a browser would sign over a list the
    /// first is about to change.
    changing: std::cell::Cell<bool>,
}

/// Rebuild a window on the next event-loop tick.
///
/// ⚠ **A rebuild may never run inside the old window's own callback.**
/// Replacing the cell DROPS the previous window, which frees the very button
/// whose callback is on the stack — a use-after-free with the player's finger
/// still on it. A timeout runs outside every widget callback, which is what
/// makes the swap safe. The delay is small and non-zero so the press paints
/// first.
fn defer(ui: &Rc<Ui>, what: fn(&Rc<Ui>, bool)) {
    let ui = Rc::clone(ui);
    app::add_timeout3(0.05, move |_| what(&ui, true));
}

/// Put a rebuilt window where the one it replaces was standing.
///
/// **A refresh is not a new window to the player**, whatever it is to this
/// program. Pressing Refresh rebuilt the widgets and put the result back at
/// the constructor's hard-coded coordinate, so a Store the player had dragged
/// onto their second monitor jumped home — and a game installed from the Store
/// refreshes Games too, which meant a window moving on its own while nobody
/// had touched it.
///
/// The SIZE is deliberately not carried. A window's height is a function of
/// how many rows there are, and a library that just lost a game would keep a
/// well of empty olive where the game used to be.
fn place(new: &mut fltk::window::Window, previous: Option<(i32, i32)>, drift: (i32, i32)) {
    match previous {
        Some((x, y)) => new.set_pos(x, y),
        None => new.set_pos(new.x() + drift.0, new.y() + drift.1),
    }
}

/// Read off the window's thread, then build on it.
///
/// Every window that shows what the origin or the chain says is put up this
/// way. The reads used to run right here, so the whole client — the menu, a
/// consent prompt, the relock — froze for as long as the slowest answer took:
/// a few seconds at startup on a good line, up to a minute a read with the
/// origin down. `put` runs on this thread from a timer, never inside a widget
/// callback, which is what makes swapping a window in safe (see [`defer`]).
fn load<T: Send + 'static>(
    ui: &Rc<Ui>,
    show: bool,
    read: impl FnOnce() -> T + Send + 'static,
    put: impl FnOnce(&Rc<Ui>, T, bool) + 'static,
) {
    let ui = Rc::clone(ui);
    wiring::off_thread(read, move |got| put(&ui, got, show));
}

/// Was this window asked for before its first read landed? Then it shows the
/// moment it is built, rather than the click being lost.
fn wanted(ui: &Rc<Ui>, name: &'static str, show: bool) -> bool {
    ui.want.borrow_mut().remove(name) || show
}

/// Read the catalog AND the library, then put up both — at startup, and after
/// an install, when both have changed. One read of the shelf serves both,
/// because the library takes its names and icons from it.
fn refresh_shelves(ui: &Rc<Ui>) {
    let (host, root) = (ui.host.clone(), ui.root.clone());
    load(
        ui,
        false,
        move || {
            let shelf = read_shelf(&host, &root);
            let library = read_library(&host, &root, &shelf.0);
            (shelf, library)
        },
        |ui, (shelf, library), _| {
            put_store(ui, shelf, false);
            put_games(ui, library, false);
        },
    );
}

/// Read the shelf and put up a Store window over whatever was there.
fn refresh_store(ui: &Rc<Ui>, show: bool) {
    let (host, root) = (ui.host.clone(), ui.root.clone());
    load(ui, show, move || read_shelf(&host, &root), put_store);
}

fn put_store(ui: &Rc<Ui>, (rows, reachable, why): (Vec<windows::Shelf>, bool, String), show: bool) {
    let show = wanted(ui, "Store", show);
    println!(
        "elo-gui: shelf {}",
        if reachable {
            format!("{} title(s)", rows.len())
        } else {
            format!("unreadable ({why}) — the Store will say so")
        }
    );
    let mut store_w = windows::store(&rows, reachable, &why);
    // Where the window it replaces was standing, before that one is dropped.
    place(
        &mut store_w.window,
        ui.store.borrow().as_ref().map(|s| (s.window.x(), s.window.y())),
        ui.drift,
    );
    *ui.shelf.borrow_mut() = rows;

    // Installing from the Store puts a game in the library, so the library is
    // re-read when it does. Without this the Store says *"it is in Games now"*
    // over a Games window that will not show it until the next launch.
    let after = {
        let ui = Rc::clone(ui);
        Rc::new(move || refresh_shelves(&ui)) as Rc<dyn Fn()>
    };
    wiring::wire_store(&store_w, Rc::clone(&ui.front), wiring::real_tell(),
                       wiring::real_then(), after);

    let mut refresh = store_w.refresh.clone();
    let ui_c = Rc::clone(ui);
    refresh.set_callback(move |b| {
        b.set_label("Reading…");
        b.deactivate();
        defer(&ui_c, refresh_store);
    });

    let mut window = store_w.window.clone();
    let previous = ui.store.replace(Some(store_w));
    if let Some(old) = previous {
        let mut w = old.window.clone();
        let was_up = w.shown();
        w.hide();
        if was_up {
            window.show();
        }
    }
    if show {
        window.show();
    }
}

/// Re-read what is installed and put up a Games window over whatever was there.
fn refresh_games(ui: &Rc<Ui>, show: bool) {
    let (host, root, shelf) = (ui.host.clone(), ui.root.clone(), ui.shelf.borrow().clone());
    load(ui, show, move || read_library(&host, &root, &shelf), put_games);
}

fn put_games(ui: &Rc<Ui>, rows: Vec<Row>, show: bool) {
    let show = wanted(ui, "Games", show);
    println!("elo-gui: {} install(s) under {}", rows.len(), ui.root.display());
    let mut games_w = windows::games(&rows);
    place(
        &mut games_w.window,
        ui.games.borrow().as_ref().map(|g| (g.window.x(), g.window.y())),
        ui.drift,
    );
    let after = {
        let ui = Rc::clone(ui);
        Rc::new(move || refresh_games(&ui, false)) as Rc<dyn Fn()>
    };
    wiring::wire_games(&games_w, Rc::clone(&ui.front), wiring::real_tell(),
                       wiring::real_then(), after);

    let mut refresh = games_w.refresh.clone();
    let ui_c = Rc::clone(ui);
    refresh.set_callback(move |b| {
        b.set_label("Reading…");
        b.deactivate();
        defer(&ui_c, refresh_games);
    });

    let mut window = games_w.window.clone();
    let previous = ui.games.replace(Some(games_w));
    if let Some(old) = previous {
        let mut w = old.window.clone();
        let was_up = w.shown();
        w.hide();
        if was_up {
            window.show();
        }
    }
    if show {
        window.show();
    }
}

/// Read the friends list and put up a Friends window over whatever was there.
///
/// The same shape as [`refresh_games`]: built off a read, placed where the
/// old one stood, and swapped in on a timer rather than inside a callback.
fn refresh_friends(ui: &Rc<Ui>, show: bool) {
    let wallet = known_address(&ui.signer).filter(|w| elo_net::friends::is_wallet(w));
    let host = ui.host.clone();
    load(ui, show, move || read_friends(&host, wallet.as_deref()), put_friends_read);
}

fn put_friends_read(ui: &Rc<Ui>, state: windows::Friends, show: bool) {
    println!(
        "elo-gui: friends {}",
        match &state {
            windows::Friends::Listed { view } => format!("{} listed", view.friends.len()),
            windows::Friends::Unreadable { why, .. } => {
                format!("unreadable ({why}) — the window will say so")
            }
            windows::Friends::NoWallet => "— no wallet on this machine".into(),
        }
    );
    put_friends(ui, &state, show, None);
}

/// Read Gates' shard list and put up a Servers window over whatever was there.
///
/// Read on open, like Friends: the window used to be built once at startup,
/// so its counts were an hour old by evening, a list the origin failed to
/// serve stayed failed all session, and a Join that said "not installed"
/// stayed off after the game was installed.
fn refresh_servers(ui: &Rc<Ui>, show: bool) {
    let host = ui.host.clone();
    load(ui, show, move || read_shards(&host, SERVERS_SLUG), put_servers);
}

fn put_servers(ui: &Rc<Ui>, shards: windows::Shards, show: bool) {
    // Only a list we actually READ names a url worth passing on. `Unreadable`
    // carries one too, and handing the game a url this launcher just failed to
    // read would ask it to make the same failed request again.
    let list_url = match &shards {
        windows::Shards::Listed { url, .. } => Some(url.clone()),
        _ => None,
    };
    let mut servers_w = windows::servers(SERVERS_SLUG, &shards);
    place(
        &mut servers_w.window,
        ui.servers.borrow().as_ref().map(|s| (s.window.x(), s.window.y())),
        ui.drift,
    );
    wire_servers(&servers_w, SERVERS_SLUG, &ui.host, &ui.root, list_url.as_deref());
    let mut window = servers_w.window.clone();
    if let Some(old) = ui.servers.replace(Some(servers_w)) {
        let mut w = old.window.clone();
        w.hide();
    }
    if show {
        window.show();
    }
}

/// Orders: the book on this machine, a form, the settings, and the runner.
///
/// The runner is a thread that ticks the book every 15 seconds with the shared
/// engine (`elo_net::trading::tick`) — the same one `elo trade orders run`
/// uses — signing with the local keystore through a masked passphrase
/// (`elo_vault::masked`). It stops when Stop is pressed or elo closes. What it
/// says comes back on a channel this window polls, never from its thread.
fn refresh_orders(ui: &Rc<Ui>, show: bool) {
    use elo_net::{guard, trading};
    let book = trading::load_orders(&trading::orders_path());
    let settings = guard::Settings::load(&guard::settings_path());
    let running = ui.orders_run.borrow().is_some();
    let has_account = elo_vault::LocalSigner::at(elo_vault::keystore_path()).address().is_some();
    let mut w = windows::orders(&book, &settings, running, has_account);
    place(&mut w.window, ui.orders.borrow().as_ref().map(|o| (o.window.x(), o.window.y())), ui.drift);
    // The form survives the rebuild. A running book rebuilds this window on
    // every line the runner says, and an order half typed into it was wiped
    // each time — the player watching their orders was the one who lost work.
    if let Some(old) = ui.orders.borrow().as_ref() {
        carry_order_form(old, &mut w);
    }

    {   // the field line follows the kind
        let mut hint = w.hint.clone();
        w.kind.clone().set_callback(move |k| {
            let i = k.value().max(0) as usize;
            hint.set_label(windows::ORDER_KINDS.get(i).map(|x| x.1).unwrap_or(""));
        });
    }
    {   // place
        let (kind, coin, side, amount) = (w.kind.clone(), w.coin.clone(), w.side.clone(), w.amount.clone());
        let (a, b, c, d, cap) = (w.a.clone(), w.b.clone(), w.c.clone(), w.d.clone(), w.cap.clone());
        let mut status = w.status.clone();
        let ui_c = Rc::clone(ui);
        w.place.clone().set_callback(move |_| {
            let k = windows::ORDER_KINDS.get(kind.value().max(0) as usize).map(|x| x.0).unwrap_or("limit");
            let path = trading::orders_path();
            let mut book = trading::load_orders(&path);
            let id = book.iter().map(|o| o.id).max().unwrap_or(0) + 1;
            let sd = if side.value() == 1 { "sell" } else { "buy" };
            match elo_net::orders::from_fields(id, k, &coin.value(), sd, &amount.value(), &a.value(), &b.value(),
                                               &c.value(), &d.value(), &cap.value()) {
                Ok(o) => {
                    book.push(o);
                    match trading::save_orders(&path, &book) {
                        Ok(()) => {
                            // Placed, so the form is done with: emptied here,
                            // the rebuild carries the empty form over.
                            for mut f in [amount.clone(), a.clone(), b.clone(), c.clone(), d.clone(),
                                          cap.clone()] {
                                f.set_value("");
                            }
                            defer(&ui_c, refresh_orders)
                        }
                        Err(e) => say(&mut status, &e, elo_ui::theme::GOLD),
                    }
                }
                Err(e) => say(&mut status, &e, elo_ui::theme::GOLD),
            }
        });
    }
    for (id, rm, resume) in &w.rows {
        let (id, ui_c) = (*id, Rc::clone(ui));
        rm.clone().set_callback(move |_| {
            let path = trading::orders_path();
            let mut book = trading::load_orders(&path);
            book.retain(|o| o.id != id);
            let _ = trading::save_orders(&path, &book);
            defer(&ui_c, refresh_orders);
        });
        if let Some(r) = resume {
            let ui_c = Rc::clone(ui);
            r.clone().set_callback(move |_| {
                let path = trading::orders_path();
                let mut book = trading::load_orders(&path);
                book.iter_mut().filter(|o| o.id == id).for_each(|o| o.paused = None);
                let _ = trading::save_orders(&path, &book);
                defer(&ui_c, refresh_orders);
            });
        }
    }
    for (name, cb) in [("approve_once", w.approve_once.clone()), ("check", w.check.clone()),
                       ("allow_outside", w.allow_outside.clone())] {
        cb.clone().set_callback(move |b| {
            let p = guard::settings_path();
            let mut s = guard::Settings::load(&p);
            if s.set(name, b.is_checked()).is_ok() {
                let _ = s.save(&p);
            }
        });
    }
    {   // start / stop
        let ui_c = Rc::clone(ui);
        let mut status = w.status.clone();
        w.run.clone().set_callback(move |_| {
            if let Some(flag) = ui_c.orders_run.borrow_mut().take() {
                flag.store(false, std::sync::atomic::Ordering::SeqCst);
                defer(&ui_c, refresh_orders);
                return;
            }
            let signer = elo_vault::LocalSigner::at(elo_vault::keystore_path());
            let Some(addr) = signer.address() else { return };
            let Ok(me) = elo_vault::parse_address(&addr) else { return };
            let ask = wiring::real_ask();
            let Some(pass) = ask("Start orders", "Your passphrase, held masked in memory while orders run:") else {
                return;
            };
            // the passphrase is checked now, not when an order fires at 3am
            let probe = elo_vault::tx::Tx { nonce: 0, max_priority_fee: 0, max_fee: 0, gas: 21000, to: [0u8; 20],
                                            value: 0, data: vec![] };
            if let Err(e) = signer.sign_transaction(&pass, &probe) {
                say(&mut status, &format!("that passphrase does not open this account: {e}"), elo_ui::theme::GOLD);
                return;
            }
            let Ok(masked) = elo_vault::masked::Masked::new(pass) else { return };
            let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            {
                let (flag, host) = (Arc::clone(&flag), ui_c.host.clone());
                std::thread::spawn(move || {
                    let rpc = elo_net::chain::HttpRpc::new();
                    let sign = |t: &elo_vault::tx::Tx| masked.with(|p| signer.sign_transaction(p, t));
                    let path = trading::orders_path();
                    while flag.load(std::sync::atomic::Ordering::SeqCst) {
                        let settings = guard::Settings::load(&guard::settings_path());
                        let tx2 = tx.clone();
                        for line in trading::tick(&rpc, &host, &path, &me, &settings, &guard::activity_path(), 100,
                                                  &sign, &move |l: &str| { let _ = tx2.send(l.trim().to_string()); }) {
                            let _ = tx.send(line);
                        }
                        for _ in 0..15 {
                            if !flag.load(std::sync::atomic::Ordering::SeqCst) { break; }
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                    }
                });
            }
            *ui_c.orders_run.borrow_mut() = Some(flag);
            *ui_c.orders_said.borrow_mut() = Some(rx);
            let poll_ui = Rc::clone(&ui_c);
            app::add_timeout3(1.0, move |h| {
                if poll_ui.orders_run.borrow().is_none() {
                    return;
                }
                let mut heard = Vec::new();
                if let Some(rx) = poll_ui.orders_said.borrow().as_ref() {
                    while let Ok(l) = rx.try_recv() { heard.push(l); }
                }
                if !heard.is_empty() {
                    println!("elo-gui: orders — {}", heard.join(" · "));
                    refresh_orders(&poll_ui, false);
                    if let Some(o) = poll_ui.orders.borrow().as_ref() {
                        let mut s = o.status.clone();
                        // one line that fits; the whole of it is in the activity log
                        let last = heard.last().map(String::as_str).unwrap_or("");
                        let short: String = if last.chars().count() > 96 {
                            last.chars().take(95).collect::<String>() + "…"
                        } else { last.to_string() };
                        say(&mut s, &short, elo_ui::theme::INK);
                    }
                }
                app::repeat_timeout3(1.0, h);
            });
            defer(&ui_c, refresh_orders);
        });
    }

    let mut window = w.window.clone();
    if let Some(old) = ui.orders.replace(Some(w)) {
        let was_up = old.window.shown();
        // Deleted, not hidden: this rebuilds on every line a running book
        // says, and a hidden window per line was a leak that grew all night.
        // Safe because nothing outside the window holds its widgets — the
        // runner's poll reads `ui.orders`, which is the new one by now.
        fltk::window::Window::delete(old.window);
        if was_up {
            window.show();
        }
    }
    if show {
        window.show();
    }
}

/// What the player had typed into the Orders form, put into its rebuild.
fn carry_order_form(old: &windows::OrdersWindow, new: &mut windows::OrdersWindow) {
    new.kind.set_value(old.kind.value());
    new.side.set_value(old.side.value());
    for (from, to) in [(&old.coin, &mut new.coin), (&old.amount, &mut new.amount), (&old.a, &mut new.a),
                       (&old.b, &mut new.b), (&old.c, &mut new.c), (&old.d, &mut new.d),
                       (&old.cap, &mut new.cap)] {
        to.set_value(&from.value());
    }
    let i = new.kind.value().max(0) as usize;
    new.hint.set_label(windows::ORDER_KINDS.get(i).map(|x| x.1).unwrap_or(""));
}

/// The origin's markets, as `(rows, reachable, why)`.
fn read_pairs(host: &str) -> (Vec<windows::PairLine>, bool, String) {
    let got = elo_net::Net::new().get_json(&format!("{host}/api/pairs"));
    let Some(v) = got.value else { return (Vec::new(), false, got.why) };
    let num = |x: Option<&serde_json::Value>| x.and_then(|v| v.as_f64());
    // An answer with no list in it is a read that failed, not "no markets".
    let Some(listed) = v.get("pairs").and_then(|p| p.as_array()) else {
        return (Vec::new(), false, "the origin's answer had no markets in it".into());
    };
    let rows = listed.iter().filter_map(|p| {
        let key = p.get("key")?.as_str()?.to_string();
        let base = p.get("base").and_then(|b| b.as_str()).unwrap_or("?");
        let quote = p.get("quote").and_then(|b| b.as_str()).unwrap_or("?");
        Some(windows::PairLine {
            label: format!("{base} / {quote}"),
            price: num(p.get("price")),
            change_24h: num(p.get("change_24h")),
            venue: p.get("venue").and_then(|b| b.as_str()).unwrap_or("").to_string(),
            key,
        })
    }).collect();
    (rows, true, String::new())
}

/// Read the markets and put up a Pairs window over whatever was there.
fn refresh_pairs(ui: &Rc<Ui>, show: bool) {
    let host = ui.host.clone();
    load(ui, show, move || read_pairs(&host), put_pairs);
}

fn put_pairs(ui: &Rc<Ui>, (rows, reachable, why): (Vec<windows::PairLine>, bool, String), show: bool) {
    let mut pairs_w = windows::pairs(&rows, reachable, &why);
    place(&mut pairs_w.window, ui.pairs.borrow().as_ref().map(|p| (p.window.x(), p.window.y())), ui.drift);
    for (key, b) in &pairs_w.rows {
        let (mut b, front) = (b.clone(), Rc::clone(&ui.front));
        let url = format!("{}/terminal.html?pair={key}#markets", ui.host);
        b.set_callback(move |_| {
            if let Err(e) = front.open(&url) {
                eprintln!("elo-gui: {e}");
            }
        });
    }
    let mut refresh = pairs_w.refresh.clone();
    let ui_c = Rc::clone(ui);
    refresh.set_callback(move |b| {
        b.set_label("Reading…");
        b.deactivate();
        defer(&ui_c, refresh_pairs);
    });
    let mut window = pairs_w.window.clone();
    if let Some(old) = ui.pairs.replace(Some(pairs_w)) {
        let was_up = old.window.shown();
        // Deleted, not hidden — nothing outside the window holds its widgets.
        fltk::window::Window::delete(old.window);
        if was_up {
            window.show();
        }
    }
    if show {
        window.show();
    }
}

/// What Skins draws: this account's items in the chosen collection and its
/// open listings, read now (`elo_net::skins`) — and the collection's address,
/// read ONCE here for every item's OpenSea link. It was read once per item,
/// on the window's thread, every time the window opened.
fn read_skins(host: &str, key: &str) -> (windows::Skins, Option<String>) {
    let Some(addr) = elo_vault::LocalSigner::at(elo_vault::keystore_path()).address() else {
        return (windows::Skins::NoAccount, None);
    };
    let who = addr.to_lowercase();
    let rpc = elo_net::chain::HttpRpc::new();
    let collection = elo_net::skins::collection(&rpc, host, key).ok()
        .and_then(|c| c.get("address").and_then(Value::as_str).map(str::to_string));
    let held = match elo_net::skins::held(&rpc, host, key, &who) {
        Ok(h) => h,
        Err(why) => return (windows::Skins::Unreadable { why }, collection),
    };
    let (listed, listings_why) = match elo_net::skins::listings(&rpc, host, key, &who) {
        Ok(l) => (l, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let rows = held.into_iter().map(|h| {
        let l = listed.iter().find(|l| l.id == h.id)
            .map(|l| (l.price_eth.map(|p| format!("{p}")).unwrap_or_else(|| "?".into()), l.order.clone()));
        windows::SkinLine { id: h.id, name: h.name, listed: l }
    }).collect();
    (windows::Skins::Held { rows, listings_why }, collection)
}

/// Skins: sell, relist, cancel, take the best offer, or open OpenSea, for each
/// item this account holds. The origin prepares; `elo_net::skins::check_sell`
/// holds a listing to what was typed; the review shows it; the passphrase is
/// asked for each one; the steps run on a thread (a receipt can take a while)
/// and come back on a channel this window polls.
fn refresh_skins(ui: &Rc<Ui>, show: bool) {
    let key = windows::SKIN_COLLECTIONS[ui.skins_key.get().min(windows::SKIN_COLLECTIONS.len() - 1)].0;
    let host = ui.host.clone();
    load(ui, show, move || read_skins(&host, key), move |ui, read, show| put_skins(ui, key, read, show));
}

fn put_skins(ui: &Rc<Ui>, key: &'static str, (state, collection): (windows::Skins, Option<String>),
             show: bool) {
    let mut w = windows::skins(&state, ui.skins_key.get());
    place(&mut w.window, ui.skins.borrow().as_ref().map(|o| (o.window.x(), o.window.y())), ui.drift);

    {
        let ui_c = Rc::clone(ui);
        w.collection.clone().set_callback(move |c| {
            ui_c.skins_key.set(c.value().max(0) as usize);
            defer(&ui_c, refresh_skins);
        });
        let ui_c = Rc::clone(ui);
        w.refresh.clone().set_callback(move |b| {
            b.set_label("Reading…");
            b.deactivate();
            defer(&ui_c, refresh_skins);
        });
        let (front, url) = (Rc::clone(&ui.front), format!("{}/terminal.html?collection={key}#col", ui.host));
        w.market.clone().set_callback(move |_| {
            if let Err(e) = front.open(&url) {
                eprintln!("elo-gui: {e}");
            }
        });
    }
    for windows::SkinRow { id, sell, cancel, offer, open } in &w.rows {
        {   // OpenSea
            let url = collection.as_ref().map(|a| format!("https://opensea.io/item/robinhood/{a}/{id}"));
            let front = Rc::clone(&ui.front);
            open.clone().set_callback(move |_| {
                if let Some(u) = &url {
                    if let Err(e) = front.open(u) {
                        eprintln!("elo-gui: {e}");
                    }
                }
            });
        }
        {   // Sell / Relist
            let (ui_c, id, price, days, mut status) = (Rc::clone(ui), id.clone(), w.price.clone(), w.days.clone(),
                                                       w.status.clone());
            sell.clone().set_callback(move |_| {
                let d = windows::LISTING_DAYS.get(days.value().max(0) as usize).copied().unwrap_or(7);
                match prepare_listing(&ui_c.host, key, &id, &price.value(), d) {
                    Ok(job) => start_skins_job(&ui_c, job, &mut status),
                    Err(e) => say(&mut status, &e, elo_ui::theme::GOLD),
                }
            });
        }
        if let Some((order, b)) = cancel {
            let (ui_c, id, order, mut status) = (Rc::clone(ui), id.clone(), order.clone(), w.status.clone());
            b.clone().set_callback(move |_| {
                match prepare_steps(&ui_c.host, key, "cancel", &id, &order) {
                    Ok(job) => start_skins_job(&ui_c, job, &mut status),
                    Err(e) => say(&mut status, &e, elo_ui::theme::GOLD),
                }
            });
        }
        {   // Best offer
            let (ui_c, id, mut status) = (Rc::clone(ui), id.clone(), w.status.clone());
            offer.clone().set_callback(move |_| {
                match prepare_steps(&ui_c.host, key, "accept", &id, "") {
                    Ok(job) => start_skins_job(&ui_c, job, &mut status),
                    Err(e) => say(&mut status, &e, elo_ui::theme::GOLD),
                }
            });
        }
    }

    let mut window = w.window.clone();
    if let Some(old) = ui.skins.replace(Some(w)) {
        let mut ow = old.window.clone();
        let was_up = ow.shown();
        ow.hide();
        if was_up {
            window.show();
        }
    }
    if show {
        window.show();
    }
}

/// A sale, cancel or offer, checked and reviewed, waiting for its passphrase.
struct SkinsJob {
    key: &'static str,
    prep: serde_json::Value,
    /// the listing the vault will sign, when this is a sale
    order: Option<elo_vault::seaport::Order>,
    heading: String,
    lines: Vec<(String, String)>,
    notes: Vec<String>,
    go: &'static str,
}

fn prepare_listing(host: &str, key: &'static str, id: &str, price: &str, days: u32) -> Result<SkinsJob, String> {
    let signer = elo_vault::LocalSigner::at(elo_vault::keystore_path());
    let addr = signer.address().ok_or("no account on this machine")?;
    let me = elo_vault::parse_address(&addr)?;
    let price_wei = elo_net::skins::wei(price)?;
    let rpc = elo_net::chain::HttpRpc::new();
    let col = elo_net::skins::collection(&rpc, host, key)?;
    let col_addr = elo_vault::parse_address(col.get("address").and_then(|v| v.as_str()).unwrap_or(""))
        .map_err(|_| "the origin names no address for that collection".to_string())?;
    let prep = elo_net::skins::prepare_sell(&rpc, host, key, id, price, days, &addr.to_lowercase())?;
    let order = elo_net::skins::check_sell(&prep, &col_addr, id, price_wei, &me)?;
    let r = |k: &str| prep.pointer(&format!("/review/{k}")).and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let mut lines = vec![("Sell".to_string(), r("give")), ("Price".to_string(), r("price"))];
    for f in prep.pointer("/review/fees").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        lines.push(("Fee".into(), format!("{} to {}", f.get("amount").and_then(|v| v.as_str()).unwrap_or("?"),
                                          f.get("to").and_then(|v| v.as_str()).unwrap_or("?"))));
    }
    lines.push(("You get".into(), r("you_get")));
    lines.push(("Ends".into(), r("ends")));
    let approvals = prep.get("txs").and_then(|t| t.as_array()).map(Vec::len).unwrap_or(0);
    let mut notes: Vec<String> = prep.pointer("/review/note").and_then(|v| v.as_str()).map(|n| vec![n.to_string()])
        .unwrap_or_default();
    notes.push("It stays in your wallet, and in game, until it sells. OpenSea takes its fee; we take none.".into());
    if approvals > 0 {
        notes.push("First, a one-time approval so OpenSea's Seaport can deliver it when it sells.".into());
    }
    Ok(SkinsJob { key, prep, order: Some(order), heading: "check this listing".into(), lines, notes, go: "List it" })
}

fn prepare_steps(host: &str, key: &'static str, verb: &str, id: &str, order: &str) -> Result<SkinsJob, String> {
    let addr = elo_vault::LocalSigner::at(elo_vault::keystore_path()).address().ok_or("no account on this machine")?;
    let who = addr.to_lowercase();
    let rpc = elo_net::chain::HttpRpc::new();
    if verb == "cancel" {
        let prep = elo_net::skins::prepare_cancel(&rpc, host, key, order, &who)?;
        return Ok(SkinsJob { key, prep, order: None, heading: "cancel this listing".into(),
                             lines: vec![("Cancel".into(), format!("the listing of #{id}"))],
                             notes: vec!["A transaction from your account; it costs a little gas.".into()],
                             go: "Cancel it" });
    }
    let prep = elo_net::skins::prepare_accept(&rpc, host, key, id, &who)?;
    let o = prep.get("offer").cloned().unwrap_or(serde_json::Value::Null);
    let amount = format!("{} {}", o.get("amount").and_then(|v| v.as_f64()).map(|x| format!("{x}")).unwrap_or_else(|| "?".into()),
                         o.get("currency").and_then(|v| v.as_str()).unwrap_or(""));
    Ok(SkinsJob { key, prep, order: None, heading: "sell to the best offer".into(),
                  lines: vec![("Sell".into(), format!("#{id}")), ("For".into(), amount)],
                  notes: vec!["The offer is OpenSea's best right now; the item leaves your wallet when it lands.".into()],
                  go: "Sell" })
}

/// Review, passphrase, then the steps on a thread; the answer comes back on a
/// channel this window polls.
fn start_skins_job(ui: &Rc<Ui>, job: SkinsJob, status: &mut fltk::frame::Frame) {
    if ui.skins_said.borrow().is_some() {
        say(status, "one at a time: the last one is still going", elo_ui::theme::GOLD);
        return;
    }
    if !wiring::review_in_window_as(&job.heading, &job.lines, &job.notes, "Your passphrase is asked next.", job.go) {
        return;
    }
    let ask = wiring::real_ask();
    let Some(pass) = ask("Skins", "Your passphrase:") else { return };
    let signer = elo_vault::LocalSigner::at(elo_vault::keystore_path());
    let Some(addr) = signer.address() else { return };
    let Ok(me) = elo_vault::parse_address(&addr) else { return };
    let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let host = ui.host.clone();
    std::thread::spawn(move || {
        let rpc = elo_net::chain::HttpRpc::new();
        let settings = elo_net::guard::Settings::load(&elo_net::guard::settings_path());
        let sign = |t: &elo_vault::tx::Tx| signer.sign_transaction(&pass, t);
        let has_steps = job.prep.get("txs").and_then(|t| t.as_array()).is_some_and(|t| !t.is_empty());
        let ran = if has_steps {
            elo_net::trading::execute(&job.prep, &rpc, &me, &settings, &elo_net::guard::activity_path(), "skins",
                                      &sign, &|l: &str| println!("elo-gui: skins — {}", l.trim())).map(Some)
        } else { Ok(None) };
        let out = ran.and_then(|hash| match &job.order {
            Some(order) => {
                let sig = signer.sign_seaport_listing(&pass, order)?;
                elo_net::guard::log(&elo_net::guard::activity_path(), serde_json::json!({
                    "source": "skins", "to": "Seaport 1.6 (OpenSea)", "verdict": "ok", "what": "a listing",
                    "order": job.prep.get("order_hash").cloned().unwrap_or_default()}));
                elo_net::skins::submit(&rpc, &host, job.key, &job.prep, &sig).map(|page| format!("listed: {page}"))
            }
            None => Ok(format!("done: {}", hash.map(|h| elo_net::chain::tx_url(&h)).unwrap_or_default())),
        });
        let _ = tx.send(out);
    });
    *ui.skins_said.borrow_mut() = Some(rx);
    say(status, "working — the chain answers in a few seconds", elo_ui::theme::INK);
    let poll_ui = Rc::clone(ui);
    app::add_timeout3(0.5, move |h| {
        let got = poll_ui.skins_said.borrow().as_ref().and_then(|rx| rx.try_recv().ok());
        let Some(got) = got else {
            app::repeat_timeout3(0.5, h);
            return;
        };
        *poll_ui.skins_said.borrow_mut() = None;
        refresh_skins(&poll_ui, false);
        if let Some(sw) = poll_ui.skins.borrow().as_ref() {
            let mut s = sw.status.clone();
            match got {
                Ok(line) => say(&mut s, &line, elo_ui::theme::INK),
                Err(e) => say(&mut s, &e, elo_ui::theme::GOLD),
            }
        }
    });
}

/// Put up a Friends window drawn from `state` over whatever was there, with
/// `said` on its status line. A change that landed comes back with the list
/// after it, and is drawn from that — no second read.
///
/// ⚠ Never from inside one of the old window's own callbacks: it drops that
/// window (see [`defer`]). Its callers are timers.
fn put_friends(ui: &Rc<Ui>, state: &windows::Friends, show: bool,
               said: Option<(&str, fltk::enums::Color)>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut friends_w = windows::friends(state, now);
    place(
        &mut friends_w.window,
        ui.friends.borrow().as_ref().map(|f| (f.window.x(), f.window.y())),
        ui.drift,
    );
    wire_friends(ui, &friends_w);
    if let Some((line, ink)) = said {
        say(&mut friends_w.status, line, ink);
    }
    if ui.changing.get() {
        // A change is still out: this window waits on it like the last one.
        hold_friend_acts(&friends_w, false);
    }

    let mut refresh = friends_w.refresh.clone();
    let ui_c = Rc::clone(ui);
    refresh.set_callback(move |b| {
        b.set_label("Reading…");
        b.deactivate();
        defer(&ui_c, refresh_friends);
    });

    let mut window = friends_w.window.clone();
    let previous = ui.friends.replace(Some(friends_w));
    if let Some(old) = previous {
        let mut w = old.window.clone();
        let was_up = w.shown();
        w.hide();
        if was_up {
            window.show();
        }
    }
    if show {
        window.show();
    }
}

/// One line on a Friends window's status — the origin's words or ours, drawn
/// as untrusted text either way.
fn say(status: &mut fltk::frame::Frame, line: &str, ink: fltk::enums::Color) {
    let safe = elo_ui::chrome::defuse(&elo_hive::render::safe_line(line));
    status.set_label(&safe);
    status.set_tooltip(&safe);
    status.set_label_color(ink);
    if let Some(mut w) = status.window() {
        w.redraw();
    }
}

/// Every change a Friends window offers, on or off together.
fn hold_friend_acts(w: &windows::FriendsWindow, on: bool) {
    let mut buttons: Vec<fltk::button::Button> = w.acts.iter().map(|a| a.button.clone()).collect();
    buttons.push(w.add.clone());
    for mut b in buttons {
        if on { b.activate() } else { b.deactivate() }
    }
}

/// A wallet's friends list, in the window's three states.
///
/// On the SHORT deadline: the player opened the window and is watching it, so
/// a dead origin costs four seconds and says so, never sixty.
fn read_friends(host: &str, wallet: Option<&str>) -> windows::Friends {
    let Some(wallet) = wallet else {
        return windows::Friends::NoWallet;
    };
    let net = elo_net::Net::new();
    let got = elo_net::friends::read_quick(&net, host, wallet);
    match got.value {
        Some(view) => windows::Friends::Listed { view },
        None => windows::Friends::Unreadable {
            url: elo_net::friends::url(host, wallet),
            why: got.why,
            reachable: got.reachable,
        },
    }
}

/// Give the Friends window's controls their behaviour.
///
/// **Join follows the friend's link the way the Servers window's Join does**:
/// the newest installed build, `{server}` from the link, `{servers}` from the
/// title's own manifest, one `launch::resolve`. It does not install — a player
/// without the game is told so on the button, and installing stays the
/// Store's verb.
///
/// **Every change is one signature** (`wiring::change_friend`), made off the
/// UI thread so a browser approval cannot freeze the launcher; the passphrase
/// prompt, which has to be a window on this thread, is carried back here.
fn wire_friends(ui: &Rc<Ui>, w: &windows::FriendsWindow) {
    for row in &w.rows {
        let link = row.link.clone();
        let root = ui.root.clone();
        let host = ui.host.clone();
        let mut join = row.join.clone();
        join.set_callback(move |b| {
            // Parsed again here, where it becomes an argv, rather than trusted
            // because the list parser accepted it once.
            let j = match elo_depot::deeplink::parse(&link) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("elo-gui: could not follow a friend's link: {e}");
                    b.set_label("Failed");
                    b.deactivate();
                    return;
                }
            };
            let Some(i) = elo_depot::newest_from(&root, &j.slug, &host) else {
                b.set_label("Not installed");
                b.deactivate();
                return;
            };
            let net = elo_net::Net::new();
            let mut values = elo_depot::launch::title_values(
                published_servers_url(&net, &host, &j.slug).as_deref(),
            );
            values.insert("server".to_string(), j.addr.clone());
            match elo_depot::launch::resolve(&i, &values, &std::collections::BTreeMap::new())
                .and_then(|l| elo_depot::launch::spawn(&l))
            {
                Ok(pid) => println!("elo-gui: joined {} on {} (pid {pid})", j.slug, j.addr),
                Err(e) => {
                    eprintln!("elo-gui: could not join {}: {e}", j.addr);
                    b.set_label("Failed");
                    b.deactivate();
                }
            }
        });

        let link = row.link.clone();
        let mut copy = row.copy.clone();
        copy.set_callback(move |b| {
            fltk::app::copy(&link);
            b.set_label("Copied");
        });
    }

    for act in &w.acts {
        let (ui, list) = (Rc::clone(ui), w.wallet.clone());
        let (op, wallet, called) = (act.op, act.wallet.clone(), act.called.clone());
        let mut b = act.button.clone();
        b.set_callback(move |_| start_friend_change(&ui, list.as_deref(), op, &wallet, &called));
    }
    let ask = {
        let (ui, list, input) = (Rc::clone(ui), w.wallet.clone(), w.add_input.clone());
        move || {
            let typed = input.value().trim().to_string();
            let called = if elo_net::friends::is_wallet(&typed) {
                elo_net::friends::short(&typed)
            } else {
                typed.clone()
            };
            start_friend_change(&ui, list.as_deref(), "ask", &typed, &called);
        }
    };
    let mut add = w.add.clone();
    let on_add = ask.clone();
    add.set_callback(move |_| on_add());
    // Enter in the box is the same act as pressing Ask.
    let mut input = w.add_input.clone();
    input.set_trigger(fltk::enums::CallbackTrigger::EnterKeyAlways);
    input.set_callback(move |_| ask());
}

/// What the thread making a change sends back: a prompt to put up here, or
/// what the change came to.
enum FromChange {
    Ask(String, String, std::sync::mpsc::Sender<Option<wiring::Secret>>),
    Done(Result<wiring::Changed, String>),
}

/// Start one friends change: `op` about `who`, as the wallet the window lists.
fn start_friend_change(ui: &Rc<Ui>, list: Option<&str>, op: &'static str, who: &str,
                       called: &str) {
    use elo_net::friends::short;
    let status = |line: &str, ink| {
        if let Some(w) = ui.friends.borrow().as_ref() {
            say(&mut w.status.clone(), line, ink);
        }
    };
    if ui.changing.get() {
        status("one change at a time — the last one is still waiting", elo_ui::theme::GOLD);
        return;
    }
    let mut via = wiring::FriendSigner::pick(&ui.signer, paired());
    // The window is one wallet's list. A change is made as the wallet that
    // signs, so a list read for another — a pairing that lapsed since, an
    // account unlocked since — is refused here rather than signed past.
    if let (Some(list), Some(me)) = (list, via.address()) {
        if list != me {
            status(&format!("this list is {}'s and this machine now signs as {} — \
                             press Refresh first", short(list), short(&me)),
                   elo_ui::theme::GOLD);
            return;
        }
    }
    ui.changing.set(true);
    if let Some(w) = ui.friends.borrow().as_ref() {
        hold_friend_acts(w, false);
    }
    status(match &via {
        wiring::FriendSigner::Paired(_) => "preparing — then approve it in your browser",
        _ => "preparing…",
    }, elo_ui::theme::MUTED);

    let (tx, rx) = std::sync::mpsc::channel::<FromChange>();
    {
        let (host, who) = (ui.host.clone(), who.to_string());
        std::thread::spawn(move || {
            let prompt = tx.clone();
            let ask = move |heading: &str, text: &str| -> Option<wiring::Secret> {
                let (back, answer) = std::sync::mpsc::channel();
                prompt.send(FromChange::Ask(heading.into(), text.into(), back)).ok()?;
                answer.recv().ok().flatten()
            };
            let got = wiring::change_friend(&mut via, &host, op, &who, &ask,
                                            &elo_ui::friendsop::RealFriendsHttp);
            let _ = tx.send(FromChange::Done(got));
        });
    }
    poll_friend_change(Rc::clone(ui), Rc::new(rx), op, who.to_string(), called.to_string());
}

/// Wait on a change from the UI thread: put up its prompt when it asks, and
/// draw what it came to on whichever Friends window is up by then.
fn poll_friend_change(ui: Rc<Ui>, rx: Rc<std::sync::mpsc::Receiver<FromChange>>,
                      op: &'static str, who: String, called: String) {
    app::add_timeout3(0.2, move |_| {
        let got = match rx.try_recv() {
            Ok(FromChange::Ask(heading, text, back)) => {
                let _ = back.send(wiring::ask_in_window(&heading, &text));
                None
            }
            Ok(FromChange::Done(got)) => Some(got),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("the change stopped without an answer — Refresh shows where the \
                          list stands".into()))
            }
        };
        let Some(got) = got else {
            poll_friend_change(Rc::clone(&ui), Rc::clone(&rx), op, who.clone(), called.clone());
            return;
        };
        ui.changing.set(false);
        let tell = |line: &str, ink| {
            if let Some(w) = ui.friends.borrow().as_ref() {
                hold_friend_acts(w, true);
                say(&mut w.status.clone(), line, ink);
            }
        };
        match got {
            Ok(wiring::Changed::Done(view)) => {
                let line = elo_ui::friendsop::done_line(op, &called, &view, &who);
                println!("elo-gui: friends — {line}");
                put_friends(&ui, &windows::Friends::Listed { view }, false,
                            Some((&line, elo_ui::theme::INK)));
            }
            Ok(wiring::Changed::HandedOff(url)) => match ui.front.open(&url) {
                Ok(()) => tell(&format!("no signer on this machine — finish it in your \
                                         browser at {url}"), elo_ui::theme::MUTED),
                Err(e) => tell(&format!("{e} — finish it at {url}"), elo_ui::theme::GOLD),
            },
            Err(e) => tell(&e, elo_ui::theme::GOLD),
        }
    });
}

/// The real storefront — what the Store's and Games' buttons actually do.
///
/// **Every verb here is the CLI's verb**, reached through the same
/// `elo-depot` calls `elo install`, `elo verify` and `elo play` make. A
/// second install path in the window layer would be a second place for the
/// hash rules, the staging directory and the argv substitution to be got
/// wrong, and only one of the two would have tests.
struct Front {
    host: String,
    root: std::path::PathBuf,
    /// The games Play started, so their rows can say Running and nothing
    /// installs under them. Reaped as they exit.
    started: std::cell::RefCell<Vec<(String, std::process::Child)>>,
    /// Titles an install is running for — see [`Installing`].
    installing: Installing,
}

impl Front {
    /// Is any game Play started still running? Reaps the ones that exited.
    ///
    /// Asked from the relock's timer, so it must never panic on a borrow held
    /// elsewhere. If it cannot look it answers yes: the cost of that is one
    /// relock put off by thirty seconds, and the cost of no is the gate up
    /// over a game.
    fn any_running(&self) -> bool {
        let Ok(mut started) = self.started.try_borrow_mut() else { return true };
        started.retain_mut(|(_, child)| matches!(child.try_wait(), Ok(None)));
        !started.is_empty()
    }
}

impl wiring::Storefront for Front {
    /// Fetch, hash-verify and place a build — as [`Storefront::install_job`]
    /// runs it, with the window's progress ignored. Kept for the trait; the
    /// windows run the job.
    fn install(&self, slug: &str) -> Result<String, String> {
        self.install_job(slug)(&|_, _| {})
    }

    /// The install, for another thread. What only this one can answer is
    /// asked here: a running game is not updated under itself, and a title
    /// already installing is not installed twice at once — the Store's Install
    /// and the library's Update are two doors to one directory.
    fn install_job(&self, slug: &str) -> wiring::Job {
        // An update replaces the files the game is running from and then
        // removes the old build, so it waits until the game has exited.
        if self.running(slug) {
            let said = format!("{slug} is running. Quit it first, then update.");
            return Box::new(move |_| Err(said));
        }
        let Some(claim) = Installing::claim(&self.installing, slug) else {
            let said = format!("{slug} is already installing — it will be ready in a moment.");
            return Box::new(move |_| Err(said));
        };
        let (host, root, slug) = (self.host.clone(), self.root.clone(), slug.to_string());
        Box::new(move |tick| {
            let _claim = claim;
            install_title(&host, &root, &slug, tick)
        })
    }

    fn verify(&self, slug: &str, build: &str) -> Result<String, String> {
        verify_title(&self.root, slug, build)
    }

    /// Hashing is the whole of a verify, and it is CPU: a few hundred
    /// megabytes is a second or two the window should not spend frozen.
    fn verify_job(&self, slug: &str, build: &str) -> wiring::Job {
        let (root, slug, build) = (self.root.clone(), slug.to_string(), build.to_string());
        Box::new(move |_| verify_title(&root, &slug, &build))
    }

    fn running(&self, slug: &str) -> bool {
        let mut started = self.started.borrow_mut();
        started.retain_mut(|(_, child)| matches!(child.try_wait(), Ok(None)));
        started.iter().any(|(s, _)| s == slug)
    }

    fn play(&self, slug: &str) -> Result<String, String> {
        if self.running(slug) {
            return Err(format!("{slug} is already running."));
        }
        if self.installing.has(slug) {
            return Err(format!("{slug} is still installing — it will be ready in a moment."));
        }
        // The most recently INSTALLED build, by directory mtime — see
        // `newest_of`. A build id ends in a hex sha, so name order is noise.
        // Only a build this host installed (`newest_from`): every manifest
        // this window reads is the host's, and a `gates` installed from
        // another url is not its Gates.
        let Some(i) = elo_depot::newest_from(&self.root, slug, &self.host) else {
            return Err(format!("{slug} is not installed — install it from the Store."));
        };
        // `{servers}` is the TITLE's and comes from its manifest, exactly as
        // `elo play` fills it — the same document the Servers window reads, so
        // a game started from either door draws the same shard browser.
        //
        // ⚠ Both maps were empty here until 2026-08-14, and an empty map is not
        // the same as no placeholder: `launch::fill` writes the empty string for
        // a known name it has no value for, so a depot asking for `{servers}`
        // got `--servers ""` and its in-game browser fell back to loopback. The
        // Servers window one screen away was passing the real url the whole
        // time, which is what made this look like a Gates bug.
        //
        // `server` and `wallet` stay absent BY DESIGN and that is not the same
        // omission. They are the player's, and on Play the player named neither
        // — Play is "start the game", not "join this shard". The wallet reaches
        // the game over the broker door, where it is a signature rather than an
        // address on a command line that `ps` can read.
        let net = elo_net::Net::new();
        let values = elo_depot::launch::title_values(
            published_servers_url(&net, &self.host, slug).as_deref(),
        );
        let launch = elo_depot::launch::resolve(&i, &values, &std::collections::BTreeMap::new())
            .map_err(|e| e.to_string())?;
        let child = elo_depot::launch::spawn_supervised(&launch).map_err(|e| e.to_string())?;
        let pid = child.id();
        self.started.borrow_mut().push((slug.to_string(), child));
        Ok(format!("{slug} started (pid {pid})."))
    }

    fn open(&self, url: &str) -> Result<(), String> {
        elo_depot::launch::open_page(url).map_err(|e| e.to_string())
    }

    fn page_url(&self, slug: &str) -> String {
        format!("{}/game.html?slug={slug}", self.host)
    }
}

/// The store shelf, as `(rows, reachable, why)`.
///
/// Reads the same route the CLI's `elo games` reads, so the two halves of the
/// client cannot disagree about what is listed — and then asks two more
/// questions per title that the manifest cannot answer on its own: **what does
/// a copy cost** (`/api/ticket/{slug}`) and **what does this title's art look
/// like** (`art.icon`, which the origin derives from the shelf row).
///
/// ⚠ **Every "we could not look" survives to the window.** A title whose price
/// route failed becomes `Price::Unknown` and not `Price::Free`: reading a
/// dropped packet as "free" is how a store gives a game away, and it is the
/// repo's own trap wearing a price tag.
/// The titles an install is running for right now, so a second door to the
/// same directory is refused instead of racing the first. Held by the job, on
/// whatever thread runs it, and let go when the job ends however it ends.
#[derive(Clone, Default)]
struct Installing(Arc<Mutex<std::collections::BTreeSet<String>>>);

impl Installing {
    fn claim(this: &Installing, slug: &str) -> Option<Claim> {
        let mut busy = this.0.lock().ok()?;
        busy.insert(slug.to_string()).then(|| Claim(this.clone(), slug.to_string()))
    }
    fn has(&self, slug: &str) -> bool {
        self.0.lock().map(|b| b.contains(slug)).unwrap_or(true)
    }
}

struct Claim(Installing, String);

impl Drop for Claim {
    fn drop(&mut self) {
        if let Ok(mut busy) = (self.0).0.lock() {
            busy.remove(&self.1);
        }
    }
}

/// Fetch, hash-verify and place a title's build — every verb here is the
/// CLI's, reached through the same `elo-depot` calls `elo install` makes.
/// Runs on a job's thread; `tick` hears `(done, total)` bytes.
fn install_title(host: &str, root: &std::path::Path, slug: &str, tick: &dyn Fn(i64, i64))
    -> Result<String, String> {
    let net = elo_net::Net::new();
    let url = format!("{host}/api/launcher/manifest/{slug}");
    let got = net.get_json(&url);
    let Some(v) = got.value else {
        return Err(format!(
            "could not read {slug}'s manifest — {}\n\n{url}",
            got.why
        ));
    };
    // ⚠ **`rebase` or the depot url is unusable.** The origin serves a
    // manifest whose `depot` is a PATH — `/api/launcher/depot/gates/…` —
    // and the client completes it against the host it asked. Without this
    // the check fails with *"bad uri"* and the row says the origin could
    // not be reached, which is a sentence about the network for a bug in
    // this file. `load_manifest` in the CLI does the same thing and this
    // is the same call.
    let mut m = elo_depot::parse_manifest(&v).map_err(|e| e.to_string())?;
    m.rebase(host);
    let checked = elo_depot::check(&m, &elo_depot::platform_tag(), root, &net, false);
    match &checked.state {
        elo_depot::UpdateState::Unpublished => {
            return Err(format!(
                "{} publishes no build for {} yet.\n\n\
                 This is a normal state, not a fault — the row says so and the \
                 button stays off until a depot is published.",
                m.name,
                elo_depot::platform_tag()
            ))
        }
        // A 401 IS the ticket door and not a fault: the title is sold, and
        // the download wants the grant a copy buys. The CLI says the same
        // sentence for the same case (`main.rs`, the install arm).
        elo_depot::UpdateState::Unknown { why } if why.contains("401") => {
            return Err(format!(
                "This game is sold — the download needs the grant your copy buys.\n\n\
                 In a terminal:  elo entitle {slug}\n\
                 then press Install again.\n\n\
                 No copy yet? The price and the contract:\n{}/api/ticket/{slug}",
                host
            ))
        }
        elo_depot::UpdateState::Unknown { why } => {
            return Err(format!("refusing to install — {why}"))
        }
        // `check` hands back the depot for a current build too, and this
        // used to download all of it again into the build being played.
        elo_depot::UpdateState::Current { build, .. } => {
            return Ok(format!("{} is already up to date (build {build}).", m.name))
        }
        _ => {}
    }
    let Some(depot) = checked.depot else {
        return Ok(format!("{} is already up to date.", m.name));
    };
    let plan = elo_depot::plan(&depot, root);
    let mut told = |done: i64, total: i64, _: &str, _: bool| tick(done, total);
    let at = elo_depot::install(&depot, root, &net, Some(&plan), Some(&mut told))
        .map_err(|e| e.to_string())?;
    // The digest goes in the message. It is the one number a player can
    // take to a block explorer, and an install that computes it and keeps
    // it quiet has wasted the only property no other storefront has.
    let digest = depot.digest().map_err(|e| e.to_string())?;
    // **Sweep the builds this one replaces.** Keeping the old build until
    // the new one is whole is the safe half of an update; keeping it
    // FOREVER is how a player ends up with six directories of one game, a
    // library listing each as its own row, and a Play button choosing
    // between them. Steam does not show you last week's build.
    //
    // A failure here is not the install's failure: the new build is
    // complete and playable, and a directory we could not delete costs
    // disk rather than correctness. So it is reported in the message and
    // never raised.
    let mut swept = 0usize;
    let mut kept_back = Vec::new();
    // By the depot's slug, which `check` has bound to the manifest's: the
    // title that was installed is the one whose old builds go.
    for old in elo_depot::superseded(root, &depot.slug, &depot.build) {
        match elo_depot::uninstall(&old.path) {
            Ok(()) => swept += 1,
            Err(e) => kept_back.push(format!("{} ({e})", old.build)),
        }
    }
    // Same notice the CLI prints, for the same reason: the bytes came from
    // a host this build's own document does not name, and that is a thing
    // to say out loud rather than a convenience to hide. The depot was
    // packaged before a domain move; see `Depot::heal_retired_root`.
    let healed = match &depot.healed_from {
        Some(was) => format!(
            "\n\nnote: this build names its files on {}, which is retired. They \
             were downloaded from {} instead, and every file was still checked \
             against the sha256 in the depot this origin served.",
            elo_depot::origin_of(was),
            elo_depot::origin_of(&depot.root)
        ),
        None => String::new(),
    };
    let sweep = match (swept, kept_back.is_empty()) {
        (0, true) => String::new(),
        (n, true) => format!("\n\n{n} older build(s) removed."),
        (n, false) => format!(
            "\n\n{n} older build(s) removed; could not remove {}",
            kept_back.join(", ")
        ),
    };
    Ok(format!(
        "{} {} is installed.\n\n{}\n\ndepot digest\n{digest}{sweep}{healed}\n\n\
         It is in Games, and Play now starts it.",
        m.name,
        depot.build,
        at.display()
    ))
}

fn verify_title(root: &std::path::Path, slug: &str, build: &str) -> Result<String, String> {
    let Some(i) = elo_depot::installed(root)
        .into_iter()
        .find(|i| i.slug == slug && i.build == build)
    else {
        return Err(format!("{slug} {build} is not installed any more."));
    };
    let v = elo_depot::verify(&i.path);
    if v.ok {
        Ok(format!(
            "{slug} {build} verified.\n\n{}\n\n\
             depot digest\n{}\n\n\
             That digest is the number to look up on chain — it is what the \n\
             notary event names.",
            v.line(),
            i.depot_digest
        ))
    } else {
        // A failed hash is the one thing in this client that means the
        // bytes on disk are not the bytes that were published.
        let named: Vec<String> = v
            .missing
            .iter()
            .chain(v.changed.iter())
            .take(8)
            .cloned()
            .collect();
        Err(format!(
            "{slug} {build} does NOT match its depot.\n\n{}\n\n{}\n\n\
             Re-installing replaces every file the depot names.",
            v.line(),
            named.join("\n")
        ))
    }
}

/// `f` over every item, up to eight at a time, answers in the items' order.
///
/// The shelf's per-title reads (a price, an icon) and the library's per-title
/// checks are independent round trips; one after another they were most of the
/// time a shelf took to load, and the count grows with the catalog.
fn par_map<I: Sync, O: Send>(items: &[I], f: impl Fn(&I) -> O + Sync) -> Vec<O> {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    let next = AtomicUsize::new(0);
    let mut out: Vec<(usize, O)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..items.len().min(8))
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Relaxed);
                        let Some(item) = items.get(i) else { break mine };
                        mine.push((i, f(item)));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    });
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, o)| o).collect()
}

fn read_shelf(host: &str, root: &std::path::Path) -> (Vec<windows::Shelf>, bool, String) {
    let net = elo_net::Net::new();
    let got = net.get_json(&format!("{host}/api/launcher/manifests"));
    let Some(v) = got.value else {
        // Not an empty shelf. `reachable` false is the whole point of this
        // return type — see the Store window's two branches.
        return (Vec::new(), false, got.why);
    };
    // An answer with no list in it is a read that failed, not an empty shelf.
    let Some(manifests) = v.get("manifests").and_then(|m| m.as_array()).cloned() else {
        return (Vec::new(), false, "the origin's answer had no catalog in it".into());
    };
    let here = elo_depot::platform_tag();
    let on_disk = elo_depot::installed(root);
    // Each title's price and icon, all titles at once.
    let mut extras = par_map(&manifests, |m| {
        m.get("slug")
            .and_then(Value::as_str)
            .map(|slug| (read_price(&net, host, slug), read_icon(&net, host, m)))
    })
    .into_iter();

    let mut rows = Vec::new();
    for m in manifests.iter() {
        let extra = extras.next().flatten();
        let (Some(slug), Some((price, icon))) = (m.get("slug").and_then(Value::as_str), extra) else {
            continue;
        };
        let name = m.get("name").and_then(Value::as_str).unwrap_or(slug);
        let blurb = m
            .get("blurb")
            .and_then(Value::as_str)
            .unwrap_or("no description");
        // Installable means a build for THIS machine whose depot is actually
        // published — not merely a row in the manifest. A title can be listed,
        // live, and have nothing this platform can run, and the shelf says so
        // rather than offering an Install that would 404.
        let installable = m
            .get("builds")
            .and_then(Value::as_array)
            .map(|bs| {
                bs.iter().any(|b| {
                    b.get("platform").and_then(Value::as_str) == Some(here.as_str())
                        && b.get("depot_state").and_then(Value::as_str) == Some("published")
                })
            })
            .unwrap_or(false);
        rows.push(windows::Shelf {
            slug: slug.to_string(),
            name: name.to_string(),
            blurb: blurb.to_string(),
            state: m
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            installable,
            installed: on_disk.iter().any(|i| i.slug == slug),
            price,
            icon,
        });
    }
    (rows, true, String::new())
}

/// The library — what is installed, measured against what is published.
///
/// Two things this adds that `Row::from_install` cannot, because an install
/// receipt knows neither:
///
/// * **the title's name and its icon**, taken from the shelf that was just
///   read. A library that says `gates` and draws a lettered box next to a
///   store that says **Gates** over the real capsule is one client disagreeing
///   with itself about the same game.
/// * **whether the build is current.** Nothing asked the origin at startup, so
///   every row said *"not checked — the origin was not reached"* — a reason
///   that was never measured and was usually false — and the **Update** button
///   `Row::action` exists for could not appear at all.
///
/// One `check` per SLUG, not per row: what is published is a property of the
/// title, and each row then holds its own build against it.
///
/// ⚠ **The comparison is on the DIGEST and never on the build name**, and that
/// is `elo-depot`'s rule rather than a preference here: *"a title is current
/// if ANY installed build carries the published digest"* (`update.rs`). Two
/// builds with the same digest are the same bytes whatever they are called, so
/// comparing names would offer **Update** over a build already identical to
/// the published one — and, the other way, call a genuinely stale build
/// current on a name collision.
fn read_library(host: &str, root: &std::path::Path, shelf: &[windows::Shelf]) -> Vec<Row> {
    let net = elo_net::Net::new();
    let installs = elo_depot::installed(root);
    // slug → the published depot digest, or why we could not name one.
    let mut published: std::collections::BTreeMap<String, Result<String, String>> =
        std::collections::BTreeMap::new();

    let mut slugs: Vec<&str> = installs.iter().map(|i| i.slug.as_str()).collect();
    slugs.sort_unstable();
    slugs.dedup();
    let answers = par_map(&slugs, |slug| {
        let url = format!("{host}/api/launcher/manifest/{slug}");
        let got = net.get_json(&url);
        let answer = match got.value.as_ref().map(elo_depot::parse_manifest) {
            Some(Ok(mut m)) => {
                // The depot url is a path on the origin; complete it against
                // the host we asked. See `Front::install`.
                m.rebase(host);
                let checked =
                    elo_depot::check(&m, &elo_depot::platform_tag(), root, &net, false);
                match checked.state {
                    elo_depot::UpdateState::Current { digest, .. } => Ok(digest),
                    elo_depot::UpdateState::Stale { to_digest, .. } => Ok(to_digest),
                    elo_depot::UpdateState::NotInstalled { .. } => {
                        Err("the origin publishes no build for this platform".into())
                    }
                    elo_depot::UpdateState::Unpublished => {
                        Err("no desktop build is published yet".into())
                    }
                    elo_depot::UpdateState::Unknown { why } => Err(why),
                }
            }
            Some(Err(e)) => Err(e.to_string()),
            None if got.reachable => Err(got.why.clone()),
            None => Err(format!("could not reach the origin — {}", got.why)),
        };
        answer
    });
    for (slug, answer) in slugs.iter().zip(answers) {
        published.insert(slug.to_string(), answer);
    }

    // **One row per TITLE, not per build.** The updater keeps the previous
    // build until the new one is whole, so a title a player has updated twice
    // has three directories on disk — and this listed each of them as its own
    // game. A player on 2026-08-15 saw Gates three times, two of them offering
    // Update, and updating one did not remove the others: there was nothing to
    // remove, they were all real installs of the same title. A library is a
    // list of what you own, and you own the title once.
    let mut seen = std::collections::BTreeSet::new();
    installs
        .iter()
        .filter(|i| {
            // `installs` is name-ordered, so filtering to "the first of each
            // slug" would keep an arbitrary build. Keep the one `newest_from`
            // would run, so the row and the Play button agree.
            elo_depot::newest_from(root, &i.slug, host)
                .map(|n| n.build == i.build)
                .unwrap_or(false)
                && seen.insert(i.slug.clone())
        })
        .map(|i| {
            let mut row = Row::from_install(i);
            if let Some(t) = shelf.iter().find(|t| t.slug == i.slug) {
                row.name = Some(t.name.clone());
                row.icon = t.icon.clone();
            }
            match published.get(&i.slug) {
                Some(Ok(digest)) => row.stale = Some(*digest != i.depot_digest),
                Some(Err(why)) => row.why = Some(why.clone()),
                None => {}
            }
            row
        })
        .collect()
}

/// What a copy of one title costs, off `/api/ticket/{slug}`.
///
/// **This client does no arithmetic on money and that is why the line says
/// dollars.** The origin publishes `price_usd` as a number it computed; the
/// per-rail amounts are wei strings, and turning one into "0.0025 ETH" here
/// would be a second implementation of a figure a buyer acts on, in the one
/// place nothing checks it. The exact amount is on the page where the wallet
/// signs — the launcher names the rail and the dollar price, and stops.
fn read_price(net: &elo_net::Net, host: &str, slug: &str) -> windows::Price {
    let got = net.get_json(&format!("{host}/api/ticket/{slug}"));
    let Some(v) = got.value else {
        return windows::Price::Unknown {
            why: if got.reachable {
                got.why
            } else {
                format!("could not reach the origin — {}", got.why)
            },
        };
    };
    // A title with no ticket contract is FREE, and that is a real answer the
    // origin states in words rather than an absence we inferred.
    if v.get("ticketed").and_then(Value::as_bool) != Some(true) {
        return windows::Price::Free;
    }
    // Ticketed but the chain read failed: the contract exists and we do not
    // know what it is charging. Never "free", never a number.
    if v.get("reachable").and_then(Value::as_bool) == Some(false) {
        return windows::Price::Unknown {
            why: "the contract did not answer".into(),
        };
    }
    let open: Vec<&str> = ["eth", "elo", "usdg"]
        .into_iter()
        .filter(|r| {
            v.get("rails")
                .and_then(|rs| rs.get(r))
                .and_then(|x| x.get("open"))
                .and_then(Value::as_bool)
                == Some(true)
        })
        .collect();
    if open.is_empty() {
        // A contract with every rail posted at 0. The sale is shut, and that
        // closes buying without closing resale — so it is not "free" either.
        return windows::Price::Posted {
            line: "nothing — the sale is not open yet".into(),
        };
    }
    let rails = open.join(" or ").to_uppercase();
    match v.get("price_usd").and_then(Value::as_f64) {
        Some(usd) => windows::Price::Posted {
            line: format!("${usd:.2}, paid in {rails}"),
        },
        // A dollar figure the origin left null is one nothing may invent.
        None => windows::Price::Posted {
            line: format!("an amount posted in {rails} — the page has the figure"),
        },
    }
}

/// A title's icon, if the origin published one and it decoded.
///
/// Every failure here is a placeholder and none of them is reported to the
/// player: art is decoration on a row whose words are already right
/// (`elo_ui::art`).
fn read_icon(net: &elo_net::Net, host: &str, manifest: &Value) -> Option<art::Icon> {
    let url = manifest.get("art")?.get("icon")?.as_str()?;
    // The origin serves these as site-relative paths. Anything absolute is
    // taken as written, so a title hosting its own art still works.
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("{host}/{}", url.trim_start_matches('/'))
    };
    art::Icon::decode(net.art(&url).value?)
}

/// The door's own answers, tested where they are written.
///
/// A `[[bin]]` cannot be reached by an integration test, so `examples/click.rs`
/// drives a *copy* of this `Host` to prove the threading and the signature.
/// That copy is only worth something if the real one refuses the same way, and
/// these are what hold it to that — the two refusals below are the whole
/// difference between a launcher that says why it cannot sign and one that
/// looks broken.
#[cfg(test)]
mod tests {
    use super::*;
    use elo_ui::consent::Doorbell;

    /// **No core dump of the launcher, and no same-user ptrace into it.** An
    /// unlocked account is a key in this process's memory; a crash's core
    /// file is a copy of that memory left on disk.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_launcher_leaves_no_core_dump() {
        unsafe extern "C" {
            fn prctl(option: core::ffi::c_int, ...) -> core::ffi::c_int;
        }
        const PR_GET_DUMPABLE: core::ffi::c_int = 3;
        assert!(elo_vault::harden_process().is_empty());
        // SAFETY: a read of this process's own flag; no pointer is passed.
        assert_eq!(unsafe { prctl(PR_GET_DUMPABLE) }, 0);
    }

    /// **No launch site in this file may pass a bare empty values map.**
    ///
    /// This is a source scan and it is deliberate, because the defect it pins
    /// cannot be reached any other way: both launch sites compile, both run,
    /// and the broken one produced `--servers ""` — a well-formed argv that a
    /// game correctly reads as "no shard list". Nothing typed, nothing panicked
    /// and nothing logged. The Servers window one screen away was passing the
    /// real url the whole time, which is what made it look like a Gates bug for
    /// as long as it did.
    ///
    /// `launch::title_values` is the only thing that may build the map, so the
    /// rule about where `{servers}` comes from lives in one place for all three
    /// doors. If a third launch site appears in this file, it fails here until
    /// it uses that function.
    #[test]
    fn no_launch_site_passes_an_empty_values_map() {
        // Production code only — this module names `launch::resolve` in its own
        // prose and would otherwise match itself, which is a scan that fails for
        // being written rather than for what it found.
        let whole = include_str!("elo-gui.rs");
        let src = whole.split("#[cfg(test)]").next().expect("the file has a body");
        let lines: Vec<&str> = src.lines().collect();
        let mut sites = 0;
        for (n, line) in lines.iter().enumerate() {
            if !line.contains("launch::resolve(") {
                continue;
            }
            sites += 1;
            // The values argument is the one after `&i`, inline or on the next
            // lines, so read a small window rather than one line.
            let window = lines[n..lines.len().min(n + 4)].join(" ");
            assert!(
                window.contains("title_values") || window.contains("&values"),
                "elo-gui.rs:{}: this launch site builds its values inline. \
                 Use `elo_depot::launch::title_values` so `{{servers}}` carries \
                 the title's shard list — an empty map renders it as \"\", which \
                 a game reads as \"no shards\" and nothing reports.",
                n + 1
            );
        }
        assert!(sites >= 2, "the scan found no launch sites — it has stopped checking anything");
    }

    /// **The heal notice claims only what was checked.** The window compares
    /// each file to the sha256 in the depot the origin served and never reads
    /// a notary — and the builds that heal were packaged before any digest was
    /// sealed — so telling a player their files matched "the notarized depot"
    /// was a reassurance a substituted depot got for free. Same law as the
    /// CLI's (`tests/origin.rs`), held here by a scan because the notice is
    /// built inside the install thread.
    #[test]
    fn the_heal_notice_does_not_claim_a_notary() {
        let whole = include_str!("elo-gui.rs");
        let src = whole.split("#[cfg(test)]").next().expect("the file has a body");
        let at = src.find("which is retired. They").expect("the heal notice is still here");
        let notice = &src[at..src.len().min(at + 400)];
        assert!(!notice.contains(concat!("notar", "ized")), "{notice}");
        assert!(notice.contains("the depot this origin served"), "{notice}");
    }

    /// A launcher pointed at a keystore that does not exist — the state a
    /// player is in before they make an account.
    ///
    /// The doorbell is impatient on purpose: the client waits
    /// `consent::PROMPT_LIMIT` because that is what its SDK clients wait, and a
    /// test that inherited it would take five minutes to check the giving-up
    /// path once.
    /// Every test below that reads or writes `ELO_PAIRING` holds this.
    ///
    /// The environment is per PROCESS and cargo runs tests on threads, so a
    /// test that points the pairing at a real file and one that points it at
    /// nowhere will happily undo each other — intermittently, which is the
    /// worst way for a suite to be wrong.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn no_pairing() -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("elo-gui-test-{}", std::process::id()))
            .join("no-such-pairing.json")
    }

    fn accountless() -> Launcher {
        let nowhere = std::env::temp_dir()
            .join(format!("elo-gui-test-{}", std::process::id()))
            .join("no-such-keystore.json");
        // ⚠ **And no browser pairing either, or this suite reads the DEVELOPER's.**
        // `sign` falls through to `paired()`, which resolves the real config
        // directory — so on a machine that happens to be paired these tests
        // would take the browser branch and make a network call. Pointed at a
        // path that cannot exist, once: every test in this module wants the
        // same "no signer at all" machine.
        std::env::set_var("ELO_PAIRING", no_pairing());
        Launcher {
            host: "https://elopros.com".into(),
            games_root: std::env::temp_dir(),
            signer: Arc::new(Mutex::new(elo_vault::LocalSigner::at(&nowhere))),
            net: elo_net::Net::new(),
            consent: Doorbell::with_patience(std::time::Duration::from_millis(150)),
        }
    }

    /// No account and a locked account are different problems with different
    /// fixes. One message covering both would send a player who already has an
    /// account off to make a second one.
    #[test]
    fn the_two_reasons_a_sign_fails_are_not_the_same_sentence() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let mut l = accountless();
        let refusal = l.sign("elo play\nx", "why").expect_err("no account cannot sign");
        let said = refusal.to_json()["reason"].as_str().unwrap_or_default().to_string();
        assert!(said.contains("no signer"), "say which problem it is: {said}");
        // ⚠ **BOTH doors, and this half is new.** The sentence said "no
        // account — make one", which was the whole truth while a key on this
        // machine was the only way to sign. It stopped being true the day the
        // launcher could borrow a browser wallet, and a refusal that names one
        // of two fixes sends half its readers to do unnecessary work.
        assert!(said.contains("Account") || said.contains("account new"),
                "say where the fix is: {said}");
        assert!(said.contains("pair") || said.contains("browser"),
                "name the other door too — a key here is no longer the only \
                 way to sign: {said}");
        // The old refusal blamed a missing consent window. That window exists
        // now, and a stale reason would send a player looking for a bug that
        // was fixed.
        assert!(!said.contains("no window"), "the consent window is built: {said}");
    }

    /// `signer_name` is what makes a game offer a Sign button at all, so it
    /// must not claim a backend before there is one.
    #[test]
    fn a_launcher_with_no_account_reports_no_signer() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(accountless().signer_name(), "none");
    }

    /// The door does not decide; it asks. With nothing answering the doorbell
    /// the ask must come back refused rather than allowed — the direction that
    /// costs a click instead of a signature.
    /// A machine with no key and a live browser pairing is a machine that CAN
    /// sign, and every verb that answers "who signs here" has to say so.
    ///
    /// Before the pairing existed all three of these answered "nobody", which
    /// was true then and is the exact shape of a stale claim: a game would be
    /// told there is no signer while a wallet it could reach in one round trip
    /// sat paired two inches away.
    #[test]
    fn a_paired_browser_is_a_signer_this_machine_reaches() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("elo-gui-paired-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pairing.json");
        let addr = "0x00000000000000000000000000000000000000ab";
        elo_net::pairing::save(
            &path,
            &elo_net::pairing::Pairing {
                host: "https://elopros.com".into(),
                code: "ABCD2345".into(),
                secret: "0".repeat(32),
                address: addr.into(),
                expires: elo_net::pairing::now() + 3600,
            },
        )
        .unwrap();

        let mut l = accountless();          // sets ELO_PAIRING to nowhere…
        std::env::set_var("ELO_PAIRING", &path); // …and this points it at a real one

        assert_eq!(l.signer_name(), "browser-paired",
                   "a game asks this to decide whether to offer a Sign button");
        assert_eq!(l.address().as_deref(), Some(addr),
                   "on a keyless machine the browser's wallet IS who this player is");

        // A game server on the pairing's own site is signed in under that
        // site's name, so the wallet's sign-in check passes and Gates' shards
        // accept it; a server anywhere else is refused here, with the reason,
        // before anything is asked of the browser.
        assert_eq!(l.prove_domain("game.elopros.com").as_deref(), Some("elopros.com"));
        assert_eq!(l.prove_domain("play.example"), None);
        let said = l
            .prove("play.example wants you to sign in with your Ethereum account:\n")
            .expect_err("a server on another site is not signed in through the browser")
            .to_json()["reason"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(said.contains("play.example") && said.contains("elopros.com"),
                "name the server and the site it is not on: {said}");

        std::env::set_var("ELO_PAIRING", no_pairing());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unanswered_doorbell_refuses() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let l = accountless();
        let bell = l.consent.clone();
        let asked = std::thread::spawn(move || {
            let mut host = l;
            host.ask_consent("gates", "play", "elo play\nx", "settling")
        });
        while bell.waiting() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // What a launcher shutting down looks like from the door's side.
        drop(bell);
        // Not a grant, and not a "no" either: the broker signs nothing for
        // it and does not remember it as the player refusing.
        assert_eq!(asked.join().unwrap(), Some(elo_broker::UNANSWERED),
                   "an unanswered prompt must not grant");
    }

    /// The prompt is handed the game's words unaltered — flattening for
    /// display happens where it is drawn, not here.
    #[test]
    fn the_ask_carries_the_games_own_words_to_the_player() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let mut l = accountless();
        let bell = l.consent.clone();
        let door = std::thread::spawn(move || {
            l.ask_consent("gates", "play", "elo play\ndetail: ETH up 5", "settling round 41")
        });
        while bell.waiting() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut seen = None;
        bell.pump(&mut |ask| {
            seen = Some(ask.clone());
            Some(1)
        });
        assert_eq!(door.join().unwrap(), Some(1));
        let ask = seen.expect("the player must have been asked");
        assert_eq!(ask.game, "gates");
        assert_eq!(ask.family, "play");
        assert!(ask.text.contains("ETH up 5"), "the message must arrive verbatim");
        assert_eq!(ask.why, "settling round 41");
    }
}


