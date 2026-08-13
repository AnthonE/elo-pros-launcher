//! A title's icon — the small square on a shelf row.
//!
//! The operator's ask (2026-08-12): *"make sure we show of the games with a
//! tiny icon or something, whatever gates has or placeholder. should pull from
//! the backend."* So the bytes come from the origin's own listing art
//! (`meter/launcher.py` puts an `art.icon` url on every manifest, derived from
//! the shelf row rather than typed), and everything below is what happens to
//! them after they arrive.
//!
//! # Three rules, and each one is a refusal
//!
//! 1. **A missing icon is a placeholder, never a blank.** A row with no art is
//!    a normal state — a title publishes art when it has some — and the
//!    placeholder says which title it is (the initial) rather than pretending
//!    the shelf failed to draw.
//! 2. **A decode that fails is a placeholder too, and it never panics.** These
//!    are bytes from an origin, run through a C image parser. `SvgImage` and
//!    `PngImage` both return `Err` on garbage and that is the whole handling:
//!    the row keeps its words, which were never the image's to carry.
//! 3. **Art is never load-bearing.** Nothing on a row's meaning is in the
//!    picture. Take every icon away and the Store still says what is for sale,
//!    what is free, and what is installed — which is why fetching them on the
//!    quick agent (`scry_net::Net::art`) costs nothing worth having.
//!
//! # Why the KIND is sniffed and not taken from the url
//!
//! A url's extension is a claim by whoever wrote the url. The magic bytes are
//! a claim by whoever wrote the file, which is the same party — but sniffing
//! means a `.svg` that is really a PNG still draws, and neither one is handed
//! to the wrong parser. FLTK's SVG reader is nanosvg and its PNG reader is
//! libpng; feeding either the other's bytes is the one avoidable way to reach
//! a C parser's error path on purpose.

use crate::theme;
use fltk::{enums, frame, image, prelude::*};

/// How big a **store** icon is drawn, in px. The Store is a shelf somebody
/// browses one row at a time — three lines of text per row, a price among them
/// — so the art is given room to be looked at.
pub const SHELF_ICON: i32 = 44;

/// How big a **library** icon is drawn, in px.
///
/// Operator, 2026-08-13: *"we should we using smaller icons for games."* Two
/// numbers where there was one, and the split is a rule rather than a taste:
/// **the Store is browsed and the library is scanned.** A player opens Games
/// to find a title they already own and start it, which is a list — and a list
/// is measured in how many rows reach the eye at once, not in how big the art
/// is. At 44px a 620px window held nine games; at 28 it holds thirteen, and
/// the row still carries its name, its size and its state on two lines.
///
/// ⚠ The pair is kept HERE rather than one each in the two windows, which is
/// what the single number was protecting against. `windows::ROW_H` and
/// `windows::SHELF_H` are both derived from these, so a row can never be
/// shorter than the square it has to hold.
pub const ROW_ICON: i32 = 28;

/// A title's icon bytes, with the parser they belong to already decided.
///
/// Cheap to clone and holds no FLTK handle: an `Icon` can be built on any
/// thread and carried into the window that draws it, which is what lets the
/// binary fetch a shelf's art before a single widget exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    bytes: Vec<u8>,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Svg,
    Png,
    Jpeg,
}

impl Icon {
    /// Take bytes off the wire, or refuse them.
    ///
    /// `None` for anything this cannot name — which is the honest answer for a
    /// 404 page, an HTML error body, or a format nobody wired. The caller draws
    /// the placeholder and the row is unharmed.
    pub fn decode(bytes: Vec<u8>) -> Option<Icon> {
        let kind = sniff(&bytes)?;
        Some(Icon { bytes, kind })
    }

    /// Draw into a frame at `size`, or say it could not.
    ///
    /// ⚠ **The image is built HERE and not in `decode`**, because an
    /// `SvgImage` is an FLTK handle and FLTK handles belong to the thread that
    /// draws. `decode` runs wherever the fetch did — a background read, a
    /// test, a headless job — and this runs on the UI thread with the frame in
    /// hand.
    pub fn draw_into(&self, f: &mut frame::Frame, size: i32) -> bool {
        // Each arm sets its own concrete image type rather than converting to
        // a common one. `SharedImage::from_image` reinterprets its argument as
        // an `Fl_RGB_Image`, which is a lie about a vector: an SVG has no
        // pixels at all until something asks it for a size.
        //
        // Proportional, never stretched — a capsule is 2:1 and a card is 3:4,
        // and a squashed logo reads as a broken client rather than as a crop.
        match self.kind {
            Kind::Svg => match image::SvgImage::from_data(&String::from_utf8_lossy(&self.bytes)) {
                Ok(mut i) => {
                    i.scale(size, size, true, true);
                    f.set_image(Some(i));
                    true
                }
                Err(_) => false,
            },
            Kind::Png => match image::PngImage::from_data(&self.bytes) {
                Ok(mut i) => {
                    i.scale(size, size, true, true);
                    f.set_image(Some(i));
                    true
                }
                Err(_) => false,
            },
            Kind::Jpeg => match image::JpegImage::from_data(&self.bytes) {
                Ok(mut i) => {
                    i.scale(size, size, true, true);
                    f.set_image(Some(i));
                    true
                }
                Err(_) => false,
            },
        }
    }

