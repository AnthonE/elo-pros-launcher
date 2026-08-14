//! The widget kit — bevels and buttons, the era's whole visual grammar.
//!
//! A 1px two-tone border and nothing else. `raised` lights the top-left;
//! sunken lights the bottom-right, which is what makes a list well read as a
//! hole rather than a rectangle. The original client drew these with 1px frames
//! because that is how the era did it and because it renders identically on
//! every window manager; FLTK has the same shapes as real frame types, so here
//! they are the toolkit's own `ThinUpBox`/`ThinDownBox` and cost no widgets.

use crate::theme::{self, Tone};
use fltk::{button, enums, frame, group, prelude::*};

/// A raised panel — a region sitting on the window body.
pub fn panel(x: i32, y: i32, w: i32, h: i32) -> group::Group {
    let mut g = group::Group::new(x, y, w, h, None);
    g.set_frame(enums::FrameType::ThinUpBox);
    g.set_color(theme::PANEL);
    g.end();
    g
}

/// A sunken well — a list, a text area, the store's scroll region. Sunken on
/// purpose: it is the shape that says "things go in here".
pub fn well(x: i32, y: i32, w: i32, h: i32) -> group::Group {
    let mut g = group::Group::new(x, y, w, h, None);
    g.set_frame(enums::FrameType::ThinDownBox);
    g.set_color(theme::WELL);
    g.end();
    g
}

/// A heading, or any label on the window body.
pub fn label(x: i32, y: i32, w: i32, h: i32, text: &str, colour: enums::Color,
             size: i32) -> frame::Frame {
    let mut f = frame::Frame::new(x, y, w, h, None);
    f.set_label(text);
    f.set_label_color(colour);
    f.set_label_size(size);
    f.set_align(enums::Align::Left | enums::Align::Inside);
    f
}

/// A label carrying text this program did not write.
///
/// ⚠ **The only difference from [`label`] is `Align::Clip`, and it is a safety
/// difference rather than a cosmetic one.** FLTK draws a label at the text's
/// natural width and will run it straight past the widget's box — off the edge
/// of the window, or across whatever sits beside it. A bounded character count
/// is a different bound and not a substitute: a hundred `W`s is three times
/// the width of a hundred `i`s, so the only way to keep drawn text inside a
/// box is to clip it there.
///
/// Anything a game, an origin or a stranger supplies is drawn through this.
pub fn label_untrusted(x: i32, y: i32, w: i32, h: i32, text: &str, colour: enums::Color,
                       size: i32) -> frame::Frame {
    let mut f = label(x, y, w, h, text, colour, size);
    f.set_align(enums::Align::Left | enums::Align::Inside | enums::Align::Clip);
    f
}

/// A pale beveled push button that flips its bevel on press, like the era's.
///
/// [`Tone`] is the only way to change how it looks, which is deliberate — see
/// `theme::Tone`. There is no colour parameter, so no caller can paint a
/// button green without asking for the tone whose name says what green means.
pub fn button(x: i32, y: i32, w: i32, h: i32, text: &str, tone: Tone) -> button::Button {
    let mut b = button::Button::new(x, y, w, h, None);
    b.set_label(text);
    b.set_color(tone.face());
    b.set_label_color(tone.ink());
    b.set_label_size(13);
    b.set_frame(enums::FrameType::ThinUpBox);
    b.set_down_frame(enums::FrameType::ThinDownBox);
    if tone == Tone::Gold {
        // The outline, not a fill. `Tone::Gold.face()` is already the window
        // body; this draws the gold as a border so the control reads as
        // present-but-not-armed rather than as a dark button.
        b.set_frame(enums::FrameType::BorderBox);
        b.set_selection_color(theme::BG);
    }
    b
}

/// A disabled button — present, legible, and obviously not pressable. Kept as
/// its own call rather than a flag so the ink change cannot be forgotten.
pub fn button_off(x: i32, y: i32, w: i32, h: i32, text: &str) -> button::Button {
    let mut b = button(x, y, w, h, text, Tone::Plain);
    b.set_label_color(theme::BTN_INK_OFF);
    b.set_color(theme::BTN_LO);
    b.deactivate();
    b
}
