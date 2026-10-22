//! Streamer mode — for a player who shows the launcher or a game on a stream.
//!
//! Rust's streamer mode hides who you are and where you are playing, so a
//! viewer cannot follow you onto a server or read your account off the screen.
//! Here that is three things:
//!
//! - the windows stop drawing your wallet, your friends' wallets and server
//!   addresses (the windows read [`on`]; they do no I/O of their own);
//! - every game this launcher starts gets `ELO_STREAMER=1` in its environment
//!   ([`launch_env`], added in `launch::resolve` so no launch door can miss
//!   it), and Gates starts with its own streamer mode on;
//! - nothing else. Signing, joining and copying an invite still work, because
//!   a streamer still plays.
//!
//! The switch is a file, `<state>/streamer`, so the CLI and the window agree
//! (`elo streamer on|off`, or the check box on the main menu), and
//! `ELO_STREAMER=1` in the launcher's own environment turns it on for one run.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The variable a started game reads, and the launcher's own one-run override.
pub const ENV: &str = "ELO_STREAMER";

/// What a hidden value is drawn as.
pub const HIDDEN: &str = "hidden (streamer mode)";

static ON: AtomicBool = AtomicBool::new(false);

/// Whether streamer mode is on in this process.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Set it for this process only. [`load`] and [`save`] call this.
pub fn set_on(v: bool) {
    ON.store(v, Ordering::Relaxed);
}

pub fn path(state: &Path) -> PathBuf {
    state.join("streamer")
}

/// Read the switch at startup: the file under `state`, or `ELO_STREAMER=1`.
pub fn load(state: &Path) -> bool {
    let v = stored(state);
    set_on(v);
    v
}

/// Turn it on or off and remember that under `state`.
pub fn save(state: &Path, v: bool) -> std::io::Result<()> {
    write(state, v)?;
    set_on(v);
    Ok(())
}

/// What `state` says, without touching this process's switch.
pub fn stored(state: &Path) -> bool {
    let forced = std::env::var(ENV).map(|v| v.trim() == "1").unwrap_or(false);
    forced || path(state).is_file()
}

fn write(state: &Path, v: bool) -> std::io::Result<()> {
    let p = path(state);
    if v {
        std::fs::create_dir_all(state)?;
        std::fs::write(&p, "on\n")
    } else {
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// What a started game is told, given the switch.
pub fn launch_env() -> Vec<(String, String)> {
    env_for(on())
}

fn env_for(on: bool) -> Vec<(String, String)> {
    if on {
        vec![(ENV.to_string(), "1".to_string())]
    } else {
        Vec::new()
    }
}

/// A wallet or a server address as a window should draw it.
pub fn shown(value: &str) -> String {
    shown_for(on(), value)
}

fn shown_for(on: bool, value: &str) -> String {
    if on {
        HIDDEN.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The process-wide switch is never flipped here: cargo runs tests on
    // threads, and a launch test resolving while it was on would see the env.
    #[test]
    fn the_switch_is_a_file_and_a_started_game_hears_it() {
        let dir = std::env::temp_dir().join(format!("elo-streamer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let forced = std::env::var(ENV).map(|v| v.trim() == "1").unwrap_or(false);
        assert_eq!(stored(&dir), forced, "no file is off unless the env says on");
        write(&dir, true).unwrap();
        assert!(stored(&dir));
        write(&dir, false).unwrap();
        write(&dir, false).unwrap(); // off twice is still off, not an error
        assert!(!path(&dir).exists());
        assert_eq!(env_for(true), vec![(ENV.to_string(), "1".to_string())]);
        assert!(env_for(false).is_empty());
        assert_eq!(shown_for(true, "0xabc"), HIDDEN);
        assert_eq!(shown_for(false, "0xabc"), "0xabc");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
