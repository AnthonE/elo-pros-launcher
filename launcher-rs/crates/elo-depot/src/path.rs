//! Path safety — installing a depot writes files a stranger named.
//!
//! Two checks that look redundant and are not. `safe_relpath` catches the
//! textual attacks; `resolve_into` catches the one that is not textual — a
//! symlinked component already on disk pointing out of the tree. Dropping
//! either is how a traversal lands.

use crate::error::{DepotError, Result};
use std::path::{Path, PathBuf};

pub const MAX_PATH_LEN: usize = 1024;

/// Accept a depot path or refuse it. There is no sanitising middle.
///
/// Refused: absolute, `..` anywhere, a drive letter, a NUL, an empty
/// component, a trailing separator, and a backslash — which is a legal
/// filename character on Linux and would let `a\..\..\b` read as traversal on
/// the Windows build of the same depot.
///
/// Also refused: any path that is not already canonical (`./a`, `a//b`). Those
/// normalise to something safe, so this is **not** a traversal rule — it is a
/// uniqueness rule. `a/b` and `./a/b` are two strings for one file, so a depot
/// could list both, slip the duplicate check, and let whichever downloaded
/// last silently win. One spelling per file.
///
/// The same rule, for the disks players actually have. Refused too, because
/// each is a second spelling of a name on Windows or macOS: a `:` anywhere
/// (a drive letter, or `run:x` — an NTFS alternate stream of `run`), a
/// component ending in `.` or a space (Windows trims both, so `run.` IS
/// `run`), a Windows device name (`nul`, `com1.txt` open the device, not a
/// file), and anything outside ASCII — macOS normalises Unicode, so `é`
/// typed two ways is one file there and two here, and no crate in this
/// tree's budget does that normalisation to compare them.
pub fn safe_relpath(raw: &str) -> Result<String> {
    if raw.is_empty() || raw.len() > MAX_PATH_LEN {
        return Err(DepotError::new(format!("bad depot path: {raw:?}")));
    }
    if raw.contains('\0') || raw.contains('\\') {
        return Err(DepotError::new(format!(
            "bad depot path (NUL or backslash): {raw:?}"
        )));
    }
    if raw.starts_with('/') || raw.ends_with('/') {
        return Err(DepotError::new(format!(
            "depot paths must be relative files: {raw:?}"
        )));
    }
    // `c:` and `run:stream` alike. Anywhere, not only at index 1: a colon
    // later in a name is an alternate data stream on NTFS, which writes into
    // a file the depot lists under another name.
    if raw.contains(':') {
        return Err(DepotError::new(format!(
            "a depot path may not hold ':' (a drive, or an NTFS stream): {raw:?}"
        )));
    }
    if !raw.is_ascii() {
        return Err(DepotError::new(format!(
            "depot paths are ASCII — macOS folds two spellings of one accented \
             name into one file: {raw:?}"
        )));
    }
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err(DepotError::new(format!(
            "depot path escapes the build directory: {raw:?}"
        )));
    }
    for p in &parts {
        if p.ends_with('.') || p.ends_with(' ') {
            return Err(DepotError::new(format!(
                "a depot path component ends in '.' or a space, which Windows \
                 drops — {p:?} is another file's name there: {raw:?}"
            )));
        }
        // The device check is on the name before its first dot, the way
        // Windows makes it: `nul.txt` and `CON.tar.gz` are devices too.
        let stem = p.split('.').next().unwrap_or("").trim_end_matches(' ');
        if WINDOWS_DEVICES.iter().any(|d| stem.eq_ignore_ascii_case(d)) {
            return Err(DepotError::new(format!(
                "{p:?} is a Windows device name, not a file: {raw:?}"
            )));
        }
    }
    // The canonical check. With no empty/./.. components left, re-joining is
    // the normalised spelling, so any difference means the input was not it.
    let joined = parts.join("/");
    if joined != raw {
        return Err(DepotError::new(format!(
            "depot path is not canonical (write {joined:?}): {raw:?}"
        )));
    }
    Ok(joined)
}

/// Names Windows opens as a device whatever the directory or extension.
const WINDOWS_DEVICES: [&str; 26] = [
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$",
    "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The one name a case-insensitive disk sees for a depot path: lowercased,
/// with the trailing dots and spaces Windows drops trimmed from every
/// component. Two depot files with one key are one file on NTFS and APFS, so
/// this — never the raw string — is what a duplicate check compares.
pub fn alias_key(rel: &str) -> String {
    rel.split('/')
        .map(|c| c.trim_end_matches(['.', ' ']).to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

/// Join, then prove the result is under `root` on the real filesystem.
///
/// The parent is resolved rather than the target because the target is what we
/// are about to create. Resolution is lenient about components that do not
/// exist yet (Python's `os.path.realpath` is), so this is a symlink check and
/// never an existence check. The target itself is resolved too: a leaf that is
/// already a symlink out of the tree is refused, not followed.
pub fn resolve_into(root: &Path, rel: &str) -> Result<PathBuf> {
    let safe = safe_relpath(rel)?;
    let target = root.join(&safe);
    let base = realpath_lenient(root);
    let parent = target
        .parent()
        .map(realpath_lenient)
        .unwrap_or_else(|| base.clone());
    for got in [parent, realpath_lenient(&target)] {
        if got != base && !got.starts_with(&base) {
            return Err(DepotError::new(format!(
                "path {rel:?} resolves outside the build directory"
            )));
        }
    }
    Ok(target)
}

/// `os.path.realpath` semantics: resolve as far as the filesystem goes, then
/// append what does not exist yet. `fs::canonicalize` alone refuses a path
/// with a missing tail, which is every path we are about to write.
pub fn realpath_lenient(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    let mut unresolved: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        match cur.parent() {
            Some(parent) => {
                if let Some(name) = cur.file_name() {
                    unresolved.push(name.to_os_string());
                }
                if let Ok(resolved) = std::fs::canonicalize(parent) {
                    let mut out = resolved;
                    for part in unresolved.iter().rev() {
                        out.push(part);
                    }
                    return out;
                }
                cur = parent.to_path_buf();
            }
            None => return path.to_path_buf(),
        }
    }
}
