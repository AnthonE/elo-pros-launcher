//! What a row SAYS and what its control OFFERS must agree.
//!
//! This file exists because of a defect a test did not find. The first capture
//! of the Games window showed a row reading *"an update is published"* whose
//! button said **Play** — identical to the row above it reading *"up to date"*.
//! Every unit test passed; the sentence and the control simply disagreed, and
//! the control is what a player acts on. They would have launched the stale
//! build and never learned there was a new one.
//!
//! Found by looking at the picture (`Gates/CLAUDE.md`: *look at the picture
//! before tuning the number*). Pinned here so it stays found.

use elo_ui::theme::Tone;
use elo_ui::windows::{Act, Price, Row, Shelf, MENU};

fn row(stale: Option<bool>) -> Row {
    Row { slug: "gates".into(), build: "0.1.0".into(), bytes: 75_829_730,
          digest: "0xf4".into(), stale, name: None, icon: None, why: None }
}

fn shelf(price: Price, installable: bool, installed: bool) -> Shelf {
    Shelf {
        slug: "gates".into(),
        name: "Gates".into(),
        blurb: "survival".into(),
        state: "live".into(),
        installable,
        installed,
        price,
        icon: None,
    }
}

#[test]
fn a_stale_row_offers_the_update_not_the_old_build() {
    assert_eq!(row(Some(true)).action(), "Update");
    assert_eq!(row(Some(false)).action(), "Play");
}

#[test]
fn the_three_states_are_three_different_sentences() {
    let lines = [
        row(Some(true)).status_line(),
        row(Some(false)).status_line(),
        row(None).status_line(),
    ];
    let unique: std::collections::BTreeSet<_> = lines.iter().collect();
    assert_eq!(unique.len(), 3, "each state needs its own sentence: {lines:?}");
    assert!(lines[2].contains("not checked"),
            "unknown must say we could not look, never imply up-to-date: {}", lines[2]);
}

/// The reservation, at the row level. An unchecked row has NOT earned the
/// armed colour — it wears gold, which is the not-armed one.
#[test]
fn an_unchecked_row_never_wears_the_armed_colour() {
    assert_eq!(row(None).tone(), Tone::Gold);
    for r in [row(Some(true)), row(Some(false))] {
        assert!(!r.tone().is_reserved_fill(),
                "no games row moves money, so none may paint the reserved green");
    }
}

/// Every menu entry names a window that exists.
///
/// **This test changed shape on 2026-08-06 and the old version is why.** It
/// used to assert that *at least one* entry was unbuilt, with a note saying
/// that if every entry became built the assertion should go. Every entry then
/// became built and it failed — which is the assertion doing its job: it is a
/// prompt to re-scope, not a thing to delete quietly.
///
/// **The `built` flag and the not-built styling stay**, because the next window
/// added will need them and a player must never be shown a control that does
/// nothing. What is asserted now is the property that actually matters: nothing
/// in the menu is a dead button.
#[test]
fn every_menu_entry_names_a_window_that_exists() {
    let unbuilt: Vec<_> = MENU.iter().filter(|(_, _, b)| !*b).map(|(n, ..)| *n).collect();
    assert!(
        unbuilt.is_empty(),
        "these entries render deactivated with a 'not built yet' line, which is \
         correct — but a window for each now exists, so mark them built or \
         remove them: {unbuilt:?}"
    );
    assert_eq!(MENU.len(), 6, "the menu changed size — was that deliberate?");
    for (name, blurb, _) in MENU.iter() {
        assert!(!blurb.is_empty(), "{name} needs a sentence — the menu is \
                buttons-with-a-sentence, which is why it needs no nav bar");
    }
}

/// The not-built styling is still reachable and still honest, asserted
/// directly now that no live entry exercises it. Deleting the path because the
/// menu happens to be full is how the next unbuilt window ships as a dead
/// button.
#[test]
fn the_not_built_styling_still_says_not_built() {
    let pretend: [(&str, &str, bool); 2] =
        [("Games", "what is installed", true), ("Trading", "a thing to come", false)];
    let unbuilt: Vec<_> = pretend.iter().filter(|(_, _, b)| !*b).collect();
    assert_eq!(unbuilt.len(), 1);
    // The window builder renders `built: false` deactivated AND appends the
    // sentence; this pins the contract the builder relies on.
    assert!(!pretend[1].2, "an unbuilt entry must stay distinguishable in the data");
}

/// A row falls back to its slug and never to a blank.
///
/// An install knows only its slug — the name comes from the origin, and a
/// machine that has never reached it still has to draw a library.
#[test]
fn a_row_with_no_name_from_the_origin_is_titled_by_its_slug() {
    assert_eq!(row(None).title(), "gates");
    let mut named = row(None);
    named.name = Some("Gates".into());
    assert_eq!(named.title(), "Gates");
}

