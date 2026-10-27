//! The local stream the game connects over — and the one file that differs
//! between Ubuntu and Windows.
//!
//! On unix it is a UNIX stream socket at `$XDG_RUNTIME_DIR/elo/launcher.sock`.
//! The runtime dir is the right home: 0700, per-user, and cleared at logout, so
//! a stale socket cannot outlive the session that made it.
//!
//! On Windows there is no `AF_UNIX` in Rust's `std` and CPython does not expose
//! one either, so the transport is a **named pipe** at
//! `\\.\pipe\elo-launcher-<user>`. That is the platform's own answer to the
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
//! The named pipe is created with an explicit DACL — its owner and SYSTEM,
//! nobody else (the default one also admits Everyone and anonymous for
//! reading) — and one instance is always pending, so another user can neither
//! hold the door's instances nor take the name between connections. It
//! refuses remote clients: the `\\.\pipe\` namespace reaches SMB otherwise.

use std::io;
// `PathBuf` is unix-only here: the windows half addresses a pipe by name and
// never touches the filesystem.
#[cfg(unix)]
use std::path::PathBuf;

/// The environment variable the launcher sets on every native build it starts.
/// A game reads this first and falls back to the platform default.
pub const SOCKET_ENV: &str = "ELO_LAUNCHER_SOCKET";

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
            return format!("{rt}/elo/launcher.sock");
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return "./elo-launcher.sock".into();
    }
    format!("{home}/.cache/elo/launcher/launcher.sock")
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
    format!(r"\\.\pipe\elo-launcher-{user}")
}

/// A duplex byte stream to one connected game. Both platforms produce
/// something that is `Read + Write`, which is all the protocol needs.
pub trait Conn: io::Read + io::Write + Send {}
impl<T: io::Read + io::Write + Send> Conn for T {}

// ── unix ─────────────────────────────────────────────────────────────────────

/// How many games may hold the door open at once.
///
/// Not a security boundary — the socket is `0600` inside a `0700` directory,
/// so everything that can reach it already runs as the player. It bounds a
/// **leak**: a game that opens a connection per signature and never closes one
/// would otherwise spawn threads until the process died, and dying is a worse
/// answer than refusing the thirty-third.
pub const MAX_OPEN: usize = 32;

/// Serve the door until the listener stops accepting, **one thread per
/// connection**.
///
/// ⚠ **The one-at-a-time version of this loop was a real bug and the reason
/// this function exists.** It read as sufficient — *"a player runs one game"* —
/// but the count that matters is connections per game, not games. A game holds
/// the connection it made at boot for the life of the process (that is what the
/// SDK's `Overlay` is for) and opens a SECOND one whenever it needs a
/// signature. A door parked inside the first connection never accepts the
/// second, so on unix the game sits on the SDK's 300-second read timeout and
/// then concludes no launcher is running.
///
/// What that looked like from the player's side is why it survived: the boot
/// connection is answered, so the game prints a correct identity line with a
/// real address — and then every signature after it fails as though the
/// launcher were closed. `sdk/rust/elo_overlay.rs` had already written down
/// the diagnosis without knowing it applied here: *"a launcher that accepts a
/// connection and then never answers will hang the calling game … That is a
/// launcher bug in both cases."*
///
/// A panic in one connection cannot take the door or another game with it.
pub fn serve_forever<H, F>(listener: Listener, make_host: F)
where
    H: crate::Host + Send + 'static,
    F: Fn() -> H + Send + 'static,
{
    serve_forever_counted(listener, Default::default(), make_host)
}

