//! Reading a secret from a terminal without painting it on the screen.
//!
//! # Why this is here and not a crate
//!
//! `rpassword` is the obvious answer and it is a fine crate. It is also three
//! packages to clear one bit in one struct, against a budget with four left in
//! it (`tests/supply_chain.rs`). `libc` is already in this tree — `getrandom`
//! brings it — so asking it directly adds a dependency EDGE and no package,
//! which is the whole difference between a cost and a free one.
//!
//! # Two refusals worth naming
//!
//! **A pipe is not a terminal, and this reads one anyway.** `isatty` failing is
//! the normal state under a test harness, a CI job or `echo pw | elo ...`, and
//! a prompt that refused there would make the account commands the one part of
//! this client an agent cannot drive — on a platform whose players include
//! agents (`GATES.md` §0). So a non-tty falls through to a plain read. What it
//! does NOT do is pretend: nothing here claims the input was hidden when it
//! was not.
//!
//! **Echo is restored by `Drop`, not at the end of the happy path.** A panic
//! between clearing the flag and restoring it would hand the player back a
//! terminal that silently types nothing, which they would reasonably read as a
//! hung machine. The guard makes unwinding restore it too.
//!
//! **And by Ctrl-C, which `Drop` never sees.** Every money prompt says
//! "(Ctrl-C to stop)", and a ^C kills the process without unwinding — so the
//! one exit the prompt advertises was the one that left the shell typing
//! blind. While echo is off a SIGINT handler puts the saved terminal back,
//! then hands the signal to whatever was there before, so a ^C still ends
//! the command the way a ^C does.

use elo_vault::Zeroizing;
use std::io::{BufRead, Write};

/// Read a line with echo suppressed where that is possible.
///
/// Returns the line with its trailing newline removed and **nothing else
/// trimmed** — a passphrase may legitimately begin or end with a space, and
/// silently trimming one produces a keystore whose passphrase is not the one
/// the holder believes they typed.
///
/// `Zeroizing`: what was typed here is a passphrase or a private key, and it
/// is wiped when the caller drops it rather than left in freed memory after
/// the account is locked.
pub fn secret(label: &str) -> Result<Zeroizing<String>, String> {
    // ⚠ **The prompt goes to STDERR, and that is not a style choice.** It went
    // to stdout once and the first thing anyone tried broke:
    //
    //     SIG=$(elo account sign "$STMT")
    //
    // captured `"passphrase: 0x…"` — 144 characters where a signature is 130 —
    // and the verifier rejected it for a length nobody could explain. A prompt
    // is something a human reads; the answer on stdout is the product. Keeping
    // them on the same stream makes every account command unscriptable, which
    // on a client whose players include agents is most of the point of it.
    eprint!("{label}");
    std::io::stderr().flush().ok();
    let _guard = EchoOff::engage();
    let line = read_line()?;
    // The player's Enter was not echoed, so the cursor is still on the prompt
    // line. Without this the next message overwrites the prompt — and this too
    // goes to stderr, or a blank line lands in the middle of the output.
    if _guard.engaged {
        eprintln!();
    }
    Ok(line)
}

fn read_line() -> Result<Zeroizing<String>, String> {
    // Room up front, so a growing buffer does not leave a reallocated copy
    // of the secret behind that nothing wipes.
    let mut buf = Zeroizing::new(String::with_capacity(512));
    let n = std::io::stdin()
        .lock()
        .read_line(&mut buf)
        .map_err(|e| format!("cannot read from stdin: {e}"))?;
    if n == 0 {
        return Err("stdin closed before anything was typed".into());
    }
    while buf.ends_with('\n') || buf.ends_with('\r') {
        buf.pop();
    }
    Ok(buf)
}

/// Terminal echo, off for as long as this value lives.
struct EchoOff {
    engaged: bool,
    #[cfg(unix)]
    original: Option<libc::termios>,
    /// Whether [`on_sigint`] is installed for this prompt.
    #[cfg(unix)]
    armed: bool,
}

#[cfg(unix)]
impl EchoOff {
    fn engage() -> EchoOff {
        use std::mem::MaybeUninit;
        let off = EchoOff { engaged: false, original: None, armed: false };
        unsafe {
            let fd = libc::STDIN_FILENO;
            if libc::isatty(fd) != 1 {
                return off;
            }
            let mut term = MaybeUninit::<libc::termios>::uninit();
            if libc::tcgetattr(fd, term.as_mut_ptr()) != 0 {
                return off;
            }
            let original = term.assume_init();
            // Armed BEFORE echo goes off, so there is no moment in which a ^C
            // finds the terminal quiet and nothing to put it back.
            let armed = on_sigint::arm(&original);
            let mut quiet = original;
            quiet.c_lflag &= !libc::ECHO;
            if libc::tcsetattr(fd, libc::TCSANOW, &quiet) != 0 {
                if armed {
                    on_sigint::disarm();
                }
                return off;
            }
            EchoOff { engaged: true, original: Some(original), armed }
        }
    }
}

