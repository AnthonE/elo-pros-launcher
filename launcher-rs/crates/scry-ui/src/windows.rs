//! The windows off the main menu.
//!
//! Multi-window like the original client: each screen is its own top-level
//! rather than a page swapped into one frame. That is the 2003 shape and it is
//! also the honest one for a launcher — a player can leave Games open while
//! the Store downloads.
//!
//! **These build a window and return it.** None of them run an event loop, and
//! none of them read the network or the disk on their own: a caller passes in
//! what `scry-depot` already measured. That keeps every fact on screen
//! traceable to the crate whose tests cover it, and it is what makes these
//! constructible in a test with no display and no games root.

use crate::chrome;
use crate::theme::{self, Tone};
use fltk::{button, enums, frame, input, prelude::*, window};
use scry_depot::{shardlist::Shard, Install};

/// One row on the shelf, already measured. The window renders this and
/// computes nothing from it.
///
/// `stale` is an `Option` on purpose and the third state is the point:
/// `None` means **we could not look**, which is not the same as up to date and
/// must not render as it. Same rule as `net::Fetched.reachable` in the Python
/// client and `UpdateState::Unknown` in `scry-depot` — the repo's own trap is
/// a reader that returns the empty answer for both "nothing" and "no idea".
pub struct Row {
    pub slug: String,
    pub build: String,
    pub bytes: i64,
    pub digest: String,
    pub stale: Option<bool>,
}

impl Row {
    /// Build a row from an install with no staleness check performed. The
    /// caller runs `scry_depot::check` and fills `stale` in; until it does,
    /// the honest answer is "unknown" and that is what this produces.
    pub fn from_install(i: &Install) -> Row {
        Row {
            slug: i.slug.clone(),
            build: i.build.clone(),
            bytes: i.total_bytes,
            digest: i.depot_digest.clone(),
            stale: None,
        }
    }

    /// The line under the title. Three states, three sentences, and the third
    /// says which of the two it is not.
    pub fn status_line(&self) -> String {
        let size = human_bytes(self.bytes);
        match self.stale {
            Some(true) => format!("{} · an update is published", size),
            Some(false) => format!("{} · up to date", size),
            None => format!("{} · not checked — the origin was not reached", size),
        }
    }

    /// What the row's own control says. **`Gold` when we could not look**, not
    /// `Live`: an unknown state has not earned the armed colour.
    pub fn tone(&self) -> Tone {
        match self.stale {
            Some(true) => Tone::Plain,
            Some(false) => Tone::Plain,
            None => Tone::Gold,
        }
    }

    /// What the control DOES, which is not always "play".
    ///
    /// Caught by looking at a capture rather than by a test: a row reading
    /// *"an update is published"* was offering the same **Play** button as one
    /// reading *"up to date"*, so the sentence and the control disagreed and
    /// the control won. A player would have launched the old build and never
    /// known there was a new one — the client would have been technically
    /// honest and practically silent.
    ///
    /// Playing a stale build stays possible; it is just not what the button
    /// offers first. `scry play` runs whatever is installed.
    pub fn action(&self) -> &'static str {
        match self.stale {
            Some(true) => "Update",
            _ => "Play",
        }
    }
}

/// `72.3 MB`, the way the CLI already prints it. One implementation would be
/// better than two; this is the smaller of the two and the CLI's is not public.
fn human_bytes(n: i64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if n <= 0 {
        return "0 B".into();
    }
    let f = n as f64;
    if f >= MB {
        format!("{:.1} MB", f / MB)
    } else if f >= 1024.0 {
        format!("{:.0} KB", f / 1024.0)
    } else {
        format!("{n} B")
    }
}

/// The menu, as data — so the window can be sized from it and a test can read
/// it without a display. `built: false` renders deactivated AND says so; a
/// hidden feature and an unbuilt one look identical to a player, and only one
/// of those is honest.
pub const MENU: [(&str, &str, bool); 6] = [
    ("Games", "what is installed, and what is stale", true),
    ("Store", "the curated shelf", true),
    ("Servers", "browse a title's shards", true),
    ("Account", "the address the launcher watches", true),
    ("Signing", "which backend signs, and what it can do", true),
    ("About", "what this is, and what it is built on", true),
];

