//! The elo desktop client's windows.
//!
//! This crate owns every window. It draws and it dispatches; it computes
//! nothing. Anything that decides — what is installed, whether a build is
//! stale, what a depot digest is — comes from `elo-depot`, which is the same
//! rule Gates keeps on its renderer (`Gates/CLAUDE.md`: *Bevy draws, it does
//! not decide*) and for the same reason: state that leaks into the widget
//! layer stops being covered by the tests that guard it.
//!
//! # Credit — required, and no gate can see it
//!
//! **This program is based in part on the work of the FLTK project
//! (<https://www.fltk.org>).**
//!
//! That sentence is an obligation, not a courtesy. `fltk` and `fltk-sys` both
//! declare `MIT` — that is the Rust binding — while `fltk-sys` vendors the
//! FLTK C++ library itself, which is the **FLTK License: LGPL with
//! exceptions**. Exception 3 says static linking is not a derivative work and
//! needs no source disclosure; exception 4 waives shipping the licence text.
//! What survives both is one requirement: *"programs must still identify their
//! use of FLTK"*, with that exact example statement.
//!
//! ⚠ **`cargo deny check licenses` reads crate metadata, sees MIT, and passes
//! whether or not this notice exists.** So the notice is asserted by
//! `tests/credit.rs` instead. A licence obligation that only a human
//! remembers is one release away from being unmet.

/// A title's icon, and the placeholder for a title that publishes none. The
/// bytes come from the origin; the refusals are here.
pub mod art;
pub mod chrome;
/// How a game's ask for a signature reaches the player, across the one thread
/// boundary in this client. No FLTK, so the rules are testable with no display.
pub mod consent;
pub mod theme;
pub mod windows;
/// What the windows' controls do. Separate from `windows` on purpose — see
/// that module's header: the windows draw and this decides.
pub mod wiring;

/// The credit required by the FLTK License, exactly as its exception 4 words
/// it. Rendered in the client's About window and asserted by `tests/credit.rs`
/// — kept as a constant so there is one copy and it cannot drift out of the
/// window it is shown in.
pub const FLTK_CREDIT: &str =
    "This program is based in part on the work of the FLTK project (https://www.fltk.org).";
