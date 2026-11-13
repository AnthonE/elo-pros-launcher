//! A crash dump on a bug report (`meter/bugs.py`): the two formats a report
//! carries, their caps, and what each holds, to show the player before they
//! choose to send one ([`crate::Client::report_bug`]).

pub const MINIDUMP_MAX: usize = 4 << 20;
pub const BACKTRACE_MAX: usize = 256 << 10;

/// What a minidump holds.
pub const MINIDUMP_CONTAINS: &str = "memory of the game process at the crash: each thread's stack \
    and registers, the loaded modules, and any heap the game chose to include, which can hold \
    whatever the game had in memory (text typed into it, too)";
/// What a backtrace holds.
pub const BACKTRACE_CONTAINS: &str =
    "the text the game wrote as it crashed: its call stack, as function names and source lines";

/// `"minidump"` (a file starting `MDMP`: Windows, breakpad, crashpad),
/// `"backtrace"` (UTF-8 text), or None for anything a report cannot carry.
pub fn dump_format(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"MDMP") {
        return Some("minidump");
    }
    if data.is_empty() || data.starts_with(b"\x7fELF") || data.contains(&0) {
        return None;
    }
    std::str::from_utf8(data).ok().map(|_| "backtrace")
}

/// What a format holds ([`dump_format`]'s answer), or None for another word.
pub fn dump_contains(format: &str) -> Option<&'static str> {
    match format {
        "minidump" => Some(MINIDUMP_CONTAINS),
        "backtrace" => Some(BACKTRACE_CONTAINS),
        _ => None,
    }
}

/// A format's cap in bytes.
pub fn dump_cap(format: &str) -> usize {
    if format == "minidump" {
        MINIDUMP_MAX
    } else {
        BACKTRACE_MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_formats() {
        assert_eq!(dump_format(b"MDMP\x00"), Some("minidump"));
        assert_eq!(
            dump_format(b"thread 'main' panicked at a.rs:1"),
            Some("backtrace")
        );
        assert_eq!(dump_format(b"\xff\xfe"), None);
        assert_eq!(dump_format(b"t\x00t"), None);
        assert_eq!(dump_format(b"\x7fELF\x02"), None);
        assert_eq!(dump_format(b""), None);
        assert!(dump_contains("minidump")
            .unwrap()
            .contains("memory of the game process"));
    }
}