/// The main menu — buttons-with-a-sentence, which is the original's shape and
/// the reason the client never needed a nav bar.
///
/// Every entry names a window; the ones whose window does not exist yet are
/// **deactivated and say so**, rather than being hidden. A hidden feature and
/// an unbuilt one look identical to a player, and only one of them is honest.
/// The main menu, plus the one handle startup writes to after it is shown.
pub struct MainMenu {
    pub window: window::Window,
    /// The corner version — `v0.1.0`, DIM, drawn from the binary's own
    /// manifest so it is true with no network. Startup rewrites it GOLD when
    /// the origin's card names a NEWER published client; a card we could not
    /// read leaves it exactly as built, because a version is local truth and
    /// currency is a measurement — "could not look" must never render as
    /// "up to date" (`CLAUDE.md` §traps, the same rule `scry check` keeps).
    pub version_line: frame::Frame,
}

pub fn main_menu(has_account: bool, version: &str) -> MainMenu {
    // Sized to the content rather than to a round number — the first capture
    // had 150px of empty olive under the last row, which reads as a window
    // that failed to draw something rather than as a window that is finished.
    let height = 64 + (MENU.len() as i32 * 38) + 46 + if has_account { 0 } else { 20 };
    let mut w = window::Window::new(100, 100, 560, height, None);
    w.set_label("scry");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 22, "scry", theme::HEAD, 19);
    chrome::label(16, 34, 480, 18, "the town's games, on your machine", theme::MUTED, 12);
    // The corner. Right-aligned so a longer "— 0.2.0 is out" grows leftward
    // into empty header instead of off the window.
    let mut version_line = chrome::label(280, 12, 264, 18, &format!("v{version}"),
                                         theme::DIM, 11);
    version_line.set_align(enums::Align::Right | enums::Align::Inside);

    let mut y = 64;
    for &(name, blurb, built) in MENU.iter() {
        let mut b = if built {
            chrome::button(16, y, 150, 30, name, Tone::Plain)
        } else {
            chrome::button_off(16, y, 150, 30, name)
        };
        b.set_align(enums::Align::Center | enums::Align::Inside);
        let (text, ink) = if built {
            (blurb.to_string(), theme::MUTED)
        } else {
            // Named, not hidden. `GOLD` is the not-armed colour and this is
            // exactly what it is for.
            (format!("{blurb} — not built yet"), theme::GOLD)
        };
        chrome::label(178, y + 6, 366, 18, &text, ink, 12);
        y += 38;
    }

    // The one setup nudge this menu carries, and only while it is true. A
    // player measured the alternative as "idk how to make a new key" — the
    // fact was only discoverable behind the Account button. GOLD because it
    // is an offer, not a fault: playing without an account is a normal state.
    if !has_account {
        chrome::label(16, y + 8, 520, 16,
                      "no account on this machine yet — Account makes one \
                       (games still play without it)",
                      theme::GOLD, 11);
        y += 20;
    }

    chrome::label(16, y + 12, 520, 16,
                  "nothing you own lives in here — the pass is a token and items are on chain",
                  theme::DIM, 11);
    w.end();
    MainMenu { window: w, version_line }
}

/// The Games window — installed builds, and whether each is current.
///
/// Takes rows rather than a games root: the window does no I/O, so it can be
/// built in a test with no display, no disk and no origin.
pub fn games(rows: &[Row]) -> window::Window {
    // The well fits the rows, with a floor so an empty shelf is still a window
    // and not a slot. Same reason the menu is sized to its entries: acres of
    // empty olive read as a failed draw, not as a finished screen.
    let well_h = (rows.len() as i32 * 48 + 24).clamp(96, 520);
    let mut w = window::Window::new(120, 120, 620, well_h + 100, None);
    w.set_label("scry — games");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "my games", theme::HEAD, 16);

    let well = chrome::well(16, 40, 588, well_h);
    if rows.is_empty() {
        // The baseline restating itself, said once and plainly. Not an error
        // state and not styled as one.
        chrome::label(28, 56, 560, 18, "nothing installed yet", theme::MUTED, 13);
        chrome::label(28, 78, 560, 18,
                      "a title with a published desktop build appears here once it is installed",
                      theme::DIM, 11);
    } else {
        let mut y = 48;
        for (i, row) in rows.iter().enumerate() {
            if i % 2 == 1 {
                let mut band = fltk::frame::Frame::new(18, y - 4, 584, 44, None);
                band.set_frame(enums::FrameType::FlatBox);
                band.set_color(theme::WELL_ALT);
            }
            chrome::label(28, y, 300, 18, &row.slug, theme::INK, 14);
            chrome::label(28, y + 18, 400, 16, &row.status_line(), theme::MUTED, 11);
            let mut act = chrome::button(486, y + 2, 100, 28, row.action(), row.tone());
            act.set_align(enums::Align::Center | enums::Align::Inside);
            y += 48;
        }
    }
    well.end();

    chrome::label(16, well_h + 52, 400, 16,
                  "verify re-hashes every file the depot names", theme::DIM, 11);
    w.end();
    w
}

