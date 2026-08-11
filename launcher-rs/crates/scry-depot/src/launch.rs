//! Starting things — a browser, or a binary.
//!
//! The native path is arbitrary code by construction; `depot.rs` says why that
//! is acceptable and what bounds it. What is bounded **here** is the launch
//! itself: no shell, an argv list, a cwd inside the build, and an environment
//! that cannot be used to redirect the dynamic loader at something outside the
//! build directory.

use crate::error::{DepotError, Result};
use crate::install::{Install, RECEIPT_NAME};
use crate::path::resolve_into;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Loader-sensitive variables a depot may never set.
///
/// `LD_LIBRARY_PATH` is deliberately **not** here — game builds legitimately
/// need it — but it is the one variable whose *value* is checked, below, and
/// it must point inside the build.
pub const ENV_DENY: [&str; 9] = [
    "LD_PRELOAD",
    "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "PATH",
    "PYTHONPATH",
    "GTK_MODULES",
    "GIO_MODULE_DIR",
    "XDG_DATA_DIRS",
];

/// Placeholders a depot's argv may use. Anything else is a refusal.
pub const ARG_VARS: [&str; 4] = ["server", "wallet", "build_dir", "host"];

/// The depot platform string for this machine.
pub fn platform_tag() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "linux" => format!("linux-{arch}"),
        "macos" => {
            if arch == "aarch64" {
                "mac-arm64".into()
            } else {
                "mac-x86_64".into()
            }
        }
        "windows" => "win-x86_64".into(),
        other => format!("{other}-{arch}"),
    }
}

/// Open a page.
///
/// https(s) only — a launcher that will hand any scheme a catalog gives it to
/// the desktop will happily open `file:///` or a custom handler.
pub fn browser_argv(url: &str, browser_cmd: &str) -> Result<Vec<String>> {
    let scheme = url.split_once("://").map(|(s, _)| s).unwrap_or("");
    if scheme != "https" && scheme != "http" {
        return Err(DepotError::new(format!(
            "refusing to open a {} url",
            if scheme.is_empty() { "schemeless" } else { scheme }
        )));
    }
    let mut argv: Vec<String> = split_cmd(browser_cmd);
    // `{url}` lets a browser command express app mode — `chromium --app={url}`
    // needs the URL inside one argument, which appending never produces.
    if argv.iter().any(|a| a.contains("{url}")) {
        for a in argv.iter_mut() {
            *a = a.replace("{url}", url);
        }
    } else if argv.is_empty() {
        argv = vec!["xdg-open".into(), url.to_string()];
    } else {
        argv.push(url.to_string());
    }
    Ok(argv)
}

/// Minimal POSIX-ish word splitting with quotes. A browser command is a
/// setting a human types; it is not a shell line and is never run by one.
fn split_cmd(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for ch in cmd.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                has = true;
            }
            None if ch.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            None => cur.push(ch),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn check_env(env: &BTreeMap<String, String>, build: &Path) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (k, v) in env {
        if k.is_empty()
            || !k.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(DepotError::new(format!("bad environment variable name {k:?}")));
        }
        if ENV_DENY.contains(&k.as_str()) {
            return Err(DepotError::new(format!("a depot may not set {k}")));
        }
        if k == "LD_LIBRARY_PATH" {
            // Every entry must be a relative path inside the build, made
            // absolute here. Otherwise the depot picks which libc you run.
            let mut parts = Vec::new();
            for entry in v.split(':').filter(|e| !e.is_empty()) {
                parts.push(resolve_into(build, entry)?.to_string_lossy().to_string());
            }
            out.insert(k.clone(), parts.join(":"));
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    Ok(out)
}