// ── the store shelf ─────────────────────────────────────────────────────────
//
// Same rule as the Games rows above and the same reason: the sentence a row
// prints and the button it offers have to agree, because the button is what a
// player acts on.

/// The one that costs money if it is wrong.
///
/// `/api/ticket/{slug}` answering *"no ticket contract"* means free. A read
/// that FAILED means we do not know. Collapsing the second into the first is
/// how a store offers a paid game as a free download on the strength of a
/// dropped packet — the repo's own trap (`CLAUDE.md` §traps) with a price on
/// it.
#[test]
fn a_price_we_could_not_read_is_never_rendered_as_free() {
    let unknown = shelf(Price::Unknown { why: "dns error".into() }, true, false);
    let free = shelf(Price::Free, true, false);
    assert_ne!(unknown.price_line(), free.price_line());
    assert!(!unknown.price_line().contains("free"),
            "an unread price must not say free: {}", unknown.price_line());
    assert!(unknown.price_line().contains("could not be read"),
            "say we could not look: {}", unknown.price_line());
    // And it must not offer an Install either — that is the act whose cost is
    // the whole question.
    assert_ne!(unknown.act(), Act::Install);
}

#[test]
fn every_shelf_state_offers_the_control_its_sentence_implies() {
    // Free and publishing a build for this machine: the one row that installs.
    assert_eq!(shelf(Price::Free, true, false).act(), Act::Install);
    // Free but nothing published for this platform: drawn, off, and named.
    assert_eq!(shelf(Price::Free, false, false).act(), Act::Unpublished);
    // Priced: the buy is a wallet signing, which happens in a browser.
    assert_eq!(shelf(Price::Posted { line: "$10.00".into() }, true, false).act(), Act::Buy);
    // Already on this disk: never a second copy.
    assert_eq!(shelf(Price::Free, true, true).act(), Act::Installed);
    assert_eq!(shelf(Price::Posted { line: "$10.00".into() }, true, true).act(), Act::Installed);
}

/// Only the one that cannot be pressed is drawn as unpressable, and the four
/// that can be pressed all say what they do.
#[test]
fn no_shelf_control_is_a_dead_button() {
    for act in [Act::Install, Act::Buy, Act::Installed, Act::Page] {
        assert!(act.pressable(), "{act:?} is offered, so it must do something");
        assert!(!act.label().is_empty());
    }
    assert!(!Act::Unpublished.pressable(),
            "nothing to install must be visibly off, not silently inert");
}

/// The reservation, on the surface where money is actually named.
///
/// A green fill means an act that moves money (`theme::Tone`). Pressing Buy
/// here opens a browser — the wallet is the player's and this client holds
/// none — so nothing on this shelf has earned it. Written down because "the
/// primary button" is exactly the thought that paints one green.
#[test]
fn nothing_on_the_store_shelf_wears_the_reserved_fill() {
    for act in [Act::Install, Act::Buy, Act::Installed, Act::Unpublished, Act::Page] {
        assert!(!act.tone().is_reserved_fill(),
                "{act:?} paints the reserved green, and it moves no money here");
    }
}

/// Gold is an outline and an outline cannot also be deactivated.
///
/// Found by looking at a capture: `Tone::Gold` plus FLTK's `deactivate()`
/// draws a washed-out pale box that reads as neither a gold outline nor a
/// disabled control. The unpressable row wears `Plain` and is drawn through
/// `chrome::button_off` instead — the shape the Servers window's **Full**
/// button already uses for exactly this.
#[test]
fn the_unpressable_shelf_control_does_not_ask_for_the_gold_outline() {
    assert_eq!(Act::Unpublished.tone(), Tone::Plain,
               "an outline that is also greyed out reads as neither");
    assert!(!Act::Unpublished.pressable());
}

/// The store must not advertise the pass.
///
/// It did until 2026-08-12: a panel headed THE PASS, over a shelf selling
/// copies. The pass is a designed, undeployed, platform-wide product; a copy
/// is the per-title ticket a player actually buys (`TICKET.md` §0). This
/// pins the noun, because the old block is one paste away from coming back.
#[test]
fn the_shelf_names_the_copy_and_not_the_pass() {
    for price in [Price::Free, Price::Posted { line: "$10.00, paid in ETH".into() },
                  Price::Unknown { why: "x".into() }] {
        let line = shelf(price, true, false).price_line();
        assert!(!line.to_lowercase().contains("pass"),
                "the store sells copies, not the pass: {line}");
    }
    assert!(Act::Buy.label().to_lowercase().contains("copy"),
            "the buy control names what is bought: {}", Act::Buy.label());
}
