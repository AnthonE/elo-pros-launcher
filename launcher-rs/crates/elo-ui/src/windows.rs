//! The windows off the main menu.
//!
//! Multi-window like the original client: each screen is its own top-level
//! rather than a page swapped into one frame. That is the 2003 shape and it is
//! also the honest one for a launcher — a player can leave Games open while
//! the Store downloads.
//!
//! **These build a window and return it.** None of them run an event loop, and
//! none of them read the network or the disk on their own: a caller passes in
//! what `elo-depot` already measured. That keeps every fact on screen
//! traceable to the crate whose tests cover it, and it is what makes these
//! constructible in a test with no display and no games root.

use crate::art::{self, Icon};
use crate::chrome;
use crate::theme::{self, Tone};
use fltk::{browser, button, enums, frame, image, input, menu, prelude::*, text, window};
use elo_depot::{shardlist::Shard, Install};

/// The mark — the same tile the site serves at `/favicon.svg`. Compiled in rather
/// than read from `watchtower/`, because the public repo carries `launcher-rs/`
/// without the site; `build_release.py` refuses to package when this copy and
/// the site's drift, which is the same weld the masthead CSS is under
/// (`test_site.py`, "the mark").
///
/// The file was `orb.svg` until 2026-08-20, when the green glass orb it held was
/// replaced by the Elo Pros seat the collection uses as its own face. The name
/// moved with the picture: a constant called `ORB` holding a gold creature is
/// the kind of thing a reader trusts and should not. Since 2026-10-12 it is a
/// lime tile with two chevrons, drawn by `watchtower/art/make_logo.py`, which
/// writes this file and the favicon from one string.
const MARK: &str = include_str!("../assets/mark.svg");

/// Dress a top-level so the desktop knows whose window it is: the mark as the
/// icon (dock, switcher, title bar), and `elo` as the WM class so a desktop
/// can match the live window back to `elo.desktop` (`StartupWMClass=elo` —
/// without it GNOME shows a running window with a blank gear where the menu
/// icon is). One call per window, because FLTK's default-icon static has no
/// Rust binding. A window whose icon failed to parse is still a working
/// window, so the parse is allowed to fail quietly — it cannot, for a
/// compiled-in file, but the window must not hinge on that.
fn mark(w: &mut window::Window) {
    w.set_xclass("elo");
    if let Ok(mut mark) = image::SvgImage::from_data(MARK) {
        // Rasterised here rather than handed over as a vector: `set_icon` gives
        // the pixels to the window manager, so something has to choose a size,
        // and 48 is the tile every desktop in this client's range asks for.
        mark.scale(48, 48, true, true);
        w.set_icon(Some(mark));
    }
}

/// One row on the shelf, already measured. The window renders this and
/// computes nothing from it.
///
/// `stale` is an `Option` on purpose and the third state is the point:
/// `None` means **we could not look**, which is not the same as up to date and
/// must not render as it. Same rule as `net::Fetched.reachable` in the Python
/// client and `UpdateState::Unknown` in `elo-depot` — the repo's own trap is
/// a reader that returns the empty answer for both "nothing" and "no idea".
pub struct Row {
    pub slug: String,
    pub build: String,
    pub bytes: i64,
    pub digest: String,
    pub stale: Option<bool>,
    /// The title's own name, when the origin could be asked for one. An
    /// install knows only its slug, so this stays `None` on a machine that has
    /// never reached the catalog — and a row falls back to the slug rather
    /// than to a blank.
    pub name: Option<String>,
    /// The shelf art, already fetched and sniffed. `None` is a normal state
    /// and draws the placeholder (`art::icon_box`).
    pub icon: Option<Icon>,
    /// Why `stale` is `None`, when the caller knows. Never a guess — see
    /// [`Row::status_line`].
    pub why: Option<String>,
}

impl Row {
    /// Build a row from an install with no staleness check performed. The
    /// caller runs `elo_depot::check` and fills `stale` in; until it does,
    /// the honest answer is "unknown" and that is what this produces.
    pub fn from_install(i: &Install) -> Row {
        Row {
            slug: i.slug.clone(),
            build: i.build.clone(),
            bytes: i.total_bytes,
            digest: i.depot_digest.clone(),
            stale: None,
            name: None,
            icon: None,
            why: None,
        }
    }

    /// What the row is titled. The origin's name when there is one, the slug
    /// when there is not — never an empty line, and never an invented name.
    pub fn title(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.slug)
    }

    /// The line under the title. Three states, three sentences, and the third
    /// says which of the two it is not.
    ///
    /// ⚠ **The third one used to assert a reason it had not measured.** It read
    /// *"not checked — the origin was not reached"*, which was written for a
    /// startup that asked the origin nothing at all: the origin might be fine
    /// and the row still said it was not reached. It carries [`Row::why`] now
    /// when there is one, and claims nothing when there is not.
    pub fn status_line(&self) -> String {
        let size = human_bytes(self.bytes);
        match self.stale {
            Some(true) => format!("{} · an update is published", size),
            Some(false) => format!("{} · up to date", size),
            None => match &self.why {
                Some(w) => format!("{size} · not checked — {w}"),
                None => format!("{size} · not checked against the origin"),
            },
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
    /// offers first. `elo play` runs whatever is installed.
    pub fn action(&self) -> &'static str {
        match self.stale {
            Some(true) => "Update",
            _ => "Play",
        }
    }
}

/// How tall one library row is, in px. Set by the icon and not by the text:
/// the square is [`art::ROW_ICON`] and a row has to hold it with a hairline of
/// padding either side, or the art clips into the row above.
const ROW_H: i32 = art::ROW_ICON + 12;

/// How wide a shelf's scrollbar is allowed to be, in px — the figure every
/// row's right-hand column is laid out short of.
///
/// ⚠ **Reserved whether or not a bar is showing.** `chrome::shelf` only draws
/// one when the list overflows, and FLTK puts it INSIDE the well rather than
/// beside it — so a Verify button laid out to the well's edge is a button the
/// bar sits on top of the moment a tenth game is installed. Laying out short
/// of it costs a few pixels of margin on a short list and costs nothing on a
/// long one.
const BAR: i32 = 17;

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
pub const MENU: [(&str, &str, bool); 12] = [
    ("Games", "what is installed, and what is stale", true),
    ("Store", "the curated shelf", true),
    ("Pairs", "every market: price and 24h change", true),
    ("Orders", "limits, stops, trailing, DCA — run while elo is open", true),
    ("Skins", "your items: sell, cancel or take an offer on OpenSea", true),
    ("Chat", "talk with other players, live", true),
    ("Servers", "browse a title's shards", true),
    ("Friends", "who you play with, and where they are", true),
    ("Wallet", "your coins — see them, send them, receive", true),
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
    /// "up to date" (`CLAUDE.md` §traps, the same rule `elo check` keeps).
    pub version_line: frame::Frame,
    /// Streamer mode (`elo_depot::streamer`). Drawn checked when it is on;
    /// startup wires what pressing it does.
    pub streamer: button::CheckButton,
    /// The line beside it, which says what turning it on does and, after a
    /// press, that windows already drawn keep what they drew until a restart.
    pub streamer_line: frame::Frame,
    /// Game mode (`elo_depot::gamemode`), and the line saying what it does on
    /// this machine.
    pub gamemode: button::CheckButton,
    pub gamemode_line: frame::Frame,
}

/// What the streamer line says, given the switch.
pub fn streamer_note(on: bool, changed: bool) -> &'static str {
    match (on, changed) {
        (true, true) => "on — games get it now; restart elo to hide what is already drawn",
        (false, true) => "off — restart elo to show what it hid",
        (true, false) => "on — wallets and server addresses are hidden",
        (false, false) => "hides wallets and server addresses, in here and in games",
    }
}

/// What the game mode line says, given the switch and what it does here.
/// GOLD when it is on and doing nothing, because that wants an install.
pub fn gamemode_note(on: bool, effect: &elo_depot::gamemode::Effect) -> (&'static str, enums::Color) {
    use elo_depot::gamemode::Effect;
    match (on, effect) {
        (true, Effect::GameMode(_)) => ("on — games start through GameMode", theme::GOLD),
        (true, Effect::AboveNormal) => ("on — games start at above-normal priority", theme::GOLD),
        (true, Effect::Missing) => ("on, but GameMode is not installed — install `gamemode`",
                                    theme::GOLD),
        (false, Effect::GameMode(_)) => ("starts games through GameMode: CPU at full speed",
                                         theme::MUTED),
        (false, Effect::AboveNormal) => ("starts games at above-normal priority", theme::MUTED),
        (false, Effect::Missing) => ("needs GameMode — install your distro's `gamemode`",
                                     theme::MUTED),
        (_, Effect::Nothing) => ("does nothing on this system", theme::DIM),
    }
}

pub fn main_menu(has_account: bool, version: &str) -> MainMenu {
    // Sized to the content rather than to a round number — the first capture
    // had 150px of empty olive under the last row, which reads as a window
    // that failed to draw something rather than as a window that is finished.
    let height = 64 + (MENU.len() as i32 * 38) + 46 + 60 + if has_account { 0 } else { 20 };
    let mut w = window::Window::new(100, 100, 560, height, None);
    mark(&mut w);
    w.set_label("elo");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 22, "elo", theme::HEAD, 19);
    chrome::label(16, 34, 480, 18, "Elo Pros, on your desktop", theme::MUTED, 12);
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

    // Streamer mode: a switch rather than a window, so it sits under them.
    let on = elo_depot::streamer::on();
    let mut streamer = button::CheckButton::new(16, y + 4, 150, 22, "streamer mode");
    streamer.set_label_color(theme::INK);
    streamer.set_label_size(13);
    streamer.set_checked(on);
    let streamer_line = chrome::label(178, y + 6, 366, 18, streamer_note(on, false),
                                      if on { theme::GOLD } else { theme::MUTED }, 12);
    y += 30;

    // Game mode: the same kind of switch, for the next game started.
    let on = elo_depot::gamemode::on();
    let mut gamemode = button::CheckButton::new(16, y + 4, 150, 22, "game mode");
    gamemode.set_label_color(theme::INK);
    gamemode.set_label_size(13);
    gamemode.set_checked(on);
    let (note, ink) = gamemode_note(on, &elo_depot::gamemode::effect());
    let gamemode_line = chrome::label(178, y + 6, 366, 18, note, ink, 12);
    y += 30;

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

    // ⚠ This said *"the pass is a token"* until 2026-08-12. The pass is a
    // designed, undeployed platform product; the thing a player actually owns
    // is the COPY — the title's ticket, an ERC-721 (`TICKET.md` §0). Naming
    // the wrong object on the first screen of the client is how a stranger
    // learns the wrong word for what they bought.
    chrome::label(16, y + 12, 520, 16,
                  "nothing you own lives in here — your copies are tokens and items are on chain",
                  theme::DIM, 11);
    w.end();
    MainMenu { window: w, version_line, streamer, streamer_line, gamemode, gamemode_line }
}

/// The controls on one drawn Games row, handed back for a caller to wire.
///
/// ⚠ **This struct is the fix for a dead button, not a refactor.** Until
/// 2026-08-12 `games()` built its Play button as a local and dropped it, so
/// every row in the library was painted and pressable and did **nothing** —
/// the exact defect `MENU`'s `built` flag exists to prevent, one window in. A
/// screenshot could not see it and no test could reach it, because there was
/// nothing to reach. Handing the widgets back is what makes `wiring::wire_games`
/// possible at all.
#[derive(Clone)]
pub struct GameControls {
    pub slug: String,
    pub build: String,
    /// Play, or Update when a newer build is published — the label is
    /// [`Row::action`] and the caller reads it back to know which it wired.
    pub act: button::Button,
    /// Re-hash every file the depot names. Its own control, because verifying
    /// is the one thing this client can do that no other storefront's can.
    pub verify: button::Button,
}

/// The Games window and the controls on it.
pub struct GamesWindow {
    pub window: window::Window,
    /// One per drawn row, in row order. Empty when nothing is installed.
    pub rows: Vec<GameControls>,
    /// Re-read what is on disk and re-ask the origin what is published. The
    /// library was measured once at startup, so a game installed from the
    /// Store did not appear here until the next launch — a restart standing in
    /// for a read.
    pub refresh: button::Button,
}

/// The Games window — installed builds, and whether each is current.
///
/// Takes rows rather than a games root: the window does no I/O, so it can be
/// built in a test with no display, no disk and no origin.
pub fn games(rows: &[Row]) -> GamesWindow {
    // The well fits the rows, with a floor so an empty shelf is still a window
    // and not a slot, and a ceiling so a big library does not open a window
    // taller than the desktop. Same reason the menu is sized to its entries:
    // acres of empty olive read as a failed draw, not as a finished screen.
    //
    // ⚠ **The ceiling is a viewport now and not a cut.** It used to be both —
    // the well stopped at 520px and the rows kept being drawn down the window
    // past it — so the eleventh game was painted onto the footer and the
    // fifteenth onto nothing at all. `chrome::shelf` scrolls, so the clamp
    // decides how much is on screen and never how much exists.
    let well_h = (rows.len() as i32 * ROW_H + 16).clamp(96, 520);
    let mut w = window::Window::new(120, 120, 660, well_h + 100, None);
    mark(&mut w);
    w.set_label("elo — games");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "my games", theme::HEAD, 16);
    let mut refresh = chrome::button(516, 14, 128, 24, "Refresh", Tone::Plain);
    refresh.set_align(enums::Align::Center | enums::Align::Inside);
    refresh.set_tooltip("re-read what is installed, and ask the origin what is published");

    let mut controls = Vec::new();
    let shelf = chrome::shelf(16, 40, 628, well_h);
    if rows.is_empty() {
        // The baseline restating itself, said once and plainly. Not an error
        // state and not styled as one.
        chrome::label(28, 56, 600, 18, "nothing installed yet", theme::MUTED, 13);
        chrome::label(28, 78, 600, 18,
                      "install one from the Store — it appears here with a Play button",
                      theme::DIM, 11);
    } else {
        let mut y = 48;
        for (i, row) in rows.iter().enumerate() {
            if i % 2 == 1 {
                let mut band = fltk::frame::Frame::new(18, y - 4, 624, ROW_H - 2, None);
                band.set_frame(enums::FrameType::FlatBox);
                band.set_color(theme::WELL_ALT);
            }
            // Centred on the two lines of text rather than hung off the top:
            // at 28px the square is shorter than the words beside it, which
            // is the opposite of what it was at 44.
            art::icon_box(26, y + 2, art::ROW_ICON, &row.slug, row.icon.as_ref());
            // A title's name and its build id both come from a depot somebody
            // else wrote, so both are clipped — see `chrome::label_untrusted`.
            chrome::label_untrusted(66, y, 290, 18, row.title(), theme::INK, 14);
            chrome::label_untrusted(66, y + 18, 340, 15, &row.status_line(),
                                    theme::MUTED, 11);
            // Laid out from the well's inner right edge (644) backwards, so
            // the bar's reservation is in the arithmetic and not in a pair of
            // hand-tuned numbers that would drift from it.
            let verify_x = 644 - BAR - 7 - 96;
            let act_x = verify_x - 104;
            let mut act = chrome::button(act_x, y + 4, 96, 26, row.action(), row.tone());
            act.set_align(enums::Align::Center | enums::Align::Inside);
            let mut verify = chrome::button(verify_x, y + 4, 96, 26, "Verify", Tone::Plain);
            verify.set_align(enums::Align::Center | enums::Align::Inside);
            verify.set_tooltip("re-check every installed file against the published build");
            controls.push(GameControls {
                slug: row.slug.clone(),
                build: row.build.clone(),
                act,
                verify,
            });
            y += ROW_H;
        }
    }
    shelf.end();

    chrome::label(16, well_h + 52, 600, 16,
                  "Verify re-checks every installed file against the published build",
                  theme::DIM, 11);
    w.end();
    grows_downward(&mut w, &shelf, 100 + ROW_H * 2);
    GamesWindow { window: w, rows: controls, refresh }
}

