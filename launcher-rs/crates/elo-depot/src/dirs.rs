//! Where the client keeps its state — one copy, for both binaries.
//!
//! These lived in the CLI's `main.rs` until the window grew a chat pane. The
//! moment `elo-gui` needed the chat key, a second copy of this logic would have
//! been a second place for the CLI and the window to disagree about which file
//! is the key: one makes a voice, the other cannot see it, and nothing errors.
//! `elo_vault::keystore_path` says the same thing about the account, for the
//! same reason.

use std::path::PathBuf;

/// The client's state directory: sessions, the chat key, the chat cache.
///
/// `ELO_STATE` wins; otherwise `$XDG_STATE_HOME/elo` (`~/.local/state/elo`) on
/// unix and `%LOCALAPPDATA%\elo` on Windows.
pub fn state_root() -> PathBuf {
    if let Ok(p) = std::env::var("ELO_STATE") {
        return PathBuf::from(p);
    }
    home_based("XDG_STATE_HOME", ".local/state").join("elo")
}

/// Where Workshop mods are kept: one folder per game, one per mod inside it
/// (`workshop::sync`).
///
/// `ELO_MODS` wins; otherwise beside the games, `$XDG_DATA_HOME/elo/mods`
/// (`~/.local/share/elo/mods`) on unix and `%LOCALAPPDATA%\elo\mods` on
/// Windows. Not inside the games root: a build may be called `mods`, and a
/// title's folder holds only its builds.
pub fn mods_root() -> PathBuf {
    if let Ok(p) = std::env::var("ELO_MODS") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    home_based("XDG_DATA_HOME", ".local/share").join("elo").join("mods")
}

/// The XDG dir on unix, `%LOCALAPPDATA%` on Windows — and never a path rooted
/// at `/` because an env var was missing.
///
/// ⚠ **This is why it is a function.** Both callers read `$HOME` directly and
/// `unwrap_or_default()`'d it, which is empty on Windows — `HOME` is not set
/// there, `USERPROFILE` is. That produced `/.local/share/elo/games`: an
/// absolute path at the filesystem root, which is the wrong place on unix with
/// no `HOME` and unwritable nonsense on Windows. An empty base is now a real
/// failure rather than a silent root-relative write, and it falls back to the
/// current directory so a game lands somewhere the caller can find rather than
/// somewhere they cannot.
pub fn home_based(xdg_var: &str, unix_tail: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let _ = (xdg_var, unix_tail);
        for var in ["LOCALAPPDATA", "APPDATA", "USERPROFILE"] {
            if let Ok(p) = std::env::var(var) {
                if !p.is_empty() {
                    return PathBuf::from(p);
                }
            }
        }
        PathBuf::from(".")
    }
    #[cfg(not(windows))]
    {
        // Absolute only, as the XDG spec says: a relative one is a path under
        // whatever directory the launcher was started in, which is the
        // "somewhere the caller cannot find" this function exists to prevent.
        if let Ok(p) = std::env::var(xdg_var) {
            if std::path::Path::new(&p).is_absolute() {
                return PathBuf::from(p);
            }
        }
        match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => PathBuf::from(h).join(unix_tail),
            _ => PathBuf::from("."),
        }
    }
}