#[cfg(unix)]
impl Drop for EchoOff {
    fn drop(&mut self) {
        if let Some(original) = self.original {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &original);
            }
        }
        // After the restore: a ^C landing between the two finds the handler
        // still in place, and it restores the same terminal again.
        if self.armed {
            on_sigint::disarm();
        }
    }
}

/// The SIGINT handler that gives a quiet terminal back before the process
/// dies. `libc` and nothing else, for the budget reason this file opens with.
///
/// A handler may only touch what it can reach without locking, so the saved
/// terminal and the disposition it replaced live in statics, written before
/// it is installed and only read by it after. `tcsetattr`, `sigaction` and
/// `raise` are all on POSIX's async-signal-safe list.
#[cfg(unix)]
mod on_sigint {
    use std::cell::UnsafeCell;
    use std::mem::MaybeUninit;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Saved {
        term: UnsafeCell<MaybeUninit<libc::termios>>,
        before: UnsafeCell<MaybeUninit<libc::sigaction>>,
    }
    // Written only while `ARMED` is false and no handler of ours is
    // installed; read only by that handler, or by `disarm`, after.
    unsafe impl Sync for Saved {}

    static SAVED: Saved = Saved {
        term: UnsafeCell::new(MaybeUninit::uninit()),
        before: UnsafeCell::new(MaybeUninit::uninit()),
    };
    static ARMED: AtomicBool = AtomicBool::new(false);

    extern "C" fn restore_then_reraise(sig: libc::c_int) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, (*SAVED.term.get()).as_ptr());
            // The signal is blocked while this runs, so the raise is held
            // until it returns and is then delivered to what was there
            // before — by default, the end of the process with a ^C's status.
            libc::sigaction(sig, (*SAVED.before.get()).as_ptr(), std::ptr::null_mut());
            libc::raise(sig);
        }
    }

    /// Install the handler for `term`. False when it was not: SIGINT is
    /// already ignored (`nohup`, a background job), so a ^C cannot end this
    /// process mid-prompt — or a prompt is already armed.
    pub(super) fn arm(term: &libc::termios) -> bool {
        unsafe {
            let mut before = MaybeUninit::<libc::sigaction>::zeroed();
            if libc::sigaction(libc::SIGINT, std::ptr::null(), before.as_mut_ptr()) != 0 {
                return false;
            }
            let before = before.assume_init();
            if before.sa_sigaction == libc::SIG_IGN || ARMED.load(Ordering::Acquire) {
                return false;
            }
            (*SAVED.term.get()).write(*term);
            (*SAVED.before.get()).write(before);
            let mut ours: libc::sigaction = std::mem::zeroed();
            ours.sa_sigaction = restore_then_reraise as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut ours.sa_mask);
            if libc::sigaction(libc::SIGINT, &ours, std::ptr::null_mut()) != 0 {
                return false;
            }
            ARMED.store(true, Ordering::Release);
            true
        }
    }

    /// Put back the disposition [`arm`] replaced.
    pub(super) fn disarm() {
        unsafe {
            libc::sigaction(libc::SIGINT, (*SAVED.before.get()).as_ptr(), std::ptr::null_mut());
        }
        ARMED.store(false, Ordering::Release);
    }
}

/// Windows has a console-mode equivalent and this does not reach for it yet.
///
/// ⚠ It says so rather than failing closed: `LAUNCHER.md` §12 has the Windows
/// port as owed work, the build there has never been executed, and a client
/// that refused to make an account on Windows would be worse than one that
/// makes it visibly. The caller prints the warning — see `account_warn_echo`.
#[cfg(not(unix))]
impl EchoOff {
    fn engage() -> EchoOff {
        EchoOff { engaged: false }
    }
}

/// A yes/no question, defaulting to NO.
///
/// Echoing, because the answer is not a secret — what matters is the default:
/// Enter, EOF, and anything that is not a plain yes all refuse. A signing
/// prompt whose easiest keystroke was approval would train exactly the reflex
/// the prompt exists to interrupt.
pub fn confirm(label: &str) -> Result<bool, String> {
    eprint!("{label}");
    std::io::stderr().flush().ok();
    let mut buf = String::new();
    let n = std::io::stdin()
        .lock()
        .read_line(&mut buf)
        .map_err(|e| format!("cannot read from stdin: {e}"))?;
    if n == 0 {
        // stdin closed mid-question: the player cannot answer, so the answer
        // is no — not an error, because a refusal is a normal outcome.
        return Ok(false);
    }
    let t = buf.trim().to_ascii_lowercase();
    Ok(t == "y" || t == "yes")
}

/// Whether a secret typed right now would appear on screen. The account
/// commands say so before asking, because a holder who does not know their key
/// is being painted cannot decide not to type it.
pub fn echo_is_visible() -> bool {
    #[cfg(unix)]
    unsafe {
        libc::isatty(libc::STDIN_FILENO) == 1 && {
            use std::mem::MaybeUninit;
            let mut term = MaybeUninit::<libc::termios>::uninit();
            libc::tcgetattr(libc::STDIN_FILENO, term.as_mut_ptr()) != 0
        }
    }
    #[cfg(not(unix))]
    {
        // No console-mode call here yet, so on Windows a tty echoes.
        true
    }
}
