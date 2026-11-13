//! Crash dumps: what a game left when it died, and the report that carries one.
//!
//! **The known place is `<install>/crashes/`.** Every launch tells the game
//! where it is ([`ENV`], set in `launch::resolve` beside `ELO_STREAMER`, so no
//! launch door misses it), and a game with a crash handler writes there:
//! breakpad or crashpad, Windows' `MiniDumpWriteDump`, or a Rust panic hook
//! writing its backtrace. The launcher never guesses at other folders.
//!
//! Two formats, the two `meter/bugs.py` takes:
//!
//! | format | the file | at most |
//! |---|---|---|
//! | `minidump` | starts `MDMP` (Windows, breakpad and crashpad all write it) | [`MINIDUMP_MAX`] |
//! | `backtrace` | `.txt` or `.log`, UTF-8 text | [`BACKTRACE_MAX`] |
//!
//! A core file is never offered: it is the whole process image.
//!
//! **Nothing is sent from here, and nothing is sent unless the player says
//! yes at the moment of sending.** [`find`] lists what is on disk;
//! [`report_body`] builds the one document the player is asked about. The
//! dump is the only file it carries, and the report says what that file
//! holds ([`Format::contains`]).

use crate::error::{DepotError, Result};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The folder under an install a game writes its dumps to.
pub const DIR: &str = "crashes";
/// The variable every started game reads for that folder's absolute path.
pub const ENV: &str = "ELO_CRASH_DIR";
/// The meter's caps (`meter/bugs.py` `MINIDUMP_MAX`, `BACKTRACE_MAX`).
pub const MINIDUMP_MAX: u64 = 4 << 20;
pub const BACKTRACE_MAX: u64 = 256 << 10;
/// A dump the player has sent is renamed with this, so it is not offered twice.
pub const SENT_SUFFIX: &str = ".sent";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Minidump,
    Backtrace,
}

impl Format {
    /// The name the meter takes (`dump.format`).
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Minidump => "minidump",
            Format::Backtrace => "backtrace",
        }
    }

    pub fn cap(self) -> u64 {
        match self {
            Format::Minidump => MINIDUMP_MAX,
            Format::Backtrace => BACKTRACE_MAX,
        }
    }

    /// What the file holds, in the words the player is shown before they
    /// choose, and the meter's own (`meter/bugs.py` `DUMP_CONTAINS`).
    pub fn contains(self) -> &'static str {
        match self {
            Format::Minidump => {
                "memory of the game process at the crash: each thread's stack and registers, \
                 the loaded modules, and any heap the game chose to include, which can hold \
                 whatever the game had in memory (text typed into it, too)"
            }
            Format::Backtrace => {
                "the text the game wrote as it crashed: its call stack, as function names and \
                 source lines"
            }
        }
    }
}

/// The format of a file from its name and first bytes, or None for anything
/// this launcher will not offer.
pub fn sniff(name: &str, head: &[u8]) -> Option<Format> {
    if head.starts_with(b"MDMP") {
        return Some(Format::Minidump);
    }
    let lower = name.to_ascii_lowercase();
    if !(lower.ends_with(".txt") || lower.ends_with(".log")) || head.is_empty() || head.contains(&0)
    {
        return None;
    }
    match std::str::from_utf8(head) {
        Ok(_) => Some(Format::Backtrace),
        // A head cut inside a character is still text.
        Err(e) if e.error_len().is_none() => Some(Format::Backtrace),
        Err(_) => None,
    }
}

/// One dump on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dump {
    pub path: PathBuf,
    pub format: Format,
    pub bytes: u64,
    pub modified: SystemTime,
}

impl Dump {
    pub fn too_big(&self) -> bool {
        self.bytes > self.format.cap()
    }

    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    }

    /// The whole file, refused past its cap (the meter would refuse it too).
    pub fn read(&self) -> Result<Vec<u8>> {
        if self.too_big() {
            return Err(DepotError::new(format!(
                "{} is {} bytes; a {} is sent only up to {} bytes",
                self.name(),
                self.bytes,
                self.format.as_str(),
                self.format.cap()
            )));
        }
        let mut data = Vec::new();
        std::fs::File::open(&self.path)?
            .take(self.format.cap() + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 > self.format.cap() {
            return Err(DepotError::new(format!("{} grew past its cap", self.name())));
        }
        Ok(data)
    }
}

/// The crash folder of an install.
pub fn dir(install_path: &Path) -> PathBuf {
    install_path.join(DIR)
}

/// Every dump in `dir` written at or after `since`, newest first. A sent one,
/// a folder, a link, and a file of any other kind are skipped.
pub fn find(dir: &Path, since: Option<SystemTime>) -> Vec<Dump> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.ends_with(SENT_SUFFIX) || name.starts_with('.') {
            continue;
        }
        // symlink_metadata: a link in the folder is never followed out of it.
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if since.is_some_and(|s| modified < s) {
            continue;
        }
        let mut head = [0u8; 4096];
        let n = std::fs::File::open(e.path()).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
        if let Some(format) = sniff(&name, &head[..n]) {
            out.push(Dump { path: e.path(), format, bytes: meta.len(), modified });
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.path.cmp(&a.path)));
    out
}

/// Mark a dump sent, so it is not offered again. The file stays the player's.
pub fn mark_sent(d: &Dump) -> Result<PathBuf> {
    let to = d.path.with_file_name(format!("{}{SENT_SUFFIX}", d.name()));
    std::fs::rename(&d.path, &to)?;
    Ok(to)
}