/// Let the player make a shelf window taller, and nothing else about it.
///
/// **The height is theirs and the width is ours.** A taller window is worth
/// having for the obvious reason — it is more rows, on a desktop big enough to
/// hold them — and it costs nothing, because `chrome::shelf` already handles
/// the difference between what exists and what fits. A *wider* one would cost
/// a layout: every row here is laid out in fixed pixels, so widening the
/// window would stretch the well and leave the buttons stranded mid-row with
/// olive behind them. Pinning the width is one line; reflowing a row on resize
/// is a different change, and this is not it.
///
/// `min_h` is the window's floor — enough for the chrome plus a row or two, so
/// a window dragged shut still shows what it is.
fn grows_downward(w: &mut window::Window, shelf: &fltk::group::Scroll, min_h: i32) {
    // The shelf absorbs the change. Everything under it — the footer lines —
    // moves down with the bottom edge rather than staying put and being
    // covered, which is FLTK's own rule for children below the resizable one.
    w.resizable(shelf);
    let width = w.width();
    w.size_range(width, min_h, width, 0);
}

/// The About window — and the one place the FLTK credit is shown to a human.
pub fn about() -> window::Window {
    let mut w = window::Window::new(140, 140, 520, 300, None);
    mark(&mut w);
    w.set_label("elo — about");
    w.set_color(theme::BG);

    chrome::label(16, 14, 400, 22, "elo", theme::HEAD, 18);
    chrome::label(16, 40, 480, 18, "the desktop client for Elo Pros", theme::MUTED, 12);

    // ⚠ This read *"It holds no key of yours."*, which stopped being true the
    // day the client could make an account (`CLAUDE.md`, the client's first
    // rule): while one is unlocked, this process holds a key. What is true is
    // where the key lives, and that is what the line says now.
    chrome::label(16, 78, 480, 18, "Your key stays on this machine.", theme::INK, 13);
    chrome::label(16, 100, 490, 16,
                  "An account made here is generated locally and sent nowhere.",
                  theme::MUTED, 11);
    chrome::label(16, 122, 490, 16,
                  "Delete this program and you still own everything you bought.",
                  theme::MUTED, 11);

    chrome::label(16, 168, 480, 16, "built on", theme::DIM, 11);
    chrome::label(16, 188, 490, 16, crate::FLTK_CREDIT, theme::MUTED, 11);

    w.end();
    w
}

/// What a copy of one title costs, as the origin actually answered.
///
/// **Three states, and the third is the one the repo's trap is about.**
/// `/api/ticket/{slug}` answers `ticketed: false` for a title with no ticket
/// contract — a real answer meaning *free to download* — and a client that
/// rendered a failed read as `Free` would be giving a game away on the strength
/// of a dropped packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Price {
    /// No ticket contract is configured, so there is nothing to buy. The
    /// origin's own words: *"free to download — nothing here is for sale"*.
    Free,
    /// A ticket contract is armed and a rail is open. `line` is the posted
    /// amount as the ORIGIN wrote it — this client never does the arithmetic
    /// on a price and never converts one.
    Posted { line: String },
    /// We could not read the ticket route. **Not free, and not for sale** —
    /// unknown, said out loud.
    Unknown { why: String },
}

/// One row on the store shelf, already measured.
///
/// Every field here is somebody else's text or somebody else's measurement.
/// Nothing on this struct is computed by the window that draws it, which is
/// what keeps the Store constructible in a test with no origin.
#[derive(Clone, Debug)]
pub struct Shelf {
    pub slug: String,
    pub name: String,
    pub blurb: String,
    /// The listing's own word — `live`, `in-build`. Rendered verbatim and
    /// never translated into a promise.
    pub state: String,
    /// Is a build for THIS platform published in a depot? A title can be
    /// `live` on the shelf and have nothing this machine can install.
    pub installable: bool,
    /// Is it already on this disk? Decides whether the row offers Install or
    /// points at the library.
    pub installed: bool,
    pub price: Price,
    pub icon: Option<Icon>,
}

impl Shelf {
    /// The money line under the blurb. One sentence, and it never says free
    /// for a read that failed.
    pub fn price_line(&self) -> String {
        match &self.price {
            Price::Free => "free to download — no copy is for sale yet".into(),
            Price::Posted { line } => format!("a copy costs {line}"),
            Price::Unknown { why } => format!("the price could not be read — {why}"),
        }
    }

    /// What the row's own control says, and what it will do.
    ///
    /// **The label and the act agree by construction**, which is the rule
    /// `tests/rows.rs` was written for after a row said *"an update is
    /// published"* over a Play button. A caller reads [`Act`] rather than
    /// re-parsing this string.
    ///
    /// A priced title with a build for this machine offers **Install**, not
    /// Buy: a copy is the licence to the title's servers, not the bytes, and a
    /// player who already holds one (every player is sent a free one) had no
    /// way to install from this window at all. A depot that does want a grant
    /// answers 401, and Install says how to get one; buying stays one press
    /// away on Page….
    pub fn act(&self) -> Act {
        match (&self.price, self.installed, self.installable) {
            (_, true, _) => Act::Installed,
            (Price::Unknown { .. }, _, _) => Act::Page,
            (_, _, true) => Act::Install,
            (Price::Posted { .. }, _, false) => Act::Buy,
            (Price::Free, _, false) => Act::Unpublished,
        }
    }
}

/// What a shelf row's main control does. An enum rather than a label, because
/// the caller has to *do* one of these and reading the verb back off a string
/// is how a Buy button ends up installing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Fetch and hash-verify the build, here, now.
    Install,
    /// Open the title's buy box in a browser. **The buy is not in this
    /// client**: a purchase is a wallet signing a transaction, and this
    /// program holds no wallet and will not grow one (`TICKET.md` §8a — the
    /// launcher never renders an amount it computed).
    Buy,
    /// Already on this disk. Points at Games rather than offering a second
    /// copy.
    Installed,
    /// Nothing this machine can install yet. Drawn and not pressable — hiding
    /// it would make a listed title look unlisted.
    Unpublished,
    /// We could not read enough to offer either, so the honest control is the
    /// one that shows the player the page and lets them read it themselves.
    Page,
}

impl Act {
    pub fn label(self) -> &'static str {
        match self {
            Act::Install => "Install",
            Act::Buy => "Buy a copy…",
            Act::Installed => "Installed",
            Act::Unpublished => "No build yet",
            Act::Page => "Open the page…",
        }
    }

    /// The whole sentence, for the tooltip — where the label has room to say
    /// what it will actually do.
    ///
    /// Two of these are the reason the method exists rather than the three
    /// obvious ones: **Buy** opens a browser and moves no money in here, and
    /// **No build yet** is a title that is listed and has nothing this machine
    /// can run. A player who reads either verb as the other learns the client
    /// lied to them, and a five-word button has no room to prevent it.
    pub fn what_it_does(self) -> &'static str {
        match self {
            Act::Install => "download it, check every file, and add it to Games",
            Act::Buy => "open the buy box in your browser and pay from your wallet",
            Act::Installed => "already on this disk — start it from Games",
            Act::Unpublished => "listed, but there is no build for this computer yet",
            Act::Page => "the price couldn't be read here — see it on the title's page",
        }
    }

    /// **Nothing on this shelf wears the reserved green fill**, and that is
    /// deliberate rather than an omission. A solid green fill is reserved for
    /// an act that moves money (`theme::Tone`); pressing Buy here opens a
    /// browser, and the act that moves money happens in a wallet this client
    /// does not hold. Painting it green would promise a purchase this window
    /// cannot make.
    ///
    /// ⚠ `Unpublished` is `Plain` and drawn through `chrome::button_off`, not
    /// `Gold`. Gold is an OUTLINE for a control whose thing is not armed — and
    /// an outline plus FLTK's `deactivate()` renders as a washed-out pale box
    /// that reads as neither. Caught by looking at the capture. The repo
    /// already has the right shape for *present and plainly not pressable* and
    /// the Servers window's **Full** button uses it, so this does too.
    pub fn tone(self) -> Tone {
        Tone::Plain
    }

    pub fn pressable(self) -> bool {
        !matches!(self, Act::Unpublished)
    }
}

/// The controls on one drawn shelf row, handed back for a caller to wire.
#[derive(Clone)]
pub struct ShelfControls {
    pub slug: String,
    pub act: Act,
    /// The main control — install, buy, or open the page.
    pub button: button::Button,
    /// Always offered, whatever the main control says: reading about a game
    /// before deciding is the normal path through a store.
    pub page: button::Button,
}

/// The Store window and the controls on it.
pub struct StoreWindow {
    pub window: window::Window,
    pub rows: Vec<ShelfControls>,
    /// Re-read the catalog. **The shelf is fetched once at startup**, so
    /// without this a player whose wifi was down when the client opened is
    /// looking at "could not read" until they restart the program — which is
    /// the *"if we need the user to reboot"* complaint in its smallest form.
    pub refresh: button::Button,
}

/// The Store — the curated shelf.
///
/// **The catalog is the origin's, never this client's.** `catalog.py` says why:
/// a second hand-kept list would be a second count and one of them would always
/// be stale. So this renders what it was handed and says plainly when it was
/// handed nothing — which is a different sentence from "the shelf is empty".
///
/// ⚠ **There is no pass block here and there was one until 2026-08-12.** The
/// window opened on a panel headed THE PASS advertising *"buy once, in SCRY —
/// the library follows your wallet"*, which is a designed, undeployed,
/// platform-wide product (`GATES.md` §4) and **not what a player buys**. What
/// they buy is a **copy of one game**: the title's ticket, an ERC-721 that is
/// the licence to that title's official servers (`TICKET.md` §0). Leading the
/// store with the other object put the wrong noun on the one screen where a
/// buyer decides. The money sentence now lives per row, where the thing being
/// sold actually is, and it comes from `/api/ticket/{slug}` rather than from a
/// paragraph typed here.
pub fn store(rows: &[Shelf], reachable: bool, why: &str) -> StoreWindow {
    let well_h = (rows.len() as i32 * SHELF_H + 24).clamp(140, 520);
    let mut w = window::Window::new(140, 140, 700, well_h + 132, None);
    mark(&mut w);
    w.set_label("elo — store");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "the store", theme::HEAD, 16);
    chrome::label(16, 34, 480, 16, "one payment, one copy, and the copy is yours to resell",
                  theme::MUTED, 11);
    let mut refresh = chrome::button(556, 14, 128, 26, "Refresh", Tone::Plain);
    refresh.set_align(enums::Align::Center | enums::Align::Inside);
    refresh.set_tooltip("load the store again");

    let mut controls = Vec::new();
    // Scrolls, for the same reason the library does: the number of titles on
    // the shelf is the origin's to decide and the window has to hold any of
    // them (`chrome::shelf`).
    let shelf = chrome::shelf(16, 58, 668, well_h);
    if !reachable {
        // The repo's own trap, at the one place it would cost a player a game.
        chrome::label(28, 74, 640, 18, "couldn't load the store", theme::GOLD, 13);
        chrome::label(28, 96, 640, 16,
                      "the store didn't answer, so this is not an empty shelf",
                      theme::MUTED, 11);
        chrome::label_untrusted(28, 116, 640, 16, why, theme::DIM, 11);
        chrome::label(28, 140, 640, 16, "press Refresh to try again", theme::DIM, 11);
    } else if rows.is_empty() {
        chrome::label(28, 74, 640, 18, "nothing listed yet", theme::MUTED, 13);
        chrome::label(28, 96, 640, 16,
                      "the store answered and has no titles on it yet",
                      theme::DIM, 11);
    } else {
        let mut y = 72;
        for (i, row) in rows.iter().enumerate() {
            if i % 2 == 1 {
                let mut band = fltk::frame::Frame::new(18, y - 6, 664, SHELF_H - 4, None);
                band.set_frame(enums::FrameType::FlatBox);
                band.set_color(theme::WELL_ALT);
            }
            art::icon_box(26, y, art::SHELF_ICON, &row.slug, row.icon.as_ref());
            // ⚠ **Every one of these four is `label_untrusted` and that is the
            // overrun fix.** A name, a blurb, a state word and a price line all
            // come from the origin, and FLTK draws a label at the TEXT's
            // natural width — straight past the widget's box, over the button
            // beside it and off the edge of the window. Gates' real blurb is
            // 140 characters and did exactly that. A character cap is not a
            // substitute (see `chrome::label_untrusted`); clipping to the box
            // is the only bound that holds for any text.
            chrome::label_untrusted(80, y, 268, 18, &row.name, theme::INK, 14);
            // The listing's own word, beside the name rather than floating
            // under it: `live` and `in-build` are a property OF the title, and
            // a tag adrift in the middle of a row reads as a stray label.
            let state_ink = if row.state == "live" { theme::MUTED } else { theme::GOLD };
            let mut state = chrome::label_untrusted(352, y + 2, 88, 15, &row.state,
                                                    state_ink, 10);
            state.set_align(enums::Align::Right | enums::Align::Inside | enums::Align::Clip);
            chrome::label_untrusted(80, y + 19, 360, 15, &row.blurb, theme::MUTED, 11);
            let price_ink = match row.price {
                Price::Unknown { .. } => theme::GOLD,
                _ => theme::DIM,
            };
            chrome::label_untrusted(80, y + 35, 360, 15, &row.price_line(), price_ink, 10);

            let act = row.act();
            // From the well's inner right edge (684) back, less the bar — see
            // the same arithmetic in `games`.
            let page_x = 684 - BAR - 12 - 84;
            let act_x = page_x - 8 - 110;
            // `button_off` for the one that cannot be pressed — see
            // `Act::tone`. Everything else is an ordinary pale button.
            let mut button = if act.pressable() {
                chrome::button(act_x, y + 8, 110, 28, act.label(), act.tone())
            } else {
                chrome::button_off(act_x, y + 8, 110, 28, act.label())
            };
            button.set_align(enums::Align::Center | enums::Align::Inside);
            button.set_tooltip(act.what_it_does());
            let mut page = chrome::button(page_x, y + 8, 84, 28, "Page…", Tone::Plain);
            page.set_align(enums::Align::Center | enums::Align::Inside);
            page.set_tooltip("open this title's page in your browser");
            controls.push(ShelfControls { slug: row.slug.clone(), act, button, page });
            y += SHELF_H;
        }
    }
    shelf.end();

    chrome::label(16, well_h + 68, 668, 16,
                  "a copy is an NFT and all sales are final — the exit is resale, never a refund",
                  theme::DIM, 11);
    chrome::label(16, well_h + 86, 668, 16,
                  "to buy a copy, press Page… — you pay from your wallet, in the browser",
                  theme::DIM, 11);
    w.end();
    grows_downward(&mut w, &shelf, 132 + SHELF_H * 2);
    StoreWindow { window: w, rows: controls, refresh }
}