/// The About window — and the one place the FLTK credit is shown to a human.
pub fn about() -> window::Window {
    let mut w = window::Window::new(140, 140, 520, 300, None);
    w.set_label("scry — about");
    w.set_color(theme::BG);

    chrome::label(16, 14, 400, 22, "scry", theme::HEAD, 18);
    chrome::label(16, 40, 480, 18, "the desktop client for the town's games", theme::MUTED, 12);

    chrome::label(16, 78, 480, 18, "It holds no key of yours.", theme::INK, 13);
    chrome::label(16, 100, 490, 16,
                  "A local account is generated on this machine and transmitted nowhere.",
                  theme::MUTED, 11);
    chrome::label(16, 122, 490, 16,
                  "Delete this program and you still own everything you bought.",
                  theme::MUTED, 11);

    chrome::label(16, 168, 480, 16, "built on", theme::DIM, 11);
    chrome::label(16, 188, 490, 16, crate::FLTK_CREDIT, theme::MUTED, 11);

    w.end();
    w
}

/// The Store — the curated shelf.
///
/// **The catalog is the origin's, never this client's.** `catalog.py` says why:
/// a second hand-kept list would be a second count and one of them would always
/// be stale. So this renders what it was handed and says plainly when it was
/// handed nothing — which is a different sentence from "the shelf is empty".
pub fn store(titles: &[(String, String)], reachable: bool) -> window::Window {
    let well_h = (titles.len() as i32 * 40 + 24).clamp(120, 460);
    let mut w = window::Window::new(140, 140, 620, well_h + 190, None);
    w.set_label("scry — store");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "the store", theme::HEAD, 16);

    // The package block the original client put at the top of its store. Ours
    // is the pass, and it is GOLD because no pass contract is deployed — an
    // outline, never a fill, so it cannot be mistaken for an armed state
    // (`GATES.md` §10 row 1, `SITE-PLATFORM.md` §14b).
    let pass = chrome::panel(16, 40, 588, 92);
    chrome::label(28, 50, 300, 18, "THE PASS", theme::GOLD, 13);
    for (i, line) in [
        "buy once, in SCRY — the library follows your wallet",
        "resale is native: sell the pass when you are done",
        "half of every pass burns under the posted split",
    ]
    .iter()
    .enumerate()
    {
        chrome::label(28, 70 + (i as i32 * 16), 420, 14, line, theme::MUTED, 11);
    }
    chrome::label(456, 56, 140, 14, "no price set", theme::GOLD, 11);
    let mut not_armed = chrome::button(456, 74, 132, 26, "NOT ARMED", Tone::Gold);
    not_armed.set_align(enums::Align::Center | enums::Align::Inside);
    not_armed.deactivate();
    pass.end();
    chrome::label(16, 136, 588, 14,
                  "No pass contract is deployed and no price is set, so this buys nothing today.",
                  theme::DIM, 11);

    let shelf = chrome::well(16, 158, 588, well_h);
    if !reachable {
        // The repo's own trap, at the one place it would cost a player a game.
        chrome::label(28, 174, 560, 18, "the catalog could not be read", theme::GOLD, 13);
        chrome::label(28, 196, 560, 16,
                      "this is not an empty shelf — the origin did not answer, so nothing is known",
                      theme::MUTED, 11);
    } else if titles.is_empty() {
        chrome::label(28, 174, 560, 18, "nothing listed yet", theme::MUTED, 13);
    } else {
        let mut y = 172;
        for (i, (name, blurb)) in titles.iter().enumerate() {
            if i % 2 == 1 {
                let mut band = fltk::frame::Frame::new(18, y - 4, 584, 38, None);
                band.set_frame(enums::FrameType::FlatBox);
                band.set_color(theme::WELL_ALT);
            }
            chrome::label(28, y, 240, 16, name, theme::INK, 13);
            chrome::label(28, y + 16, 550, 14, blurb, theme::MUTED, 11);
            y += 40;
        }
    }
    shelf.end();
    w.end();
    w
}

