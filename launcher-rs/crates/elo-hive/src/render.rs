//! Printing a stranger's text into a terminal, safely.
//!
//! ## The threat the browser handled for us
//!
//! `hive.html` writes hive content into a DOM. The browser escapes it, the CSP
//! bounds it, and the worst a hostile message achieves is ugly. **A terminal
//! has neither.** Hive content is written by whoever signed it — the town
//! stream carries anything anyone posted — so a chat line is untrusted input
//! being handed to a device that interprets control codes.
//!
//! What that buys an attacker in a naive client:
//!
//! * `\x1b[2J` clears the screen; `\x1b[H` homes the cursor. Together they let
//!   one chat message repaint the terminal and print, say, a fake
//!   *"depot digest verified"* line above itself.
//! * `\r` rewrites the current line, so the visible text differs from what was
//!   signed — and the signature still checks out, because the bytes are the
//!   bytes.
//! * `\n` inside one message forges extra rows with attacker-chosen speakers.
//! * OSC sequences (`\x1b]…`) set the window title, and on some terminals
//!   `\x1b]52` reads or writes the **clipboard** — which matters rather a lot
//!   in a client next to a wallet.
//! * A bidi override (`U+202E`) reverses display order, so a link can read as
//!   one domain and resolve to another.
//!
//! None of these need a bug in this client. They need only that it prints what
//! it was given. So it does not: everything off the relay goes through
//! [`safe_line`] first, and the test suite pins each case above.

/// Strip what a terminal would execute, keep what a person meant to say.
///
/// Deliberately a **replace**, not a refusal: a message that is 95% text and
/// 5% escape should still be readable, with the removed part visible as `·` so
/// the reader can see something was taken out rather than silently altered.
pub fn safe_line(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // ESC — drop it and the sequence it introduces.
            '\x1b' => {
                out.push('·');
                match chars.peek() {
                    // CSI: parameters/intermediates, then a final byte @..~
                    Some('[') => {
                        chars.next();
                        for c in chars.by_ref() {
                            if ('\x40'..='\x7e').contains(&c) {
                                break;
                            }
                        }
                    }
                    // OSC: runs until BEL or ST (ESC \). The clipboard one.
                    Some(']') => {
                        chars.next();
                        while let Some(c) = chars.next() {
                            if c == '\x07' {
                                break;
                            }
                            if c == '\x1b' && chars.peek() == Some(&'\\') {
                                chars.next();
                                break;
                            }
                        }
                    }
                    // Any other two-character escape.
                    Some(_) => {
                        chars.next();
                    }
                    None => {}
                }
            }
            // A tab is legitimate whitespace; every other C0 control, plus
            // DEL, is not. Newline included — one message is one row, and a
            // message that forges rows forges speakers.
            '\t' => out.push(' '),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push('·'),
            // C1 controls, reachable as real characters in UTF-8.
            c if (0x80..=0x9f).contains(&(c as u32)) => out.push('·'),
            // Bidi overrides and isolates: a link that reads as one domain and
            // resolves to another. Zero-width joiners are left alone — they are
            // load-bearing in real scripts and in emoji.
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => out.push('·'),
            c => out.push(c),
        }
    }
    out
}

/// Cut to a column budget, on character boundaries, with an ellipsis.
///
/// Counts `char`s, which is right for the CJK-free case and wrong for wide
/// glyphs — a full answer needs a width table, and that is a dependency this
/// does not yet earn. What it must never do is slice a UTF-8 boundary, and it
/// does not.
pub fn clip(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let keep: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{keep}…")
}

