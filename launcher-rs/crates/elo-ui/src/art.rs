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
//!    quick agent (`elo_net::Net::art`) costs nothing worth having.
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
    ///
    /// ⚠ **A raster is refused on its declared SIZE, before any decoder.**
    /// libpng and libjpeg read width and height from the header and FLTK
    /// allocates `w * h * d` at once, in `int`, under a `max_size` guard that
    /// defaults to `SIZE_MAX`. A 470 KB PNG declaring 11k x 11k is half a
    /// gigabyte; a 142-byte JPEG declaring 40000 x 40000 overflows the `int`
    /// and the decoder writes past the buffer. A header with no size this can
    /// read is refused too, because nothing then bounds it. A PNG of any shape
    /// but 8-bit RGB/RGBA, not interlaced, is refused as well (`plain_png`).
    pub fn decode(bytes: Vec<u8>) -> Option<Icon> {
        let kind = sniff(&bytes)?;
        let size = match kind {
            Kind::Png => png_size(&bytes),
            Kind::Jpeg => jpeg_size(&bytes),
            Kind::Svg => Some((1, 1)),
        };
        let (w, h) = size?;
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return None;
        }
        if kind == Kind::Png && !plain_png(&bytes) {
            return None;
        }
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

    /// The decoded image at `size`, for a caller that draws it itself — the
    /// chat feed paints faces into its own gutter rather than into a frame.
    ///
    /// Same thread rule as [`Icon::draw_into`]: call it where the drawing
    /// happens, and keep the result rather than decoding per paint.
    pub fn picture(&self, size: i32) -> Option<Picture> {
        let mut p = match self.kind {
            Kind::Svg => Picture::Svg(
                image::SvgImage::from_data(&String::from_utf8_lossy(&self.bytes)).ok()?,
            ),
            Kind::Png => Picture::Png(image::PngImage::from_data(&self.bytes).ok()?),
            Kind::Jpeg => Picture::Jpeg(image::JpegImage::from_data(&self.bytes).ok()?),
        };
        p.scale(size);
        Some(p)
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

/// A decoded picture, held to be painted again and again.
///
/// One enum rather than a shared image type for the reason `draw_into` gives:
/// `SharedImage::from_image` would reinterpret a vector as pixels.
pub enum Picture {
    Svg(image::SvgImage),
    Png(image::PngImage),
    Jpeg(image::JpegImage),
}

impl Picture {
    fn scale(&mut self, size: i32) {
        match self {
            Picture::Svg(i) => i.scale(size, size, true, true),
            Picture::Png(i) => i.scale(size, size, true, true),
            Picture::Jpeg(i) => i.scale(size, size, true, true),
        }
    }

    pub fn draw(&mut self, x: i32, y: i32, w: i32, h: i32) {
        match self {
            Picture::Svg(i) => i.draw(x, y, w, h),
            Picture::Png(i) => i.draw(x, y, w, h),
            Picture::Jpeg(i) => i.draw(x, y, w, h),
        }
    }
}

/// The largest side a raster may declare, in px. The widest art slot is the
/// 3840 px hero (`meter/storekeeper.py` `ART_SLOTS`), and the meter refuses an
/// upload past this same number; at it, the worst decode is 4096² x 4 bytes.
pub const MAX_SIDE: u32 = 4096;

/// `(width, height)` from a PNG's IHDR, which the format puts first.
fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 24 || &b[12..16] != b"IHDR" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    Some((be(16), be(20)))
}