/// What the caller measured about a title's shard list.
///
/// **Three states, and the third is the whole point.** "We could not look" is
/// not "there is nothing up", and a launcher that renders an unreachable list
/// as an empty one is lying about the game — the same trap `net::Fetched`'s
/// `reachable` flag exists for, arriving one layer up.
pub enum Shards {
    /// The title publishes no `servers.url`. Dark by design, not by failure.
    Unpublished,
    /// We looked and could not read it.
    Unreadable {
        url: String,
        why: String,
        /// Did we reach the origin at all? Decides which sentence is drawn,
        /// because "their list is broken" and "your wifi is down" are
        /// different problems with different owners.
        reachable: bool,
    },
    /// We read it. **May legitimately be empty** — nobody is running a shard.
    Listed { url: String, rows: Vec<Shard> },
}

/// The controls on one drawn shard row, handed back for a caller to wire.
///
/// Same rule as [`AccountWindow`]: the window draws and decides nothing, so
/// what a Join does — resolve an install, substitute `{server}`, start it —
/// stays in the binary next to the I/O.
#[derive(Clone)]
pub struct ShardControls {
    /// `host:port`, straight from the row. What goes in as `{server}`.
    pub addr: String,
    /// The canonical `scry://join/…` for this row, for the clipboard.
    pub link: String,
    pub join: button::Button,
    pub copy: button::Button,
}

/// The Servers window and the controls on it.
pub struct ServersWindow {
    pub window: window::Window,
    /// One per drawn shard, in list order. Empty when the list is dark,
    /// unreadable, or genuinely lists nothing.
    pub rows: Vec<ShardControls>,
}

/// Servers — browse a title's shards.
///
/// **The shard list is the game's to serve, never the launcher's to invent.**
/// This window fetches nothing: a caller hands it what `scry-net` already
/// measured, which is what keeps it constructible in a test with no display
/// and no origin, and what keeps every number on screen traceable to the crate
/// whose tests cover it.
///
/// **Counts are polled, not read out of the document.** By the time rows reach
/// here the caller has asked each shard's own `status_url`, because a count
/// baked into a served file is stale before anyone reads it. A row whose shard
/// did not answer keeps whatever it stated — usually nothing — and draws `?`.
/// Nothing here ever invents a number, and a `0` on screen means a shard
/// answered "nobody", which is a measurement.
pub fn servers(slug: &str, shards: &Shards) -> ServersWindow {
    let count = match shards {
        Shards::Listed { rows, .. } => rows.len() as i32,
        _ => 0,
    };
    let well_h = (count * 46 + 24).clamp(120, 520);
    let mut w = window::Window::new(160, 160, 640, well_h + 108, None);
    w.set_label("scry — servers");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "servers", theme::HEAD, 16);

    let mut controls = Vec::new();
    let well = chrome::well(16, 40, 608, well_h);
    match shards {
        Shards::Unpublished => {
            chrome::label(28, 56, 580, 18,
                          &format!("{slug} publishes no shard list"), theme::MUTED, 13);
            chrome::label(28, 80, 580, 16,
                          "this window fills itself the moment a title sets `servers.url`",
                          theme::DIM, 11);
            chrome::label(28, 100, 580, 16,
                          "in its manifest — the list is the game's to serve, not ours to invent",
                          theme::DIM, 11);
        }
        // Two sentences, because the two failures have different owners: a
        // list we reached and could not parse is the title's bug, and one we
        // never reached is probably this machine's network.
        Shards::Unreadable { url, why, reachable } => {
            chrome::label(28, 56, 580, 18,
                          if *reachable { "the shard list did not read" }
                          else { "could not reach the shard list" },
                          theme::GOLD, 13);
            chrome::label_untrusted(28, 80, 580, 16, url, theme::DIM, 11);
            // The reason comes from an origin we do not control.
            chrome::label_untrusted(28, 100, 580, 16, why, theme::MUTED, 11);
            chrome::label(28, 124, 580, 16,
                          "this is not `no shards up` — nobody knows what is up right now",
                          theme::DIM, 11);
        }
        Shards::Listed { url, rows } if rows.is_empty() => {
            chrome::label(28, 56, 580, 18,
                          &format!("{slug} lists no shards up"), theme::MUTED, 13);
            chrome::label(28, 80, 580, 16,
                          "the list is up and says nobody is running one — a real answer,",
                          theme::DIM, 11);
            chrome::label(28, 100, 580, 16, "not a failure to read it", theme::DIM, 11);
            chrome::label_untrusted(28, 124, 580, 16, url, theme::DIM, 11);
        }
        Shards::Listed { rows, .. } => {
            let mut y = 52;
            for s in rows.iter() {
                // A name and a map come from a document a title serves, so
                // they are drawn clipped — see `chrome::label_untrusted`.
                chrome::label_untrusted(28, y, 250, 18, &s.name, theme::INK, 13);
                chrome::label(286, y, 70, 18, &s.population(),
                              if s.full() { theme::GOLD } else { theme::MUTED }, 12);
                chrome::label_untrusted(28, y + 18, 250, 14, &s.addr, theme::DIM, 10);
                if let Some(m) = &s.map {
                    chrome::label_untrusted(286, y + 18, 160, 14, m, theme::DIM, 10);
                }

                // A full shard is drawn and not pressable. Hiding it would
                // make a busy server look like one that does not exist.
                let join = if s.full() {
                    chrome::button_off(452, y + 2, 74, 26, "Full")
                } else {
                    chrome::button(452, y + 2, 74, 26, "Join", Tone::Plain)
                };
                // Always offered, even on a full shard: sending a friend a
                // link to a server you cannot get into right now is normal.
                let copy = chrome::button(534, y + 2, 74, 26, "Invite", Tone::Plain);

                controls.push(ShardControls {
                    addr: s.addr.clone(),
                    // A row whose address the link writer refuses cannot be
                    // invited to. Empty rather than a broken string, and the
                    // caller deactivates on it — `parse` already refused the
                    // whole document for a bad address, so this is belt and
                    // braces rather than an expected state.
                    link: scry_depot::deeplink::link_for(slug, &s.addr).unwrap_or_default(),
                    join,
                    copy,
                });
                y += 46;
            }
        }
    }
    well.end();

    chrome::label(16, well_h + 50, 600, 16,
                  "joining passes the address into the game's launch args as {server}",
                  theme::DIM, 11);
    chrome::label(16, well_h + 68, 600, 16,
                  "counts are polled from each shard — the launcher does not proxy, cache or rank",
                  theme::DIM, 11);
    w.end();
    ServersWindow { window: w, rows: controls }
}

