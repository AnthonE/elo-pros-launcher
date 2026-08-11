//! The local stream the game connects over — and the one file that differs
//! between Ubuntu and Windows.
//!
//! On unix it is a UNIX stream socket at `$XDG_RUNTIME_DIR/scry/launcher.sock`.
//! The runtime dir is the right home: 0700, per-user, and cleared at logout, so
//! a stale socket cannot outlive the session that made it.
//!
//! On Windows there is no `AF_UNIX` in Rust's `std` and CPython does not expose
//! one either, so the transport is a **named pipe** at
//! `\\.\pipe\scry-launcher-<user>`. That is the platform's own answer to the
//! same question and it is what Steam's client actually uses, which is the
//! thing this whole door is ripped from.
//!
//! # What this is NOT, said here as it is said in `broker.py`
//!
//! **Against a process running as you, this is not a security boundary.** A
//! game you launched runs as your uid and can already read your files, your
//! browser profile, and this pipe's name. The boundary is the *signer* behind
//! the broker — point it at an arca and every request is re-derived by a
//! process running as someone else. Nothing here pretends otherwise.
//!
//! The named pipe is created with the default DACL, which grants the creating
//! user and SYSTEM. It is **not** `PIPE_ACCESS_INBOUND` from a network — the
//! `\\.\pipe\` namespace is local unless a machine explicitly shares it.

use std::io;
// `PathBuf` is unix-only here: the windows half addresses a pipe by name and
// never touches the filesystem.
#[cfg(unix)]
use std::path::PathBuf;

/// The environment variable the launcher sets on every native build it starts.
/// A game reads this first and falls back to the platform default.
pub const SOCKET_ENV: &str = "SCRY_LAUNCHER_SOCKET";

/// Where the door is, when nobody named one.
///
/// Kept as one function returning a platform string rather than two call sites
/// with `cfg!` in them, because the *game* side reads this too (the vendored
/// SDK) and the two must agree exactly or a game finds no launcher on a
/// machine that is running one.
pub fn default_endpoint() -> String {
    if let Ok(named) = std::env::var(SOCKET_ENV) {
        if !named.is_empty() {
            return named;
        }
    }
    platform_default()
}

#[cfg(unix)]
fn platform_default() -> String {
    // XDG_RUNTIME_DIR when it exists and is a directory; otherwise the cache
    // dir, which is the documented fallback (`sdk/PROTOCOL.md` §finding the socket).
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        if !rt.is_empty() && PathBuf::from(&rt).is_dir() {
            return format!("{rt}/scry/launcher.sock");
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return "./scry-launcher.sock".into();
    }
    format!("{home}/.cache/scry/launcher/launcher.sock")
}

#[cfg(windows)]
fn platform_default() -> String {
    // One pipe per user, because the pipe namespace is machine-wide. Two
    // people logged into the same box must not land on one door — a game
    // would otherwise ask the wrong session's launcher to sign.
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".into());
    let user: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let user = if user.is_empty() { "default".to_string() } else { user };
    format!(r"\\.\pipe\scry-launcher-{user}")
}

/// A duplex byte stream to one connected game. Both platforms produce
/// something that is `Read + Write`, which is all the protocol needs.
pub trait Conn: io::Read + io::Write + Send {}
impl<T: io::Read + io::Write + Send> Conn for T {}

// ── unix ─────────────────────────────────────────────────────────────────────

#[cfg(unix)]
pub use imp_unix::Listener;

#[cfg(unix)]
mod imp_unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    pub struct Listener {
        inner: UnixListener,
        path: PathBuf,
    }

    impl Listener {
        /// Bind, replacing a stale socket left by a crashed launcher.
        ///
        /// ⚠ The unlink-then-bind is deliberate and is not a race we can lose
        /// meaningfully: two launchers racing here end with one door and one
        /// error, and the loser exits. Leaving a stale socket instead means
        /// every game gets `ECONNREFUSED` from a file that looks alive.
        pub fn bind(endpoint: &str) -> io::Result<Listener> {
            let path = PathBuf::from(endpoint);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
                // 0700: the socket's directory is the access control here.
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            let inner = UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Listener { inner, path })
        }

        pub fn accept(&self) -> io::Result<UnixStream> {
            self.inner.accept().map(|(s, _)| s)
        }

        pub fn endpoint(&self) -> String {
            self.path.display().to_string()
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // The door closes with the launcher. A socket file outliving the
            // process is the stale-door problem stated above, from the other
            // end.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// ── windows ──────────────────────────────────────────────────────────────────

#[cfg(windows)]
pub use imp_windows::Listener;

#[cfg(windows)]
mod imp_windows {
    use super::*;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
    const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
    const PIPE_WAIT: u32 = 0x0000_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const INVALID_HANDLE_VALUE: isize = -1;
    const ERROR_PIPE_CONNECTED: u32 = 535;

    type Handle = *mut core::ffi::c_void;

    extern "system" {
        fn CreateNamedPipeW(
            name: *const u16, open_mode: u32, pipe_mode: u32, max_instances: u32,
            out_buf: u32, in_buf: u32, timeout: u32, security: *mut core::ffi::c_void,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut core::ffi::c_void) -> i32;
        fn GetLastError() -> u32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct Listener {
        endpoint: String,
    }

    impl Listener {
        /// Named pipes have no bind/listen split — an instance IS the pending
        /// connection, so a fresh instance is created per accept. The name is
        /// validated here so a bad endpoint fails at startup rather than on
        /// the first game that knocks.
        pub fn bind(endpoint: &str) -> io::Result<Listener> {
            if !endpoint.starts_with(r"\\.\pipe\") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("a windows endpoint must be a named pipe: {endpoint}"),
                ));
            }
            Ok(Listener { endpoint: endpoint.to_string() })
        }

        pub fn accept(&self) -> io::Result<std::fs::File> {
            let name = wide(&self.endpoint);
            // SAFETY: `name` is a NUL-terminated UTF-16 buffer that outlives
            // the call. A null security descriptor means the default DACL:
            // the creating user and SYSTEM, which is the intent.
            let handle = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    // REJECT_REMOTE_CLIENTS is the one that matters: without
                    // it the pipe namespace is reachable over SMB, and this
                    // door is local by design.
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT
                        | PIPE_REJECT_REMOTE_CLIENTS,
                    PIPE_UNLIMITED_INSTANCES,
                    64 * 1024, 64 * 1024, 0,
                    std::ptr::null_mut(),
                )
            };
            if handle as isize == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: handle is a valid pipe instance from the call above.
            let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
            if connected == 0 {
                // A client that connected between create and connect is a
                // success, not a failure — Win32 reports it as this error.
                let err = unsafe { GetLastError() };
                if err != ERROR_PIPE_CONNECTED {
                    return Err(io::Error::from_raw_os_error(err as i32));
                }
            }
            // SAFETY: ownership of the handle moves into the File, which
            // closes it on drop.
            Ok(unsafe { std::fs::File::from(OwnedHandle::from_raw_handle(handle)) })
        }

        pub fn endpoint(&self) -> String {
            self.endpoint.clone()
        }
    }
}
