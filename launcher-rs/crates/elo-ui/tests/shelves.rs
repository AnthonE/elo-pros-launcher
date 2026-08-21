//! A shelf must hold every row it was given, whatever the number is.
//!
//! This file exists because of a defect that every unit test passed over and
//! every screenshot hid: the two shelves sized their well with `.clamp(_, 520)`
//! and then drew their rows down the window at a fixed pitch. With three games
//! installed that is a window. With fifteen it is nine games, five rows painted
//! across the footer and off the bottom edge, and rows that never reached a
//! pixel — **with their Play and Verify buttons**. Nothing said a row was
//! missing and nothing could scroll to one: the only way to reach the tenth
//! game was to uninstall one of the first nine.
//!
//! How many rows there are is the origin's business and the player's. The
//! window's business is to hold any of them, so the well scrolls
//! (`chrome::shelf`), and the properties below are what keep it doing so.
//!
//! ⚠ **One widget-building test per file, and that is a toolkit rule rather
//! than a style.** FLTK pins itself to the first thread that constructs a
//! widget and panics on every other one; the test harness gives each `#[test]`
//! its own thread. So a second test in this binary that builds a window fails
//! with `assertion failed: is_ui_thread()` — not because anything is wrong,
//! but because it went second. `credit.rs` and `shards.rs` have the same shape
//! for the same reason. Split this file rather than adding to it.
//!
//! FLTK needs no display to construct widgets, so all of this runs headless.

use fltk::prelude::*;
use elo_ui::art;
use elo_ui::windows::{self, Price, Row, Shelf};

fn row(i: usize) -> Row {
    Row {
        slug: format!("title-{i}"),
        build: "0.1.0".into(),
        bytes: 75_829_730,
        digest: "0xf4".into(),
        stale: Some(false),
        name: Some(format!("Title {i}")),
        icon: None,
        why: None,
    }
}

fn shelf_row(i: usize) -> Shelf {
    Shelf {
        slug: format!("title-{i}"),
        name: format!("Title {i}"),
        blurb: "a game".into(),
        state: "live".into(),
        installable: true,
        installed: false,
        price: Price::Free,
        icon: None,
    }
}

/// The group a row was drawn into, having first refused the one that broke.
///
/// A row whose parent is the WINDOW is a row nothing can scroll — it is drawn
/// at whatever coordinate the loop reached and the window's edge is the end of
/// it. That is the defect, so it is the first thing checked.
fn shelf_of<W: GroupExt, C: WidgetExt>(window: &W, control: &C) -> fltk::group::Group {
    let parent = control.parent().expect("a drawn control has a parent");
    assert!(
        !parent.is_same(window),
        "a row is a direct child of the window, so nothing can scroll it — \
         this is the state where the tenth game was drawn off the bottom edge"
    );
    parent
}

/// Every square among a group's children, by side length. A title's icon is
/// the one frame as wide as it is tall, so this finds it without the window
/// having to hand anything back.
fn squares<G: GroupExt>(g: &G) -> Vec<i32> {
    (0..g.children())
        .filter_map(|i| g.child(i))
        .filter(|c| c.width() == c.height())
        .map(|c| c.width())
        .collect()
}

/// **The library draws the smaller square, and the store keeps the bigger.**
///
/// Operator, 2026-08-13: *"we should we using smaller icons for games."* Kept
/// as a relation rather than as two numbers, because the numbers are allowed
/// to be tuned and the relation is not: the Store is browsed one row at a time
/// and the library is scanned, so shrinking the shelf's art to match would
/// cost the shelf the thing it is a shelf for.
///
/// No widgets here — see the file header for why that matters.
///
/// ⚠ `assertions_on_constants` is allowed rather than worked around. Clippy is
/// right that both sides are known at compile time; that IS the assertion —
/// the relation between two constants is exactly the thing an edit is allowed
/// to break silently, and a test that reads them is where it stops being
/// silent.
#[test]
#[allow(clippy::assertions_on_constants)]
fn the_library_scans_small_and_the_store_browses_big() {
    assert!(
        art::ROW_ICON < art::SHELF_ICON,
        "the library's square must be the smaller of the two: {} vs {}",
        art::ROW_ICON,
        art::SHELF_ICON
    );
    assert!(
        art::ROW_ICON >= 16,
        "smaller is not free: below about 16px a title's art is a smudge and \
         the placeholder letter stops being readable ({})",
        art::ROW_ICON
    );
}