/// A window plus the controls a caller must give behaviour to.
///
/// **This does not bend the rule at the top of this module.** The windows still
/// draw and decide nothing: handing back widgets rather than taking callbacks
/// is what keeps that true. Every decision — what a passphrase opens, what a
/// failure says, where a keystore lives — stays in the binary next to the I/O,
/// and this file remains constructible in a test with no disk and no origin.
/// ⚠ `Clone` here shares the underlying widget rather than copying it — that
/// is FLTK's model, and it is what makes these usable from inside a callback:
/// a cloned handle mutates the same button on screen. It is not a duplicate.
#[derive(Clone)]
pub struct AccountWindow {
    pub window: window::Window,
    /// The two doors into having an account, present **only when there is no
    /// keystore yet**. Once one exists both are `None`, because this window
    /// never offers to replace a key: that is the one action whose misfire
    /// cannot be undone.
    pub make: Option<button::Button>,
    pub import: Option<button::Button>,
    /// The address line, so a caller can fill it in after an account is made
    /// without rebuilding the window and losing where the player was.
    pub address: frame::Frame,
    pub note: frame::Frame,
    /// The website pairing — present **only when an account exists**, because
    /// pairing IS this account proving itself to a browser. The code is typed
    /// into the field, never clicked (`docs/client/SIGN-IN.md` §1).
    pub signin_code: Option<input::Input>,
    pub signin: Option<button::Button>,
}