/// How tall one store row is. Taller than a library row twice over: it carries
/// a money sentence the library does not, and its icon is the bigger of the
/// two ([`art::SHELF_ICON`] — a shelf is browsed, a library is scanned).
const SHELF_H: i32 = art::SHELF_ICON + 20;

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
    /// The canonical `elo://join/…` for this row, for the clipboard.
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
/// This window fetches nothing: a caller hands it what `elo-net` already
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
    mark(&mut w);
    w.set_label("elo — servers");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "servers", theme::HEAD, 16);

    let mut controls = Vec::new();
    // Scrolls. A popular title's list is the longest thing this client draws
    // and it is served by the game, so the count is nobody's to cap here.
    let well = chrome::shelf(16, 40, 608, well_h);
    match shards {
        Shards::Unpublished => {
            chrome::label(28, 56, 580, 18,
                          &format!("{slug} has no server list yet"), theme::MUTED, 13);
            chrome::label(28, 80, 580, 16,
                          "when the game publishes one, its servers show up here",
                          theme::DIM, 11);
        }
        // Two sentences, because the two failures have different owners: a
        // list we reached and could not parse is the title's bug, and one we
        // never reached is probably this machine's network.
        Shards::Unreadable { url, why, reachable } => {
            chrome::label(28, 56, 580, 18,
                          if *reachable { "the server list came back unreadable" }
                          else { "couldn't reach the server list" },
                          theme::GOLD, 13);
            chrome::label_untrusted(28, 80, 580, 16, url, theme::DIM, 11);
            // The reason comes from an origin we do not control.
            chrome::label_untrusted(28, 100, 580, 16, why, theme::MUTED, 11);
            chrome::label(28, 124, 580, 16,
                          "so this doesn't mean no servers are up — open Servers again to retry",
                          theme::DIM, 11);
        }
        Shards::Listed { url, rows } if rows.is_empty() => {
            chrome::label(28, 56, 580, 18,
                          &format!("no {slug} servers are up right now"), theme::MUTED, 13);
            chrome::label(28, 80, 580, 16,
                          "the list was read, and nobody is running one",
                          theme::DIM, 11);
            chrome::label_untrusted(28, 104, 580, 16, url, theme::DIM, 11);
        }
        Shards::Listed { rows, .. } => {
            let mut y = 52;
            for s in rows.iter() {
                // A name and a map come from a document a title serves, so
                // they are drawn clipped — see `chrome::label_untrusted`.
                chrome::label_untrusted(28, y, 250, 18, &s.name, theme::INK, 13);
                chrome::label(286, y, 70, 18, &s.population(),
                              if s.full() { theme::GOLD } else { theme::MUTED }, 12);
                chrome::label_untrusted(28, y + 18, 250, 14, &elo_depot::streamer::shown(&s.addr),
                                        theme::DIM, 10);
                if let Some(m) = &s.map {
                    chrome::label_untrusted(286, y + 18, 140, 14, m, theme::DIM, 10);
                }

                // A full shard is drawn and not pressable. Hiding it would
                // make a busy server look like one that does not exist.
                // From the well's inner right edge (624) back, less the bar.
                let invite_x = 624 - BAR - 13 - 74;
                let join_x = invite_x - 8 - 74;
                let mut join = if s.full() {
                    chrome::button_off(join_x, y + 2, 74, 26, "Full")
                } else {
                    chrome::button(join_x, y + 2, 74, 26, "Join", Tone::Plain)
                };
                join.set_tooltip("start your installed copy and connect it to this server");
                // Always offered, even on a full shard: sending a friend a
                // link to a server you cannot get into right now is normal.
                let mut copy = chrome::button(invite_x, y + 2, 74, 26, "Invite", Tone::Plain);
                copy.set_tooltip("copy a join link for this server — a friend opens it to join you");

                controls.push(ShardControls {
                    addr: s.addr.clone(),
                    // A row whose address the link writer refuses cannot be
                    // invited to. Empty rather than a broken string, and the
                    // caller deactivates on it — `parse` already refused the
                    // whole document for a bad address, so this is belt and
                    // braces rather than an expected state.
                    link: elo_depot::deeplink::link_for(slug, &s.addr).unwrap_or_default(),
                    join,
                    copy,
                });
                y += 46;
            }
        }
    }
    well.end();

    chrome::label(16, well_h + 50, 600, 16,
                  "Join starts your installed copy and connects it to that server",
                  theme::DIM, 11);
    chrome::label(16, well_h + 68, 600, 16,
                  "player counts come straight from each server",
                  theme::DIM, 11);
    w.end();
    grows_downward(&mut w, &well, 108 + 46 * 2);
    ServersWindow { window: w, rows: controls }
}

/// One market, as the origin's `/api/pairs` answered it. Numbers are what
/// the origin measured; a price it could not read is `None` and draws `—`.
#[derive(Debug, Clone)]
pub struct PairLine {
    pub key: String,
    pub label: String,
    pub price: Option<f64>,
    pub change_24h: Option<f64>,
    pub venue: String,
}

/// The Pairs window and the controls on it.
pub struct PairsWindow {
    pub window: window::Window,
    pub refresh: button::Button,
    /// `(pair key, its Open button)`, in list order.
    pub rows: Vec<(String, button::Button)>,
}

/// A price for one line: plain above 1, scientific below, `—` for unread.
pub fn pair_price(p: Option<f64>) -> String {
    match p {
        Some(v) if v.is_finite() && v > 0.0 => if v >= 1.0 { format!("{v:.4}") } else { format!("{v:.3e}") },
        _ => "—".into(),
    }
}

/// Pairs — every market on the store's chain, read when opened.
///
/// It fetches nothing itself: the caller hands it the rows `/api/pairs`
/// returned, or the reason it could not read them, which keeps it buildable
/// with no display and no origin. A list we could not read says so; it is
/// never drawn as "no markets".
pub fn pairs(rows: &[PairLine], reachable: bool, why: &str) -> PairsWindow {
    let well_h = (rows.len() as i32 * 34 + 24).clamp(120, 480);
    let mut w = window::Window::new(150, 150, 600, well_h + 116, None);
    mark(&mut w);
    w.set_label("elo — pairs");
    w.set_color(theme::BG);
    chrome::label(16, 12, 300, 20, "pairs", theme::HEAD, 16);
    let refresh = chrome::button(600 - 16 - 90, 10, 90, 26, "Refresh", Tone::Plain);

    let mut controls = Vec::new();
    let well = chrome::shelf(16, 44, 568, well_h);
    if !reachable {
        chrome::label(28, 60, 540, 18, "could not read the markets", theme::GOLD, 13);
        chrome::label_untrusted(28, 84, 540, 16, why, theme::MUTED, 11);
        chrome::label(28, 104, 540, 16, "this is not `no markets` — nobody knows the prices right now",
                      theme::DIM, 11);
    } else if rows.is_empty() {
        chrome::label(28, 60, 540, 18, "the origin lists no markets", theme::MUTED, 13);
    } else {
        chrome::label(28, 52, 200, 14, "pair", theme::DIM, 10);
        chrome::label(236, 52, 110, 14, "price, in the quote coin", theme::DIM, 10);
        chrome::label(356, 52, 70, 14, "24h", theme::DIM, 10);
        let mut y = 70;
        for r in rows {
            chrome::label_untrusted(28, y, 200, 18, &r.label, theme::INK, 13);
            chrome::label_untrusted(28, y + 16, 200, 12, &r.venue, theme::DIM, 10);
            chrome::label(236, y, 110, 18, &pair_price(r.price), theme::INK, 12);
            let (ch, ink) = match r.change_24h {
                Some(c) if c.is_finite() => (format!("{:+.1}%", c * 100.0),
                                             if c >= 0.0 { theme::INK } else { theme::GOLD }),
                _ => ("—".into(), theme::DIM),
            };
            chrome::label(356, y, 70, 18, &ch, ink, 12);
            let mut open = chrome::button(584 - BAR - 13 - 80, y, 80, 24, "Trade", Tone::Plain);
            open.set_tooltip("open this pair's chart and trade box in your browser");
            controls.push((r.key.clone(), open));
            y += 34;
        }
    }
    well.end();
    chrome::label(16, well_h + 52, 560, 16,
                  "Trade opens the pair in your browser, where your wallet is;",
                  theme::DIM, 11);
    chrome::label(16, well_h + 70, 560, 16,
                  "`elo trade` buys and sells from this machine's account, in a terminal",
                  theme::DIM, 11);
    w.end();
    grows_downward(&mut w, &well, 116 + 34 * 2);
    PairsWindow { window: w, refresh, rows: controls }
}

/// The Orders window's controls, handed back for `elo-gui` to wire.
pub struct OrdersWindow {
    pub window: window::Window,
    pub kind: menu::Choice,
    pub coin: input::Input,
    pub side: menu::Choice,
    pub amount: input::Input,
    pub a: input::Input,
    pub b: input::Input,
    pub c: input::Input,
    pub d: input::Input,
    pub cap: input::Input,
    pub hint: frame::Frame,
    pub place: button::Button,
    pub run: button::Button,
    pub status: frame::Frame,
    pub approve_once: button::CheckButton,
    pub check: button::CheckButton,
    pub allow_outside: button::CheckButton,
    /// `(order id, Remove, Resume when it is paused)`, in list order.
    pub rows: Vec<(u64, button::Button, Option<button::Button>)>,
}

/// The kinds, in the order the Choice lists them, with what each field means.
pub const ORDER_KINDS: [(&str, &str); 8] = [
    ("limit", "price: buy at it or under"),
    ("take-profit", "price: sell at it or over"),
    ("stop", "price: sell at it or under"),
    ("trailing-stop", "trail %: sell after this drop from the high"),
    ("trailing-buy", "trail %: buy after this rise from the low"),
    ("bracket", "take-profit, then stop: sell at either"),
    ("dca", "every (30m, 4h, 1d) and how many times; cap optional"),
    ("twap", "over (2h) and in how many parts; cap optional"),
];

/// Orders — the book on this machine, a form to add to it, and the runner.
///
/// It reads nothing itself: the caller hands it the book and the settings it
/// loaded, so it builds with no display and no origin. Orders fire only while
/// the runner is on, and the window says so in the button's own words.
pub fn orders(book: &[elo_net::orders::Order], settings: &elo_net::guard::Settings, running: bool,
              has_account: bool) -> OrdersWindow {
    let well_h = (book.len() as i32 * 34 + 24).clamp(80, 300);
    let win_h = 330 + well_h + 40;
    let mut w = window::Window::new(170, 140, 640, win_h, None);
    mark(&mut w);
    w.set_label("elo — orders");
    w.set_color(theme::BG);
    chrome::label(16, 12, 300, 20, "orders", theme::HEAD, 16);
    let mut run = chrome::button(640 - 16 - 170, 10, 170, 26,
                                 if running { "Stop orders" } else { "Start orders…" },
                                 if running { Tone::Plain } else { Tone::Live });
    run.set_tooltip("orders fire only while this is on; it asks your passphrase once");
    let status = chrome::label(16, 40, 608, 16,
        if running { "running: orders fire while elo is open" } else { "stopped: nothing fires until you start it" },
        if running { theme::INK } else { theme::GOLD }, 11);

    // the form
    chrome::label(16, 66, 150, 14, "order", theme::MUTED, 11);
    chrome::label(176, 66, 80, 14, "coin", theme::MUTED, 11);
    chrome::label(266, 66, 80, 14, "buy/sell", theme::MUTED, 11);
    chrome::label(356, 66, 268, 14, "amount (ETH to spend, or coins to sell)", theme::MUTED, 11);
    let mut kind = menu::Choice::new(16, 82, 150, 28, None);
    for (k, _) in ORDER_KINDS.iter() {
        kind.add_choice(k);
    }
    kind.set_value(0);
    let mut coin = field(176, 82, 80, 28, true);
    coin.set_value("ELO");
    let mut side = menu::Choice::new(266, 82, 80, 28, None);
    side.add_choice("buy");
    side.add_choice("sell");
    side.set_value(0);
    for c in [&mut kind, &mut side] {
        c.set_color(theme::BTN);
        c.set_text_color(theme::BTN_INK);
        c.set_text_size(13);
    }
    let amount = field(356, 82, 268, 28, true);
    let hint = chrome::label(16, 116, 608, 14, ORDER_KINDS[0].1, theme::MUTED, 11);
    let a = field(16, 134, 140, 28, true);
    let b = field(166, 134, 140, 28, true);
    let c = field(316, 134, 100, 28, true);
    let d = field(426, 134, 80, 28, true);
    let cap = field(516, 134, 108, 28, true);
    chrome::label(16, 164, 608, 14, "price · stop · every/over · times/parts · cap — fill what the line above names",
                  theme::DIM, 10);
    let mut place = chrome::button(16, 184, 160, 28, "Place the order", Tone::Plain);
    place.set_align(enums::Align::Center | enums::Align::Inside);

    // settings
    let check_box = |x: i32, y: i32, label: &str, on: bool| {
        let mut cb = button::CheckButton::new(x, y, 200, 22, None);
        cb.set_label(label);
        cb.set_label_color(theme::INK);
        cb.set_label_size(12);
        cb.set_checked(on);
        cb
    };
    let approve_once = check_box(16, 222, "approve a coin once", settings.approve_once);
    let check = check_box(226, 222, "check before signing", settings.check);
    let allow_outside = check_box(436, 222, "allow outside contracts", settings.allow_outside);

    // the book
    chrome::label(16, 254, 608, 14, "the book on this machine", theme::MUTED, 11);
    let well = chrome::shelf(16, 272, 608, well_h);
    let mut rows = Vec::new();
    if book.is_empty() {
        chrome::label(28, 286, 560, 16, "no orders yet — the form above places one", theme::DIM, 11);
    }
    let mut y = 282;
    for o in book {
        let st = &o.state;
        let state = if st.finished { "done".to_string() }
            else if let Some(p) = &o.paused { format!("paused: {p}") }
            else if st.done > 0 { format!("{} done", st.done) }
            else { "waiting".into() };
        chrome::label_untrusted(28, y, 400, 16, &format!("#{} {}", o.id, o.describe()), theme::INK, 12);
        chrome::label_untrusted(28, y + 16, 400, 12, &state, if o.paused.is_some() { theme::GOLD } else { theme::DIM }, 10);
        let rm_x = 624 - BAR - 13 - 70;
        let remove = chrome::button(rm_x, y, 70, 24, "Remove", Tone::Plain);
        let resume = o.paused.as_ref().map(|_| chrome::button(rm_x - 78, y, 70, 24, "Resume", Tone::Plain));
        rows.push((o.id, remove, resume));
        y += 34;
    }
    well.end();
    chrome::label(16, 280 + well_h, 608, 14,
                  "Each order is checked at its own size before it trades, and a failed trade pauses it.",
                  theme::DIM, 11);
    chrome::label(16, 296 + well_h, 608, 14,
                  "Your passphrase stays on this machine, masked in memory while orders run.",
                  theme::DIM, 11);
    if !has_account {
        run.deactivate();
        let mut s = status.clone();
        s.set_label("no account on this machine — Account makes one; the site runs orders with your browser wallet");
    }
    w.end();
    OrdersWindow { window: w, kind, coin, side, amount, a, b, c, d, cap, hint, place, run, status,
                   approve_once, check, allow_outside, rows }
}

/// One item this account holds, as the Skins window draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinLine {
    pub id: String,
    pub name: String,
    /// `Some((price shown, order hash))` while it is listed
    pub listed: Option<(String, String)>,
}

/// What the caller measured for the Skins window: no account, a read that
/// failed (never drawn as "you hold none"), or the rows.
pub enum Skins {
    NoAccount,
    Unreadable { why: String },
    Held { rows: Vec<SkinLine>, listings_why: Option<String> },
}

/// The Skins window's controls, handed back for `elo-gui` to wire.
pub struct SkinsWindow {
    pub window: window::Window,
    pub collection: menu::Choice,
    pub refresh: button::Button,
    pub market: button::Button,
    pub price: input::Input,
    pub days: menu::Choice,
    pub status: frame::Frame,
    pub rows: Vec<SkinRow>,
}

/// One item's buttons: Sell, Cancel while it is listed (with the listing's
/// order hash), Best offer and OpenSea.
pub struct SkinRow {
    pub id: String,
    pub sell: button::Button,
    pub cancel: Option<(String, button::Button)>,
    pub offer: button::Button,
    pub open: button::Button,
}

/// The collections the window can show, `(key, label)`, in Choice order.
pub const SKIN_COLLECTIONS: [(&str, &str); 3] = [("skins", "Gates skins"), ("pros", "Elo Pros"),
                                                  ("copies", "Gates copies")];
/// How long a listing runs, in days, in Choice order.
pub const LISTING_DAYS: [u32; 5] = [1, 3, 7, 30, 90];

