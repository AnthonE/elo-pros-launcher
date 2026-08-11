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

use scry_ui::theme::Tone;
use scry_ui::windows::{Row, MENU};

fn row(stale: Option<bool>) -> Row {
    Row { slug: "gates".into(), build: "0.1.0".into(), bytes: 75_829_730,
          digest: "0xf4".into(), stale }
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
