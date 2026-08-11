//! `scry-gui` — the launcher a player actually runs.
//!
//! Until this file existed there was a CLI and a window *library* and no
//! launcher. This is the program: it opens the menu, reads what is really
//! installed through `scry-depot`, and serves the game door through
//! `scry-broker` for as long as it is open.
//!
//! **The CLI stays separate and does not link FLTK.** `scry.exe` is 1.9 MB
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
//! 2026-08-08; the consent prompt is `scry_ui::consent` plus
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
use scry_broker::signer::Signer;
use scry_broker::{protocol::Refusal, transport, Host, What};
use scry_ui::consent;
use scry_ui::windows::{self, Row};
use scry_ui::wiring;
use serde_json::{json, Value};
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
    net: scry_net::Net,
    /// The way to the player. Every connection holds a clone of the same
    /// doorbell; the main thread is the only thing that answers it.
    consent: consent::Doorbell,
}

impl Host for Launcher {
    fn address(&self) -> Option<String> {
        // The real one now. Reading it needs no passphrase — `LocalSigner::at`
        // takes the public address out of the keystore in the clear, which is
        // why a LOCKED account can still say who it is. `None` stays a real
        // answer: "someone browsing without an address set is a normal player"
        // (sdk/PROTOCOL.md).
        self.signer.lock().ok().and_then(|s| s.address())
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
        if self.signer.lock().map(|s| s.exists()).unwrap_or(false) {
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
    /// one, so it rings a doorbell and parks — `scry_ui::consent` is that
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
        let mut signer = self
            .signer
            .lock()
            .map_err(|_| Refusal::new("the account is busy — try again"))?;
        if !signer.exists() {
            return Err(Refusal::new(
                "no account on this machine — make one in Account, or run \
                 `scry account new`. Playing without one is a normal state.",
            ));
        }
        if !signer.is_unlocked() {
            return Err(Refusal::new(
                "the account is locked — unlock it in Signing. It stays \
                 unlocked until this launcher closes.",
            ));
        }
        signer.sign(text, why)
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
            return Err(Refusal::new(
                "no account on this machine — make one in Account, or run \
                 `scry account new`. Playing without one is a normal state; \
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
                "profile": scry_broker::protocol::flatten_who(&body, address),
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
            // The shard list is the GAME's to serve and this launcher does not
            // invent one. Until a title publishes `servers.url`, refusing is
            // the honest answer — an empty list would read as "no shards".
            What::Servers => None,
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

// Where the keystore lives is `scry_vault::keystore_path()` and is defined
// there ONLY. This file used to carry its own copy, which was fine while the
// GUI was the only thing that could hold a key — the moment `scry account new`
// existed, two copies of that logic became two places for the CLI and the
// window to disagree about which file is the account.

fn games_root() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("SCRY_GAMES") {
        return std::path::PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        let base = std::env::var("LOCALAPPDATA")
            .or_else(|_| std::env::var("APPDATA"))
            .unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(base).join("scry").join("games")
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            if home.is_empty() { ".".into() } else { format!("{home}/.local/share") }
        });
        std::path::PathBuf::from(base).join("scry").join("games")
    }
}

/// Serve the game door for as long as the launcher is open.
///
/// One thread, one connection at a time — which is enough, because a player
/// runs one game. It is spawned detached: if the door cannot be opened the
/// launcher still runs, because being unable to serve games is not a reason to
/// refuse to show a player their library.
fn open_the_door(host: String, games_root: std::path::PathBuf,
                 signer: wiring::Shared, consent: consent::Doorbell) -> Option<String> {
    let endpoint = transport::default_endpoint();
    let listener = match transport::Listener::bind(&endpoint) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("scry-gui: the game door did not open ({e}) — games will \
                       still install and start, but they will play anonymously");
            return None;
        }
    };
    let shown = listener.endpoint();
    std::thread::spawn(move || loop {
        match listener.accept() {
            Ok(conn) => {
                let mut launcher = Launcher {
                    host: host.clone(),
                    games_root: games_root.clone(),
                    signer: Arc::clone(&signer),
                    net: scry_net::Net::new(),
                    consent: consent.clone(),
                };
                // One game at a time. A panic in a connection must not take
                // the launcher with it.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = scry_broker::serve_conn(conn, &mut launcher);
                }));
            }
            Err(e) => {
                eprintln!("scry-gui: the door stopped accepting ({e})");
                return;
            }
        }
    });
    Some(shown)
}

