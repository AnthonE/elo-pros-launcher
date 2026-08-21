//! The skin — the 2003/2004 Steam client, in FLTK.
//!
//! The client's palette, and the split is the same,
//! because it is a rule and not a style:
//!
//!   the CHROME is OG-Steam's olive. Sampled off the client screenshots and
//!   cross-checked against `resource/styles/steam.styles` in ungstein/OG-Steam,
//!   which is where `--store-muted: #a0aa95` on the website already came from
//!   (`SITE-PLATFORM.md` §14a). Window body, bevels, buttons, list wells.
//!
//!   the MEANING colours are elo's. [`LIVE`] fills a control that moves money
//!   or reads a chain that is armed. [`GOLD`] marks in-build / not-armed.
//!   Green ink and green rules are decoration and promise nothing; **a solid
//!   green FILL is reserved.** That is the website's law (`SITE-PLATFORM.md`
//!   §14b), and it does not get relaxed because the widget toolkit changed.
//!   [`Tone`] is the one place it is enforced here, `tests/palette.rs` and
//!   `tests/rows.rs` are what fail when it is not.
//!
//! The values were kept byte-identical to the Python client's while both
//! existed — two clients wearing almost the same olive is worse than two
//! wearing different ones. That client was deleted on 2026-08-07, so these are
//! now the only copy and there is nothing left to compare them against;
//! `tests/palette.rs` gates what survives that, which is the reservation
//! itself.

use fltk::enums::Color;

/// `#rrggbb` as FLTK wants it. `const fn` so every colour below is a compile
/// time constant and a typo is a build error rather than a wrong pixel.
const fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

// ── the olive (OG-Steam) ─────────────────────────────────────────────────────
pub const BG: Color = rgb(0x4c5844); // window body
pub const PANEL: Color = rgb(0x57634e); // a raised region on the body
pub const WELL: Color = rgb(0x3e4838); // sunken: lists, text areas
pub const WELL_ALT: Color = rgb(0x454f3e); // every other row in a list
pub const SEL: Color = rgb(0x2b3325); // a selected row
pub const LINE_HI: Color = rgb(0x7f8d72); // bevel, light edge
pub const LINE_LO: Color = rgb(0x2f3729); // bevel, dark edge
pub const EDGE: Color = rgb(0x3a4433); // a plain 1px rule

pub const INK: Color = rgb(0xe6ecdd); // primary text on olive
pub const MUTED: Color = rgb(0xa0aa95); // steam.styles Text="160 170 149"
pub const DIM: Color = rgb(0x8a9689); // tertiary — still legible on BG
pub const HEAD: Color = rgb(0xffffff); // a heading

pub const BTN: Color = rgb(0xc3c9b6);
pub const BTN_HI: Color = rgb(0xe2e6da);
pub const BTN_LO: Color = rgb(0x8f9784);
pub const BTN_INK: Color = rgb(0x1c2117);
pub const BTN_INK_OFF: Color = rgb(0x6b7263);

// ── the meaning colours (elo) ───────────────────────────────────────────────
/// Armed, settled, money moved. **A solid fill in this colour is a promise.**
pub const LIVE: Color = rgb(0x00c805);
/// Structure.
pub const CORE: Color = rgb(0x107c10);
/// In build · not armed · caution.
pub const GOLD: Color = rgb(0xc9a227);
/// A breach, a failed hash, an unreachable origin.
pub const RED: Color = rgb(0xff3b30);

/// What a control is allowed to look like, and the only place the reservation
/// is expressible.
///
/// There is deliberately **no** `Tone` that pairs a green fill with anything
/// unarmed. A caller cannot ask for one, so the law is enforced by the type
/// rather than by a reviewer noticing — which is the whole reason this is an
/// enum and not a colour argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// A pale button. Promises nothing. The default, and correct for almost
    /// everything.
    Plain,
    /// **A solid green fill.** Only for an act that moves money or reads a
    /// chain that is armed. If you are reaching for this to mean "the primary
    /// button", you want `Plain`.
    Live,
    /// A gold OUTLINE — the control exists, the thing behind it does not.
    /// Never a fill, so it can never be mistaken for the armed state. This is
    /// what an in-build title's Play button wears.
    Gold,
}

impl Tone {
    /// The button face. Note `Gold` returns the window body, not gold: it is
    /// an outline, and that asymmetry is the point.
    pub fn face(self) -> Color {
        match self {
            Tone::Plain => BTN,
            Tone::Live => LIVE,
            Tone::Gold => BG,
        }
    }

    /// The label ink.
    pub fn ink(self) -> Color {
        match self {
            Tone::Plain => BTN_INK,
            Tone::Live => rgb(0x04240a),
            Tone::Gold => GOLD,
        }
    }

    /// Does this tone put a solid green fill on screen? Exactly one does, and
    /// `tests/palette.rs` asserts that stays true — the reservation is a
    /// counted property, not a convention.
    pub fn is_reserved_fill(self) -> bool {
        self.face() == LIVE
    }
}
