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
    // `c:` and friends. Checked on bytes so a multi-byte first char cannot
    // shift the colon out of index 1 and slip past.
    if raw.len() > 1 && raw.as_bytes()[1] == b':' {
        return Err(DepotError::new(format!("depot path looks absolute: {raw:?}")));
    }
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err(DepotError::new(format!(
            "depot path escapes the build directory: {raw:?}"
        )));
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

/// Join, then prove the result is under `root` on the real filesystem.
///
/// The parent is resolved rather than the target because the target is what we
/// are about to create. Resolution is lenient about components that do not
/// exist yet (Python's `os.path.realpath` is), so this is a symlink check and
/// never an existence check.
pub fn resolve_into(root: &Path, rel: &str) -> Result<PathBuf> {
    let safe = safe_relpath(rel)?;
    let target = root.join(&safe);
    let base = realpath_lenient(root);
    let parent = target
        .parent()
        .map(realpath_lenient)
        .unwrap_or_else(|| base.clone());
    if parent != base && !parent.starts_with(&base) {
        return Err(DepotError::new(format!(
            "path {rel:?} resolves outside the build directory"
        )));
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