    /// What this is, for a log line. Never shown to a player.
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            Kind::Svg => "svg",
            Kind::Png => "png",
            Kind::Jpeg => "jpeg",
        }
    }
}

/// The format, off the first bytes. Nothing here trusts a filename.
fn sniff(bytes: &[u8]) -> Option<Kind> {
    if bytes.len() < 8 {
        return None;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Kind::Png);
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(Kind::Jpeg);
    }
    // SVG is XML, so the marker can be behind a declaration, a comment or a
    // doctype. Bounded on purpose: scanning a megabyte for "<svg" would let a
    // file that is not an SVG at all become one by mentioning it late.
    let head = &bytes[..bytes.len().min(1024)];
    let head = String::from_utf8_lossy(head);
    let head = head.trim_start();
    (head.starts_with("<svg")
        || head.starts_with("<?xml")
        || head.starts_with("<!DOCTYPE svg")
        || head.starts_with("<!-- "))
    .then_some(Kind::Svg)
}

/// The square drawn for a title, with its art or without it.
///
/// **Always drawn, art or no art.** A shelf where some rows have a square and
/// some have empty olive reads as a half-loaded page; one where every row has a
/// square and some squares carry a letter reads as a shelf.
pub fn icon_box(x: i32, y: i32, size: i32, slug: &str, icon: Option<&Icon>) -> frame::Frame {
    let mut f = frame::Frame::new(x, y, size, size, None);
    f.set_frame(enums::FrameType::ThinDownBox);
    f.set_color(theme::WELL);
    if let Some(i) = icon {
        if i.draw_into(&mut f, size) {
            return f;
        }
    }
    // The placeholder: the title's own initial, dim, centred. Not a broken
    // image glyph and not a logo of ours — a row is about the title, and a
    // house mark in the art slot would be scry advertising in a lister's seat.
    f.set_label(&initial(slug));
    f.set_label_color(theme::DIM);
    f.set_label_size(size / 2);
    f.set_align(enums::Align::Center | enums::Align::Inside);
    f
}

/// The letter a placeholder wears.
///
/// Uppercased from the first character rather than the first byte: a slug is
/// origin-supplied and `[..1]` on a multi-byte character panics, which would
/// make a title's NAME able to crash the shelf.
fn initial(slug: &str) -> String {
    slug.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "·".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_formats_are_told_apart_by_their_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\x00\x00"), Some(Kind::Png));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0\x00\x10JF"), Some(Kind::Jpeg));
        assert_eq!(sniff(b"<svg width=\"10\"></svg>"), Some(Kind::Svg));
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><svg></svg>"),
            Some(Kind::Svg)
        );
    }

    /// A 404 page, an error body, an empty reply. All of them arrive at this
    /// function eventually, and none of them may become an image.
    #[test]
    fn anything_that_is_not_an_image_is_refused_rather_than_guessed() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"<!DOCTYPE html><html><body>404"), None);
        assert_eq!(sniff(b"{\"error\": \"no such title\"}"), None);
        assert!(Icon::decode(b"not an image at all".to_vec()).is_none());
    }

    /// The extension is not consulted, so a mislabelled file still reaches the
    /// right parser — and, more importantly, never reaches the wrong one.
    #[test]
    fn the_kind_comes_from_the_bytes_and_not_from_a_name() {
        let png_bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        let i = Icon::decode(png_bytes).expect("png magic is a png");
        assert_eq!(i.kind_name(), "png", "a .svg url holding PNG bytes is a PNG");
    }

    /// A slug is origin-supplied. `&slug[..1]` on any of these panics, and a
    /// panicking shelf is a title's name taking the window down.
    #[test]
    fn a_placeholder_letter_never_panics_on_what_an_origin_sent() {
        assert_eq!(initial("gates"), "G");
        assert_eq!(initial("日本語"), "日");
        assert_eq!(initial("—dash-first"), "D");
        assert_eq!(initial(""), "·");
        assert_eq!(initial("···"), "·");
    }
}