/// Skins — what this account holds in one of our collections, with Sell,
/// Cancel, Best offer and OpenSea on each. It reads nothing itself.
pub fn skins(state: &Skins, collection: usize) -> SkinsWindow {
    let n = match state { Skins::Held { rows, .. } => rows.len() as i32, _ => 0 };
    let well_h = (n * 40 + 24).clamp(90, 360);
    let mut w = window::Window::new(180, 150, 640, well_h + 200, None);
    mark(&mut w);
    w.set_label("elo — skins");
    w.set_color(theme::BG);
    chrome::label(16, 12, 200, 20, "skins", theme::HEAD, 16);
    let mut col = menu::Choice::new(220, 10, 150, 26, None);
    for (_, l) in SKIN_COLLECTIONS {
        col.add_choice(l);
    }
    col.set_value(collection.min(SKIN_COLLECTIONS.len() - 1) as i32);
    let market = chrome::button(640 - 16 - 90 - 8 - 150, 10, 150, 26, "Market in browser", Tone::Plain);
    let refresh = chrome::button(640 - 16 - 90, 10, 90, 26, "Refresh", Tone::Plain);

    chrome::label(16, 44, 140, 14, "price, in ETH", theme::MUTED, 11);
    chrome::label(166, 44, 120, 14, "listed for", theme::MUTED, 11);
    let price = field(16, 60, 140, 28, true);
    let mut days = menu::Choice::new(166, 60, 120, 28, None);
    for d in LISTING_DAYS {
        days.add_choice(&format!("{d} day{}", if d == 1 { "" } else { "s" }));
    }
    days.set_value(2);
    for c in [&mut col, &mut days] {
        c.set_color(theme::BTN);
        c.set_text_color(theme::BTN_INK);
        c.set_text_size(13);
    }
    chrome::label(296, 66, 328, 16, "Sell lists the item at this price; you check it first.", theme::DIM, 11);
    let status = chrome::label(16, 96, 608, 16, "", theme::INK, 11);

    let well = chrome::shelf(16, 118, 608, well_h);
    let mut rows = Vec::new();
    match state {
        Skins::NoAccount => {
            chrome::label(28, 132, 560, 18, "no account on this machine", theme::GOLD, 13);
            chrome::label(28, 154, 560, 16, "Account makes one; the site sells with your browser wallet",
                          theme::DIM, 11);
        }
        Skins::Unreadable { why } => {
            chrome::label(28, 132, 560, 18, "could not read what this account holds", theme::GOLD, 13);
            chrome::label_untrusted(28, 154, 560, 16, why, theme::MUTED, 11);
            chrome::label(28, 174, 560, 16, "this is not `you hold none` — nobody looked", theme::DIM, 11);
        }
        Skins::Held { rows: lines, listings_why } => {
            if lines.is_empty() {
                chrome::label(28, 132, 560, 18, "this account holds none of these", theme::MUTED, 13);
            }
            let mut y = 128;
            for r in lines {
                let title = if r.name.is_empty() { format!("#{}", r.id) } else { format!("{} #{}", r.name, r.id) };
                chrome::label_untrusted(28, y, 250, 18, &title, theme::INK, 13);
                let sub = match &r.listed {
                    Some((p, _)) => format!("listed at {p} ETH"),
                    None => String::new(),
                };
                chrome::label_untrusted(28, y + 18, 250, 12, &sub, theme::GOLD, 10);
                let mut x = 624 - BAR - 13 - 76;
                let open = chrome::button(x, y, 76, 26, "OpenSea", Tone::Plain);
                x -= 84;
                let offer = chrome::button(x, y, 80, 26, "Best offer", Tone::Plain);
                let cancel = r.listed.as_ref().map(|(_, order)| {
                    x -= 74;
                    (order.clone(), chrome::button(x, y, 70, 26, "Cancel", Tone::Plain))
                });
                x -= 66;
                let sell = chrome::button(x, y, 62, 26, if r.listed.is_some() { "Relist" } else { "Sell" }, Tone::Live);
                rows.push(SkinRow { id: r.id.clone(), sell, cancel, offer, open });
                y += 40;
            }
            if let Some(why) = listings_why {
                let mut s = status.clone();
                s.set_label(&format!("couldn't read your listings: {why}"));
                s.set_label_color(theme::GOLD);
            }
        }
    }
    well.end();
    chrome::label(16, well_h + 126, 608, 14,
                  "A listing keeps the item in your wallet, and in game, until it sells.", theme::DIM, 11);
    chrome::label(16, well_h + 142, 608, 14,
                  "OpenSea takes its fee; we take none. Your passphrase is asked for each one.", theme::DIM, 11);
    w.end();
    SkinsWindow { window: w, collection: col, refresh, market, price, days, status, rows }
}

/// What the caller measured about a wallet's friends list.
///
/// The same three states as [`Shards`], for the same reason: a list nobody
/// could read is not an empty one. The fourth answer — read, and nobody on it —
/// is `Listed` with no rows, and draws its own sentence.
pub enum Friends {
    /// This machine has no wallet to read a list for: no account, no pairing.
    NoWallet,
    /// We asked and could not read the answer.
    Unreadable { url: String, why: String, reachable: bool },
    /// The list as the origin answered it. Its presence carries its own
    /// `reachable` — see `elo_net::friends::Presence`.
    Listed { view: elo_net::friends::View },
}

/// The controls on one drawn friend or buddy, handed back for a caller to
/// wire. Only a row with a join link gets one, because only it has anything
/// to follow.
#[derive(Clone)]
pub struct FriendControls {
    pub wallet: String,
    /// `elo://join/<slug>/<addr>`, already accepted by `deeplink::parse`.
    pub link: String,
    pub join: button::Button,
    pub copy: button::Button,
    /// A row from `buddies` — one of the wallet's own agents.
    pub buddy: bool,
}

/// One button that makes a signed change about one wallet: Accept and Decline
/// on an ask, Cancel on your own, Remove on a friend.
#[derive(Clone)]
pub struct FriendAct {
    /// The wire's op — `accept`, `decline`, `cancel` or `remove`.
    pub op: &'static str,
    pub wallet: String,
    /// What the row calls them, for the line that says what happened.
    pub called: String,
    pub button: button::Button,
}

/// The Friends window and the controls on it.
pub struct FriendsWindow {
    pub window: window::Window,
    /// Join and Invite, per row with a link.
    pub rows: Vec<FriendControls>,
    /// Every change a row offers, in the order drawn.
    pub acts: Vec<FriendAct>,
    /// A wallet or a name to ask, and the button that asks.
    pub add_input: input::Input,
    pub add: button::Button,
    /// One line under the add box: what the last change came to, or why not.
    pub status: frame::Frame,
    /// Whose list this is, when one was read — the wallet a change is made as.
    pub wallet: Option<String>,
    /// Read the list again. Presence moves while the window is open, and a
    /// place shown as current that nobody re-read is wall 3's cached balance.
    pub refresh: button::Button,
}

/// How tall a friend's row is, and an ask's, and a section heading's, in px.
const FRIEND_H: i32 = 46;
const ASK_H: i32 = 34;
const HEAD_H: i32 = 22;
/// Everything under the well: the add box, the status line and two notes.
const FRIENDS_FOOT: i32 = 160;

/// Friends — who you play with, where they are right now, and the changes.
///
/// Asks waiting on you come first, with **Accept** and **Decline**; then your
/// friends, with **Join** and **Invite** where their server is known and
/// **Remove**; then your own agents, marked, with Join and Invite and nothing
/// to remove — no ask made them and no change here touches them; then the asks
/// you made, with **Cancel**. The box under the list asks a wallet or a name.
/// Every change is one signature, made by the caller (`wiring::change_friend`)
/// — this window draws the buttons and decides nothing. `now` is the caller's
/// clock, so "last online" is a function of what was measured and not of when
/// the test ran.
pub fn friends(state: &Friends, now: i64) -> FriendsWindow {
    use elo_net::friends::short;
    let empty = Vec::new();
    let (asks_in, list, buddies, asks_out) = match state {
        Friends::Listed { view } => (&view.asks_in, &view.friends, &view.buddies, &view.asks_out),
        _ => (&empty, &Vec::new(), &Vec::new(), &empty),
    };
    let nothing = asks_in.is_empty() && list.is_empty() && buddies.is_empty() && asks_out.is_empty();
    let heads = [!asks_in.is_empty(), !buddies.is_empty(), !asks_out.is_empty()]
        .iter()
        .filter(|b| **b)
        .count() as i32;
    let content = (asks_in.len() + asks_out.len()) as i32 * ASK_H
        + (list.len() + buddies.len()) as i32 * FRIEND_H
        + heads * HEAD_H;
    let well_h = (content + 24).clamp(120, 520);
    // One step down the cascade from Servers, so the two never open stacked.
    // Wider than Servers by a button: a friend's row carries Remove too.
    let mut w = window::Window::new(170, 170, 720, well_h + FRIENDS_FOOT, None);
    mark(&mut w);
    w.set_label("elo — friends");
    w.set_color(theme::BG);

    chrome::label(16, 12, 100, 20, "friends", theme::HEAD, 16);
    if let Friends::Listed { view } = state {
        chrome::label(120, 14, 280, 18, &elo_depot::streamer::shown(&short(&view.wallet)),
                      theme::DIM, 11);
    }
    let mut refresh = chrome::button(576, 10, 128, 24, "Refresh", Tone::Plain);
    refresh.set_align(enums::Align::Center | enums::Align::Inside);
    refresh.set_tooltip("ask the origin where everyone is again");

    // From the well's inner right edge (704) back, less the bar — the Servers
    // window's arithmetic with one more button, so the columns line up.
    let third_x = 704 - BAR - 13 - 74;
    let second_x = third_x - 8 - 74;
    let first_x = second_x - 8 - 74;

    let mut controls = Vec::new();
    let mut acts = Vec::new();
    let well = chrome::shelf(16, 40, 688, well_h);
    match state {
        Friends::NoWallet => {
            chrome::label(28, 56, 660, 18, "no wallet on this machine yet", theme::MUTED, 13);
            chrome::label(28, 80, 660, 16,
                          "Account makes one, or pairs the wallet in your browser",
                          theme::DIM, 11);
            chrome::label(28, 98, 660, 16,
                          "until then, Ask below finishes the change on the website",
                          theme::DIM, 11);
        }
        Friends::Unreadable { url, why, reachable } => {
            chrome::label(28, 56, 660, 18,
                          if *reachable { "the friends list came back unreadable" }
                          else { "couldn't reach the friends list" },
                          theme::GOLD, 13);
            chrome::label_untrusted(28, 80, 660, 16, url, theme::DIM, 11);
            chrome::label_untrusted(28, 100, 660, 16, why, theme::MUTED, 11);
            chrome::label(28, 124, 660, 16,
                          "so this isn't an empty list — press Refresh to try again",
                          theme::DIM, 11);
        }
        Friends::Listed { .. } if nothing => {
            chrome::label(28, 56, 660, 18, "no friends yet", theme::MUTED, 13);
            chrome::label(28, 80, 660, 16,
                          "type their wallet or name below and press Ask",
                          theme::DIM, 11);
        }
        Friends::Listed { view } => {
            let mut y = 52;
            let mut act = |op: &'static str, x: i32, y: i32, label: &str, tip: &str,
                           wallet: &str, called: &str| {
                let mut b = chrome::button(x, y, 74, 26, label, Tone::Plain);
                b.set_tooltip(tip);
                acts.push(FriendAct { op, wallet: wallet.to_string(), called: called.to_string(),
                                      button: b });
            };
            if !asks_in.is_empty() {
                chrome::label(28, y, 660, 18, "asking to be friends", theme::GOLD, 11);
                y += HEAD_H;
                for a in asks_in {
                    let called = a.label();
                    chrome::label_untrusted(28, y + 4, 250, 18, &called, theme::INK, 13);
                    if !elo_depot::streamer::on() {
                        chrome::label(286, y + 6, 140, 16, &short(&a.wallet), theme::DIM, 10);
                    }
                    act("accept", second_x, y + 2, "Accept", "say yes — one signature",
                        &a.wallet, &called);
                    act("decline", third_x, y + 2, "Decline", "say no — one signature",
                        &a.wallet, &called);
                    y += ASK_H;
                }
            }
            let mut friend_row = |f: &elo_net::friends::Friend, y: i32,
                                  acts: &mut dyn FnMut(&str, &str)| {
                // A name is the friend's own and a title or server name is a
                // game's, so every one of them is drawn clipped.
                let agent = f.agent || f.buddy.is_some();
                let name = if agent { format!("{} (agent)", f.label()) } else { f.label() };
                chrome::label_untrusted(28, y, 250, 18, &name, theme::INK, 13);
                if f.name.is_some() && !elo_depot::streamer::on() {
                    chrome::label(286, y, 120, 18, &short(&f.wallet), theme::DIM, 10);
                }
                // Streamer mode names the game and never the server: where a
                // friend is playing is where a viewer would go looking.
                let line = match &f.playing {
                    Some(p) if elo_depot::streamer::on() && view.presence.reachable => {
                        format!("playing {}", p.title)
                    }
                    _ => f.presence_line(&view.presence, now),
                };
                let ink = if f.playing.is_some() && view.presence.reachable {
                    theme::INK
                } else {
                    theme::MUTED
                };
                chrome::label_untrusted(28, y + 18, first_x - 36, 14, &line, ink, 11);
                if let Some(link) = f.join_link().filter(|_| view.presence.reachable) {
                    let mut join = chrome::button(first_x, y + 2, 74, 26, "Join", Tone::Plain);
                    join.set_tooltip("start the installed build on the server they are on");
                    let mut copy = chrome::button(second_x, y + 2, 74, 26, "Invite", Tone::Plain);
                    copy.set_tooltip("copy their server's elo:// link to the clipboard");
                    controls.push(FriendControls {
                        wallet: f.wallet.clone(),
                        link: link.to_string(),
                        join,
                        copy,
                        buddy: f.buddy.is_some(),
                    });
                }
                acts(&f.wallet, &f.label());
            };
            for f in list {
                friend_row(f, y, &mut |wallet, called| {
                    act("remove", third_x, y + 2, "Remove",
                        "end the friendship, for both of you — one signature", wallet, called)
                });
                y += FRIEND_H;
            }
            if !buddies.is_empty() {
                chrome::label(28, y, 660, 18, "your agents — listed with no ask, and nothing here changes them",
                              theme::DIM, 11);
                y += HEAD_H;
                for b in buddies {
                    friend_row(b, y, &mut |_, _| {});
                    y += FRIEND_H;
                }
            }
            if !asks_out.is_empty() {
                chrome::label(28, y, 660, 18, "you asked — waiting on them", theme::DIM, 11);
                y += HEAD_H;
                for a in asks_out {
                    let called = a.label();
                    chrome::label_untrusted(28, y + 4, 250, 18, &called, theme::MUTED, 13);
                    if !elo_depot::streamer::on() {
                        chrome::label(286, y + 6, 140, 16, &short(&a.wallet), theme::DIM, 10);
                    }
                    act("cancel", third_x, y + 2, "Cancel", "take back your ask — one signature",
                        &a.wallet, &called);
                    y += ASK_H;
                }
            }
        }
    }
    well.end();

    // The add box. Plain, like every button here: a friendship moves no money.
    let y = 40 + well_h + 10;
    chrome::label(16, y, 600, 16, "ask someone to be friends — their wallet (0x…) or name:",
                  theme::MUTED, 11);
    let mut add_input = input::Input::new(16, y + 20, first_x - 24, 26, None);
    add_input.set_color(theme::WELL);
    add_input.set_text_color(theme::INK);
    add_input.set_text_size(13);
    add_input.set_selection_color(theme::SEL);
    add_input.set_cursor_color(theme::INK);
    let mut add = chrome::button(first_x, y + 20, 74, 26, "Ask", Tone::Plain);
    add.set_tooltip("ask them — one signature");
    let status = chrome::label_untrusted(16, y + 52, 688, 16, "", theme::MUTED, 11);

    // The notes say only what is true: a line about presence comes from a
    // list we read, and the public line is said where a change is made.
    let mut notes: Vec<(String, enums::Color)> = Vec::new();
    if let Friends::Listed { view } = state {
        if !view.presence.reachable {
            notes.push((
                format!(
                    "couldn't see what your friends are playing{}",
                    view.presence.why.as_deref().map(|w| format!(" — {w}")).unwrap_or_default()
                ),
                theme::GOLD,
            ));
        }
    }
    notes.push(("friends lists are public, like playtime".into(), theme::DIM));
    let mut y = y + 76;
    for (text, ink) in notes.iter().take(2) {
        chrome::label_untrusted(16, y, 688, 16, text, *ink, 11);
        y += 18;
    }
    w.end();
    grows_downward(&mut w, &well, FRIENDS_FOOT + 46 * 2);
    let wallet = match state {
        Friends::Listed { view } => Some(view.wallet.clone()),
        _ => None,
    };
    FriendsWindow { window: w, rows: controls, acts, add_input, add, status, wallet, refresh }
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
    /// The OTHER pairing, and the one that needs no account at all: lend the
    /// launcher the wallet already in a browser (`SIGN-IN.md` §8). Present in
    /// **both** states, because it is a third door for someone with no key and
    /// a second signer for someone who has one.
    pub pair: Option<button::Button>,
    /// Where the minted code is shown. It is shown HERE and typed THERE —
    /// the reflection of §1's rule, and the reason this is a frame the player
    /// reads rather than a link they press.
    pub pair_note: frame::Frame,
    /// The code row, present in both states. See [`PairRow`].
    pub pair_row: Option<PairRow>,
    /// Your profile picture and name. Shown when there is an account, and
    /// built hidden when there is not, so an account made in this window shows
    /// them in place. Both start empty and are filled from `/api/face`.
    pub face: Option<frame::Frame>,
    pub profile_name: Option<frame::Frame>,
    /// Change the name and picture the store, the games and chat show.
    /// Hidden, like the two above, until there is an account.
    pub edit_profile: Option<button::Button>,
    /// The origin the profile is read from, so an account made in this
    /// window can show its face without the window being built again.
    pub host: String,
}