/// [`serve_forever`], with the count of open connections kept where the caller
/// can read it.
///
/// A game holds its boot connection for as long as it runs, so a count above
/// zero means a game is running. The window reads it to know the player is
/// playing rather than away (`elo_ui::wiring::serve_relock`), and the count
/// was already here for [`MAX_OPEN`].
pub fn serve_forever_counted<H, F>(
    listener: Listener,
    open: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    make_host: F,
) where
    H: crate::Host + Send + 'static,
    F: Fn() -> H + Send + 'static,
{
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    // One table of refusals for every connection this door serves: a game
    // that hangs up and reconnects is still the game the player refused.
    let refusals = crate::SharedRefusals::default();
    loop {
        match listener.accept() {
            Ok(conn) => {
                if open.load(Ordering::Relaxed) >= MAX_OPEN {
                    // Closed rather than queued. A game reads a closed door as
                    // "no launcher", which is a state it already handles, and
                    // queueing would reintroduce exactly the wait this loop
                    // exists to end.
                    eprintln!("elo: {MAX_OPEN} games already hold the door — refusing one more");
                    drop(conn);
                    continue;
                }
                open.fetch_add(1, Ordering::Relaxed);
                let mut host = make_host();
                let counter = Arc::clone(&open);
                let refusals = Arc::clone(&refusals);
                if let Err(e) = std::thread::Builder::new()
                    .name("elo-door".into())
                    .spawn(move || {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let _ = crate::serve_conn_sharing(conn, &mut host, refusals);
                        }));
                        counter.fetch_sub(1, Ordering::Relaxed);
                    })
                {
                    // Out of threads. Say so and keep the door open — the next
                    // connection may well succeed, and a launcher that stopped
                    // accepting would be the larger failure.
                    eprintln!("elo: could not serve a game ({e})");
                    open.fetch_sub(1, Ordering::Relaxed);
                }
            }
            Err(e) => {
                eprintln!("elo: the door stopped accepting ({e})");
                return;
            }
        }
    }
}

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
        /// The socket file this listener made, as (device, inode), so `Drop`
        /// removes it only while it is still ours.
        made: Option<(u64, u64)>,
    }

    impl Listener {
        /// Bind, replacing a stale socket left by a crashed launcher — and
        /// refusing a live one.
        ///
        /// ⚠ A socket file is stale only if nothing answers on it. Unlinking
        /// one that answers took the door from a launcher that was still
        /// running: the second copy bound "successfully", and when the first
        /// one quit its `Drop` deleted the second's socket, so every game
        /// then found no launcher at all. A live door is `AddrInUse`, which
        /// is the "another copy of elo is already running" the window says.
        pub fn bind(endpoint: &str) -> io::Result<Listener> {
            let path = PathBuf::from(endpoint);
            if let Some(dir) = path.parent() {
                if !dir.as_os_str().is_empty() && !dir.exists() {
                    std::fs::create_dir_all(dir)?;
                    // 0700: the socket's directory is the access control
                    // here — set on a directory this made, never on one a
                    // named socket path happens to sit in (`$HOME`, say).
                    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
                }
            }
            if path.exists() {
                // A launcher that is restarting hands the door over by
                // exiting, so give a live answer a moment to stop.
                let mut tries = 0;
                while UnixStream::connect(&path).is_ok() {
                    tries += 1;
                    if tries >= 10 {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            format!("another launcher is already serving games at {}", path.display()),
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = std::fs::remove_file(&path);
            }
            let inner = UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            let made = identity(&path);
            Ok(Listener { inner, path, made })
        }

        pub fn accept(&self) -> io::Result<UnixStream> {
            self.inner.accept().map(|(s, _)| s)
        }

        pub fn endpoint(&self) -> String {
            self.path.display().to_string()
        }
    }

    fn identity(path: &std::path::Path) -> Option<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path).ok().map(|m| (m.dev(), m.ino()))
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // The door closes with the launcher. A socket file outliving the
            // process is the stale-door problem stated above, from the other
            // end — but only this listener's file: a path that now names
            // another launcher's socket is not ours to delete.
            if self.made.is_some() && identity(&self.path) == self.made {
                let _ = std::fs::remove_file(&self.path);
            }
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
    use std::sync::Mutex;

    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
    const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
    const PIPE_WAIT: u32 = 0x0000_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const INVALID_HANDLE_VALUE: isize = -1;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const ERROR_NO_DATA: u32 = 232;
    const SDDL_REVISION_1: u32 = 1;

    /// Who may open the door: the pipe's owner — the player who started the
    /// launcher — and SYSTEM, and nobody else. `P` stops a parent's ACEs
    /// being inherited. The default DACL this replaced also lets Everyone and
    /// the anonymous account open the pipe for reading, which is enough for
    /// another user on the machine to hold every instance open.
    const DOOR_SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

    type Handle = *mut core::ffi::c_void;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut core::ffi::c_void,
        inherit: i32,
    }

    extern "system" {
        fn CreateNamedPipeW(
            name: *const u16, open_mode: u32, pipe_mode: u32, max_instances: u32,
            out_buf: u32, in_buf: u32, timeout: u32, security: *mut SecurityAttributes,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut core::ffi::c_void) -> i32;
        fn DisconnectNamedPipe(pipe: Handle) -> i32;
        fn GetLastError() -> u32;
        fn CloseHandle(h: Handle) -> i32;
        fn LocalFree(mem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl: *const u16, revision: u32, descriptor: *mut *mut core::ffi::c_void,
            size: *mut u32,
        ) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// One pipe instance, with the door's DACL. `first` asks Windows to fail
    /// if any instance of this name already exists — someone else's.
    fn instance(name: &[u16], first: bool) -> io::Result<Handle> {
        let sddl = wide(DOOR_SDDL);
        let mut descriptor: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated and outlives the call; on success
        // `descriptor` is a LocalAlloc'd block freed below.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut sa = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor,
            inherit: 0,
        };
        let open_mode = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
        // SAFETY: `name` is a NUL-terminated UTF-16 buffer and `sa` points at
        // a live descriptor; both outlive the call.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                open_mode,
                // REJECT_REMOTE_CLIENTS: without it the pipe namespace is
                // reachable over SMB, and this door is local by design.
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024, 64 * 1024, 0,
                &mut sa,
            )
        };
        let err = io::Error::last_os_error();
        // SAFETY: the descriptor came from the conversion above and is not
        // used after this; CreateNamedPipeW copied what it needed.
        unsafe { LocalFree(descriptor) };
        if handle as isize == INVALID_HANDLE_VALUE {
            return Err(err);
        }
        Ok(handle)
    }

    /// The door on Windows.
    ///
    /// **One instance is always waiting**, and the first is made with
    /// `FILE_FLAG_FIRST_PIPE_INSTANCE`. A pipe name exists only while an
    /// instance does, so a door that created an instance per accept left a
    /// gap after each connection in which another user could create the name
    /// and be the launcher for every game that knocked next. With an instance
    /// always pending, the name is ours from `bind` until the launcher exits,
    /// and only its DACL decides who may add instances to it.
    pub struct Listener {
        endpoint: String,
        name: Vec<u16>,
        /// The instance the next `accept` connects, as an address-sized int
        /// so the listener stays `Send`.
        pending: Mutex<isize>,
    }

    impl Listener {
        /// Creates the first instance now, so a name someone else already
        /// holds fails here — at startup, loudly — rather than handing games
        /// to whoever made it.
        pub fn bind(endpoint: &str) -> io::Result<Listener> {
            if !endpoint.starts_with(r"\\.\pipe\") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("a windows endpoint must be a named pipe: {endpoint}"),
                ));
            }
            let name = wide(endpoint);
            let first = instance(&name, true).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("{endpoint} is already open by another process ({e}) — \
                             is a second launcher running?"),
                )
            })?;
            Ok(Listener {
                endpoint: endpoint.to_string(),
                name,
                pending: Mutex::new(first as isize),
            })
        }

        pub fn accept(&self) -> io::Result<std::fs::File> {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| io::Error::other("the door's pipe lock is poisoned"))?;
            let handle = *pending as Handle;
            loop {
                // SAFETY: `handle` is the live pending instance this listener owns.
                let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
                if connected != 0 {
                    break;
                }
                let err = unsafe { GetLastError() };
                match err {
                    // A client that connected between create and connect is a
                    // success, not a failure — Win32 reports it as this error.
                    ERROR_PIPE_CONNECTED => break,
                    // One that connected and left before we got here. Reset
                    // the instance and keep waiting on it.
                    ERROR_NO_DATA => {
                        unsafe { DisconnectNamedPipe(handle) };
                    }
                    _ => return Err(io::Error::from_raw_os_error(err as i32)),
                }
            }
            // The next instance exists BEFORE this one is handed off, so the
            // name never lapses.
            let next = match instance(&self.name, false) {
                Ok(next) => next,
                Err(e) => {
                    // No instance to take over: drop this client and keep its
                    // instance pending, so the handle has one owner and the
                    // name stays claimed.
                    // SAFETY: `handle` is still this listener's pending instance.
                    unsafe { DisconnectNamedPipe(handle) };
                    return Err(e);
                }
            };
            *pending = next as isize;
            // SAFETY: ownership of the connected handle moves into the File,
            // which closes it on drop; the listener now owns `next` instead.
            Ok(unsafe { std::fs::File::from(OwnedHandle::from_raw_handle(handle)) })
        }

        pub fn endpoint(&self) -> String {
            self.endpoint.clone()
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            if let Ok(p) = self.pending.lock() {
                // SAFETY: the pending instance is owned by this listener and
                // never handed out.
                unsafe { CloseHandle(*p as Handle) };
            }
        }
    }
}