/// One chat line, safe to print: `HH:MM  speaker  text`.
///
/// `sworn` decides the marker. A sworn name comes from the register; a display
/// name is self-declared and gets `~`, so the two can never be read as the
/// same claim — the achievements rule (*a badge renders carrying its own
/// evidence kind*) applied to a speaker.
pub fn chat_line(when: &str, speaker: &str, sworn: bool, content: &str, width: usize) -> String {
    let mark = if sworn { ' ' } else { '~' };
    let name = clip(&safe_line(speaker), 18);
    let body = clip(&safe_line(content), width.saturating_sub(30).max(20));
    format!("{when:>8}  {mark}{name:<18}  {body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clear_screen_sequence_cannot_survive_a_message() {
        // The one that lets a chat line repaint the terminal and print a fake
        // "verified" above itself.
        let hostile = "hello\x1b[2J\x1b[Hdepot digest verified";
        let out = safe_line(hostile);
        assert!(!out.contains('\x1b'), "{out:?}");
        assert!(!out.contains("[2J"), "the sequence body must go too: {out:?}");
        assert!(out.contains("hello") && out.contains("verified"));
    }

    #[test]
    fn a_carriage_return_cannot_rewrite_the_visible_line() {
        // The bytes are the bytes and the signature still checks out — which
        // is exactly why the RENDERER has to be the thing that refuses.
        let out = safe_line("the house says hi\rEVERYTHING IS FINE");
        assert!(!out.contains('\r'));
        assert!(out.contains("the house says hi"));
    }

    #[test]
    fn a_newline_cannot_forge_extra_rows() {
        let out = safe_line("first\nthe house: second");
        assert!(!out.contains('\n'), "one message is one row: {out:?}");
    }

    #[test]
    fn an_osc_sequence_cannot_reach_the_clipboard() {
        // `\x1b]52` is read/write clipboard on several terminals, which is not
        // a thing a chat message gets to do next to a wallet.
        for hostile in [
            "x\x1b]52;c;aGFjaw==\x07y",
            "x\x1b]52;c;aGFjaw==\x1b\\y",
            "x\x1b]0;window title\x07y",
        ] {
            let out = safe_line(hostile);
            assert!(!out.contains('\x1b') && !out.contains("52;"), "{out:?}");
            assert!(out.contains('x') && out.contains('y'), "{out:?}");
        }
    }

    #[test]
    fn a_bidi_override_cannot_disguise_a_link() {
        let out = safe_line("click elopros.com\u{202e}gpj.exe");
        assert!(!out.contains('\u{202e}'), "{out:?}");
    }

    #[test]
    fn c1_controls_and_del_go_too() {
        assert!(!safe_line("a\u{0085}b\u{009b}c\x7fd").contains(['\u{0085}', '\u{009b}', '\x7f']));
    }

    #[test]
    fn ordinary_text_survives_intact() {
        // The failure mode on the other side: a sanitiser that eats real
        // messages is a client nobody uses.
        for good in [
            "the record anchored — root 0x2b8822f2e0e995… · 23 vows",
            "https://elopros.com/api/anchors",
            "emoji 🐝 and CJK 龍 and accents éàü",
            "punctuation: {}[]()<>|\\/*&^%$#@!~`",
        ] {
            assert_eq!(safe_line(good), good, "mangled: {good}");
        }
        // A tab becomes a space rather than vanishing — alignment is a lie in
        // a chat column, but the word break is real.
        assert_eq!(safe_line("a\tb"), "a b");
    }

    #[test]
    fn clipping_never_splits_a_character() {
        assert_eq!(clip("hello", 10), "hello");
        assert_eq!(clip("hello", 3), "he…");
        assert_eq!(clip("🐝🐝🐝🐝", 3), "🐝🐝…");
        assert_eq!(clip("龍龍龍", 0), "");
    }

    #[test]
    fn a_self_declared_name_never_wears_a_sworn_ones_clothes() {
        let sworn = chat_line("2h ago", "the house", true, "hi", 80);
        let claimed = chat_line("2h ago", "the house", false, "hi", 80);
        assert_ne!(sworn, claimed);
        assert!(claimed.contains('~'), "{claimed}");
        assert!(!sworn.contains('~'), "{sworn}");
    }

    #[test]
    fn a_hostile_speaker_name_is_sanitised_too() {
        // The name is as untrusted as the body — it is a kind:0 profile field
        // anyone can set.
        let line = chat_line("now", "ho\x1b[31muse", false, "hi", 80);
        assert!(!line.contains('\x1b'), "{line}");
    }
}