/// Account — the address the launcher watches, and the two ways to get one.
///
/// ⚠ **An address, never authentication.** It is what the player asked the
/// launcher to watch; a game that needs it to matter asks for a signature and
/// verifies it. This window says that rather than assuming a reader knows it.
pub fn account(address: Option<&str>, host: &str) -> AccountWindow {
    let mut w = window::Window::new(180, 180, 580, 400, None);
    mark(&mut w);
    w.set_label("elo — account");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "account", theme::HEAD, 16);
    let well = chrome::well(16, 40, 548, 130);
    let (addr_line, note_line, face, profile_name) = match address {
        Some(a) => {
            chrome::label(28, 56, 452, 16, "the launcher is watching", theme::MUTED, 11);
            let addr = chrome::label(28, 76, 452, 18, &elo_depot::streamer::shown(a), theme::INK, 13);
            let note = chrome::label(28, 104, 452, 16, &format!("on {host}"), theme::DIM, 11);
            // The face sits in the well's right-hand corner, beside the
            // address it belongs to. A tile until `/api/face` answers.
            let mut face = frame::Frame::new(PROFILE_FACE_X, 52, PROFILE_FACE, PROFILE_FACE, None);
            face.set_frame(enums::FrameType::ThinDownBox);
            face.set_color(theme::WELL);
            let name = chrome::label_untrusted(28, 128, 452, 16, "", theme::MUTED, 11);
            (addr, note, Some(face), Some(name))
        }
        None => {
            let addr = chrome::label(28, 56, 452, 18, "no address set", theme::MUTED, 13);
            let note = chrome::label(28, 80, 452, 16,
                          "a normal state — games play anonymously and say so",
                          theme::DIM, 11);
            // Built hidden, so an account made here shows its face and name
            // in place — nobody restarts a program to gain a picture.
            let mut face = frame::Frame::new(PROFILE_FACE_X, 52, PROFILE_FACE, PROFILE_FACE, None);
            face.set_frame(enums::FrameType::ThinDownBox);
            face.set_color(theme::WELL);
            face.hide();
            let mut name = chrome::label_untrusted(28, 128, 452, 16, "", theme::MUTED, 11);
            name.hide();
            (addr, note, Some(face), Some(name))
        }
    };
    well.end();

    // Plain, never gold and never green: making a key on your own machine is
    // an ordinary act, and the reserved fill belongs to something that moves
    // money (`SITE-PLATFORM.md` §14b).
    let edit_profile;
    let (make, import, signin_code, signin, pair, pair_note,
         pair_row, base) = if address.is_none() {
        let mut make = chrome::button(16, 180, 168, 28, "Make an account…", Tone::Plain);
        make.set_align(enums::Align::Center | enums::Align::Inside);
        let mut import = chrome::button(196, 180, 168, 28, "Restore or import…", Tone::Plain);
        import.set_tooltip("your recovery words from another wallet, or a private key");
        import.set_align(enums::Align::Center | enums::Align::Inside);
        // ⚠ THE THIRD DOOR, and the reason this window had a gap: a player
        // whose wallet is a browser extension was offered two ways to make a
        // key and no way to say they already have one. Same row as the other
        // two because it is the same question — "how does this launcher sign
        // for me" — and not a lesser answer to it.
        let mut pair = chrome::button(376, 180, 188, 28, "Use my browser wallet…", Tone::Plain);
        pair.set_align(enums::Align::Center | enums::Align::Inside);
        // The code field is built in BOTH branches now. A player with no
        // account is the likeliest presser of "Use my browser wallet…" — that
        // door exists for exactly them — so building the copyable code only in
        // the has-an-account branch left the common case retyping again.
        let note = chrome::label(16, 214, 548, 16,
                      "the browser wallet keeps its key — this machine stores nothing that can sign",
                      theme::DIM, 11);
        // Under the note, so the reserved blank the hidden row leaves sits at
        // the bottom of this section rather than as a hole punched through the
        // middle of it.
        let row = pair_code_row(16, 240);
        // Where the two doors were, once an account exists.
        let mut edit = chrome::button(16, 180, 168, 28, "Edit profile…", Tone::Plain);
        edit.set_align(enums::Align::Center | enums::Align::Inside);
        edit.set_tooltip("the name and picture the store, the games and chat show for this wallet");
        edit.hide();
        edit_profile = Some(edit);
        (Some(make), Some(import), None, None, Some(pair),
         note, Some(row), 322)
    } else {
        // Unlocking lives in Signing, deliberately: it is the act that puts a
        // key in memory, and it belongs beside the window that says what can
        // then be signed with it.
        chrome::label(16, 180, 380, 16, "unlock it in Signing when a game asks for a signature",
                      theme::DIM, 11);
        let mut edit = chrome::button(404, 176, 160, 26, "Edit profile…", Tone::Plain);
        edit.set_align(enums::Align::Center | enums::Align::Inside);
        edit.set_tooltip("the name and picture the store, the games and chat show for this wallet");
        edit_profile = Some(edit);
        // The website pairing (`docs/client/SIGN-IN.md`). TYPE the code — the
        // sentence below is the anti-phish wall, said where the field is.
        chrome::label(16, 206, 548, 16,
                      "sign a website in — type the code the page shows:",
                      theme::MUTED, 11);
        // ⚠ This field was the ONE input in the client with no colours set, so
        // it drew as FLTK's stock white box in an olive window — the same
        // mismatch the stock dialogs were removed for on 2026-08-12, surviving
        // in the field this whole flow is typed into. Operator, 2026-08-23:
        // the sign-in "didnt seem to treat it like a first class citizen."
        let mut code = input::Input::new(16, 226, 150, 28, None);
        code.set_text_size(16);
        code.set_color(theme::WELL);
        code.set_text_color(theme::INK);
        code.set_selection_color(theme::SEL);
        let mut b = chrome::button(178, 226, 190, 28, "Sign the browser in…", Tone::Plain);
        b.set_align(enums::Align::Center | enums::Align::Inside);
        chrome::label(16, 256, 548, 14,
                      "type it yourself, off your own screen — nobody legitimate sends you a code",
                      theme::DIM, 10);
        // The reverse pairing, offered here too: having a key on this machine
        // is not a reason to be unable to sign with the one in your browser,
        // and the two doors answer opposite questions rather than competing.
        chrome::label(16, 282, 548, 16,
                      "or sign with the wallet in your browser instead:",
                      theme::MUTED, 11);
        let mut pair = chrome::button(16, 302, 240, 26, "Use my browser wallet…", Tone::Plain);
        pair.set_align(enums::Align::Center | enums::Align::Inside);
        // The code, then the warning about it — in that order. Drawn the other
        // way round the wall arrived before the thing it guards, and a reader
        // met "nobody will ever send you a code" above a blank space.
        let row = pair_code_row(16, 334);
        // Status only — "paired with 0x…". The anti-phish line belongs to the
        // row now, so this stays empty until there is something to report.
        let note = chrome::label(16, 402, 548, 16, "", theme::DIM, 11);
        (None, None, Some(code), Some(b), Some(pair),
         note, Some(row), 428)
    };

    chrome::label(16, base, 548, 16, "We never see your key.", theme::INK, 12);
    chrome::label(16, base + 20, 548, 14,
                  "An account made here is generated on this machine and transmitted nowhere.",
                  theme::MUTED, 11);
    chrome::label(16, base + 38, 548, 14,
                  "An address is a claim, not a login — anything that matters asks for a signature.",
                  theme::MUTED, 11);
    chrome::label(16, base + 62, 548, 14,
                  "Delete this program and you still own everything you bought.",
                  theme::DIM, 11);
    // The sentence a holder needs BEFORE they rely on the account, not after
    // they have lost it. There is no reset and no copy anywhere else.
    chrome::label(16, base + 84, 548, 14,
                  "Your recovery words are the backup. Keep them offline; no passphrase can be reset.",
                  theme::GOLD, 11);
    // Sized from the content rather than typed: the two branches are different
    // heights now, and a fixed 400 clipped the taller one's last line — the
    // sentence about the keystore being the only copy, which is the one line
    // in this window that must never be the one that falls off.
    w.set_size(580, base + 116);
    w.end();
    AccountWindow { window: w, make, import, address: addr_line, note: note_line,
                    signin_code, signin, pair, pair_note,
                    pair_row, face, profile_name, edit_profile, host: host.to_string() }
}

/// How big your face is in the Account window, and where it sits.
pub const PROFILE_FACE: i32 = 64;
const PROFILE_FACE_X: i32 = 492;

/// The profile editor and its controls.
#[derive(Clone)]
pub struct ProfileWindow {
    pub window: window::Window,
    pub preview: frame::Frame,
    pub name: input::Input,
    pub pick: button::Button,
    /// Which file is picked, or what a picture must be.
    pub picked: frame::Frame,
    /// Why Save will not work yet.
    pub status: frame::Frame,
    pub save: button::Button,
    pub cancel: button::Button,
}

/// Your profile — the name and picture the store, the games and chat show.
///
/// The words say what you get and what it costs, and stop: a picture and a
/// name, one signature by this account. `current` is the name already set,
/// drawn through the label escape like anything the origin returned.
pub fn profile_editor(current: Option<&str>) -> ProfileWindow {
    let mut w = window::Window::new(260, 220, 460, 252, None);
    mark(&mut w);
    w.set_label("elo — your profile");
    w.set_color(theme::BG);

    chrome::label(16, 12, 428, 22, "your profile", theme::HEAD, 16);
    chrome::label(16, 36, 428, 16, "the name and picture the store, the games and chat show",
                  theme::MUTED, 11);

    let mut preview = frame::Frame::new(16, 64, 96, 96, None);
    preview.set_frame(enums::FrameType::ThinDownBox);
    preview.set_color(theme::WELL);

    chrome::label(128, 62, 316, 16, "Name", theme::MUTED, 11);
    let mut name = input::Input::new(128, 80, 316, 28, None);
    name.set_color(theme::WELL);
    name.set_text_color(theme::INK);
    name.set_text_size(14);
    name.set_selection_color(theme::SEL);
    name.set_cursor_color(theme::INK);
    name.set_frame(enums::FrameType::ThinDownBox);
    name.set_maximum_size(crate::profile::NAME_MAX as i32);
    if let Some(n) = current {
        name.set_value(n);
    }

    let mut pick = chrome::button(128, 118, 180, 26, "Choose a picture…", Tone::Plain);
    pick.set_align(enums::Align::Center | enums::Align::Inside);
    let picked = chrome::label_untrusted(128, 148, 316, 16,
        "a square PNG, 64 to 512 px, up to 256 KB", theme::DIM, 11);

    let status = chrome::label_untrusted(16, 172, 428, 16, "", theme::GOLD, 11);

    let mut cancel = chrome::button(204, 206, 110, 30, "Cancel", Tone::Plain);
    cancel.set_align(enums::Align::Center | enums::Align::Inside);
    let mut save = chrome::button(334, 206, 110, 30, "Save…", Tone::Plain);
    save.set_align(enums::Align::Center | enums::Align::Inside);
    save.set_tooltip("one signature by this account; the passphrase is asked next");

    w.end();
    w.make_modal(true);
    ProfileWindow { window: w, preview, name, pick, picked, status, save, cancel }
}

/// The minted pairing code, in a row a player can act on rather than transcribe.
///
/// **One unit, shown and hidden together.** It is built hidden and switched on
/// by [`crate::wiring::wire_pair`] when a code actually exists: an empty
/// copyable field beside a Copy button is an offer to copy nothing, and a page
/// link with no code to carry sends someone to a form they cannot fill.
///
/// ⚠ Every piece belongs to the row, INCLUDING the two lines of prose. Hiding
/// the widgets and leaving the hint was the first bug the capture caught —
/// "your code — copy it" standing over an empty gap, which is every state
/// before the button is pressed. Grouping them is what stops that recurring:
/// there is one `show`, one `hide`, and nothing to forget.
#[derive(Clone)]
pub struct PairRow {
    pub hint: frame::Frame,
    /// The code, in a field the player can SELECT and COPY.
    ///
    /// ⚠ **This does not relax `SIGN-IN.md` §1, and the distinction is the
    /// whole reason it is allowed.** §1 forbids a code that ARRIVES — a deep
    /// link, a stranger's code pasted from a chat — because a code you did not
    /// mint yourself is the phish. A code this launcher minted, on this
    /// screen, carried by this player to a page they opened themselves, has
    /// exactly the property §1 protects: it came from nobody else. Copying is
    /// not clicking.
    ///
    /// Operator, 2026-08-23: *"i couldnt even copy my code."* It was a
    /// `frame::Frame` label, which FLTK cannot select, so the only way to move
    /// eight characters was to retype them off the screen.
    pub code: input::Input,
    pub copy: button::Button,
    /// Opens `{host}/pair.html` — the PAGE, never the code. A url carrying the
    /// code would be the deep link §1 refuses; a url carrying nothing is the
    /// address this note used to ask the player to retype into a browser bar.
    pub open: button::Button,
    /// The anti-phish wall, and it lives BELOW the code rather than above it.
    /// Drawn the other way round the warning arrived before the thing it
    /// guards, and a reader met "nobody will ever send you a code" over a
    /// blank space.
    pub warn: frame::Frame,
}

impl PairRow {
    pub fn show(&mut self) {
        self.hint.show();
        self.code.show();
        self.copy.show();
        self.open.show();
        self.warn.show();
    }
    pub fn hide(&mut self) {
        self.hint.hide();
        self.code.hide();
        self.copy.hide();
        self.open.hide();
        self.warn.hide();
    }
}

fn pair_code_row(x: i32, y: i32) -> PairRow {
    let hint = chrome::label(x, y, 548, 14,
                  "your code — copy it, then type or paste it on the page:",
                  theme::MUTED, 11);
    let mut code = input::Input::new(x, y + 18, 150, 28, None);
    code.set_text_size(16);
    code.set_readonly(true);
    // The code is the one thing on this window a player has to carry somewhere
    // else, so it is drawn in the ink the eye lands on and not the muted grey
    // the surrounding advice uses.
    //
    // ⚠ Both colours or neither. Setting the ink and leaving the background
    // was measured on 2026-08-23 as the WORST of the three states: INK is
    // light because it is meant for olive, so on FLTK's stock white field the
    // code came out washed to near-invisible — the least legible thing on a
    // window whose entire job was to show it.
    code.set_color(theme::WELL);
    code.set_text_color(theme::INK);
    code.set_selection_color(theme::SEL);
    let mut copy = chrome::button(x + 160, y + 18, 96, 28, "Copy", Tone::Plain);
    copy.set_align(enums::Align::Center | enums::Align::Inside);
    let mut open = chrome::button(x + 264, y + 18, 190, 28, "Open the pairing page", Tone::Plain);
    open.set_align(enums::Align::Center | enums::Align::Inside);
    let warn = chrome::label(x, y + 52, 548, 14,
                  "nobody legitimate will ever send you a code", theme::DIM, 11);
    let mut row = PairRow { hint, code, copy, open, warn };
    row.hide();
    row
}

