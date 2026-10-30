//! Game mode — the part of a "game booster" that measurably does something.
//!
//! When it is on, every game this launcher starts gets, on this machine:
//!
//! - **Linux:** started through Feral GameMode's `gamemoderun`, when it is
//!   installed. GameMode moves the CPU governor to performance, raises the
//!   game's CPU and I/O priority, asks the GPU driver for its performance
//!   level where it allows that, and holds off the screensaver. It puts every
//!   one of those back when the game exits, and when it dies: its daemon reaps
//!   a registered pid that is gone. Not installed → the game starts exactly as
//!   it would have, and [`Effect::Missing`] says what to install.
//! - **Windows:** started at above-normal priority. A creation flag, so there
//!   is nothing to undo — the priority ends with the process.
//! - **macOS:** nothing. There is no knob here worth turning.
//!
//! What it refuses, each of which a Windows booster ships:
//!
//! - **no admin prompt**, and nothing that would need one;
//! - **no killing or suspending** the player's other programs;
//! - **no "RAM cleaning"**, registry or network tweaks, or driver updates;
//! - **no power plan switch.** It is machine-wide and outlives a launcher that
//!   crashes mid-game, many laptops no longer have the plan, and a battery
//!   pays for it. GameMode's governor change is the version of this that
//!   cleans up after itself;
//! - **no speed claim.** It says what it changed, never what you gained. A
//!   gain is a frame-time measurement or it is not said.
//!
//! The switch is a file, `<state>/gamemode`, so the CLI and the window agree
//! (`elo gamemode on|off`, or the check box on the main menu), and
//! `ELO_GAMEMODE=1` in the launcher's own environment turns it on for one run.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The launcher's one-run override.
pub const ENV: &str = "ELO_GAMEMODE";

/// Feral GameMode's launcher. It execs the game, so the pid we supervise is
/// still the game's.
pub const RUNNER: &str = "gamemoderun";

/// `ABOVE_NORMAL_PRIORITY_CLASS`. Not `HIGH_PRIORITY_CLASS`: a game at high
/// priority can starve the input and audio threads it depends on.
pub const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;

static ON: AtomicBool = AtomicBool::new(false);

/// Whether game mode is on in this process.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Set it for this process only. [`load`] and [`save`] call this.
pub fn set_on(v: bool) {
    ON.store(v, Ordering::Relaxed);
}

pub fn path(state: &Path) -> PathBuf {
    state.join("gamemode")
}

/// Read the switch at startup: the file under `state`, or `ELO_GAMEMODE=1`.
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

/// What game mode does on this machine, whether or not it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Games start through this `gamemoderun`.
    GameMode(PathBuf),
    /// Linux without GameMode installed: nothing happens until it is.
    Missing,
    /// Games start at above-normal priority.
    AboveNormal,
    /// A platform with nothing worth doing.
    Nothing,
}

impl Effect {
    /// One line for the CLI and the window.
    pub fn describe(&self) -> String {
        match self {
            Effect::GameMode(p) => format!(
                "games start through GameMode ({}): CPU at full speed and the game's \
                 priority raised, put back when it exits",
                p.display()
            ),
            Effect::Missing => "GameMode is not installed — install your distro's `gamemode` \
                                package and games start through it"
                .to_string(),
            Effect::AboveNormal => "games start at above-normal priority".to_string(),
            Effect::Nothing => "does nothing on this system".to_string(),
        }
    }
}

/// Where the runner is looked for after the launcher's own `PATH`. Debian and
/// Ubuntu install it to `/usr/games`, which a program started from a desktop
/// menu may not have on its `PATH`.
pub const FALLBACK_DIRS: [&str; 3] = ["/usr/games", "/usr/bin", "/usr/local/bin"];

/// What game mode does here. Looks on the launcher's own `PATH`, then
/// [`FALLBACK_DIRS`] — a depot may never set `PATH` (`launch::ENV_DENY`), so a
/// game cannot choose the runner.
pub fn effect() -> Effect {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::env::split_paths(&path).chain(FALLBACK_DIRS.iter().map(PathBuf::from));
    effect_for(std::env::consts::OS, find_in(dirs, RUNNER).as_deref())
}

fn effect_for(os: &str, runner: Option<&Path>) -> Effect {
    match (os, runner) {
        ("linux", Some(p)) => Effect::GameMode(p.to_path_buf()),
        ("linux", None) => Effect::Missing,
        ("windows", _) => Effect::AboveNormal,
        _ => Effect::Nothing,
    }
}

/// What a launch resolved now gets: [`Effect`], when the switch is on.
pub fn boost() -> Effect {
    if on() {
        effect()
    } else {
        Effect::Nothing
    }
}

fn find_in(dirs: impl Iterator<Item = PathBuf>, name: &str) -> Option<PathBuf> {
    dirs.filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The process-wide switch is never flipped here, for the reason
    // `streamer.rs` gives: a launch test resolving while it was on would get
    // a runner.
    #[test]
    fn the_switch_is_a_file() {
        let dir = std::env::temp_dir().join(format!("elo-gamemode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let forced = std::env::var(ENV).map(|v| v.trim() == "1").unwrap_or(false);
        assert_eq!(stored(&dir), forced, "no file is off unless the env says on");
        write(&dir, true).unwrap();
        assert!(stored(&dir));
        write(&dir, false).unwrap();
        write(&dir, false).unwrap(); // off twice is still off, not an error
        assert!(!path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_platform_gets_only_what_is_there() {
        let p = Path::new("/usr/bin/gamemoderun");
        assert_eq!(effect_for("linux", Some(p)), Effect::GameMode(p.to_path_buf()));
        assert_eq!(effect_for("linux", None), Effect::Missing);
        assert_eq!(effect_for("windows", None), Effect::AboveNormal);
        assert_eq!(effect_for("windows", Some(p)), Effect::AboveNormal);
        assert_eq!(effect_for("macos", Some(p)), Effect::Nothing);
    }

    #[test]
    fn the_runner_is_found_only_as_an_executable_file() {
        let dir = std::env::temp_dir().join(format!("elo-gamemode-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(RUNNER)).unwrap(); // a directory is not it
        assert!(!is_executable(&dir.join(RUNNER)));
        let file = dir.join("plain");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(!is_executable(&file), "a file without an x bit is not run");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(is_executable(&file));

        // The first executable one wins; a relative directory is never searched.
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::copy(&file, b.join("plain")).unwrap();
        let dirs = vec![PathBuf::from("relative"), dir.clone(), a.clone(), b.clone()];
        assert_eq!(find_in(dirs.into_iter(), "plain"), Some(dir.join("plain")));
        assert_eq!(find_in(vec![a, b.clone()].into_iter(), "plain"), Some(b.join("plain")));
        assert_eq!(find_in(vec![dir.clone()].into_iter(), RUNNER), None, "a directory is not it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
