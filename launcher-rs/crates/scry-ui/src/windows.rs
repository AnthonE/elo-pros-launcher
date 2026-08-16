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

use crate::art::{self, Icon};
use crate::chrome;
use crate::theme::{self, Tone};
use fltk::{button, enums, frame, image, input, prelude::*, window};
use scry_depot::{shardlist::Shard, Install};

/// The mark — the same green orb the site serves at `/favicon.svg`. Compiled
/// in rather than read from `watchtower/`, because the public repo carries
/// `launcher-rs/` without the site; `build_release.py` refuses to package when
/// this copy and the site's drift, which is the same weld the masthead CSS is
/// under (`test_site.py`, "the mark").
const ORB: &str = include_str!("../assets/orb.svg");

/// Dress a top-level so the desktop knows whose window it is: the orb as the
/// icon (dock, switcher, title bar), and `scry` as the WM class so a desktop
/// can match the live window back to `scry.desktop` (`StartupWMClass=scry` —
/// without it GNOME shows a running window with a blank gear where the menu
/// icon is). One call per window, because FLTK's default-icon static has no
/// Rust binding. A window whose icon failed to parse is still a working
/// window, so the parse is allowed to fail quietly — it cannot, for a
/// compiled-in file, but the window must not hinge on that.
fn mark(w: &mut window::Window) {
    w.set_xclass("scry");
    if let Ok(orb) = image::SvgImage::from_data(ORB) {
        w.set_icon(Some(orb));
    }
}

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
    /// caller runs `scry_depot::check` and fills `stale` in; until it does,
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
    /// offers first. `scry play` runs whatever is installed.
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
    mark(&mut w);
    w.set_label("scry");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

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

    // ⚠ This said *"the pass is a token"* until 2026-08-12. The pass is a
    // designed, undeployed platform product; the thing a player actually owns
    // is the COPY — the title's ticket, an ERC-721 (`TICKET.md` §0). Naming
    // the wrong object on the first screen of the client is how a stranger
    // learns the wrong word for what they bought.
    chrome::label(16, y + 12, 520, 16,
                  "nothing you own lives in here — your copies are tokens and items are on chain",
                  theme::DIM, 11);
    w.end();
    MainMenu { window: w, version_line }
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
    /// The download meter, hidden until an update is moving bytes. It stands
    /// where [`status`](Self::status) stands, and the wiring swaps them: while
    /// bytes land, the truest status line IS the meter.
    pub progress: fltk::misc::Progress,
    /// The row's status line ([`Row::status_line`]), handed back so the meter
    /// can stand in for it during a download and give it back after.
    pub status: frame::Frame,
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
    w.set_label("scry — games");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

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
            let status = chrome::label_untrusted(66, y + 18, 340, 15, &row.status_line(),
                                                 theme::MUTED, 11);
            // The meter stands exactly where the status line does — while an
            // update is moving bytes, the truest status line IS the meter.
            // Shorter than the label it replaces because the byte count draws
            // beside it (`chrome::meter`), in the rest of the same rect.
            let progress = chrome::meter(66, y + 18, 190, 15);
            // Laid out from the well's inner right edge (644) backwards, so
            // the bar's reservation is in the arithmetic and not in a pair of
            // hand-tuned numbers that would drift from it.
            let verify_x = 644 - BAR - 7 - 96;
            let act_x = verify_x - 104;
            let mut act = chrome::button(act_x, y + 4, 96, 26, row.action(), row.tone());
            act.set_align(enums::Align::Center | enums::Align::Inside);
            let mut verify = chrome::button(verify_x, y + 4, 96, 26, "Verify", Tone::Plain);
            verify.set_align(enums::Align::Center | enums::Align::Inside);
            verify.set_tooltip("re-hash every file the depot names and check the digest");
            controls.push(GameControls {
                slug: row.slug.clone(),
                build: row.build.clone(),
                act,
                verify,
                progress,
                status,
            });
            y += ROW_H;
        }
    }
    shelf.end();

    chrome::label(16, well_h + 52, 600, 16,
                  "verify re-hashes every file the depot names, then checks the digest",
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
    w.set_label("scry — about");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

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
    pub fn act(&self) -> Act {
        match (&self.price, self.installed, self.installable) {
            (_, true, _) => Act::Installed,
            (Price::Posted { .. }, _, _) => Act::Buy,
            (Price::Unknown { .. }, _, _) => Act::Page,
            (Price::Free, _, true) => Act::Install,
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
            Act::Install => "fetch the build, hash-verify every file, and place it",
            Act::Buy => "open the buy box in your browser — the wallet is yours, not this client's",
            Act::Installed => "already on this disk — start it from Games",
            Act::Unpublished => "listed, but no build for this platform is published yet",
            Act::Page => "not enough could be read to offer a price — read the page yourself",
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
    /// The download meter, hidden until an install is moving bytes. Stands
    /// where the price line stands — see [`GameControls::progress`] for the
    /// swap this row makes with [`price`](Self::price).
    pub progress: fltk::misc::Progress,
    /// The row's price line, handed back so the meter can stand in for it
    /// during a download and give it back after.
    pub price: frame::Frame,
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
    w.set_label("scry — store");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

    chrome::label(16, 12, 400, 20, "the store", theme::HEAD, 16);
    chrome::label(16, 34, 480, 16, "one payment, one copy, and the copy is yours to resell",
                  theme::MUTED, 11);
    let mut refresh = chrome::button(556, 14, 128, 26, "Refresh", Tone::Plain);
    refresh.set_align(enums::Align::Center | enums::Align::Inside);
    refresh.set_tooltip("ask the origin for its catalog again — nothing here needs a restart");

    let mut controls = Vec::new();
    // Scrolls, for the same reason the library does: the number of titles on
    // the shelf is the origin's to decide and the window has to hold any of
    // them (`chrome::shelf`).
    let shelf = chrome::shelf(16, 58, 668, well_h);
    if !reachable {
        // The repo's own trap, at the one place it would cost a player a game.
        chrome::label(28, 74, 640, 18, "the catalog could not be read", theme::GOLD, 13);
        chrome::label(28, 96, 640, 16,
                      "this is not an empty shelf — the origin did not answer, so nothing is known",
                      theme::MUTED, 11);
        chrome::label_untrusted(28, 116, 640, 16, why, theme::DIM, 11);
        chrome::label(28, 140, 640, 16, "Refresh asks again — nothing here needs a restart",
                      theme::DIM, 11);
    } else if rows.is_empty() {
        chrome::label(28, 74, 640, 18, "nothing listed yet", theme::MUTED, 13);
        chrome::label(28, 96, 640, 16,
                      "the origin answered and its shelf is empty — a real answer, not a failed read",
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
            let price = chrome::label_untrusted(80, y + 35, 360, 15, &row.price_line(),
                                                price_ink, 10);
            // Where the meter stands while an install moves bytes — over the
            // price line, because that is the row's one line about THIS act:
            // the money moved (or was never asked for), and now the bytes are.
            let progress = chrome::meter(80, y + 35, 190, 15);

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
            page.set_tooltip("open this title's page on the origin, in your browser");
            controls.push(ShelfControls {
                slug: row.slug.clone(),
                act,
                button,
                page,
                progress,
                price,
            });
            y += SHELF_H;
        }
    }
    shelf.end();

    chrome::label(16, well_h + 68, 668, 16,
                  "a copy is an NFT and all sales are final — the exit is resale, never a refund",
                  theme::DIM, 11);
    chrome::label(16, well_h + 86, 668, 16,
                  "buying opens your browser: the wallet is yours and this client never holds one",
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
    mark(&mut w);
    w.set_label("scry — servers");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

    chrome::label(16, 12, 400, 20, "servers", theme::HEAD, 16);

    let mut controls = Vec::new();
    // Scrolls. A popular title's list is the longest thing this client draws
    // and it is served by the game, so the count is nobody's to cap here.
    let well = chrome::shelf(16, 40, 608, well_h);
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
                join.set_tooltip("start the installed build against this address");
                // Always offered, even on a full shard: sending a friend a
                // link to a server you cannot get into right now is normal.
                let mut copy = chrome::button(invite_x, y + 2, 74, 26, "Invite", Tone::Plain);
                copy.set_tooltip("copy this shard's scry:// link to the clipboard");

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
    grows_downward(&mut w, &well, 108 + 46 * 2);
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
    mark(&mut w);
    w.set_label("scry — account");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

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
    pub window: window::Window,
}

/// `unlockable` is separate from `kind` on purpose: the backend is `local`
/// whether or not a keystore exists yet, and a key that has not been made
/// cannot be unlocked — so the controls are drawn deactivated with a sentence
/// saying why, rather than hidden or omitted.
pub fn signing(kind: &str, status: &str, address: Option<&str>, unlockable: bool) -> SigningWindow {
    let mut w = window::Window::new(200, 200, 620, 320, None);
    mark(&mut w);
    w.set_label("scry — signing");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

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
            chrome::label(28, 132, 270, 16, "no account on this machine yet — make one in Account",
                          theme::GOLD, 11);
        }
        (Some(b), Some(l))
    } else {
        let mut test = chrome::button(456, 138, 132, 26, "Nothing to unlock", tone);
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
    mark(&mut w);
    w.set_label("scry — a game wants to sign");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

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
    w.set_label(&format!("scry — {title}"));
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

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
    /// Present only when the caller asked for it. See [`notice`].
    pub restart: Option<button::Button>,
}

/// Tell the player something, in the client's own skin.
///
/// `restart` offers a **Restart scry now** button beside the dismissal.
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
pub fn notice(kind: Note, title: &str, body: &str, restart: bool) -> NoticeWindow {
    let lines = body.lines().count().max(1) as i32;
    let text_h = (lines * 17 + 8).clamp(40, 320);
    let win_h = 58 + text_h + 56;

    let mut w = window::Window::new(280, 220, 520, win_h, None);
    mark(&mut w);
    w.set_label(&format!("scry — {title}"));
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

    let (head_ink, head) = match kind {
        Note::Done => (theme::HEAD, title),
        // GOLD and not RED: most refusals in this client are a wrong
        // passphrase or an origin that did not answer, and painting those the
        // breach colour teaches a player to ignore it. RED stays for a failed
        // hash.
        Note::Refused => (theme::GOLD, title),
    };
    chrome::label(16, 12, 488, 22, head, head_ink, 15);

    let mut y = 44;
    for line in body.lines() {
        // A refusal quotes an origin, a vault or a game, so every line of it
        // is drawn clipped.
        chrome::label_untrusted(16, y, 488, 16, line, theme::MUTED, 11);
        y += 17;
    }

    let restart_b = restart.then(|| {
        let mut b = chrome::button(16, win_h - 44, 160, 30, "Restart scry now", Tone::Plain);
        b.set_align(enums::Align::Center | enums::Align::Inside);
        b
    });
    let mut ok = chrome::button(384, win_h - 44, 120, 30, "OK", Tone::Plain);
    ok.set_align(enums::Align::Center | enums::Align::Inside);

    w.end();
    w.make_modal(true);
    NoticeWindow { window: w, ok, restart: restart_b }
}

/// The lock screen and its controls.
#[derive(Clone)]
pub struct LockWindow {
    pub window: window::Window,
    pub field: input::SecretInput,
    pub unlock: button::Button,
    /// The only other way out, and it is honest about what it does. There is
    /// deliberately no *"skip"* — see [`lock`].
    pub quit: button::Button,
    pub status: frame::Frame,
}

/// The lock screen — the passphrase gate in front of the whole client.
///
/// Operator, 2026-08-12: *"when i return to the app it should prompt for my
/// passphrase and not go pass that."*
///
/// **Two buttons, and the missing third is the design.** Unlock, or quit.
/// There is no *"browse locked"* door, because the ask was for a gate and a
/// gate with a bypass is a notification. What that costs is real and is stated
/// on the window itself: a player who only wants to read the store still has
/// to type their passphrase, and `SCRY_LOCK_SCREEN=0` is the posted way out
/// (`CLAUDE.md` invariant 10 — every shortcut keeps a door).
///
/// **What it is not: a security boundary.** The keystore is encrypted at rest
/// and this window does not change that in either direction. Anyone who can
/// run programs as you can read the same file with or without this screen;
/// what it narrows is the walk-by window on an unlocked key, which is the same
/// thing the idle relock buys and the same thing `scry-vault/local.rs` says
/// about itself. Do not let it grow a claim it cannot keep.
///
/// `why` says which of the two arrivals this is — a launch, or a relock while
/// the player was away — because those want different sentences and a single
/// generic one would be wrong for both.
pub fn lock(address: &str, why: &str) -> LockWindow {
    let mut w = window::Window::new(300, 240, 520, 296, None);
    mark(&mut w);
    w.set_label("scry — locked");
    w.set_color(theme::BG);
    chrome::wear_icon(&mut w);

    chrome::label(16, 14, 488, 22, "scry is locked", theme::HEAD, 17);
    chrome::label(16, 40, 488, 16, why, theme::MUTED, 11);

    let well = chrome::well(16, 66, 488, 62);
    chrome::label(28, 76, 464, 16, "the account on this machine", theme::DIM, 11);
    // An address is a value read out of a keystore file — clipped like
    // anything else this program did not author.
    chrome::label_untrusted(28, 96, 464, 18, address, theme::INK, 13);
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

    let mut quit = chrome::button(16, 216, 140, 30, "Quit scry", Tone::Plain);
    quit.set_align(enums::Align::Center | enums::Align::Inside);
    let mut unlock = chrome::button(384, 216, 120, 30, "Unlock", Tone::Plain);
    unlock.set_align(enums::Align::Center | enums::Align::Inside);

    chrome::label(16, 254, 488, 14,
                  "no passphrase can be reset — the keystore is the only copy of your key",
                  theme::DIM, 10);
    chrome::label(16, 270, 488, 14,
                  "SCRY_LOCK_SCREEN=0 turns this screen off; games play without an account",
                  theme::DIM, 10);

    w.end();
    w.make_modal(true);
    LockWindow { window: w, field, unlock, quit, status }
}
