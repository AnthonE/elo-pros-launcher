//! The green fill is reserved, and here is the gate that says so.
//!
//! `SITE-PLATFORM.md` §14b is the law and `theme::Tone` is where the client
//! makes it un-askable: **a solid green fill means an act that moves money or
//! reads a chain that is armed**, and there is deliberately no `Tone` pairing
//! that colour with anything unarmed.
//!
//! ⚠ **This file exists because the gate it describes did not.** `theme.rs`
//! cited a `tests/palette.rs` that read the values out of the Python client's
//! `theme.py` and compared — a real test until the Python client was deleted
//! on 2026-08-07, after which the citation outlived both the file and the
//! thing it compared against. A law whose only enforcement is a comment
//! claiming a test is worse than one with no comment at all: it reads as
//! covered.
//!
//! So what is asserted is what can still be true with one client: the
//! reservation is a **counted** property of the enum, not a convention, and
//! adding a fourth tone that paints green is a failing test rather than a
//! judgement call in review.

use elo_ui::theme::{self, Tone};

/// Every tone the client can ask for. Listed by hand on purpose — `Tone` is
/// not an iterable enum, and a new variant should have to be added here, which
/// is the moment somebody reads the rest of this file.
const EVERY_TONE: [Tone; 3] = [Tone::Plain, Tone::Live, Tone::Gold];

/// Exactly one tone puts the reserved colour on screen as a fill.
#[test]
fn the_reserved_fill_belongs_to_exactly_one_tone() {
    let reserved: Vec<_> = EVERY_TONE.iter().filter(|t| t.is_reserved_fill()).collect();
    assert_eq!(
        reserved.len(),
        1,
        "the green fill is reserved for one meaning; {} tones paint it: {reserved:?}",
        reserved.len()
    );
    assert!(Tone::Live.is_reserved_fill(), "and it must be the one named for it");
}

/// Gold is an OUTLINE, and the asymmetry is the whole point: an in-build
/// control has to be un-mistakable for an armed one, so it never fills.
#[test]
fn gold_never_becomes_a_fill() {
    assert_eq!(Tone::Gold.face(), theme::BG, "gold fills with the window body, not with gold");
    assert_eq!(Tone::Gold.ink(), theme::GOLD);
    assert!(!Tone::Gold.is_reserved_fill());
}

/// The default tone promises nothing, and almost everything in the client
/// wears it — including every button on the consent prompt, which approves a
/// signature and moves no money.
#[test]
fn the_default_tone_promises_nothing() {
    assert!(!Tone::Plain.is_reserved_fill());
    assert_ne!(Tone::Plain.face(), theme::LIVE);
    // Legible: the pale face needs dark ink, and a tone whose label matched
    // its face would be a control nobody can read.
    assert_ne!(Tone::Plain.face(), Tone::Plain.ink());
}

/// Consent is not an armed act.
///
/// An EIP-191 message cannot be a transaction — the `0x19` prefix makes it
/// structurally impossible — so no control that approves one has earned the
/// reserved fill. Written as its own test because "Allow 100" is exactly the
/// kind of button somebody paints green to mean "the primary one".
#[test]
fn approving_a_signature_is_not_an_act_that_moves_money() {
    // The consent window builds every one of its buttons with `Tone::Plain`;
    // this pins the property that makes that correct rather than incidental.
    assert!(
        !Tone::Plain.is_reserved_fill(),
        "the consent prompt's buttons wear Plain — if Plain ever fills green, \
         every allowance button starts promising money"
    );
    for (label, _) in elo_ui::consent::ALLOWANCES {
        assert!(
            !label.to_ascii_lowercase().contains("pay")
                && !label.to_ascii_lowercase().contains("buy"),
            "{label} reads as a purchase; signing a message spends nothing"
        );
    }
}

/// The ink stays legible on the olive it sits on. Not a taste check — a
/// tertiary colour that drifted into the background is a line that silently
/// stops being read, and `DIM` is the one carrying the consent prompt's
/// "there is no always" note.
#[test]
fn every_ink_is_distinct_from_the_ground_it_sits_on() {
    for (name, ink) in [("INK", theme::INK), ("MUTED", theme::MUTED), ("DIM", theme::DIM)] {
        assert_ne!(ink, theme::BG, "{name} is invisible on the window body");
        assert_ne!(ink, theme::WELL, "{name} is invisible in a well");
    }
}
