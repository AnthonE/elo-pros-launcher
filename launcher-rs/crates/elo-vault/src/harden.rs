//! Keep other programs, and crash files, out of this process's memory.
//!
//! While an account is unlocked its key is in this process. Two ordinary
//! things would copy it out without anyone attacking anything:
//!
//! * **a core dump.** A crash writes the whole heap to disk, where a crash
//!   reporter may upload it. `RLIMIT_CORE = 0` stops the file being written.
//! * **another program running as you.** On Linux any same-user process may
//!   attach with `ptrace` or read `/proc/<pid>/mem` unless Yama says
//!   otherwise, and plenty of distributions leave it permissive.
//!   `PR_SET_DUMPABLE = 0` closes both, which is what `ssh-agent` and
//!   `gpg-agent` do for the same reason.
//!
//! Neither survives `exec`, so the games this launcher starts are unaffected.
//!
//! What this does not cover, said so nobody reasons from it: root, the
//! kernel, a debugger the user starts on purpose, or swap (the key is 32
//! bytes and not locked in RAM). macOS already refuses same-user
//! `task_for_pid` to unentitled callers, so only the core limit applies there.
//! On Windows nothing here runs: the platform's process ACLs are the guard.

/// Call once, first thing in `main`. Returns what could not be done, so a
/// caller can say so; an empty list means everything here took.
pub fn harden_process() -> Vec<String> {
    #[allow(unused_mut)]
    let mut failed = Vec::new();
    #[cfg(unix)]
    {
        let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: setrlimit reads one struct we own, and has no other effect.
        if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } != 0 {
            failed.push(format!("core dumps stay on: {}", std::io::Error::last_os_error()));
        }
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: PR_SET_DUMPABLE takes an integer and changes one flag on
        // this process.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
            failed.push(format!(
                "other programs can still attach: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    failed
}

/// Whether another same-user program is kept out right now. Linux only;
/// `None` where the question has no cheap answer.
pub fn attach_blocked() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: PR_GET_DUMPABLE takes no pointer and only reads.
        let v = unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) };
        Some(v == 0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