/// Account — the address the launcher watches, and the two ways to get one.
///
/// ⚠ **An address, never authentication.** It is what the player asked the
/// launcher to watch; a game that needs it to matter asks for a signature and
/// verifies it. This window says that rather than assuming a reader knows it.
pub fn account(address: Option<&str>, host: &str) -> AccountWindow {
    let mut w = window::Window::new(180, 180, 580, 400, None);
    w.set_label("scry — account");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "account", theme::HEAD, 16);
    let well = chrome::well(16, 40, 548, 130);
    let (addr_line, note_line) = match address {
        Some(a) => {
            chrome::label(28, 56, 520, 16, "the launcher is watching", theme::MUTED, 11);
            let addr = chrome::label(28, 76, 520, 18, a, theme::INK, 13);
            let note = chrome::label(28, 104, 520, 16, &format!("on {host}"), theme::DIM, 11);
            (addr, note)
        }
        None => {
            let addr = chrome::label(28, 56, 520, 18, "no address set", theme::MUTED, 13);
            let note = chrome::label(28, 80, 520, 16,
                          "a normal state — games play anonymously and say so",
                          theme::DIM, 11);
            (addr, note)
        }
    };
    well.end();

    // Plain, never gold and never green: making a key on your own machine is
    // an ordinary act, and the reserved fill belongs to something that moves
    // money (`SITE-PLATFORM.md` §14b).
    let (make, import, signin_code, signin) = if address.is_none() {
        let mut make = chrome::button(16, 180, 168, 28, "Make an account…", Tone::Plain);
        make.set_align(enums::Align::Center | enums::Align::Inside);
        let mut import = chrome::button(196, 180, 168, 28, "Import a key…", Tone::Plain);
        import.set_align(enums::Align::Center | enums::Align::Inside);
        (Some(make), Some(import), None, None)
    } else {
        // Unlocking lives in Signing, deliberately: it is the act that puts a
        // key in memory, and it belongs beside the window that says what can
        // then be signed with it.
        chrome::label(16, 180, 548, 16, "unlock it in Signing when a game asks for a signature",
                      theme::DIM, 11);
        // The website pairing (`docs/client/SIGN-IN.md`). TYPE the code — the
        // sentence below is the anti-phish wall, said where the field is.
        chrome::label(16, 206, 548, 16,
                      "sign a website in — type the code the page shows:",
                      theme::MUTED, 11);
        let mut code = input::Input::new(16, 226, 150, 26, None);
        code.set_text_size(13);
        let mut b = chrome::button(178, 226, 190, 26, "Sign the browser in…", Tone::Plain);
        b.set_align(enums::Align::Center | enums::Align::Inside);
        chrome::label(16, 256, 548, 14,
                      "type it yourself, off your own screen — nobody legitimate sends you a code",
                      theme::DIM, 10);
        (None, None, Some(code), Some(b))
    };

    chrome::label(16, 282, 548, 16, "scry never sees a key.", theme::INK, 12);
    chrome::label(16, 302, 548, 14,
                  "An account made here is generated on this machine and transmitted nowhere.",
                  theme::MUTED, 11);
    chrome::label(16, 320, 548, 14,
                  "An address is a claim, not a login — anything that matters asks for a signature.",
                  theme::MUTED, 11);
    chrome::label(16, 344, 548, 14,
                  "Delete this program and you still own everything you bought.",
                  theme::DIM, 11);
    // The sentence a holder needs BEFORE they rely on the file, not after they
    // have lost it. There is no reset and no copy anywhere else.
    chrome::label(16, 366, 548, 14,
                  "The keystore is the only copy of your key. Back it up; no passphrase can be reset.",
                  theme::GOLD, 11);
    w.end();
    AccountWindow { window: w, make, import, address: addr_line, note: note_line,
                    signin_code, signin }
}

/// Signing — which backend signs, and what that means.
///
/// The five rungs of `CLAUDE.md`'s ladder do not collapse into "a signer", so
/// this window names the one in use and what it can actually do.
/// Cloned handles share the widget — see [`AccountWindow`].
#[derive(Clone)]
pub struct SigningWindow {
    pub window: window::Window,
    /// Present only for the one backend that can be unlocked — `local`. The
    /// others hold no key here, so there is nothing an unlock could open, and
    /// a button that did nothing would be the dead control this menu already
    /// refuses elsewhere.
    pub unlock: Option<button::Button>,
    /// Unlock's inverse, present alongside it: forget the key NOW, without
    /// waiting for the idle relock or closing the window. The button a
    /// player presses before walking away from a shared desk.
    pub lock: Option<button::Button>,
    /// The status line, refreshed after an unlock rather than rebuilt.
    pub status: frame::Frame,
}