/// How a game ended, in words, or None when it ended normally.
pub fn abnormal(status: &std::process::ExitStatus) -> Option<String> {
    if status.success() {
        return None;
    }
    if let Some(code) = status.code() {
        return Some(format!("exit code {code}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return Some(format!("signal {sig}{}", signal_name(sig)));
        }
    }
    Some("an abnormal exit".into())
}

#[cfg(unix)]
fn signal_name(sig: i32) -> &'static str {
    match sig {
        4 => " (SIGILL)",
        6 => " (SIGABRT)",
        7 => " (SIGBUS)",
        8 => " (SIGFPE)",
        9 => " (SIGKILL)",
        11 => " (SIGSEGV)",
        _ => "",
    }
}

/// The meter's platform word for this machine (`meter/bugs.py` `PLATFORMS`).
pub fn platform_word() -> Option<&'static str> {
    match std::env::consts::OS {
        "linux" => Some("linux"),
        "windows" => Some("windows"),
        "macos" => Some("macos"),
        _ => None,
    }
}

/// What a crash report says about the build and the crash. Nothing about the
/// machine or the player: the dump is the evidence, and the player chose it.
pub struct Crash<'a> {
    pub slug: &'a str,
    pub build: &'a str,
    pub platform: &'a str,
    /// [`abnormal`]'s words, or "a crash" when the launcher did not see it end.
    pub ended: &'a str,
    /// What the player typed, if anything.
    pub note: Option<&'a str>,
    /// When it is sent, ISO 8601 UTC.
    pub reported: &'a str,
}

/// A build id as the meter's `build.version` rule takes it (`[0-9A-Za-z.+_-]{1,40}`).
fn version_of(build: &str) -> String {
    let v: String = build
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || ".+_-".contains(*c))
        .take(40)
        .collect();
    if v.is_empty() { "unknown".into() } else { v }
}

/// The commit a build id ends in (`0.5.0-abcdef1`), or "unknown".
fn commit_of(build: &str) -> String {
    let last = build.rsplit(['-', '_', '.']).next().unwrap_or("");
    if (7..=40).contains(&last.len()) && last.chars().all(|c| c.is_ascii_hexdigit()) {
        last.to_ascii_lowercase()
    } else {
        "unknown".into()
    }
}

/// The first line a person reads in a backtrace: the panic, else the first line.
fn headline(text: &str) -> &str {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.clone().next().unwrap_or("");
    lines.find(|l| l.contains("panicked at")).unwrap_or(first)
}

/// The report document (`meter/bugs.py` `report_error` is its rule), grouped
/// on the board by the build, the format, and how the game ended (or, for a
/// backtrace, its first line).
pub fn report(c: &Crash, format: Format, dump: &[u8]) -> Value {
    use sha2::{Digest as _, Sha256};
    let what = match format {
        Format::Backtrace => headline(&String::from_utf8_lossy(dump)).to_string(),
        Format::Minidump => c.ended.to_string(),
    };
    let fp = Sha256::digest(format!("crash|{}|{}|{}|{what}", c.slug, c.build, format.as_str()));
    let fingerprint: String = fp.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let mut title = format!("[crash] {} {} ended with {}", c.slug, version_of(c.build), c.ended);
    title.retain(|ch| !ch.is_control());
    title.truncate(200);
    let body = c.note.unwrap_or("").chars().filter(|ch| *ch == '\n' || !ch.is_control()).take(4000);
    json!({
        "format": 1,
        "kind": "crash",
        "fingerprint": fingerprint,
        "title": title,
        "body": body.collect::<String>(),
        "issue_title": title,
        "read_first": "README.md",
        "reported": c.reported,
        "build": {"version": version_of(c.build), "commit": commit_of(c.build),
                  "platform": c.platform, "runtime": "desktop"},
        "crash": {"ended": c.ended, "dump": format.as_str(), "from": "launcher"},
    })
}

/// The whole POST body for `/api/bugs`: the report and the dump, marked as the
/// player's choice. Build this only after they said yes.
pub fn report_body(c: &Crash, format: Format, dump: &[u8]) -> Value {
    json!({
        "game": c.slug,
        "report": report(c, format, dump),
        "dump": {"format": format.as_str(), "b64": elo_vault::solana::b64_encode(dump),
                 "consent": true},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_the_two_formats_and_nothing_else() {
        assert_eq!(sniff("x.dmp", b"MDMP\x93\xa7"), Some(Format::Minidump));
        assert_eq!(sniff("anything", b"MDMP"), Some(Format::Minidump));
        assert_eq!(sniff("panic.txt", b"thread 'main' panicked"), Some(Format::Backtrace));
        assert_eq!(sniff("crash.LOG", "é".as_bytes()), Some(Format::Backtrace));
        assert_eq!(sniff("cut.txt", &"é".as_bytes()[..1]), Some(Format::Backtrace));
        assert_eq!(sniff("core", b"\x7fELF\x02"), None);
        assert_eq!(sniff("core.txt", b"\x7fELF\x00"), None);
        assert_eq!(sniff("bin.txt", b"\xff\xfe"), None);
        assert_eq!(sniff("notes.md", b"panicked"), None);
        assert_eq!(sniff("empty.txt", b""), None);
    }

    #[test]
    fn build_ids_become_the_meters_version_and_commit() {
        assert_eq!(version_of("0.5.0-abcdef1"), "0.5.0-abcdef1");
        assert_eq!(commit_of("0.5.0-abcdef1"), "abcdef1");
        assert_eq!(commit_of("nightly"), "unknown");
        assert_eq!(version_of(&"a".repeat(60)).len(), 40);
    }

    #[test]
    fn the_headline_is_the_panic() {
        assert_eq!(headline("\nstack backtrace:\nthread 'main' panicked at a.rs:1\n  0: f"),
                   "thread 'main' panicked at a.rs:1");
        assert_eq!(headline("Segmentation fault\n  0: f"), "Segmentation fault");
    }
}
