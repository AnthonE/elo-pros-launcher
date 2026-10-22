//! What `elo signin`'s watch prints for a page's signature request.
//!
//! The page writes two strings: the `text` to sign and a `why`. Both come off
//! the network and both are printed into a terminal, directly above
//! `sign it? [y/N]`. An ESC sequence in either can move the cursor up, erase
//! the real text and draw another one in its place, so the player says yes to
//! bytes they never saw.
//!
//! So: `why` is a claim and is drawn as one, through [`safe_line`]; it comes
//! FIRST, and the text comes last, right above the prompt. The text is never
//! cleaned up, because a cleaned text is not the text that would be signed —
//! a text that [`safe_line`] would change on any line is not shown at all,
//! and the watch refuses it without asking.

use elo_hive::render::safe_line;

/// The block to print before the prompt, or `None` when the text cannot be
/// shown as the exact bytes that would be signed.
pub fn ask_block(text: &str, why: &str) -> Option<String> {
    if text.split('\n').any(|line| safe_line(line) != line) {
        return None;
    }
    let mut out = String::new();
    let why = safe_line(why);
    if !why.trim().is_empty() {
        // The page's own stated reason — a claim, and drawn as one, the same
        // rule the broker's consent prompt holds.
        out.push_str(&format!("(the page says: {})\n", why.trim()));
    }
    out.push_str("the page asks for a signature over exactly this:\n──\n");
    out.push_str(text);
    out.push_str("\n──");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "elo play\naction: sit\nday: 2026-10-22";

    #[test]
    fn a_plain_ask_shows_why_first_and_the_text_last() {
        let b = ask_block(TEXT, "claim the drop").unwrap();
        assert!(b.starts_with("(the page says: claim the drop)\n"));
        assert!(b.ends_with(&format!("{TEXT}\n──")));
    }

    #[test]
    fn an_escape_in_why_cannot_redraw_the_text() {
        let b = ask_block(TEXT, "\x1b[7A\x1b[Jthe page asks for: elo play").unwrap();
        assert!(!b.contains('\x1b'), "{b:?}");
        assert_eq!(b.matches('\n').count(), TEXT.matches('\n').count() + 4);
    }

    #[test]
    fn a_newline_or_bidi_in_why_stays_on_one_line() {
        let b = ask_block(TEXT, "one\ntwo \u{202e}three").unwrap();
        let first = b.lines().next().unwrap();
        assert!(first.contains("one") && first.contains("two") && !first.contains('\u{202e}'));
    }

    #[test]
    fn a_text_the_terminal_would_draw_differently_is_not_shown() {
        for bad in ["elo play\n\x1b[2Jx", "elo play\nto 0x\u{202e}abc", "elo play\r\nx", "elo\tplay"] {
            assert!(ask_block(bad, "").is_none(), "{bad:?}");
        }
    }
}