/// `unlockable` is separate from `kind` on purpose: the backend is `local`
/// whether or not a keystore exists yet, and offering to unlock a key that has
/// not been made would be the dead control this menu refuses everywhere else.
pub fn signing(kind: &str, status: &str, address: Option<&str>, unlockable: bool) -> SigningWindow {
    let mut w = window::Window::new(200, 200, 620, 320, None);
    w.set_label("scry — signing");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "signing", theme::HEAD, 16);

    let well = chrome::well(16, 40, 588, 150);
    chrome::label(28, 56, 560, 18, &format!("backend: {kind}"), theme::INK, 14);
    let status_line = chrome::label(28, 80, 560, 16, status, theme::MUTED, 11);
    match address {
        Some(a) => chrome::label(28, 106, 560, 16, &format!("signs as {a}"), theme::MUTED, 11),
        None => chrome::label(28, 106, 560, 16, "no address known for this backend",
                              theme::DIM, 11),
    };
    // Gold, not green: a configured signer is not an armed one, and only an
    // act that moves money earns the reserved fill.
    let tone = if kind == "none" { Tone::Gold } else { Tone::Plain };
    let (unlock, lock) = if unlockable {
        let mut b = chrome::button(456, 138, 132, 26, "Unlock…", Tone::Plain);
        b.set_align(enums::Align::Center | enums::Align::Inside);
        // Beside its inverse, plain like it: locking is an ordinary act too,
        // and the one a player reaches for before walking away.
        let mut l = chrome::button(312, 138, 132, 26, "Lock now", Tone::Plain);
        l.set_align(enums::Align::Center | enums::Align::Inside);
        (Some(b), Some(l))
    } else {
        let mut test = chrome::button(456, 138, 132, 26, "Nothing to test", tone);
        test.set_align(enums::Align::Center | enums::Align::Inside);
        test.deactivate();
        (None, None)
    };
    well.end();

    chrome::label(16, 202, 588, 16, "what this program can and cannot do", theme::DIM, 11);
    for (i, line) in [
        "it never holds a key you did not put on this machine yourself",
        "a game may ask it for a signature; a game may never operate it",
        "consent is per game, per family, counted, and dies when this closes",
        "an unlocked key relocks itself when idle; Lock now forgets it at once",
    ]
    .iter()
    .enumerate()
    {
        chrome::label(16, 222 + (i as i32 * 18), 588, 14, line, theme::MUTED, 11);
    }
    w.end();
    SigningWindow { window: w, unlock, lock, status: status_line }
}

/// The consent prompt, plus the controls that answer it.
#[derive(Clone)]
pub struct ConsentWindow {
    pub window: window::Window,
    /// One button per [`crate::consent::ALLOWANCES`] entry, **in that order**,
    /// so a caller pairs them by index rather than by re-parsing a label.
    pub allow: Vec<button::Button>,
    pub refuse: button::Button,
}

