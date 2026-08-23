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

impl Host for Launcher {
    fn address(&self) -> Option<String> {
        // The real one now. Reading it needs no passphrase — `LocalSigner::at`
        // takes the public address out of the keystore in the clear, which is
        // why a LOCKED account can still say who it is. `None` stays a real
        // answer: "someone browsing without an address set is a normal player"
        // (sdk/PROTOCOL.md).
        //
        // ⚠ **Falls through to the paired browser**, because on a keyless
        // machine the browser's wallet IS who this player is. Answering `None`
        // there would tell a game "nobody is here" while a wallet it could
        // reach in one round trip was sitting paired.
        self.signer
            .lock()
            .ok()
            .and_then(|s| s.address())
            .or_else(|| paired().map(|p| p.address))
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
        // ⚠ **`local` still wins when a keystore exists, even locked**, and
        // that is not the CLI's rule. The CLI unlocks per command and forgets,
        // so a local signature always costs a prompt and the newer deliberate
        // act breaks the tie. Here an unlocked account answers a game with NO
        // interruption at all — which is the whole reason `prove` has no
        // prompt — so the signer that can answer without leaving the program
        // is the one to name. One rule, two outcomes, and
        // `elo-launcher/src/signer.rs` carries the same sentence.
        if self.signer.lock().map(|s| s.exists()).unwrap_or(false) {
            "local".into()
        } else if paired().is_some() {
            "browser-paired".into()
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

    /// The identity proof. **No consent, and no prompt** — the launcher wrote
    /// this message (`protocol::prove_message`), the game only handed over a
    /// nonce and a server name, and a signature over it authorises nothing.
    ///
    /// Refuses in two states, and each says which: no account at all, and an
    /// account that is locked. Those are different problems with different
    /// fixes, and collapsing them into "cannot sign" would send a player to
    /// make a second account they do not need.
    fn prove(&mut self, message: &str) -> Result<Value, Refusal> {
        let mut signer = self
            .signer
            .lock()
            .map_err(|_| Refusal::new("the account is busy — try again"))?;
        if !signer.exists() {
            // ⚠ **A browser pairing cannot serve this one, and the player is
            // told why rather than sent to make a key with no reason given.**
            // `prove_message` is EIP-4361, the relay refuses SIWE so the door
            // can never be used to phish a login, and that wall has no
            // exception shaped like us. Same sentence as the CLI's `prove`.
            if let Some(p) = paired() {
                return Err(Refusal::new(format!(
                    "proving to a game server is the one act a browser pairing \
                     cannot do — the message is a sign-in (EIP-4361), and \
                     sign-ins do not ride the relay, which is what stops it \
                     being used to phish a login. {} stays paired for \
                     everything else; this wants an account here.",
                    p.address
                )));
            }
            return Err(Refusal::new(
                "no account on this machine — make one in Account, or run \
                 `elo account new`. Playing without one is a normal state; \
                 a server that wants proof of who you are will not get it.",
            ));
        }
        if !signer.is_unlocked() {
            return Err(Refusal::new(
                "the account is locked — unlock it in Signing. It stays \
                 unlocked until this launcher closes.",
            ));
        }
        signer.sign(message, "prove identity")
    }

    /// A name and a face for one address. **A read, never authentication** —
    /// `prove` is the verb that establishes who is holding a key.
    ///
    /// ⚠ **The three states are kept apart, and that is the whole care here.**
    /// A 404 from `/api/who` is a real ANSWER — nobody has sworn on this
    /// address — and comes back as `ok` with a null profile. Anything else that
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
                "note": "no sworn identity for that address. Playing unsworn is a \
                         normal state, and this is a real answer rather than a \
                         failed read.",
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

    fn open(&mut self, url: &str) -> Result<(), Refusal> {
        // https-only was already checked by the protocol layer. Opening a
        // browser is the one thing this does to the player's machine, so it
        // goes through the platform's own handler and never a shell string.
        let r = if cfg!(target_os = "windows") {
            std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(url).spawn()
        };
        r.map(|_| ()).map_err(|e| Refusal::new(format!("could not open a browser: {e}")))
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
    #[cfg(windows)]
    {
        let base = std::env::var("LOCALAPPDATA")
            .or_else(|_| std::env::var("APPDATA"))
            .unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(base).join("elo").join("games")
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            if home.is_empty() { ".".into() } else { format!("{home}/.local/share") }
        });
        std::path::PathBuf::from(base).join("elo").join("games")
    }
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
                 signer: wiring::Shared, consent: consent::Doorbell) -> Option<String> {
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
        transport::serve_forever(listener, move || Launcher {
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
    let host = std::env::var("ELO_HOST")
        .unwrap_or_else(|_| "https://elopros.com".into());
    let root = games_root();

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
    let door = open_the_door(host.clone(), root.clone(), Arc::clone(&signer), doorbell.clone());
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
    wiring::lock_screen(
        &signer,
        "unlock the account on this machine to carry on",
    );

    let mut menu = windows::main_menu(has_account, env!("CARGO_PKG_VERSION"));

    // Where the client opens. Every window is constructed at a hard-coded
    // coordinate — the menu at `(100, 100)`, then a 20px cascade — so the
    // whole family is put back where the player is looking by centring the
    // menu and carrying everything else by the same amount. That keeps the
    // cascade the windows were built with instead of stacking six windows on
    // one spot (`wiring::centre`).
    let drift = wiring::centre(&mut menu.window);

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
            "The game door did not open",
            "Games will still install and start — they will just play anonymously,\n\
             because nothing can ask this launcher who is playing or for a signature.\n\n\
             The usual cause is another copy of elo already running.\n\n\
             Nothing retries this while the program is open, so a restart is the fix.",
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
    let front: Rc<dyn wiring::Storefront> = Rc::new(Front {
        host: host.clone(),
        root: root.clone(),
    });

    // Gates' shards, read at startup the same way the catalog is. The whole
    // read is one function so the window keeps taking measured facts and doing
    // no I/O of its own.
    let shards = read_shards(&host, SERVERS_SLUG);
    // Only a list we actually READ names a url worth passing on. `Unreadable`
    // carries one too, and handing the game a url this launcher just failed to
    // read would ask it to make the same failed request again.
    let list_url = match &shards {
        windows::Shards::Listed { url, .. } => Some(url.clone()),
        _ => None,
    };
    let servers_w = windows::servers(SERVERS_SLUG, &shards);
    wire_servers(&servers_w, SERVERS_SLUG, &root, list_url.as_deref());

    let account_w = {
        let s = signer.lock().expect("signer lock");
        windows::account(s.address().as_deref(), &host)
    };
    let signing_w = {
        let s = signer.lock().expect("signer lock");
        windows::signing(s.kind(), &s.status(), s.address().as_deref(), s.exists())
    };

    // The behaviour lives in `elo_ui::wiring` so it can be driven by a test
    // and by `examples/click.rs`; a callback defined in this binary would be
    // the one path in the client nothing could reach.
    wiring::wire_account(&account_w, &signing_w, &signer, &keystore,
                         wiring::real_ask(), wiring::real_tell());
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
    wiring::serve_relock(
        Arc::clone(&signer),
        &signing_w,
        std::time::Duration::from_secs(relock_min * 60),
        wiring::lock_screen_armed(),
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
    });
    refresh_store(&ui, false);
    refresh_games(&ui, false);

    let windows_by_label: Vec<(&str, fltk::window::Window)> = vec![
        ("Servers", servers_w.window.clone()),
        ("Account", account_w.window.clone()),
        ("Signing", signing_w.window.clone()),
        ("About", windows::about()),
    ];
    // Carried by the same amount the menu was, so the cascade they were built
    // with survives the move. Done before any of them is shown — see
    // `wiring::centre`.
    for (_, w) in windows_by_label.iter() {
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
                    if let Some(s) = ui_c.store.borrow().as_ref() {
                        let mut w = s.window.clone();
                        w.show();
                    }
                }),
                "Games" => child.set_callback(move |_| {
                    if let Some(g) = ui_c.games.borrow().as_ref() {
                        let mut w = g.window.clone();
                        w.show();
                    }
                }),
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
    if !has_account {
        let mut w = account_w.window.clone();
        w.show();
    }

    // Is this build current? Same read and same three answers as `elo
    // check`, whose comment carries the ruling: **a notice, never a
    // self-update** — a binary that replaces itself is the one part of this
    // client that would need absolute trust, so the swap stays with the
    // package manager or the download page, in the open. Only a MEASURED
    // newer version speaks; current, ahead, unpublished and unreadable all
    // leave the corner as built (`newer_client`'s own test pins that), so
    // "could not look" never wears "up to date".
    let card = elo_net::Net::new().get_json(&format!("{host}/api/launcher"));
    if let Some(newer) = wiring::newer_client(card.value.as_ref(), env!("CARGO_PKG_VERSION")) {
        let mine = env!("CARGO_PKG_VERSION");
        menu.version_line
            .set_label(&format!("v{mine} — {newer} is out"));
        menu.version_line.set_label_color(elo_ui::theme::GOLD);
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
        let Some(su) = r.status_url.clone() else { continue };
        if let Some(s) = net.status(&su).value {
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
    games_root: &std::path::Path,
    list_url: Option<&str>,
) {
    for row in &w.rows {
        let addr = row.addr.clone();
        let slug = slug.to_string();
        let root = games_root.to_path_buf();
        let list_url = list_url.map(str::to_string);
        let mut join = row.join.clone();
        join.set_callback(move |b| {
            // The most recently INSTALLED build, by directory mtime. Not by
            // name: a build id ends in a hex sha and hex does not sort, which
            // is how a row reading "up to date" launched a four-day-old client.
            let Some(i) = elo_depot::newest_of(&root, &slug) else {
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

/// Read the shelf and put up a Store window over whatever was there.
fn refresh_store(ui: &Rc<Ui>, show: bool) {
    let (rows, reachable, why) = read_shelf(&ui.host, &ui.root);
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
        Rc::new(move || {
            defer(&ui, |ui, _| {
                refresh_store(ui, false);
                refresh_games(ui, false);
            })
        }) as Rc<dyn Fn()>
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
    let rows = read_library(&ui.host, &ui.root, &ui.shelf.borrow());
    println!("elo-gui: {} install(s) under {}", rows.len(), ui.root.display());
    let mut games_w = windows::games(&rows);
    place(
        &mut games_w.window,
        ui.games.borrow().as_ref().map(|g| (g.window.x(), g.window.y())),
        ui.drift,
    );
    wiring::wire_games(&games_w, Rc::clone(&ui.front), wiring::real_tell(),
                       wiring::real_then());

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
}

impl wiring::Storefront for Front {
    /// Fetch, hash-verify and place a build.
    ///
    /// ⚠ **This blocks the UI thread for as long as the download takes**, and
    /// that is a real cost stated rather than hidden: the window freezes.
    /// It is still better than the alternative shipped today, which is no
    /// button at all — and the honest fix is a progress row, which is work
    /// this change does not do. The button says `Installing…` before it
    /// starts, so the freeze reads as the install and not as a crash.
    fn install(&self, slug: &str) -> Result<String, String> {
        let net = elo_net::Net::new();
        let url = format!("{}/api/launcher/manifest/{slug}", self.host);
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
        m.rebase(&self.host);
        let checked = elo_depot::check(&m, &elo_depot::platform_tag(), &self.root, &net, false);
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
                    self.host
                ))
            }
            elo_depot::UpdateState::Unknown { why } => {
                return Err(format!("refusing to install — {why}"))
            }
            _ => {}
        }
        let Some(depot) = checked.depot else {
            return Ok(format!("{} is already up to date.", m.name));
        };
        let plan = elo_depot::plan(&depot, &self.root);
        let at = elo_depot::install(&depot, &self.root, &net, Some(&plan), None)
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
        for old in elo_depot::superseded(&self.root, slug, &depot.build) {
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
                 against the sha256 in the notarized depot.",
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

    fn verify(&self, slug: &str, build: &str) -> Result<String, String> {
        let Some(i) = elo_depot::installed(&self.root)
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

    fn play(&self, slug: &str) -> Result<String, String> {
        // The most recently INSTALLED build, by directory mtime — see
        // `newest_of`. A build id ends in a hex sha, so name order is noise.
        let Some(i) = elo_depot::newest_of(&self.root, slug) else {
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
        let pid = elo_depot::launch::spawn(&launch).map_err(|e| e.to_string())?;
        Ok(format!("{slug} started (pid {pid})."))
    }

    fn open(&self, url: &str) -> Result<(), String> {
        let r = if cfg!(target_os = "windows") {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", url])
                .spawn()
        } else {
            std::process::Command::new("xdg-open").arg(url).spawn()
        };
        r.map(|_| ()).map_err(|e| format!("could not open a browser: {e}"))
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
fn read_shelf(host: &str, root: &std::path::Path) -> (Vec<windows::Shelf>, bool, String) {
    let net = elo_net::Net::new();
    let got = net.get_json(&format!("{host}/api/launcher/manifests"));
    let Some(v) = got.value else {
        // Not an empty shelf. `reachable` false is the whole point of this
        // return type — see the Store window's two branches.
        return (Vec::new(), false, got.why);
    };
    let manifests = v
        .get("manifests")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let here = elo_depot::platform_tag();
    let on_disk = elo_depot::installed(root);

    let mut rows = Vec::new();
    for m in manifests.iter() {
        let Some(slug) = m.get("slug").and_then(Value::as_str) else {
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
            price: read_price(&net, host, slug),
            icon: read_icon(&net, host, m),
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

    for i in &installs {
        if published.contains_key(&i.slug) {
            continue;
        }
        let url = format!("{host}/api/launcher/manifest/{}", i.slug);
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
        published.insert(i.slug.clone(), answer);
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
            // slug" would keep an arbitrary build. Keep the one `newest_of`
            // would run, so the row and the Play button agree.
            elo_depot::newest_of(root, &i.slug).map(|n| n.build == i.build).unwrap_or(false)
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

        // ⚠ `prove` is the one verb the pairing cannot serve, and it must say
        // WHY. `prove_message` is EIP-4361 and the relay refuses SIWE, so this
        // is a consequence of a wall rather than a missing feature — a player
        // told "make an account" with no reason will read it as a bug.
        let said = l
            .prove("anything")
            .expect_err("a pairing cannot prove")
            .to_json()["reason"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(said.contains("sign-in") || said.contains("EIP-4361"),
                "name the reason, not just the refusal: {said}");
        assert!(said.contains(addr), "say what IS still paired: {said}");

        std::env::set_var("ELO_PAIRING", no_pairing());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unanswered_doorbell_refuses() {
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
        assert_eq!(asked.join().unwrap(), None, "an unanswered prompt must not grant");
    }

    /// The prompt is handed the game's words unaltered — flattening for
    /// display happens where it is drawn, not here.
    #[test]
    fn the_ask_carries_the_games_own_words_to_the_player() {
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