/// Re-attach to the terminal that started us, when there is one.
///
/// A `windows` subsystem binary is given no console, so `println!` writes into
/// a closed handle and a player who ran `scry-gui.exe` from PowerShell on
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
    let host = std::env::var("SCRY_HOST")
        .unwrap_or_else(|_| "https://scry.moreright.xyz".into());
    let root = games_root();

    // The account first, because the door needs it: a game may ask this
    // launcher to prove who is playing, and that is served on the door's
    // thread from the same account the windows unlock.
    let keystore = scry_vault::keystore_path();
    let signer: wiring::Shared = Arc::new(Mutex::new(scry_vault::LocalSigner::at(&keystore)));

    // The doorbell, which both threads hold: the door rings it, and the main
    // loop below is the only thing that ever answers. It is made here rather
    // than inside `open_the_door` because the answering half outlives any one
    // connection — a launcher whose door failed to open still runs, and a
    // pump with nothing to pump costs nothing.
    let doorbell = consent::Doorbell::new();

    // Then the door, so its path is in the environment before any game starts.
    if let Some(endpoint) =
        open_the_door(host.clone(), root.clone(), Arc::clone(&signer), doorbell.clone())
    {
        std::env::set_var(transport::SOCKET_ENV, &endpoint);
        println!("scry-gui: the game door is open at {endpoint}");
    }

    // What is actually installed. `stale: None` is the honest state here —
    // nothing has asked the origin yet, and "not checked" must never render as
    // "up to date".
    let rows: Vec<Row> = scry_depot::installed(&root).iter().map(Row::from_install).collect();
    println!("scry-gui: {} install(s) under {}", rows.len(), root.display());

    let a = app::App::default();
    let has_account = signer.lock().map(|s| s.exists()).unwrap_or(false);
    let mut menu = windows::main_menu(has_account, env!("CARGO_PKG_VERSION"));
    menu.window.show();

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

    // The catalog, read once at startup. `reachable` is carried through rather
    // than collapsed into "empty", because an origin that lists nothing and an
    // origin that did not answer are different sentences and the Store draws
    // them differently.
    let (store_titles, store_reachable) = read_catalog(&host);
    println!(
        "scry-gui: catalog {}",
        if store_reachable {
            format!("{} title(s)", store_titles.len())
        } else {
            "unreadable — the Store will say so".into()
        }
    );

    // Gates' shards, read at startup the same way the catalog is. The whole
    // read is one function so the window keeps taking measured facts and doing
    // no I/O of its own.
    let shards = read_shards(&host, SERVERS_SLUG);
    let servers_w = windows::servers(SERVERS_SLUG, &shards);
    wire_servers(&servers_w, SERVERS_SLUG, &root);

    let account_w = {
        let s = signer.lock().expect("signer lock");
        windows::account(s.address().as_deref(), &host)
    };
    let signing_w = {
        let s = signer.lock().expect("signer lock");
        windows::signing(s.kind(), &s.status(), s.address().as_deref(), s.exists())
    };

    // The behaviour lives in `scry_ui::wiring` so it can be driven by a test
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

    // Start answering the door. This must be registered before `a.run()` and
    // never from the door's own thread — FLTK's widgets belong to this one,
    // and the whole shape of `scry_ui::consent` exists to keep that true.
    wiring::serve_consent(doorbell, Arc::clone(&signer));

    // Relock an unlocked key that sits idle — every signature refreshes the
    // clock, so play never trips it. Minutes; 0 disables (the operator's own
    // machine saying so, not a default).
    let relock_min = std::env::var("SCRY_RELOCK_MINUTES")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(30);
    wiring::serve_relock(
        Arc::clone(&signer),
        &signing_w,
        std::time::Duration::from_secs(relock_min * 60),
    );

    let windows_by_label: Vec<(&str, fltk::window::Window)> = vec![
        ("Games", windows::games(&rows)),
        ("Store", windows::store(&store_titles, store_reachable)),
        ("Servers", servers_w.window.clone()),
        ("Account", account_w.window.clone()),
        ("Signing", signing_w.window.clone()),
        ("About", windows::about()),
    ];
    for i in 0..menu.window.children() {
        if let Some(mut child) = menu.window.child(i) {
            let label = child.label();
            if let Some((_, w)) = windows_by_label.iter().find(|(n, _)| *n == label) {
                let mut w = w.clone();
                child.set_callback(move |_| { w.show(); });
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

    // Is this build current? Same read and same three answers as `scry
    // check`, whose comment carries the ruling: **a notice, never a
    // self-update** — a binary that replaces itself is the one part of this
    // client that would need absolute trust, so the swap stays with the
    // package manager or the download page, in the open. Only a MEASURED
    // newer version speaks; current, ahead, unpublished and unreadable all
    // leave the corner as built (`newer_client`'s own test pins that), so
    // "could not look" never wears "up to date".
    let card = scry_net::Net::new().get_json(&format!("{host}/api/launcher"));
    if let Some(newer) = wiring::newer_client(card.value.as_ref(), env!("CARGO_PKG_VERSION")) {
        let mine = env!("CARGO_PKG_VERSION");
        menu.version_line
            .set_label(&format!("v{mine} — {newer} is out"));
        menu.version_line.set_label_color(scry_ui::theme::GOLD);
        println!("scry-gui: client {mine} — {newer} is published at {host}/download.html");
        // One dialog, once per launch, dismissible — a nudge with the reason
        // in it, never a nag loop and never a download this program performs.
        fltk::dialog::message_default(&format!(
            "A newer scry is published: {newer} (this is {mine}).\n\n\
             Get it at {host}/download.html — or from your package manager,\n\
             if you installed it there.\n\n\
             This program does not replace itself: a binary that rewrites\n\
             itself would need absolute trust, so the swap happens in the\n\
             open, where you can see it."
        ));
    }

    a.run().unwrap();
}

/// The origin's catalog, as `(rows, reachable)`.
///
/// Reads the same route the CLI's `scry games` reads, so the two halves of the
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
fn read_shards(host: &str, slug: &str) -> windows::Shards {
    let net = scry_net::Net::new();
    let url = {
        let got = net.get_json(&format!("{host}/api/launcher/manifest/{slug}"));
        let Some(v) = got.value else {
            return windows::Shards::Unreadable {
                url: format!("{host}/api/launcher/manifest/{slug}"),
                why: got.why,
                reachable: got.reachable,
            };
        };
        match scry_depot::parse_manifest(&v) {
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
    println!("scry-gui: {} shard(s) listed for {slug}", rows.len());
    windows::Shards::Listed { url, rows }
}

/// Give the Servers window's controls their behaviour.
///
/// **Join starts the installed build with `{server}` filled in** — the same
/// substitution `scry play --server` performs, through the same
/// `launch::resolve`, because a second launch path is a second place for the
/// argv rules to be got wrong.
///
/// It deliberately does NOT install. A player who has not got the game should
/// be told so, not have a download begin because they clicked Join on a row
/// they were browsing; installing is the Store's verb and it asks first.
fn wire_servers(w: &windows::ServersWindow, slug: &str, games_root: &std::path::Path) {
    for row in &w.rows {
        let addr = row.addr.clone();
        let slug = slug.to_string();
        let root = games_root.to_path_buf();
        let mut join = row.join.clone();
        join.set_callback(move |b| {
            // `rfind`, not `find` — builds sort by name and a title can have
            // several installed, so the FIRST match is the OLDEST. `scry play`
            // makes the same choice for the same reason.
            let Some(i) = scry_depot::installed(&root).into_iter().rfind(|i| i.slug == slug)
            else {
                b.set_label("Not installed");
                b.deactivate();
                return;
            };
            let values = std::collections::BTreeMap::from([("server".to_string(), addr.clone())]);
            match scry_depot::launch::resolve(&i, &values, &std::collections::BTreeMap::new())
                .and_then(|l| scry_depot::launch::spawn(&l))
            {
                Ok(pid) => println!("scry-gui: joined {addr} (pid {pid})"),
                Err(e) => {
                    // Said on the control the player pressed. A launcher that
                    // logs a failure to a console nobody is reading has not
                    // reported it.
                    eprintln!("scry-gui: could not join {addr}: {e}");
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

fn read_catalog(host: &str) -> (Vec<(String, String)>, bool) {
    let got = scry_net::Net::new().get_json(&format!("{host}/api/launcher/manifests"));
    let Some(v) = got.value else {
        // Not an empty shelf. `reachable` false is the whole point of this
        // return type — see the Store window's two branches.
        return (Vec::new(), false);
    };
    let rows = v
        .get("manifests")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let titles = rows
        .iter()
        .filter_map(|m| {
            let name = m.get("name").and_then(|s| s.as_str())?;
            let blurb = m
                .get("blurb")
                .and_then(|s| s.as_str())
                .unwrap_or("no description");
            Some((name.to_string(), blurb.to_string()))
        })
        .collect();
    (titles, true)
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
    use scry_ui::consent::Doorbell;

    /// A launcher pointed at a keystore that does not exist — the state a
    /// player is in before they make an account.
    ///
    /// The doorbell is impatient on purpose: the client waits
    /// `consent::PROMPT_LIMIT` because that is what its SDK clients wait, and a
    /// test that inherited it would take five minutes to check the giving-up
    /// path once.
    fn accountless() -> Launcher {
        let nowhere = std::env::temp_dir()
            .join(format!("scry-gui-test-{}", std::process::id()))
            .join("no-such-keystore.json");
        Launcher {
            host: "https://scry.moreright.xyz".into(),
            games_root: std::env::temp_dir(),
            signer: Arc::new(Mutex::new(scry_vault::LocalSigner::at(&nowhere))),
            net: scry_net::Net::new(),
            consent: Doorbell::with_patience(std::time::Duration::from_millis(150)),
        }
    }

    /// No account and a locked account are different problems with different
    /// fixes. One message covering both would send a player who already has an
    /// account off to make a second one.
    #[test]
    fn the_two_reasons_a_sign_fails_are_not_the_same_sentence() {
        let mut l = accountless();
        let refusal = l.sign("scry play\nx", "why").expect_err("no account cannot sign");
        let said = refusal.to_json()["reason"].as_str().unwrap_or_default().to_string();
        assert!(said.contains("no account"), "say which problem it is: {said}");
        assert!(said.contains("Account") || said.contains("account new"),
                "say where the fix is: {said}");
        // The old refusal blamed a missing consent window. That window exists
        // now, and a stale reason would send a player looking for a bug that
        // was fixed.
        assert!(!said.contains("no window"), "the consent window is built: {said}");
    }

    /// `signer_name` is what makes a game offer a Sign button at all, so it
    /// must not claim a backend before there is one.
    #[test]
    fn a_launcher_with_no_account_reports_no_signer() {
        assert_eq!(accountless().signer_name(), "none");
    }

    /// The door does not decide; it asks. With nothing answering the doorbell
    /// the ask must come back refused rather than allowed — the direction that
    /// costs a click instead of a signature.
    #[test]
    fn an_unanswered_doorbell_refuses() {
        let l = accountless();
        let bell = l.consent.clone();
        let asked = std::thread::spawn(move || {
            let mut host = l;
            host.ask_consent("gates", "play", "scry play\nx", "settling")
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
            l.ask_consent("gates", "play", "scry play\ndetail: ETH up 5", "settling round 41")
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