/// Signing — which backend signs, and what that means.
///
/// The five rungs of `CLAUDE.md`'s ladder do not collapse into "a signer", so
/// this window names the one in use and what it can actually do.
/// Cloned handles share the widget — see [`AccountWindow`].
#[derive(Clone)]
pub struct SigningWindow {
    /// Present only for the one backend that can be unlocked — `local`. The
    /// others hold no key here, so there is nothing an unlock could open.
    ///
    /// ⚠ **`Some` does not mean pressable.** With `local` and no keystore yet
    /// the buttons are built and **deactivated**, and `wiring::adopt` activates
    /// them the moment an account exists. They used to be `None` in that state,
    /// which is why making an account ended with *"Restart the launcher to
    /// unlock it in Signing"* — the control could not be added to a window
    /// already built, so the player was sent to restart the program for a
    /// button. A window that can refresh itself beats a notice that asks for a
    /// reboot (operator, 2026-08-12).
    pub unlock: Option<button::Button>,
    /// Unlock's inverse, present alongside it: forget the key NOW, without
    /// waiting for the idle relock or closing the window. The button a
    /// player presses before walking away from a shared desk.
    pub lock: Option<button::Button>,
    /// The status line, refreshed after an unlock rather than rebuilt.
    pub status: frame::Frame,
    /// The lines that are only true with no account yet — "no account on
    /// this machine yet", "no address yet" — so making one in Account can
    /// take them down in place, along with turning Unlock on.
    pub until_account: Vec<frame::Frame>,
    pub window: window::Window,
}

/// What a signer kind is, in the words a player would use for it. The kind
/// itself (`local`, `browser-paired`) is a config value, not a sentence.
pub fn signer_name(kind: &str) -> &'static str {
    match kind {
        "local" => "the account on this machine",
        "browser" | "browser-paired" => "the wallet in your browser",
        "arca" => "the arca signing program",
        "external" => "your own signing command",
        "none" => "no signer set up",
        _ => "a signer this window does not name",
    }
}

