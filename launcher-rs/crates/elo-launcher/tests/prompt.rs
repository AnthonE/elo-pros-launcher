//! A passphrase prompt: interrupted the way its own text says to stop it,
//! and not reached at all when the answer was always going to be no.
//!
//! The interrupt is driven through a real pseudo-terminal, because the defect
//! lives between the process and its terminal: `Drop` restored echo on every
//! exit except the one the prompt advertises, and a ^C left the player's
//! shell typing blind after `elo` was gone.
#![cfg(unix)]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ELO: &str = env!("CARGO_BIN_EXE_elo");

fn echo_on(fd: &OwnedFd) -> bool {
    let mut t = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: a valid fd and a termios-sized buffer the call fills.
    unsafe {
        assert_eq!(libc::tcgetattr(fd.as_raw_fd(), t.as_mut_ptr()), 0, "tcgetattr");
        t.assume_init().c_lflag & libc::ECHO != 0
    }
}

/// A pseudo-terminal pair: the master is the terminal emulator's side, the
/// slave is what the prompt reads as its stdin.
fn pty() -> (OwnedFd, OwnedFd) {
    // SAFETY: each call is checked before its result is used, and both fds
    // are owned from here on. `ptsname` is not thread-safe; one test in this
    // file is its only caller.
    unsafe {
        let m = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(m >= 0, "posix_openpt");
        let master = OwnedFd::from_raw_fd(m);
        assert_eq!(libc::grantpt(m), 0, "grantpt");
        assert_eq!(libc::unlockpt(m), 0, "unlockpt");
        let name = libc::ptsname(m);
        assert!(!name.is_null(), "ptsname");
        let s = libc::open(name, libc::O_RDWR | libc::O_NOCTTY);
        assert!(s >= 0, "open the slave");
        (master, OwnedFd::from_raw_fd(s))
    }
}

#[test]
fn ctrl_c_at_a_passphrase_prompt_gives_the_terminal_back_with_echo_on() {
    let dir = tempfile::tempdir().unwrap();
    let (_master, slave) = pty();
    assert!(echo_on(&slave), "a fresh terminal echoes");

    let mut child = Command::new(ELO)
        .args(["account", "import"])
        .env("ELO_KEYSTORE", dir.path().join("keystore.json"))
        .env("ELO_PAIRING", dir.path().join("pairing.json"))
        .env("ELO_STATE", dir.path().join("state"))
        .env("ELO_GAMES", dir.path().join("games"))
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn elo");

    // Echo goes off when the prompt is up — and the handler is armed before
    // that, so this is the moment a player's ^C can land.
    let deadline = Instant::now() + Duration::from_secs(20);
    while echo_on(&slave) {
        assert!(child.try_wait().unwrap().is_none(), "elo exited before prompting");
        assert!(Instant::now() < deadline, "the prompt never turned echo off");
        std::thread::sleep(Duration::from_millis(10));
    }

    // SAFETY: a signal to our own child, by the pid `spawn` returned.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    let status = child.wait().unwrap();

    assert_eq!(status.signal(), Some(libc::SIGINT), "a ^C still ends the command as a ^C: {status:?}");
    assert!(echo_on(&slave), "elo is gone and the terminal is still typing blind");
    assert!(!dir.path().join("keystore.json").exists(), "an interrupted import writes nothing");
}

/// A Solana half left beside a moved-aside keystore is refused before the
/// prompt, not after a player has typed a key into it. stdin is closed, so
/// reaching the prompt would fail for a different reason than the one asserted.
#[test]
fn a_leftover_solana_half_is_refused_before_anything_is_typed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("keystore.solana.json"), "{}").unwrap();
    let out = Command::new(ELO)
        .args(["account", "import"])
        .env("ELO_KEYSTORE", dir.path().join("keystore.json"))
        .env("ELO_PAIRING", dir.path().join("pairing.json"))
        .env("ELO_STATE", dir.path().join("state"))
        .env("ELO_GAMES", dir.path().join("games"))
        .stdin(Stdio::null())
        .output()
        .expect("run elo");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("keystore.solana.json") && err.contains("only copy"), "{err}");
    assert!(!err.contains("recovery words"), "the refusal must come before the prompt: {err}");
}
