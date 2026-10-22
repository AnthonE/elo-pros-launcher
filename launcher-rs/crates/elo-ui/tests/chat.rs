//! The Chat window builds — with no relay, no origin and no display.
//!
//! ⚠ **This is the test that would have caught a crash.** The window is built
//! at launcher startup, and its first draft called `set_column_char('\0')`,
//! which fltk-rs asserts against: the launcher died before its menu drew, and
//! nothing in the suite constructed this window. Building needs no display, so
//! this runs everywhere. What the window shows once painted is
//! `chat_paint.rs`, which needs one.
//!
//! One widget-building test per file — `shelves.rs` says why.

use elo_ui::windows;
use fltk::prelude::*;

#[test]
fn the_window_builds() {
    let c = windows::chat();
    assert!(c.window.width() > 0);
    assert!(c.make_key.visible(), "a fresh window offers a chat key until a view says otherwise");
}