/// Everything that needs a widget, in the one test allowed to build one.
#[test]
fn a_shelf_holds_every_row_it_is_given() {
    // ── the library, at a size no desktop shows at once ──────────────────────
    let games = windows::games(&(0..40).map(row).collect::<Vec<_>>());
    assert_eq!(games.rows.len(), 40, "every install gets a row with its own controls");

    let shelf = shelf_of(&games.window, &games.rows[0].act);
    for c in &games.rows {
        let p = c.act.parent().expect("a drawn control has a parent");
        assert!(p.is_same(&shelf),
                "every row shares one shelf, or some of them scroll and some do not");
    }

    // The shelf is a viewport inside the window, not a hole cut in the bottom
    // of the screen.
    assert!(
        shelf.y() + shelf.height() <= games.window.height(),
        "the shelf runs past the window's own bottom edge: {} vs {}",
        shelf.y() + shelf.height(),
        games.window.height()
    );
    // And there is genuinely more content than viewport, which is the state
    // the old code drew off the edge instead of scrolling. If this ever fails
    // the test has stopped exercising the thing it is here for.
    assert!(
        games.rows[39].act.y() > shelf.y() + shelf.height(),
        "40 rows should overflow the shelf"
    );

    // ── the same property on the store, whose count is the ORIGIN's ──────────
    let store = windows::store(&(0..40).map(shelf_row).collect::<Vec<_>>(), true, "");
    assert_eq!(store.rows.len(), 40);
    let store_shelf = shelf_of(&store.window, &store.rows[0].button);
    assert!(store_shelf.y() + store_shelf.height() <= store.window.height());
    assert!(
        store.rows[39].button.y() > store_shelf.y() + store_shelf.height(),
        "40 titles should overflow the shelf"
    );

    // ── the clamp still bounds the WINDOW, which is the other half ───────────
    //
    // A window that grew with its row count would eventually be taller than
    // the desktop. What changed is that the clamp now bounds the viewport and
    // never the library.
    assert!(
        games.window.height() <= 620,
        "a 40-game library opened a {}px window",
        games.window.height()
    );

    // ── the scrollbar's reservation ──────────────────────────────────────────
    //
    // FLTK draws `Fl_Scroll`'s bar INSIDE the well rather than beside it, so a
    // control laid out to the well's edge is one the bar sits on top of —
    // exactly on the shelves long enough to have a bar. `windows::BAR` is the
    // reservation; this is it checked rather than trusted.
    let bar_left = shelf.x() + shelf.width() - 15;
    for c in &games.rows {
        let right = c.verify.x() + c.verify.width();
        assert!(right < bar_left,
                "Verify reaches {right} and the scrollbar starts at {bar_left}");
    }
    let bar_left = store_shelf.x() + store_shelf.width() - 15;
    for c in &store.rows {
        let right = c.page.x() + c.page.width();
        assert!(right < bar_left,
                "Page… reaches {right} and the scrollbar starts at {bar_left}");
    }

    // ── the smaller square, where it is actually drawn ───────────────────────
    //
    // The constants are a pair (`the_library_scans_small_and_the_store_browses_big`);
    // this is the half that catches one window quietly taking the other's
    // number. And the point of the smaller one was rows on screen, so that is
    // asserted as rows and not as pixels.
    assert!(
        squares(&shelf).contains(&art::ROW_ICON),
        "the library row draws no {}px square: {:?}",
        art::ROW_ICON,
        squares(&shelf)
    );
    assert!(
        squares(&store_shelf).contains(&art::SHELF_ICON),
        "the store row draws no {}px square: {:?}",
        art::SHELF_ICON,
        squares(&store_shelf)
    );
    let pitch = games.rows[1].act.y() - games.rows[0].act.y();
    let visible = shelf.height() / pitch;
    assert!(
        visible >= 12,
        "a {}px shelf at a {pitch}px pitch shows {visible} games; the 44px \
         square showed nine and more was the point of the smaller one",
        shelf.height()
    );

    // ── an empty shelf is still a window ─────────────────────────────────────
    //
    // The state where getting the geometry wrong is silent, because there are
    // no rows to look wrong. Not an error state and not styled as one.
    let empty = windows::games(&[]);
    assert!(empty.rows.is_empty());
    assert!(
        empty.window.width() > 300 && empty.window.height() > 120,
        "an empty library still has to look like a window"
    );
    let unreadable = windows::store(&[], false, "dns error");
    assert!(unreadable.rows.is_empty());
    assert!(unreadable.window.height() > 120);
}