/// `(width, height)` from a JPEG's first frame header, walking the segments
/// before it. Any SOF marker counts — baseline, progressive, lossless,
/// arithmetic — since each is one libjpeg would size a buffer from.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xff {
            return None;
        }
        let m = b[i + 1];
        match m {
            // Fill bytes before a marker.
            0xff => {
                i += 1;
                continue;
            }
            // Markers that carry no length.
            0x01 | 0xd0..=0xd7 => {
                i += 2;
                continue;
            }
            // Scan data or the end before any frame header: no size here.
            0xd9 | 0xda => return None,
            _ => {}
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        if matches!(m, 0xc0..=0xcf) && !matches!(m, 0xc4 | 0xc8 | 0xcc) {
            if i + 9 > b.len() {
                return None;
            }
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
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

/// Whether a PNG is the one shape handed to libpng: 8-bit RGB or RGBA, not
/// interlaced — the shape `meter/faces.py` already holds a face to.
///
/// ⚠ **This is a wall in front of a C decoder we cannot patch.** `fltk-sys`
/// vendors libpng 1.6.44 and statically links it; on arm64 its NEON palette
/// expansion reads and writes out of bounds on a crafted paletted PNG
/// (CVE-2026-33636, fixed upstream in 1.6.56), and the same release carries
/// other read-path advisories in interlace and bit-depth handling. Listing art
/// is a community developer's bytes, so the paths are refused here, before
/// libpng runs: a palette, a grey image, an interlace or a 16-bit depth is a
/// lettered placeholder. Art is never load-bearing, so the cost is a picture.
///
/// IHDR must be the first chunk (the PNG spec, and libpng enforces it), so the
/// shape read here is the shape libpng would act on.
fn plain_png(bytes: &[u8]) -> bool {
    // signature(8) · length(4) · "IHDR"(4) · width(4) · height(4) ·
    // depth · colour · compression · filter · interlace
    if bytes.len() < 29 || bytes[8..12] != [0, 0, 0, 13] || &bytes[12..16] != b"IHDR" {
        return false;
    }
    let (depth, colour, interlace) = (bytes[24], bytes[25], bytes[28]);
    depth == 8 && (colour == 2 || colour == 6) && interlace == 0
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
    // house mark in the art slot would be elo advertising in a lister's seat.
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
        let png_bytes = png_head(64, 64);
        let i = Icon::decode(png_bytes).expect("png magic is a png");
        assert_eq!(i.kind_name(), "png", "a .svg url holding PNG bytes is a PNG");
    }

    /// An 8-bit RGBA PNG's signature and IHDR at `w` x `h`.
    fn png_head(w: u32, h: u32) -> Vec<u8> {
        png_ihdr(w, h, 8, 6, 0)
    }

    /// A PNG's signature and IHDR, enough for `decode` to read size and shape.
    fn png_ihdr(w: u32, h: u32, depth: u8, colour: u8, interlace: u8) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[depth, colour, 0, 0, interlace]);
        b.extend_from_slice(&[0, 0, 0, 0]); // crc: libpng's to check, not ours
        b
    }

    /// SOI, an APP0 to walk past, then a baseline or progressive frame header.
    fn jpeg_head(sof: u8, w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
        b.extend_from_slice(b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00");
        b.extend_from_slice(&[0xff, sof, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
        b
    }

    /// **The C decoders allocate at the size the HEADER declares**, before a
    /// byte of pixel data, and FLTK's own guard (`Fl_RGB_Image::max_size`) is
    /// `SIZE_MAX`. A 470 KB PNG declaring 11k x 11k committed 484 MB; a
    /// 142-byte JPEG declaring 40000 x 40000 overflows FLTK's `int` size math
    /// into a heap overrun. So the size is read here and a picture larger
    /// than any art slot never reaches a decoder.
    #[test]
    fn an_image_declaring_a_huge_size_is_refused_before_any_decoder() {
        assert!(Icon::decode(png_head(11_000, 11_000)).is_none(), "png bomb");
        assert!(Icon::decode(png_head(64, 100_000)).is_none(), "one long side is enough");
        assert!(Icon::decode(png_head(0, 64)).is_none(), "an empty picture is not one");
        assert!(Icon::decode(jpeg_head(0xc0, 40_000, 40_000)).is_none(), "baseline jpeg bomb");
        assert!(Icon::decode(jpeg_head(0xc2, 40_000, 40_000)).is_none(), "progressive jpeg bomb");
        // A header with no size in it cannot be bounded, so it is not drawn.
        assert!(Icon::decode(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec()).is_none());
        assert!(Icon::decode(vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0, 0]).is_none());
        // Every real slot still draws: a 1000 x 1000 square, a capsule, a hero.
        assert!(Icon::decode(png_head(1000, 1000)).is_some());
        assert!(Icon::decode(jpeg_head(0xc0, 616, 353)).is_some());
        assert!(Icon::decode(jpeg_head(0xc2, 3840, 1240)).is_some());
    }

    /// **Only the PNG shape a face is held to reaches the bundled libpng.**
    /// fltk-sys vendors libpng 1.6.44, whose NEON palette expansion writes out
    /// of bounds on a crafted paletted image (CVE-2026-33636), and listing art
    /// is a community developer's bytes. A palette, an interlace or a 16-bit
    /// depth never reaches the decoder: the row draws its letter instead.
    #[test]
    fn a_png_libpng_would_expand_is_a_placeholder_not_a_decode() {
        let png_shape = |depth, colour, interlace| png_ihdr(64, 64, depth, colour, interlace);
        assert!(Icon::decode(png_shape(8, 2, 0)).is_some(), "8-bit RGB draws");
        assert!(Icon::decode(png_shape(8, 6, 0)).is_some(), "8-bit RGBA draws");
        assert!(Icon::decode(png_shape(8, 3, 0)).is_none(), "a palette is refused");
        assert!(Icon::decode(png_shape(4, 3, 0)).is_none(), "a low-depth palette is refused");
        assert!(Icon::decode(png_shape(8, 0, 0)).is_none(), "grey is refused");
        assert!(Icon::decode(png_shape(8, 2, 1)).is_none(), "interlaced is refused");
        assert!(Icon::decode(png_shape(16, 6, 0)).is_none(), "16-bit is refused");
        // A header that does not begin with IHDR is not a PNG anyone made.
        let mut odd = png_shape(8, 2, 0);
        odd[12..16].copy_from_slice(b"PLTE");
        assert!(Icon::decode(odd).is_none());
        assert!(Icon::decode(b"\x89PNG\r\n\x1a\n\x00\x00".to_vec()).is_none(), "too short to read");
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
