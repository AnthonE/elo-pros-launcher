//! The FLTK credit is an obligation, and no licence gate can see it.
//!
//! `fltk` and `fltk-sys` both declare `MIT` — that is the Rust binding.
//! `fltk-sys` vendors the FLTK C++ library, which is the **FLTK License**:
//! LGPL with exceptions. Exception 3 makes static linking not a derivative
//! work and requires no source disclosure; exception 4 waives shipping the
//! licence text. One requirement survives both:
//!
//!   > However, programs must still identify their use of FLTK. The following
//!   > example statement can be included in user documentation to satisfy this
//!   > requirement:
//!   >
//!   >     [program/widget] is based in part on the work of the
//!   >     FLTK project (https://www.fltk.org).
//!
//! ⚠ `cargo deny check licenses` reads crate metadata, sees MIT, and passes
//! whether or not that sentence exists anywhere. So it is asserted here. A
//! licence obligation that only a human remembers is one release away from
//! being unmet — and the notice costs a line, while the omission is the kind
//! of thing that gets found by someone else.

#[test]
fn the_fltk_credit_says_what_the_licence_requires() {
    let credit = scry_ui::FLTK_CREDIT;
    assert!(
        credit.contains("FLTK project"),
        "the credit must name the FLTK project: {credit:?}"
    );
    assert!(
        credit.contains("based in part on the work"),
        "use the licence's own example wording — it is what satisfies the \
         requirement without argument: {credit:?}"
    );
    assert!(
        credit.contains("https://www.fltk.org"),
        "the licence's example statement carries the url: {credit:?}"
    );
}

/// The credit must be **shown**, not merely defined. A constant nobody renders
/// satisfies nothing, so the About window is built and its labels are searched
/// for it.
#[test]
fn the_about_window_actually_renders_it() {
    // FLTK needs no display to construct widgets, so this runs headless. If
    // that ever stops being true this test fails loudly rather than skipping.
    use fltk::prelude::*;
    let w = scry_ui::windows::about();
    let mut shown = false;
    for i in 0..w.children() {
        if let Some(child) = w.child(i) {
            if child.label().contains("FLTK project") {
                shown = true;
                break;
            }
        }
    }
    assert!(
        shown,
        "scry_ui::FLTK_CREDIT is defined but the About window does not display \
         it — the licence requires the program to identify its use of FLTK, and \
         a constant no window renders identifies nothing"
    );
}