/// `unlockable` is separate from `kind` on purpose: the backend is `local`
/// whether or not a keystore exists yet, and a key that has not been made
/// cannot be unlocked — so the controls are drawn deactivated with a sentence
/// saying why, rather than hidden or omitted.
pub fn signing(kind: &str, status: &str, address: Option<&str>, unlockable: bool) -> SigningWindow {
    let mut w = window::Window::new(200, 200, 620, 320, None);
    mark(&mut w);
    w.set_label("elo — signing");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "signing", theme::HEAD, 16);

    let well = chrome::well(16, 40, 588, 150);
    chrome::label(28, 56, 560, 18, signer_name(kind), theme::INK, 14);
    let status_line = chrome::label(28, 80, 560, 16, status, theme::MUTED, 11);
    let mut until_account = Vec::new();
    // The local signer's status already leads with the address; saying it
    // twice in a three-line well reads as two different accounts.
    match address {
        Some(a) if status.contains(&elo_depot::streamer::shown(a)) => {}
        Some(a) => {
            chrome::label(28, 106, 560, 16,
                          &format!("signs as {}", elo_depot::streamer::shown(a)),
                          theme::MUTED, 11);
        }
        None => {
            until_account.push(chrome::label(28, 106, 560, 16, "no address yet", theme::DIM, 11));
        }
    }
    // Gold, not green: a configured signer is not an armed one, and only an
    // act that moves money earns the reserved fill.
    let tone = if kind == "none" { Tone::Gold } else { Tone::Plain };
    let (unlock, lock) = if kind == "local" {
        let mut b = chrome::button(456, 138, 132, 26, "Unlock…", Tone::Plain);
        b.set_align(enums::Align::Center | enums::Align::Inside);
        // Beside its inverse, plain like it: locking is an ordinary act too,
        // and the one a player reaches for before walking away.
        let mut l = chrome::button(312, 138, 132, 26, "Lock now", Tone::Plain);
        l.set_align(enums::Align::Center | enums::Align::Inside);
        if !unlockable {
            // Built, off, and SAID — the menu's own rule, one window in. What
            // this buys over omitting them: `adopt` can turn them on in place
            // when an account is made, so nobody is asked to restart.
            b.deactivate();
            l.deactivate();
            until_account.push(chrome::label(28, 132, 270, 16,
                                             "no account on this machine yet — make one in Account",
                                             theme::GOLD, 11));
        }
        (Some(b), Some(l))
    } else {
        let mut test = chrome::button(456, 138, 132, 26, "Nothing to unlock", tone);
        test.set_align(enums::Align::Center | enums::Align::Inside);
        test.deactivate();
        (None, None)
    };
    well.end();

    chrome::label(16, 202, 588, 16, "how signing works", theme::DIM, 11);
    for (i, line) in [
        "a game can ask for a signature, and you approve it in a window elo opens",
        "an approval covers one game and one kind of message, for the count you pick",
        "every approval ends when elo closes",
        "an unlocked account locks itself when idle; Lock now locks it at once",
    ]
    .iter()
    .enumerate()
    {
        chrome::label(16, 222 + (i as i32 * 18), 588, 14, line, theme::MUTED, 11);
    }
    w.end();
    SigningWindow { window: w, unlock, lock, status: status_line, until_account }
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
    mark(&mut w);
    w.set_label("elo — a game wants to sign");
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
    // `shown`, never the raw text: a NUL would cut the buffer here
    // (`Ask::shown` says why).
    buf.set_text(&ask.shown());
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
                  "this signs a message, not a transaction — it cannot move money",
                  theme::MUTED, 11);

    // Whether a grant would even produce a signature. Said before the buttons,
    // because a player allowing into a locked account should learn it here
    // rather than from a game's error message.
    let (state, ink) = match (signs_as, unlocked) {
        (Some(a), true) => (format!("signs as {}", elo_depot::streamer::shown(a)), theme::MUTED),
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
    // A kind the broker never grants a count (`elo_broker::one_at_a_time`)
    // is offered one signature only: "Allow 25" there would be a button that
    // quietly means "Just this one".
    let counted = !elo_broker::one_at_a_time(&ask.family);
    let offers = if counted {
        &crate::consent::ALLOWANCES[..]
    } else {
        &crate::consent::ALLOWANCES[..1]
    };
    // Right to left, so the widest grant is furthest from Refuse. Reversed on
    // the way in so the returned vector still matches `ALLOWANCES` order.
    for (label, _) in offers.iter().rev() {
        let mut b = chrome::button(x, btn_y, 128, 30, label, Tone::Plain);
        b.set_align(enums::Align::Center | enums::Align::Inside);
        allow.push(b);
        x -= 136;
    }
    allow.reverse();

    for (i, line) in [
        if counted {
            "allowing covers only this game and this kind of message, for that many signatures"
        } else {
            "this kind of message is asked about every time, so allowing covers this one only"
        },
        "nothing is allowed for good — every approval ends when elo closes",
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

// ── the three windows this client used to borrow from the toolkit ───────────
//
// Operator, 2026-08-12: *"the passphrase screen doesnt match anything else."*
// It did not, and it could not: `dialog::password_default` and
// `dialog::message_default` draw FLTK's own stock dialog — a light grey box
// with a blue `?` glyph — over a client whose every other pixel is the 2003
// client's olive. Three windows below replace all three stock dialogs, and
// they are windows rather than a themed wrapper because a stock dialog cannot
// be reskinned: its colours, its icon and its layout are inside the C++ side.
//
// What that buys beyond looking right: a passphrase prompt that can say WHOSE
// account it is asking about, a notice that can carry a Restart button, and a
// lock screen that can refuse to go away.

/// A prompt with a masked field, plus the controls that answer it.
#[derive(Clone)]
pub struct AskWindow {
    pub window: window::Window,
    /// `SecretInput`, so the passphrase is never painted. This is also the one
    /// field in the client a private key is typed into (Import), which is why
    /// there is no unmasked variant of this window to reach for by mistake.
    pub field: input::SecretInput,
    pub ok: button::Button,
    pub cancel: button::Button,
    /// A place to say *"that passphrase did not open it"* without building a
    /// second window and losing what the player typed.
    pub status: frame::Frame,
}

/// Ask for a secret, in the client's own skin.
///
/// `prompt` is whatever the caller wants said. It may be one line (*"Passphrase:"*)
/// or many — the browser pairing puts the ENTIRE message being signed in here,
/// deliberately, so a player reads what they are signing in the same window
/// that asks for the passphrase to sign it. So a multi-line prompt is drawn in
/// a scrolling well and a one-line prompt is drawn as a label: an unbounded
/// prompt in a fixed window would otherwise push the field off the bottom, and
/// a prompt whose Cancel is out of reach is one answered by the window manager.
pub fn passphrase(title: &str, prompt: &str) -> AskWindow {
    use fltk::text;

    let lines: Vec<&str> = prompt.lines().collect();
    let multi = lines.len() > 1;
    let well_h = if multi {
        (lines.len() as i32 * 15 + 20).clamp(60, 220)
    } else {
        0
    };
    let field_y = if multi { 74 + well_h + 14 } else { 92 };
    let win_h = field_y + 104;

    let mut w = window::Window::new(260, 200, 520, win_h, None);
    mark(&mut w);
    w.set_label(&format!("elo — {title}"));
    w.set_color(theme::BG);

    chrome::label(16, 12, 480, 22, title, theme::HEAD, 16);

    if multi {
        chrome::label(16, 46, 488, 16, "read this before you type", theme::DIM, 11);
        let well = chrome::well(16, 68, 488, well_h);
        let mut body = text::TextDisplay::new(20, 72, 480, well_h - 8, None);
        let mut buf = text::TextBuffer::default();
        buf.set_text(prompt);
        body.set_buffer(buf);
        body.set_color(theme::WELL);
        body.set_text_color(theme::INK);
        // Monospace for the same reason the consent prompt uses it: this is
        // bytes about to be signed, and a proportional font hides trailing
        // spaces and makes homoglyph padding easier to miss.
        body.set_text_font(enums::Font::Courier);
        body.set_text_size(11);
        body.set_frame(enums::FrameType::NoBox);
        body.wrap_mode(text::WrapMode::AtBounds, 0);
        well.end();
        chrome::label(16, field_y - 22, 488, 16, "Passphrase", theme::MUTED, 11);
    } else {
        chrome::label_untrusted(16, 46, 488, 18, prompt, theme::INK, 13);
        chrome::label(16, field_y - 22, 488, 16,
                      "it is never shown, never stored, and never leaves this machine",
                      theme::DIM, 11);
    }

    let mut field = input::SecretInput::new(16, field_y, 488, 28, None);
    field.set_color(theme::WELL);
    field.set_text_color(theme::INK);
    field.set_text_size(14);
    field.set_frame(enums::FrameType::ThinDownBox);
    field.set_cursor_color(theme::INK);
    // Enter answers. A prompt whose only OK is a mouse target is one that
    // interrupts typing to be dismissed.
    //
    // ⚠ `EnterKeyAlways` and not `EnterKey`. The plain one fires only when the
    // value CHANGED since the last callback — so after a refused passphrase,
    // retyping the same thing and pressing Enter does nothing at all, which is
    // the exact moment a player is most likely to try it.
    field.set_trigger(enums::CallbackTrigger::EnterKeyAlways);

    let status = chrome::label(16, field_y + 32, 488, 16, "", theme::RED, 11);

    let mut cancel = chrome::button(16, field_y + 54, 120, 30, "Cancel", Tone::Plain);
    cancel.set_align(enums::Align::Center | enums::Align::Inside);
    // Plain, never green: typing a passphrase spends nothing and unlocks a key
    // that is already on this disk (`theme::Tone`).
    let mut ok = chrome::button(384, field_y + 54, 120, 30, "Continue", Tone::Plain);
    ok.set_align(enums::Align::Center | enums::Align::Inside);

    w.end();
    // Modal, because the caller is parked on the answer.
    w.make_modal(true);
    AskWindow { window: w, field, ok, cancel, status }
}

// ── the wallet ──────────────────────────────────────────────────────────────
//
// Operator, 2026-10-22: *"the launcher SHOULD be a super clean and secure
// wallet."* One window: who you are, what you hold, and a send. It draws and
// decides nothing; `wiring::wire_wallet` reads the chain off this thread and
// every check a send passes lives in `elo_net::wallet`.

/// The Wallet window's controls.
#[derive(Clone)]
pub struct WalletWindow {
    pub window: window::Window,
    /// The account's Robinhood Chain address.
    pub address: frame::Frame,
    pub copy: button::Button,
    pub explorer: button::Button,
    /// The same account's Solana address, or how to get one.
    pub sol_address: frame::Frame,
    pub sol_copy: button::Button,
    pub sol_explorer: button::Button,
    pub refresh: button::Button,
    /// One coin per line, monospace, so amounts line up on their decimal;
    /// a heading per chain.
    pub balances: text::TextDisplay,
    /// The account's own warning (an EIP-7702 delegation), or the reason the
    /// balances could not be read. Empty when there is nothing to say.
    pub warning: frame::Frame,
    /// Every coin on both chains, each named with its chain.
    pub coin: menu::Choice,
    pub amount: input::Input,
    pub to: input::Input,
    pub review: button::Button,
    pub status: frame::Frame,
    /// Between this account's two chains: which way, how much.
    pub route: menu::Choice,
    pub move_amount: input::Input,
    pub move_review: button::Button,
    pub move_status: frame::Frame,
}

fn field(x: i32, y: i32, w: i32, h: i32, mono: bool) -> input::Input {
    let mut f = input::Input::new(x, y, w, h, None);
    f.set_color(theme::WELL);
    f.set_text_color(theme::INK);
    f.set_selection_color(theme::SEL);
    f.set_cursor_color(theme::INK);
    f.set_text_size(13);
    f.set_frame(enums::FrameType::ThinDownBox);
    if mono {
        f.set_text_font(enums::Font::Courier);
    }
    f
}

/// Wallet — the account on this machine on both of its chains, what it holds
/// on each, a send, and a move between the two.
///
/// One account, two addresses, the way Phantom and MetaMask show one: the
/// recovery words make a Robinhood Chain address and a Solana address, the
/// balances list both, a send goes on the chain of the coin picked, and a
/// move takes SOL, ETH, USDC or USDG to this same account on the other chain.
///
/// With no account, everything that would act is built deactivated and says
/// where an account comes from: a hidden control and an unbuilt one look the
/// same, and only one of those is honest.
pub fn wallet(address: Option<&str>) -> WalletWindow {
    let mut w = window::Window::new(200, 120, 580, 728, None);
    mark(&mut w);
    w.set_label("elo — wallet");
    w.set_color(theme::BG);

    chrome::label(16, 12, 400, 20, "wallet", theme::HEAD, 16);
    let well = chrome::well(16, 40, 548, 76);
    let row = |y: i32, chain: &str, text: &str, live: bool| {
        chrome::label(28, y + 3, 120, 16, chain, theme::MUTED, 11);
        let mut f = chrome::label(146, y, 308, 22, text, if live { theme::INK } else { theme::MUTED },
                                  if live { 12 } else { 11 });
        if live {
            f.set_label_font(enums::Font::Courier);
        }
        let mut c = chrome::button(462, y, 94, 22, "Copy", Tone::Plain);
        c.set_align(enums::Align::Center | enums::Align::Inside);
        (f, c)
    };
    let (addr, mut copy) = row(48, elo_net::chain::NAME,
        &address.map(elo_depot::streamer::shown)
            .unwrap_or_else(|| "no account on this machine — Account makes one".into()),
        address.is_some());
    let (sol_address, mut sol_copy) = row(82, elo_vault::solana::NAME,
        if address.is_some() { "reading…" } else { "" }, false);
    copy.set_tooltip("to receive on Robinhood Chain: give this address to whoever is sending");
    sol_copy.set_tooltip("to receive on Solana: give this address to whoever is sending");
    well.end();

    let mut explorer = chrome::button(16, 124, 170, 26, "Robinhood explorer", Tone::Plain);
    let mut sol_explorer = chrome::button(194, 124, 150, 26, "Solana explorer", Tone::Plain);
    let mut refresh = chrome::button(444, 124, 120, 26, "Refresh", Tone::Plain);
    for b in [&mut explorer, &mut sol_explorer, &mut refresh] {
        b.set_align(enums::Align::Center | enums::Align::Inside);
    }

    chrome::label(16, 158, 548, 16, "what it holds, read off each chain", theme::MUTED, 11);
    let bwell = chrome::well(16, 178, 548, 150);
    let mut balances = text::TextDisplay::new(20, 182, 540, 142, None);
    balances.set_buffer(text::TextBuffer::default());
    balances.set_color(theme::WELL);
    balances.set_text_color(theme::INK);
    balances.set_text_font(enums::Font::Courier);
    balances.set_text_size(12);
    balances.set_frame(enums::FrameType::NoBox);
    bwell.end();
    let mut warning = chrome::label(16, 332, 548, 40, "", theme::GOLD, 11);
    warning.set_align(enums::Align::Left | enums::Align::Top | enums::Align::Inside | enums::Align::Wrap);

    chrome::label(16, 376, 400, 18, "send", theme::HEAD, 14);
    chrome::label(16, 398, 150, 14, "coin", theme::MUTED, 11);
    chrome::label(194, 398, 200, 14, "amount", theme::MUTED, 11);
    let mut coin = menu::Choice::new(16, 414, 170, 28, None);
    coin.set_color(theme::BTN);
    coin.set_text_color(theme::BTN_INK);
    coin.set_text_size(13);
    let amount = field(194, 414, 200, 28, true);
    chrome::label(16, 450, 548, 14, "to — paste the address, then check it (0x… on Robinhood Chain, \
                                    base58 on Solana)", theme::MUTED, 11);
    let to = field(16, 466, 548, 28, true);
    // Plain, not green: this button checks and shows. The green Send is on
    // the review, beside what it will sign (`theme::Tone`).
    let mut review = chrome::button(16, 504, 160, 30, "Review send…", Tone::Plain);
    review.set_align(enums::Align::Center | enums::Align::Inside);
    let status = chrome::label(186, 510, 378, 18, "", theme::MUTED, 11);

    chrome::label(16, 548, 548, 18, "move between your chains", theme::HEAD, 14);
    chrome::label(16, 570, 330, 14, "to your own address on the other chain", theme::MUTED, 11);
    chrome::label(354, 570, 200, 14, "amount", theme::MUTED, 11);
    let mut route = menu::Choice::new(16, 586, 330, 28, None);
    route.set_color(theme::BTN);
    route.set_text_color(theme::BTN_INK);
    route.set_text_size(12);
    for r in elo_net::xmove::ROUTES {
        // `/` is menu structure; the label is ours, so only the arrow changes
        route.add_choice(&r.label().replace('/', " "));
    }
    route.set_value(0);
    let move_amount = field(354, 586, 210, 28, true);
    let mut move_review = chrome::button(16, 624, 160, 30, "Review move…", Tone::Plain);
    move_review.set_align(enums::Align::Center | enums::Align::Inside);
    let move_status = chrome::label(186, 630, 378, 18, "", theme::MUTED, 11);

    chrome::label(16, 668, 548, 14,
                  "Every send and move is shown in full first, then asks your passphrase. Games can never send.",
                  theme::DIM, 11);
    chrome::label(16, 686, 548, 14,
                  "None of us will ever ask for your recovery words or passphrase.",
                  theme::DIM, 11);
    chrome::label(16, 704, 548, 14,
                  "ELO itself never moves between chains, and no rate between them is implied.",
                  theme::DIM, 11);

    for b in [&mut sol_copy, &mut sol_explorer] {
        b.deactivate();
    }
    if address.is_none() {
        for b in [&mut copy, &mut explorer, &mut refresh, &mut review, &mut move_review] {
            b.deactivate();
        }
        let (mut c, mut a, mut t, mut r, mut m) =
            (coin.clone(), amount.clone(), to.clone(), route.clone(), move_amount.clone());
        c.deactivate();
        a.deactivate();
        t.deactivate();
        r.deactivate();
        m.deactivate();
    }
    w.end();
    WalletWindow { window: w, address: addr, copy, explorer, sol_address, sol_copy, sol_explorer, refresh,
                   balances, warning, coin, amount, to, review, status, route, move_amount, move_review,
                   move_status }
}

/// A send's review, and its two answers.
#[derive(Clone)]
pub struct ReviewWindow {
    pub window: window::Window,
    pub send: button::Button,
    pub cancel: button::Button,
}

/// Check this send: what will be signed, read back out of the transaction,
/// and anything worth reading first. The only green button in the wallet is
/// here.
pub fn send_review(lines: &[(String, String)], notes: &[String]) -> ReviewWindow {
    review("check this send", lines, notes, "A send cannot be undone. Your passphrase is asked next.", "Send")
}

/// The same review for any signature that moves or lists something: what will
/// be signed, read back out of what was prepared, then the one green button.
pub fn review(heading: &str, lines: &[(String, String)], notes: &[String], footer: &str, go: &str) -> ReviewWindow {
    let mut w = window::Window::new(240, 200, 560, 300, None);
    mark(&mut w);
    w.set_label(&format!("elo — {heading}"));
    w.set_color(theme::BG);
    chrome::label(16, 12, 520, 22, heading, theme::HEAD, 16);

    let well_h = lines.len() as i32 * 24 + 14;
    let well = chrome::well(16, 42, 528, well_h);
    for (i, (k, v)) in lines.iter().enumerate() {
        let y = 50 + i as i32 * 24;
        chrome::label(28, y, 72, 20, k, theme::DIM, 12);
        let mut f = chrome::label_untrusted(104, y, 432, 20, v, theme::INK, 13);
        f.set_label_font(if k == "Send" { enums::Font::CourierBold } else { enums::Font::Courier });
    }
    well.end();

    let mut y = 42 + well_h + 12;
    for n in notes {
        let wrapped = wrap_lines(n, 80);
        let h = wrapped.len() as i32 * 15 + 4;
        let ink = if n.starts_with('⚠') { theme::RED } else { theme::GOLD };
        let mut f = chrome::label_untrusted(16, y, 528, h, &wrapped.join("\n"), ink, 11);
        f.set_align(enums::Align::Left | enums::Align::Top | enums::Align::Inside);
        y += h + 6;
    }
    chrome::label(16, y + 2, 528, 14, footer, theme::MUTED, 11);
    let by = y + 28;
    let mut cancel = chrome::button(16, by, 120, 30, "Cancel", Tone::Plain);
    cancel.set_align(enums::Align::Center | enums::Align::Inside);
    let mut send = chrome::button(424, by, 120, 30, go, Tone::Live);
    send.set_align(enums::Align::Center | enums::Align::Inside);
    w.set_size(560, by + 46);
    w.end();
    w.make_modal(true);
    ReviewWindow { window: w, send, cancel }
}

/// The recovery words, shown once, plus the two ways out of them.
#[derive(Clone)]
pub struct WordsWindow {
    pub window: window::Window,
    pub ok: button::Button,
    pub cancel: button::Button,
}

/// A new account's recovery words, numbered, in three columns read top to
/// bottom — the order a holder copies them onto paper.
///
/// Labels, not a text field: there is nothing here to select, so nothing to
/// copy to a clipboard that every other program on the machine can read.
pub fn recovery_words(words: &[&str]) -> WordsWindow {
    let rows = words.len().div_ceil(3).max(1);
    let grid_h = rows as i32 * 26 + 14;
    let mut w = window::Window::new(260, 200, 520, 300, None);
    mark(&mut w);
    w.set_label("elo — your recovery words");
    w.set_color(theme::BG);

    chrome::label(16, 12, 480, 22, "your recovery words", theme::HEAD, 16);
    chrome::label(16, 40, 488, 16,
                  "Write them down on paper, in order. They are the account.",
                  theme::INK, 12);
    let well = chrome::well(16, 64, 488, grid_h);
    for (i, word) in words.iter().enumerate() {
        let (col, row) = ((i / rows) as i32, (i % rows) as i32);
        let (x, y) = (30 + col * 160, 72 + row * 26);
        let mut n = chrome::label(x, y, 26, 20, &format!("{:>2}.", i + 1), theme::DIM, 12);
        n.set_align(enums::Align::Right | enums::Align::Inside);
        let mut f = chrome::label(x + 32, y, 118, 20, word, theme::INK, 15);
        f.set_label_font(enums::Font::CourierBold);
    }
    well.end();

    let y = 64 + grid_h + 12;
    chrome::label(16, y, 488, 14,
                  "Anyone with these words has the account and everything in it.",
                  theme::GOLD, 11);
    chrome::label(16, y + 18, 488, 14,
                  "We never see them, and this app cannot show them again.",
                  theme::MUTED, 11);
    chrome::label(16, y + 36, 488, 14,
                  "No screenshot, no cloud notes, no chat. Nobody real will ever ask for them.",
                  theme::MUTED, 11);

    let by = y + 64;
    let mut cancel = chrome::button(16, by, 120, 30, "Cancel", Tone::Plain);
    cancel.set_align(enums::Align::Center | enums::Align::Inside);
    let mut ok = chrome::button(304, by, 200, 30, "I wrote them down", Tone::Plain);
    ok.set_align(enums::Align::Center | enums::Align::Inside);
    w.set_size(520, by + 46);
    w.end();
    w.make_modal(true);
    WordsWindow { window: w, ok, cancel }
}

/// Which of the two things a notice is. The window draws them differently and
/// a caller cannot pass a colour, so a refusal can never be styled as a
/// success by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    Done,
    Refused,
}

/// A notice, plus its controls.
#[derive(Clone)]
pub struct NoticeWindow {
    pub window: window::Window,
    pub ok: button::Button,
    /// The next step, present only when the caller named one. See [`notice`].
    ///
    /// ⚠ This was `restart: Option<Button>` and only a restart could use it.
    /// Generalising it is the whole of the fix for a complaint that was never
    /// about restarts: operator, 2026-08-23, *"menu navigation is kinda odd
    /// after you complete a step so anything would be helpful."* The doc below
    /// had already written the principle down for restarts alone.
    pub action: Option<button::Button>,
}

/// Tell the player something, in the client's own skin.
///
/// `action` offers one next step beside the dismissal — **Restart elo now**,
/// **Play Gates now**, whatever the finished step leads to.
/// Operator, 2026-08-12: *"if we need the user to reboot say so and try to
/// self reboot or something."* Two halves, and the first one matters more:
///
/// * **Say so.** A notice that ends *"restart the launcher"* and leaves the
///   player to find the window's close box has told them a chore, not offered
///   one.
/// * **Do it.** `wiring::restart_now` re-execs THIS binary with the same
///   arguments. That is a restart and it is not an update — the client still
///   never replaces its own bytes (`docs/client/LAUNCHER.md` §8, and the
///   update notice one screen over says why).
///
/// ⚠ **The best restart is the one that is not needed**, and the case this was
/// built for no longer exists: making an account used to end *"restart the
/// launcher to unlock it in Signing"* because the Unlock button was built at
/// startup from a keystore that did not exist yet. `signing()` now draws that
/// control always and `wiring::adopt` activates it in place. Offer a restart
/// where a restart is genuinely the fix; do not use it to paper over a window
/// that could refresh itself.
pub fn notice(kind: Note, title: &str, body: &str, action: Option<&str>) -> NoticeWindow {
    let body = wrap_lines(body, NOTICE_COLS);
    let lines = body.len().max(1) as i32;
    let text_h = (lines * 17 + 8).clamp(40, 320);
    let win_h = 58 + text_h + 56;

    let mut w = window::Window::new(280, 220, 520, win_h, None);
    mark(&mut w);
    w.set_label(&format!("elo — {title}"));
    w.set_color(theme::BG);

    let (head_ink, head) = match kind {
        Note::Done => (theme::HEAD, title),
        // GOLD and not RED: most refusals in this client are a wrong
        // passphrase or an origin that did not answer, and painting those the
        // breach colour teaches a player to ignore it. RED stays for a failed
        // hash.
        Note::Refused => (theme::GOLD, title),
    };
    chrome::label(16, 12, 488, 22, head, head_ink, 15);

    // What fits as labels; anything longer scrolls. Labels past the cap were
    // drawn anyway — under OK and off the bottom of the window — so a long
    // refusal (a Verify naming many files, an origin's essay) lost its end,
    // which is usually the line that says what to do.
    let fits = ((text_h - 8) / 17) as usize;
    if body.len() <= fits {
        let mut y = 44;
        for line in &body {
            // A refusal quotes an origin, a vault or a game, so every line of
            // it is drawn clipped.
            chrome::label_untrusted(16, y, 488, 16, line, theme::MUTED, 11);
            y += 17;
        }
    } else {
        let mut all = text::TextDisplay::new(16, 44, 488, text_h - 4, None);
        let mut buf = text::TextBuffer::default();
        // A NUL would end the buffer early (see `consent::Ask::shown`).
        buf.set_text(&body.join("\n").replace('\0', "·"));
        all.set_buffer(buf);
        all.set_color(theme::BG);
        all.set_text_color(theme::MUTED);
        all.set_text_size(11);
        all.set_frame(enums::FrameType::NoBox);
    }

    // Width from the label: "Play Gates now" and "Restart elo now" are not the
    // same length, and a fixed 160 clipped the longer ones into a lie.
    let action_b = action.map(|label| {
        let w = (label.chars().count() as i32 * 8 + 36).clamp(140, 260);
        let mut b = chrome::button(16, win_h - 44, w, 30, label, Tone::Plain);
        b.set_align(enums::Align::Center | enums::Align::Inside);
        b
    });
    let mut ok = chrome::button(384, win_h - 44, 120, 30, "OK", Tone::Plain);
    ok.set_align(enums::Align::Center | enums::Align::Inside);

    w.end();
    w.make_modal(true);
    NoticeWindow { window: w, ok, action: action_b }
}

/// How many characters of 11px text fit on one 488px notice line.
const NOTICE_COLS: usize = 78;

/// Break a notice's text into lines that fit, so an error's reason is read
/// rather than clipped. Each line is drawn clipped, and a fetch error is one
/// long line — a url, then the reason — so the reason was the part cut off.
/// Breaks after a space or a `/` where it can, mid-word where it cannot.
fn wrap_lines(body: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let mut rest: Vec<char> = line.chars().collect();
        while rest.len() > cols {
            let cut = (cols / 2..cols)
                .rev()
                .find(|&i| rest[i] == ' ' || rest[i] == '/')
                .map(|i| i + 1)
                .unwrap_or(cols);
            let head: String = rest[..cut].iter().collect();
            out.push(head.trim_end().to_string());
            rest.drain(..cut);
        }
        out.push(rest.into_iter().collect());
    }
    out
}

/// The lock screen and its controls.
#[derive(Clone)]
pub struct LockWindow {
    pub window: window::Window,
    pub field: input::SecretInput,
    pub unlock: button::Button,
    /// Go on with the wallet in the browser instead. The key stays locked —
    /// see [`lock`].
    pub browser: button::Button,
    /// Honest about what it does: it ends the program.
    pub quit: button::Button,
    pub status: frame::Frame,
}