/// Substitute `{server}`-style placeholders.
///
/// An unknown placeholder is a refusal, not a passthrough — a literal
/// `{token}` reaching a game's argv is a bug that shows up as a mystery once,
/// in production, on someone else's machine.
fn fill(args: &[String], values: &BTreeMap<String, String>) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        let mut s = String::with_capacity(a.len());
        let mut rest = a.as_str();
        while let Some(open) = rest.find('{') {
            s.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let Some(close) = after.find('}') else {
                s.push('{');
                rest = after;
                continue;
            };
            let name = &after[..close];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                if !ARG_VARS.contains(&name) {
                    return Err(DepotError::new(format!(
                        "launch arg uses unknown placeholder {{{name}}}"
                    )));
                }
                s.push_str(values.get(name).map(String::as_str).unwrap_or(""));
            } else {
                s.push('{');
                s.push_str(name);
                s.push('}');
            }
            rest = &after[close + 1..];
        }
        s.push_str(rest);
        out.push(s);
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Launch {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

/// Resolve an installed build into an argv, a cwd and an environment.
///
/// Every check here restates one the installer already made, on purpose: the
/// receipt is a file on disk and a build directory can be edited after it is
/// written. Trusting our own past output is how a verified install becomes an
/// unverified launch.
pub fn resolve(
    install: &Install,
    values: &BTreeMap<String, String>,
    extra_env: &BTreeMap<String, String>,
) -> Result<Launch> {
    let path = &install.path;
    if !path.join(RECEIPT_NAME).is_file() {
        return Err(DepotError::new(format!(
            "{}: no install receipt — refusing to launch",
            path.display()
        )));
    }
    let exec_rel = crate::path::safe_relpath(&install.launch_exec)?;
    if !install.files.iter().any(|f| f.path == exec_rel) {
        return Err(DepotError::new(format!(
            "{exec_rel} is not one of this build's files"
        )));
    }
    let binary = resolve_into(path, &exec_rel)?;
    if !binary.is_file() {
        return Err(DepotError::new(format!(
            "{exec_rel} is missing — verify the install"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&binary)?.permissions().mode();
        if mode & 0o111 == 0 {
            return Err(DepotError::new(format!("{exec_rel} is not executable")));
        }
    }
    let cwd = if install.launch_cwd == "." {
        path.clone()
    } else {
        resolve_into(path, &install.launch_cwd)?
    };
    let mut vals = values.clone();
    vals.entry("build_dir".into())
        .or_insert_with(|| path.to_string_lossy().to_string());
    let mut argv = vec![binary.to_string_lossy().to_string()];
    argv.extend(fill(&install.launch_args, &vals)?);
    // The depot's env goes through the checks; the LAUNCHER's own additions do
    // not, and the order matters — `extra_env` is applied last so a depot
    // cannot shadow the broker socket with one of its own and stand between
    // the game and the player's signer.
    let mut env = check_env(&install.launch_env, path)?;
    for (k, v) in extra_env {
        env.insert(k.clone(), v.clone());
    }
    Ok(Launch { argv, cwd, env })
}

fn command(launch: &Launch) -> Command {
    let mut cmd = Command::new(&launch.argv[0]);
    cmd.args(&launch.argv[1..]).current_dir(&launch.cwd);
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }
    cmd
}

/// Start an installed build and let it go. Returns the child's pid.
///
/// Nothing watches it after this, which is why a session started this way can
/// never yield playtime — see `session.rs`.
pub fn spawn(launch: &Launch) -> Result<u32> {
    let child = command(launch)
        .spawn()
        .map_err(|e| DepotError::new(format!("could not start {}: {e}", launch.argv[0])))?;
    Ok(child.id())
}

/// Start an installed build and hand back the child, so the caller can record
/// the pid and then wait on it.
///
/// **This is the shape that produces real playtime**, and it is what a GUI
/// client does too — Steam's client supervises the game process; that is how
/// it knows you played for two hours and can say "in-game" while you did.
pub fn spawn_supervised(launch: &Launch) -> Result<std::process::Child> {
    command(launch)
        .spawn()
        .map_err(|e| DepotError::new(format!("could not start {}: {e}", launch.argv[0])))
}
