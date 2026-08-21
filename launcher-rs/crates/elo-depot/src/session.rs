//! Sessions and playtime — knowing a game is running, and for how long.
//!
//! The original client started a process with `start_new_session=True` and
//! forgets it: no running state, no stop, no playtime. Steam's whole in-game
//! presence layer sits on exactly this, and an MMO with friends and shards
//! wants it far more than a single-player title does.
//!
//! ## The honest constraint, which shapes the whole design
//!
//! **Playtime is only real if something watched the process.** A launcher that
//! detaches and later guesses when the game ended is inventing a number, and
//! wall 3 of `LAUNCHER.md` is that this client is not a source of truth — no
//! local leaderboard, no cached balance shown as current, nothing invented.
//!
//! So a session has three shapes and they are not interchangeable:
//!
//! | | `ended_at` | `seconds` | counts toward playtime |
//! |---|---|---|---|
//! | **supervised, finished** | known | measured | **yes** |
//! | **running** | none | none yet | no — it has not happened |
//! | **detached** | never known | never known | **no**, and it says so |
//!
//! A detached session is kept because *"you launched this"* is true and worth
//! showing. What it may never do is become a number.

use crate::error::{DepotError, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SESSIONS_NAME: &str = "sessions.json";
/// Enough to answer "what have I been playing" without growing forever.
pub const MAX_SESSIONS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// Watched from start to finish. The only shape that yields playtime.
    Measured { ended_at: i64, seconds: i64, code: Option<i32> },
    /// Started and still going, as far as we know.
    Running { pid: u32 },
    /// Launched and let go. We know it began and will never know it ended.
    Detached { pid: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub slug: String,
    pub build: String,
    pub started_at: i64,
    pub ending: Ending,
}

impl Session {
    /// Seconds that may be counted. `None` is not zero — it is *unknown*, and
    /// the caller has to render the difference.
    pub fn playtime(&self) -> Option<i64> {
        match self.ending {
            Ending::Measured { seconds, .. } => Some(seconds),
            _ => None,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.ending, Ending::Running { .. })
    }

    fn to_json(&self) -> Value {
        let mut v = json!({
            "slug": self.slug, "build": self.build, "started_at": self.started_at,
        });
        let obj = v.as_object_mut().expect("just built an object");
        match &self.ending {
            Ending::Measured { ended_at, seconds, code } => {
                obj.insert("state".into(), json!("measured"));
                obj.insert("ended_at".into(), json!(ended_at));
                obj.insert("seconds".into(), json!(seconds));
                obj.insert("exit_code".into(), json!(code));
            }
            Ending::Running { pid } => {
                obj.insert("state".into(), json!("running"));
                obj.insert("pid".into(), json!(pid));
            }
            Ending::Detached { pid } => {
                obj.insert("state".into(), json!("detached"));
                obj.insert("pid".into(), json!(pid));
            }
        }
        v
    }

    fn from_json(v: &Value) -> Option<Session> {
        let pid = v.get("pid").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let ending = match v.get("state").and_then(|x| x.as_str())? {
            "measured" => Ending::Measured {
                ended_at: v.get("ended_at").and_then(|x| x.as_i64()).unwrap_or(0),
                seconds: v.get("seconds").and_then(|x| x.as_i64()).unwrap_or(0),
                code: v.get("exit_code").and_then(|x| x.as_i64()).map(|c| c as i32),
            },
            "running" => Ending::Running { pid },
            "detached" => Ending::Detached { pid },
            _ => return None,
        };
        Some(Session {
            slug: v.get("slug")?.as_str()?.to_string(),
            build: v.get("build")?.as_str()?.to_string(),
            started_at: v.get("started_at").and_then(|x| x.as_i64()).unwrap_or(0),
            ending,
        })
    }
}

pub fn sessions_path(state_root: &Path) -> PathBuf {
    state_root.join(SESSIONS_NAME)
}

/// Read the log. A missing or unreadable file is an empty log, not an error —
/// but note the asymmetry with a network read: this one really is *"nothing
/// there"*, because we looked at a local file and it was not there.
pub fn load(state_root: &Path) -> Vec<Session> {
    let Ok(raw) = std::fs::read_to_string(sessions_path(state_root)) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    v.get("sessions")
        .and_then(|s| s.as_array())
        .map(|a| a.iter().filter_map(Session::from_json).collect())
        .unwrap_or_default()
}

pub fn save(state_root: &Path, sessions: &[Session]) -> Result<()> {
    std::fs::create_dir_all(state_root)?;
    let start = sessions.len().saturating_sub(MAX_SESSIONS);
    let rows: Vec<Value> = sessions[start..].iter().map(Session::to_json).collect();
    let body = serde_json::to_string_pretty(&json!({"version": 1, "sessions": rows}))
        .map_err(|e| DepotError::new(e.to_string()))?;
    // Written through a temp file in the same directory and renamed, so a kill
    // mid-write cannot leave a truncated log that `load` silently reads as
    // "you have never played anything".
    let tmp = sessions_path(state_root).with_extension("json.tmp");
    std::fs::write(&tmp, body + "\n")?;
    std::fs::rename(&tmp, sessions_path(state_root))?;
    Ok(())
}

pub fn append(state_root: &Path, session: Session) -> Result<()> {
    let mut all = load(state_root);
    all.push(session);
    save(state_root, &all)
}

/// Close the most recent open session for a build.
///
/// Matched newest-first on `(slug, build)`, because a player can have two
/// builds of one title installed and start either.
pub fn close(
    state_root: &Path,
    slug: &str,
    build: &str,
    ended_at: i64,
    code: Option<i32>,
) -> Result<bool> {
    let mut all = load(state_root);
    let hit = all
        .iter_mut()
        .rev()
        .find(|s| s.slug == slug && s.build == build && s.is_running());
    let Some(s) = hit else { return Ok(false) };
    s.ending = Ending::Measured {
        ended_at,
        seconds: (ended_at - s.started_at).max(0),
        code,
    };
    save(state_root, &all)?;
    Ok(true)
}

/// Is a pid still alive? `kill(pid, 0)` — asks without signalling.
///
/// ⚠ **Pids are recycled**, so a `Running` row whose pid now belongs to some
/// unrelated process reads as still-playing. That is why this only ever
/// decides a *display* state and never contributes to playtime: the worst it
/// can produce is a stale "running" line, not a wrong number.
#[cfg(unix)]
pub fn is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: `kill` with signal 0 performs the permission and existence check
    // and delivers nothing. No memory is touched.
    unsafe { libc_kill(pid as i32, 0) == 0 }
}