/// Consent — the one window a game can cause to open, and the only place a
/// signature is ever approved.
///
/// ⚠ **A game may draw a window that looks exactly like this one**
/// (`docs/client/SDK.md` §2b). It owns its own pixels, so nothing on screen can prove
/// to a player which process painted it, and no amount of care here changes
/// that. What holds instead is that a forgery buys nothing: there is no
/// consent token anywhere in the protocol for a game to collect, no way to
/// mint one, and no reply this launcher accepts that says "already approved".
/// A game that fakes this window gets a click and no signature.
///
/// The parameters are measurements the caller took, not questions this window
/// answers — `signs_as` is the account's address, `unlocked` is whether it can
/// actually sign right now. Both are shown because a prompt that hides them
/// asks the player to approve into the dark.
pub fn consent(
    ask: &crate::consent::Ask,
    signs_as: Option<&str>,
    unlocked: bool,
) -> ConsentWindow {
    use fltk::text;

    // ── the layout, computed from the message ───────────────────────────────
    //
    // The well fits its content between a floor and a ceiling, which is the
    // same shape `games` uses and for the same reason at both ends: acres of
    // empty olive under a five-line message read as a window that failed to
    // draw something, and an unbounded well would push the buttons off the
    // screen for a message a game chose the length of. Past the ceiling it
    // scrolls, which is the honest way to show more than fits.
    //
    // ⚠ The per-line figure is generous on purpose. A `TextDisplay` sized to
    // the exact text height puts up a scrollbar anyway — it counts its own
    // margins — and a scrollbar on a message with nothing below it tells the
    // player there is more of the thing they are about to sign. In this window
    // that is not a cosmetic slip; it is the well implying it is hiding
    // something.
    let lines = ask.text.lines().count().max(1) as i32;
    let well_h = (lines * 16 + 24).clamp(84, 186);
    let (well_y, pad) = (102, 16);
    let eip_y = well_y + well_h + 6;
    let state_y = eip_y + 20;
    let btn_y = state_y + 32;
    let note_y = btn_y + 42;
    let win_h = note_y + 48;

    let mut w = window::Window::new(240, 160, 620, win_h, None);
    w.set_label("scry — a game wants to sign");
    w.set_color(theme::BG);

    chrome::label(pad, 12, 500, 20, "a game wants a signature", theme::HEAD, 16);
    // The launcher's own sentence — but it quotes the game's name and family
    // inside it, so it is drawn clipped like anything else carrying a value
    // this program did not choose.
    chrome::label_untrusted(pad, 38, 588, 18, &ask.summary(), theme::INK, 13);
    // The game's stated reason, attributed in the words "it says" so it can
    // never read as the launcher's own. `Ask::claim` has already flattened it
    // to one line; see `consent::one_line` for what that is guarding against.
    match ask.claim() {
        Some(claim) => chrome::label_untrusted(pad, 58, 588, 16, &claim, theme::MUTED, 11),
        None => chrome::label(pad, 58, 588, 16, "it gave no reason", theme::DIM, 11),
    };

    // ── the message, verbatim ────────────────────────────────────────────────
    //
    // ⚠ **A `TextDisplay` rather than labels, and that is a safety choice.**
    // The text is unbounded and comes from the game, so a fixed well that
    // SCROLLS is the only shape where a thousand-line message cannot push the
    // buttons off the bottom of the window — and a prompt whose Refuse has
    // been pushed out of reach is one the player answers by guessing at the
    // window manager. Wrapped at the bounds for the same reason horizontally.
    //
    // Monospace because this is bytes about to be signed, not prose: a
    // proportional font hides trailing spaces and makes homoglyph padding
    // easier to miss.
    chrome::label(pad, 84, 588, 14, "the whole of what would be signed", theme::DIM, 11);
    let well = chrome::well(pad, well_y, 588, well_h);
    let mut body = text::TextDisplay::new(pad + 4, well_y + 4, 580, well_h - 8, None);
    let mut buf = text::TextBuffer::default();
    buf.set_text(&ask.text);
    body.set_buffer(buf);
    body.set_color(theme::WELL);
    body.set_text_color(theme::INK);
    body.set_text_font(enums::Font::Courier);
    body.set_text_size(12);
    body.set_frame(enums::FrameType::NoBox);
    body.wrap_mode(text::WrapMode::AtBounds, 0);
    well.end();

    // What a signature over this can and cannot do. The EIP-191 claim is
    // structural rather than a promise — the 0x19 prefix is what makes such a
    // message unusable as a transaction — so it is safe to state flatly.
    chrome::label(pad, eip_y, 588, 14,
                  "an EIP-191 message moves no money — signing one can never be a transaction",
                  theme::MUTED, 11);

    // Whether a grant would even produce a signature. Said before the buttons,
    // because a player allowing into a locked account should learn it here
    // rather than from a game's error message.
    let (state, ink) = match (signs_as, unlocked) {
        (Some(a), true) => (format!("signs as {a}"), theme::MUTED),
        (Some(_), false) => (
            "the account is locked — allowing will not produce a signature until you \
             unlock it in Signing"
                .to_string(),
            theme::GOLD,
        ),
        (None, _) => (
            "no account on this machine — allowing will not produce a signature".to_string(),
            theme::GOLD,
        ),
    };
    chrome::label(pad, state_y, 588, 16, &state, ink, 11);

    // ── the answers ─────────────────────────────────────────────────────────
    //
    // Refuse sits apart on the left, away from the row of allowances, because
    // the two answers are not variations of each other. `Tone::Plain`
    // throughout: a green fill is reserved for an act that moves money
    // (`theme::Tone`), and this one provably does not.
    let mut refuse = chrome::button(pad, btn_y, 132, 30, "Refuse", Tone::Plain);
    refuse.set_align(enums::Align::Center | enums::Align::Inside);

    let mut allow = Vec::new();
    let mut x = 604 - 132;
    // Right to left, so the widest grant is furthest from Refuse. Reversed on
    // the way in so the returned vector still matches `ALLOWANCES` order.
    for (label, _) in crate::consent::ALLOWANCES.iter().rev() {
        let mut b = chrome::button(x, btn_y, 128, 30, label, Tone::Plain);
        b.set_align(enums::Align::Center | enums::Align::Inside);
        allow.push(b);
        x -= 136;
    }
    allow.reverse();

    for (i, line) in [
        "a grant is counted, and only for this game and this kind of message",
        "there is no \"always\" — every grant is forgotten when the launcher closes",
    ]
    .iter()
    .enumerate()
    {
        chrome::label(pad, note_y + (i as i32 * 18), 588, 14, line, theme::DIM, 11);
    }

    w.end();
    // Modal: the door thread is parked on this answer, and a prompt a player
    // can lose behind the main window is one they cannot answer.
    w.make_modal(true);
    ConsentWindow { window: w, allow, refuse }
}