/// The lock screen — the passphrase gate in front of the whole client.
///
/// Operator, 2026-08-12: *"when i return to the app it should prompt for my
/// passphrase and not go pass that."*
///
/// **Unlock, use the browser wallet, or quit.** There is still no *"browse
/// locked"* door for the key on this machine. The third button is a different
/// account rather than a skip: a player whose wallet is MetaMask was stopped
/// here by a passphrase for a key they did not mean to use (operator,
/// 2026-10-17). Choosing it leaves the key locked, and the Account window it
/// opens is where the pairing starts. `ELO_LOCK_SCREEN=0` is still the posted
/// way to turn the screen off (`CLAUDE.md` invariant 10).
///
/// **What it is not: a security boundary.** The keystore is encrypted at rest
/// and this window does not change that in either direction. Anyone who can
/// run programs as you can read the same file with or without this screen;
/// what it narrows is the walk-by window on an unlocked key, which is the same
/// thing the idle relock buys and the same thing `elo-vault/local.rs` says
/// about itself. Do not let it grow a claim it cannot keep.
///
/// `why` says which of the two arrivals this is — a launch, or a relock while
/// the player was away — because those want different sentences and a single
/// generic one would be wrong for both.
pub fn lock(address: &str, why: &str) -> LockWindow {
    let mut w = window::Window::new(300, 240, 520, 296, None);
    mark(&mut w);
    w.set_label("elo — locked");
    w.set_color(theme::BG);

    chrome::label(16, 14, 488, 22, "elo is locked", theme::HEAD, 17);
    chrome::label(16, 40, 488, 16, why, theme::MUTED, 11);

    let well = chrome::well(16, 66, 488, 62);
    chrome::label(28, 76, 464, 16, "the account on this machine", theme::DIM, 11);
    // An address is a value read out of a keystore file — clipped like
    // anything else this program did not author.
    chrome::label_untrusted(28, 96, 464, 18, &elo_depot::streamer::shown(address), theme::INK, 13);
    well.end();

    chrome::label(16, 140, 488, 16, "Passphrase", theme::MUTED, 11);
    let mut field = input::SecretInput::new(16, 160, 488, 30, None);
    field.set_color(theme::WELL);
    field.set_text_color(theme::INK);
    field.set_text_size(14);
    field.set_frame(enums::FrameType::ThinDownBox);
    field.set_cursor_color(theme::INK);
    // See `passphrase` — `EnterKeyAlways`, because a retype after a refusal is
    // not a change and the plain trigger would swallow it.
    field.set_trigger(enums::CallbackTrigger::EnterKeyAlways);

    let status = chrome::label(16, 194, 488, 16, "", theme::RED, 11);

    let mut quit = chrome::button(16, 216, 140, 30, "Quit elo", Tone::Plain);
    quit.set_align(enums::Align::Center | enums::Align::Inside);
    let mut browser = chrome::button(168, 216, 204, 30, "Use my browser wallet", Tone::Plain);
    browser.set_align(enums::Align::Center | enums::Align::Inside);
    let mut unlock = chrome::button(384, 216, 120, 30, "Unlock", Tone::Plain);
    unlock.set_align(enums::Align::Center | enums::Align::Inside);

    chrome::label(16, 254, 488, 14,
                  "no passphrase can be reset — the keystore is the only copy of your key",
                  theme::DIM, 10);
    chrome::label(16, 270, 488, 14,
                  "ELO_LOCK_SCREEN=0 turns this screen off; games play without an account",
                  theme::DIM, 10);

    w.end();
    w.make_modal(true);
    LockWindow { window: w, field, unlock, browser, quit, status }
}

/// How big a face is beside a chat message, in px: two lines of the feed.
pub const FACE: i32 = 28;
/// The feed's left margin the faces sit in.
const GUTTER: i32 = FACE + 12;
/// The key our own mark is filed under in [`Faces::pictures`].
pub const HOUSE_FACE: &str = "#house";
/// The tiles a face without a picture wears — muted, so a letter on one reads
/// as a placeholder and never as a picture somebody chose.
pub const TINTS: [enums::Color; crate::chatpane::TINTS] = [
    enums::Color::from_rgb(0x5d, 0x6e, 0x4c),
    enums::Color::from_rgb(0x6e, 0x5d, 0x4c),
    enums::Color::from_rgb(0x4c, 0x5d, 0x6e),
    enums::Color::from_rgb(0x6e, 0x4c, 0x5d),
    enums::Color::from_rgb(0x4c, 0x6e, 0x5d),
    enums::Color::from_rgb(0x5d, 0x4c, 0x6e),
];

/// One face to draw: level with the name at byte `pos` of the feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceRow {
    pub pos: i32,
    pub key: String,
    pub house: bool,
    pub initial: String,
    pub tint: usize,
}

/// What the feed's gutter paints. Filled by `wiring` on the UI thread, read
/// by the feed's own draw.
#[derive(Default)]
pub struct Faces {
    pub rows: Vec<FaceRow>,
    /// Decoded pictures by npub, plus [`HOUSE_FACE`].
    pub pictures: std::collections::HashMap<String, art::Picture>,
    /// The bytes each picture came from, so a changed picture is decoded again
    /// and an unchanged one never is.
    pub decoded: std::collections::HashMap<String, std::sync::Arc<Vec<u8>>>,
}

/// The Chat window and the controls on it. Built once and filled by
/// `wiring::wire_chat` from the engine's view; this draws and decides nothing.
#[derive(Clone)]
pub struct ChatWindow {
    pub window: window::Window,
    pub rooms: browser::HoldBrowser,
    pub room_name: frame::Frame,
    pub room_about: frame::Frame,
    pub feed: text::TextDisplay,
    pub feed_text: text::TextBuffer,
    pub feed_style: text::TextBuffer,
    pub status: frame::Frame,
    pub input: input::Input,
    pub send: button::Button,
    /// Present while this machine has no chat key and no account.
    pub make_key: button::Button,
    /// Present while there is an account and this machine's key is not linked
    /// to it — the same slot as `make_key`, never both.
    pub link: button::Button,
    /// Present while this machine's key is linked to the account — the slot
    /// again.
    pub unlink: button::Button,
    pub you: frame::Frame,
    /// Why the last note did not post, when it did not.
    pub said: frame::Frame,
    /// The faces beside the messages. Shared with the feed's draw.
    pub faces: std::rc::Rc<std::cell::RefCell<Faces>>,
}

/// One line of the room list: our ink, then the name as literal text.
///
/// ⚠ **`@.` is the safety half of this, not the colour.** `Fl_Browser` reads
/// `@` codes at the start of a line, and a room's name is the origin's to
/// write. `@.` ends the codes — *"print rest of line, don't look for more '@'
/// signs"* (`Fl_Browser.cxx`) — so whatever follows is drawn as text. The
/// colour has to ride here at all because fltk-rs exposes no text colour for
/// this widget, and the default is black on a dark olive well.
pub fn room_line(label: &str) -> String {
    format!("@C{}@.{}", theme::INK.bits(), label)
}

/// Chat — the rooms, live.
///
/// **The feed is a text widget, never labels** (`chatpane`'s header says why),
/// and every room-list line goes through [`room_line`].
pub fn chat() -> ChatWindow {
    use crate::chatpane::{S_HOUSE, S_QUIET, S_SWORN, S_TEXT, S_TIME, S_UNCHECKED};
    let (ww, wh) = (760, 492);
    let mut w = window::Window::new(150, 130, ww, wh, None);
    mark(&mut w);
    w.set_label("elo — chat");
    w.set_color(theme::BG);

    chrome::label(16, 12, 200, 22, "chat", theme::HEAD, 16);
    let mut status = chrome::label(220, 14, 524, 18, "", theme::MUTED, 11);
    status.set_align(enums::Align::Right | enums::Align::Inside | enums::Align::Clip);

    let mut rooms = browser::HoldBrowser::new(16, 44, 160, 372, None);
    rooms.set_color(theme::WELL);
    rooms.set_selection_color(theme::SEL);
    rooms.set_text_size(13);
    rooms.set_frame(enums::FrameType::ThinDownBox);
    // ⚠ No `set_column_char('\0')`: fltk-rs asserts on a NUL and the window
    // is built at startup, so that line took the whole launcher down before
    // the menu drew. It was never needed — `Fl_Browser` splits columns only
    // when column widths are set, and none are, and `safe_line` has already
    // turned any tab in a name into a space.
    rooms.set_scrollbar_size(13);

    let room_name = chrome::label_untrusted(190, 44, 554, 20, "", theme::INK, 14);
    let room_about = chrome::label_untrusted(190, 64, 554, 16, "", theme::MUTED, 11);

    let feed_text = text::TextBuffer::default();
    let feed_style = text::TextBuffer::default();
    let mut feed = text::TextDisplay::new(190, 86, 554, 330, None);
    feed.set_buffer(feed_text.clone());
    feed.set_color(theme::WELL);
    feed.set_frame(enums::FrameType::ThinDownBox);
    feed.set_text_color(theme::INK);
    feed.set_text_size(13);
    feed.set_scrollbar_size(13);
    feed.wrap_mode(text::WrapMode::AtBounds, 0);
    // The order is the style letters' order in `chatpane` — 'A' first.
    let entry = |color, font, size| text::StyleTableEntry { color, font, size };
    let styles = vec![
        (S_TIME, entry(theme::DIM, enums::Font::Helvetica, 11)),
        (S_SWORN, entry(theme::HEAD, enums::Font::HelveticaBold, 13)),
        (S_UNCHECKED, entry(theme::MUTED, enums::Font::Helvetica, 13)),
        (S_HOUSE, entry(theme::GOLD, enums::Font::HelveticaBold, 13)),
        (S_TEXT, entry(theme::INK, enums::Font::Helvetica, 13)),
        (S_QUIET, entry(theme::DIM, enums::Font::HelveticaItalic, 12)),
    ];
    debug_assert!(styles.iter().enumerate().all(|(i, (c, _))| *c as usize == 'A' as usize + i));
    feed.set_highlight_data(feed_style.clone(), styles.into_iter().map(|(_, e)| e).collect::<Vec<_>>());
    // The faces' column. FLTK's line-number margin is the one gutter a text
    // display keeps clear of text, wrapped lines included; the numbers are
    // drawn in the well's own colour, so only the faces show in it.
    feed.set_linenumber_width(GUTTER);
    feed.set_linenumber_bgcolor(theme::WELL);
    feed.set_linenumber_fgcolor(theme::WELL);

    let faces = std::rc::Rc::new(std::cell::RefCell::new(Faces::default()));
    if let Some(mark) = art::Icon::decode(MARK.as_bytes().to_vec()).and_then(|i| i.picture(FACE)) {
        faces.borrow_mut().pictures.insert(HOUSE_FACE.into(), mark);
    }
    {
        let faces = std::rc::Rc::clone(&faces);
        feed.draw(move |t| paint_faces(t, &faces));
    }

    let mut make_key = chrome::button(16, 424, 160, 28, "Make a chat key", Tone::Plain);
    make_key.set_align(enums::Align::Center | enums::Align::Inside);
    make_key.set_tooltip("a key that talks for you in chat — it can't move money");
    let mut link = chrome::button(16, 424, 160, 28, "Link to my wallet", Tone::Plain);
    link.set_align(enums::Align::Center | enums::Align::Inside);
    link.set_tooltip("use your wallet's chat key, so people see your profile name and picture");
    link.hide();
    let mut unlink = chrome::button(16, 424, 160, 28, "Unlink my wallet", Tone::Plain);
    unlink.set_align(enums::Align::Center | enums::Align::Inside);
    unlink.set_tooltip("rooms stop showing your profile name and picture; the key stays here");
    unlink.hide();
    let you = chrome::label_untrusted(16, 458, 160, 16, "", theme::DIM, 11);

    let mut input = input::Input::new(190, 424, 446, 28, None);
    input.set_color(theme::WELL);
    input.set_text_color(theme::INK);
    input.set_text_size(13);
    input.set_selection_color(theme::SEL);
    input.set_cursor_color(theme::INK);
    input.set_frame(enums::FrameType::ThinDownBox);
    // Enter sends, and a second Enter after a refusal sends again rather than
    // being swallowed as "no change".
    input.set_trigger(enums::CallbackTrigger::EnterKeyAlways);
    let mut send = chrome::button(644, 424, 100, 28, "Send", Tone::Plain);
    send.set_align(enums::Align::Center | enums::Align::Inside);
    let said = chrome::label_untrusted(190, 458, 554, 16, "", theme::GOLD, 11);

    w.end();
    // The feed takes the extra room when the window grows; the rest keeps its
    // place against the edges it was laid out from.
    w.resizable(&feed);
    w.size_range(ww, wh, 0, 0);
    ChatWindow {
        window: w, rooms, room_name, room_about, feed, feed_text, feed_style, status,
        input, send, make_key, link, unlink, you, said, faces,
    }
}

/// Paint each visible message's face into the feed's gutter, after the feed
/// has drawn itself (fltk-rs runs a draw callback after the widget's own).
///
/// A face is level with its message's name line, and only drawn when that line
/// is on screen: `position_to_xy` leaves `(0, 0)` for a position that is not.
/// The house wears our mark; a speaker with no picture wears a tile with their
/// letter, drawn with `draw_text` — which parses no `@` symbols, unlike the
/// aligned variant, and the letter is a stranger's.
fn paint_faces(t: &mut text::TextDisplay, faces: &std::rc::Rc<std::cell::RefCell<Faces>>) {
    use fltk::draw;
    let Ok(mut f) = faces.try_borrow_mut() else { return };
    if f.rows.is_empty() {
        return;
    }
    let (x0, y0, h0) = (t.x(), t.y(), t.h());
    draw::push_clip(x0 + 2, y0 + 2, GUTTER - 2, h0 - 4);
    let rows = std::mem::take(&mut f.rows);
    for r in &rows {
        let (_, y) = t.position_to_xy(r.pos);
        if y < y0 || y >= y0 + h0 {
            continue;
        }
        let (ax, ay) = (x0 + 6, y + 2);
        let key = if r.house { HOUSE_FACE } else { r.key.as_str() };
        if let Some(p) = f.pictures.get_mut(key) {
            p.draw(ax, ay, FACE, FACE);
            continue;
        }
        draw::draw_rbox(ax, ay, FACE, FACE, 6, true, TINTS[r.tint % TINTS.len()]);
        draw::set_draw_color(theme::INK);
        draw::set_font(enums::Font::HelveticaBold, 14);
        let tw = draw::width(&r.initial) as i32;
        let base = ay + (FACE + draw::height()) / 2 - draw::descent();
        draw::draw_text(&r.initial, ax + (FACE - tw) / 2, base);
    }
    f.rows = rows;
    draw::pop_clip();
}

#[cfg(test)]
mod tests {
    use super::{wrap_lines, MARK};
    use fltk::{image, prelude::*};

    #[test]
    fn a_room_line_ends_the_format_codes_before_the_name() {
        let line = super::room_line("@B1@C88 #general");
        let at = line.find("@.").expect("the stop code");
        assert!(line[at + 2..].starts_with("@B1@C88 #general"), "{line}");
        assert!(line[..at].starts_with("@C"), "{line}");
        assert!(line[2..at].chars().all(|c| c.is_ascii_digit()), "{line}");
    }

    #[test]
    fn a_long_error_wraps_and_keeps_its_reason() {
        let e = "could not fetch https://elopros.com/api/launcher/depot/gates/\
                 0.8.0-gca7362d19/linux-x86_64/files/gates: timeout: global";
        let lines = wrap_lines(e, 78);
        assert!(lines.len() > 1, "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 78), "{lines:?}");
        assert_eq!(lines.concat().replace(' ', ""), e.replace(' ', ""));
        assert!(lines.last().unwrap().ends_with("timeout: global"));
        assert_eq!(wrap_lines("short\n\nlines", 78), ["short", "", "lines"]);
    }

    /// The mark has to decode, and it has to decode as something with pixels.
    ///
    /// ⚠ **nanosvg fails quietly.** FLTK's SVG reader ignores what it does not
    /// implement rather than refusing the document, so a mark that used
    /// `<text>` or a gradient it did not support would parse fine, draw
    /// nothing, and put an empty square on every taskbar. Asserting the size
    /// is what makes the difference visible to a build rather than to a
    /// player.
    #[test]
    fn the_window_icon_is_a_picture_and_not_an_empty_square() {
        let img = image::SvgImage::from_data(MARK).expect("the mark must parse");
        assert!(img.width() > 0 && img.height() > 0,
                "the mark parsed to a {}x{} image", img.width(), img.height());
    }
}
