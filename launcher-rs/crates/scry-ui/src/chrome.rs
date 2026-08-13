//! The widget kit — bevels and buttons, the era's whole visual grammar.
//!
//! A 1px two-tone border and nothing else. `raised` lights the top-left;
//! sunken lights the bottom-right, which is what makes a list well read as a
//! hole rather than a rectangle. The original client drew these with 1px frames
//! because that is how the era did it and because it renders identically on
//! every window manager; FLTK has the same shapes as real frame types, so here
//! they are the toolkit's own `ThinUpBox`/`ThinDownBox` and cost no widgets.

use crate::theme::{self, Tone};
use fltk::{button, enums, frame, group, image, prelude::*, window};

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
///
/// ⚠ **Ends itself.** Anything built after this call is a child of the WINDOW,
/// drawn on top of the well rather than inside it, which is what the fixed
/// screens want: the well is a backdrop and the labels sit on it. [`shelf`] is
/// the other convention and the two must not be confused — see its note.
pub fn well(x: i32, y: i32, w: i32, h: i32) -> group::Group {
    let mut g = group::Group::new(x, y, w, h, None);
    g.set_frame(enums::FrameType::ThinDownBox);
    g.set_color(theme::WELL);
    g.end();
    g
}

/// A well that **scrolls** — the same sunken hole, for a list whose length is
/// not this program's to choose.
///
/// ⚠ **This is the fix for rows that were drawn off the window**, not a
/// refinement. Every shelf sized its well with `.clamp(_, 520)` and then drew
/// its rows down the window at a fixed pitch, so a library of fifteen games
/// painted five of them past the bottom edge: not clipped, not scrolled to —
/// **gone**, with their Play and Verify buttons, and nothing on screen saying
/// a row existed. A player with more games than the clamp allowed could not
/// reach them without uninstalling something. The count is the origin's and
/// the player's; the window has to hold any of them.
///
/// ⚠ **This one is left OPEN**, unlike [`well`]. Widgets built after this call
/// are children of the scroll and move with it; the caller must `end()` it.
/// That asymmetry is the whole point — a row that is not inside the scroll is
/// a row that does not scroll — and it is why this is a different function
/// with a different name rather than a flag on the other one.
pub fn shelf(x: i32, y: i32, w: i32, h: i32) -> group::Scroll {
    let mut s = group::Scroll::new(x, y, w, h, None);
    s.set_frame(enums::FrameType::ThinDownBox);
    s.set_color(theme::WELL);
    // Vertical only. A horizontal bar on a list whose columns are laid out in
    // fixed pixels would offer to scroll to nothing, and the era's lists did
    // not have one either.
    s.set_type(group::ScrollType::Vertical);
    s.set_scrollbar_size(13);
    // The bar wears the chrome too. FLTK's default is a grey that belongs to
    // the toolkit's own scheme and reads as a piece of another program.
    let mut bar = s.scrollbar();
    bar.set_color(theme::WELL);
    bar.set_selection_color(theme::BTN);
    bar.set_frame(enums::FrameType::FlatBox);
    s
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
/// ⚠ **`Align::Clip` is a safety difference rather than a cosmetic one.** FLTK
/// draws a label at the text's natural width and will run it straight past the
/// widget's box — off the edge of the window, or across whatever sits beside
/// it. A bounded character count is a different bound and not a substitute: a
/// hundred `W`s is three times the width of a hundred `i`s, so the only way to
/// keep drawn text inside a box is to clip it there.
///
/// ⚠ **`@` is DEFUSED here, and that is the second reason this function
/// exists.** FLTK reads `@` in a label as a *symbol* specification rather than
/// as text and [`defuse`] is the whole of the escaping. The clipping bound
/// would never have caught it: clipping decides where text is drawn, and this
/// decides what FLTK thinks the text *is*. It is a **parser reached by a
/// stranger's string**, which is the same shape of hole `art::sniff` refuses
/// at, one layer up.
///
/// **The full text goes on as a tooltip**, and it is defused too. A clip is the
/// only bound that holds, and a bound that holds still eats the end of a 140-
/// character blurb — the tooltip is where the rest of it stays reachable
/// without letting one row's text decide another row's layout. ⚠ FLTK draws
/// tooltips with symbols ENABLED (`Fl_Tooltip::draw_symbols_` is a compiled-in
/// `1`), so an undefused tooltip would be the same hole with a hover in front
/// of it. The escape renders back to the original characters either way.
///
/// Anything a game, an origin or a stranger supplies is drawn through this.
pub fn label_untrusted(x: i32, y: i32, w: i32, h: i32, text: &str, colour: enums::Color,
                       size: i32) -> frame::Frame {
    let safe = defuse(text);
    let mut f = label(x, y, w, h, &safe, colour, size);
    f.set_align(enums::Align::Left | enums::Align::Inside | enums::Align::Clip);
    if !text.trim().is_empty() {
        f.set_tooltip(&safe);
    }
    f
}

/// Make a stranger's text safe to hand FLTK as a label: **every `@` is
/// doubled.**
///
/// `@@` is the toolkit's own escape and `fl_draw` turns it back into one
/// literal `@`, so this changes what is parsed and not what is read.
///
/// ⚠ **Doubling only the FIRST one is not enough, and that is not the obvious
/// reading of FLTK's docs.** A label may carry a symbol at each END, and
/// `fl_draw.cxx` finds the second with `strrchr` — the *last* `@` in the whole
/// string, wherever it is:
///
/// ```text
///     if (str && (p = strrchr(str, '@')) != NULL && p > (str + 1) && p[-1] != '@')
/// ```
///
/// So a blurb reading `mail@example.test` does not draw `mail@example.test`. It
/// draws **`mail`** — the text is cut at the `@`, the rest becomes a symbol
/// name, an unknown symbol draws nothing, and the space it reserves shrinks the
/// text besides. A title called `Gates @ Home` loses everything from the `@`
/// on. The `p[-1] != '@'` in that line is why doubling every one closes both
/// ends at once.
///
/// Nothing else in a label is markup. `&` is a mnemonic only where
/// `fl_draw_shortcut` is set — menus, choices and tab labels — and none of
/// those carry a stranger's text in this client. Text this program wrote goes
/// through [`label`] and is not run through here at all: a heading that wants a
/// real symbol should be able to ask for one.
pub fn defuse(text: &str) -> String {
    text.replace('@', "@@")
}

/// The client's own mark, for the window's icon.
///
/// A scrying glass, in the chrome's own olive: circles, a rect and one stroke,
/// which is deliberately the whole of it. FLTK's SVG reader is nanosvg and
/// nanosvg **does not render `<text>`** — a mark built out of a letter would
/// draw as an empty square on every taskbar, and it would draw that way
/// silently.
const MARK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48">
<rect width="48" height="48" rx="9" fill="#3e4838"/>
<rect x="1.5" y="1.5" width="45" height="45" rx="7.5" fill="none" stroke="#7f8d72" stroke-width="1.5"/>
<path d="M28 28 L38 38" stroke="#e6ecdd" stroke-width="5" stroke-linecap="round"/>
<circle cx="21" cy="21" r="11" fill="#4c5844" stroke="#e6ecdd" stroke-width="3.5"/>
<circle cx="21" cy="21" r="4" fill="#c9a227"/>
</svg>"##;

/// Give a window the client's icon — its taskbar tile, its alt-tab card, and
/// on Windows the glyph in its own title bar.
///
/// **Every window this client opens calls this**, not just the first one. A
/// player alt-tabbing away from the consent prompt or the lock screen is
/// looking at a list of icons, and a window with none gets the desktop's
/// default: a blank page on Windows, a grey box or the raw X11 default on
/// Linux. Two of this client's windows are the ones a stranger's game can
/// cause to appear, so *"which program is asking"* is worth a picture.
///
/// ⚠ **Best effort, and silent when it fails.** `set_icon` is a window-manager
/// request: some do not honour it, and a headless run has no window manager at
/// all. A launcher that refused to open a window over its icon would be
/// trading the program for its decoration, so a decode that fails leaves the
/// window exactly as it was.
pub fn wear_icon(w: &mut window::Window) {
    if let Ok(mut img) = image::SvgImage::from_data(MARK) {
        // Rasterised here rather than left as a vector: `set_icon` hands the
        // pixels to the window manager, so something has to choose a size.
        img.scale(48, 48, true, true);
        w.set_icon(Some(img));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A name beginning with `@` is a name, not a drawing instruction.
    ///
    /// An origin's catalog, a title's manifest and a game's consent text all
    /// reach a label, and FLTK reads a leading `@` as a symbol spec. Left
    /// alone, a title called `@home` draws as a glyph or as nothing — the
    /// catalog choosing what this client renders.
    #[test]
    fn a_leading_at_sign_is_escaped_rather_than_obeyed() {
        assert_eq!(defuse("@home"), "@@home");
        assert_eq!(defuse("@"), "@@");
        // FLTK's spec is `@`, an optional transform, then a symbol name. All
        // of these are a symbol to the toolkit and text to a reader.
        assert_eq!(defuse("@2>"), "@@2>");
        assert_eq!(defuse("@+9circle"), "@@+9circle");
    }

    /// ⚠ **And every `@` after it**, which is the half that is easy to get
    /// wrong: FLTK looks for a second symbol with `strrchr`, so the LAST `@`
    /// anywhere in the string starts one. An address in a blurb draws as
    /// `mail` and a title called `Gates @ Home` draws as `Gates` — the text
    /// after the sign is taken as a symbol name and thrown away.
    #[test]
    fn an_at_sign_anywhere_else_is_escaped_too() {
        assert_eq!(defuse("mail@example.test"), "mail@@example.test");
        assert_eq!(defuse("Gates @ Home"), "Gates @@ Home");
        assert_eq!(defuse("a @ b @ c"), "a @@ b @@ c");
        // Nothing is left for `strrchr` to find that is not already doubled,
        // which is the property the whole escape rests on.
        for text in ["@home", "mail@example.test", "Gates @ Home", "who@", "a@b"] {
            let out = defuse(text);
            let bytes = out.as_bytes();
            for (i, b) in bytes.iter().enumerate() {
                if *b == b'@' {
                    assert!(
                        (i > 0 && bytes[i - 1] == b'@') || bytes.get(i + 1) == Some(&b'@'),
                        "a lone @ survived at {i} in {out:?}, and FLTK will read it"
                    );
                }
            }
        }
    }

    /// Text with no `@` in it is handed over exactly as it arrived.
    #[test]
    fn nothing_without_an_at_sign_is_touched() {
        assert_eq!(defuse("Gates"), "Gates");
        assert_eq!(defuse(""), "");
        assert_eq!(defuse("a three-room delve — fight, sneak, or leave"),
                   "a three-room delve — fight, sneak, or leave");
        // Multi-byte input goes through without panicking: a slug is
        // origin-supplied and byte-slicing one is how a title's NAME takes the
        // window down (`art::initial` learned this first).
        assert_eq!(defuse("日本語"), "日本語");
        assert_eq!(defuse("@日本語"), "@@日本語");
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