/// The same question on Windows, asked of the kernel rather than assumed.
///
/// This returned a flat `false` until 2026-08-06, which is the repo's own trap
/// — *a never-raises reader returns the empty answer for both "nothing there"
/// and "we could not look."* Every session would have read as not-running, so
/// the client's whole `── running ──` block would be permanently empty on
/// Windows and look like a correct measurement of an idle machine.
///
/// `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` then `GetExitCodeProcess`:
/// a handle that opens and reports `STILL_ACTIVE` is a live process. Declared
/// as a raw extern the way `kill` is above, so this costs no dependency — the
/// tree budget in `supply_chain.rs` is a wall, and one bool is not worth a
/// crate. ⚠ `STILL_ACTIVE` is 259, so a process that genuinely exits *with*
/// code 259 reads as alive until its handle closes. That is the documented
/// Win32 ambiguity and it lands in the same display-only place the pid-recycle
/// caveat above does: a stale "running" line, never a wrong number.
#[cfg(windows)]
pub fn is_alive(pid: u32) -> bool {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    if pid == 0 {
        return false;
    }
    // SAFETY: OpenProcess returns null on failure, which is checked before use.
    // GetExitCodeProcess writes one u32 through a pointer to a live local. The
    // handle is closed on both paths.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE
    }
}

/// Neither unix nor windows. Kept so the crate builds anywhere, and kept
/// **`false` with this comment attached** rather than quietly: there is no
/// third way to ask, and a target that reaches here has no session display.
#[cfg(not(any(unix, windows)))]
pub fn is_alive(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

#[cfg(windows)]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
    fn GetExitCodeProcess(handle: *mut core::ffi::c_void, code: *mut u32) -> i32;
    fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
}

/// Total measured playtime per title. Sessions we never watched are excluded
/// by construction — `playtime()` returns `None` for them.
pub fn totals(sessions: &[Session]) -> std::collections::BTreeMap<String, i64> {
    let mut out = std::collections::BTreeMap::new();
    for s in sessions {
        if let Some(secs) = s.playtime() {
            *out.entry(s.slug.clone()).or_insert(0) += secs;
        }
    }
    out
}

/// How many sessions we could not measure, per title — so a surface can say
/// *"3 launches not measured"* instead of quietly under-reporting.
pub fn unmeasured(sessions: &[Session]) -> std::collections::BTreeMap<String, usize> {
    let mut out = std::collections::BTreeMap::new();
    for s in sessions {
        if matches!(s.ending, Ending::Detached { .. }) {
            *out.entry(s.slug.clone()).or_insert(0) += 1;
        }
    }
    out
}
