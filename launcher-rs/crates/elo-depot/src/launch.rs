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
///
/// Compared upper-cased, so `Path` on Windows is `PATH`.
///
/// The runtimes a game is likely built on each have a variable that loads
/// code before `main`: Mono and .NET (Unity and Godot-C# builds), the CLR
/// profiler on Windows, SDL's dynamic API, the Mesa/VA/OpenCL driver loaders,
/// and the interpreters' option variables. Each lets a depot that passed
/// every hash run a library it never shipped. The `VK_`, `LIBGL_` and
/// `__EGL_` families are covered by [`ENV_DENY_PREFIX`].
pub const ENV_DENY: [&str; 41] = [
    "LD_PRELOAD",
    "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "PATH",
    "PYTHONPATH",
    "GTK_MODULES",
    "GIO_MODULE_DIR",
    "XDG_DATA_DIRS",
    "GCONV_PATH",
    "GIO_EXTRA_MODULES",
    "QT_PLUGIN_PATH",
    "GTK_PATH",
    "NODE_OPTIONS",
    "PERL5LIB",
    "RUBYLIB",
    "LOCPATH",
    "PYTHONHOME",
    "PYTHONSTARTUP",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "BASH_ENV",
    "ENV",
    "PERL5OPT",
    "RUBYOPT",
    "MONO_PATH",
    "MONO_CONFIG",
    "MONO_ENV_OPTIONS",
    "DOTNET_STARTUP_HOOKS",
    "DOTNET_ADDITIONAL_DEPS",
    "CORECLR_PROFILER",
    "CORECLR_PROFILER_PATH",
    "CORECLR_ENABLE_PROFILING",
    "COR_PROFILER",
    "COR_PROFILER_PATH",
    "COR_ENABLE_PROFILING",
    "SDL_DYNAMIC_API",
    "LIBVA_DRIVERS_PATH",
    "GBM_BACKENDS_PATH",
    "OCL_ICD_FILENAMES",
];

/// Whole families of loader variables: the glibc and macOS loaders, the
/// Vulkan, Mesa and GLVND driver loaders. Each names a library or a directory
/// to load code from. `LD_LIBRARY_PATH` is the one `LD_` name let through, and
/// only with its value forced inside the build (below). GStreamer's plugin
/// paths and Lua's module paths carry a version suffix, so they are families
/// too.
pub const ENV_DENY_PREFIX: [&str; 9] = [
    "LD_", "DYLD_", "VK_", "LIBGL_", "__EGL_", "__GLX_", "GST_PLUGIN_", "LUA_PATH", "LUA_CPATH",
];

/// Placeholders a depot's argv may use. Anything else is a refusal.
///
/// `servers` is the **manifest's** `servers.url`, not a shard address: a game
/// with its own in-client shard browser needs the list the Servers window
/// reads, and until this existed there was no way for a depot document to ask
/// for it. Gates shipped with exactly that hole — the list was published, the
/// url was set, and every launch drew an empty browser because nothing could
/// pass it. An unset one arrives as the empty string, the same shape `{wallet}`
/// already has, which a game reads as absence rather than as a bad url.
pub const ARG_VARS: [&str; 5] = ["server", "wallet", "build_dir", "host", "servers"];

/// The values a launch gets from the TITLE, before the player adds any.
///
/// One function because there are **three** launch sites — `elo play`, the
/// Servers window's Join, and the GUI's Play button — and the rule they have to
/// agree on is the one that is easy to miss: **`servers` is the title's, not
/// the player's.** The manifest already names the shard list it serves, so a
/// launcher that is holding that url and passes nothing is drawing an empty
/// browser over a live shard.
///
/// ⚠ That has now happened twice, and the second time is why this exists. The
/// first was the depot side — no argv could ask for `{servers}` at all, which
/// `ARG_VARS` above records. The second was the GUI's **Play** button passing
/// an empty map while the Servers window beside it passed a full one, so the
/// same title launched with a shard list through one door and without it
/// through the other. Two call sites, two rules, and only one of them was ever
/// tested.
///
/// The player's own values — `server`, `wallet`, `host` — are inserted by the
/// caller on top of this, because only the caller knows whether a player typed
/// or chose one. An absent value is simply not inserted; [`fill`] writes the
/// empty string for it, which every game on this launcher reads as absence
/// rather than as a bad value.
pub fn title_values(servers_url: Option<&str>) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    if let Some(u) = servers_url {
        values.insert("servers".to_string(), u.to_string());
    }
    values
}

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

/// The platform's own "open this page" command, as an argv for this machine.
pub fn open_argv(url: &str) -> Result<Vec<String>> {
    open_argv_for(cfg!(target_os = "windows"), url)
}

/// [`open_argv`], for a named platform so both halves are testable anywhere.
///
/// ⚠ **Never `cmd /C start`.** cmd parses the url as a command line, so `&`
/// in it starts a second command — and a query string is made of `&`. On
/// Windows the url goes to the url handler as one argument, and nothing reads
/// it as a command. The url is also held to RFC 3986's characters first, so a
/// quote, caret or space cannot split it on any platform.
pub fn open_argv_for(windows: bool, url: &str) -> Result<Vec<String>> {
    let scheme = url.split_once("://").map(|(s, _)| s).unwrap_or("");
    if scheme != "https" && scheme != "http" {
        return Err(DepotError::new(format!(
            "refusing to open a {} url",
            if scheme.is_empty() { "schemeless" } else { scheme }
        )));
    }
    if let Some(bad) = url
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(*c)))
    {
        return Err(DepotError::new(format!(
            "refusing to open a url carrying {bad:?} — percent-encode it"
        )));
    }
    Ok(if windows {
        vec!["rundll32.exe".into(), "url.dll,FileProtocolHandler".into(), url.to_string()]
    } else {
        vec!["xdg-open".into(), url.to_string()]
    })
}

/// Open a page with [`open_argv`]. Spawned, never waited on.
pub fn open_page(url: &str) -> Result<()> {
    let argv = open_argv(url)?;
    Command::new(&argv[0])
        .args(&argv[1..])
        .spawn()
        .map(|_| ())
        .map_err(|e| DepotError::new(format!("could not open a browser: {e}")))
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
        // Upper-cased first: Windows reads environment names without case, so
        // `Path` IS `PATH` there, and a compare on the spelling lets it by.
        let upper = k.to_ascii_uppercase();
        if ENV_DENY.contains(&upper.as_str())
            || (upper != "LD_LIBRARY_PATH"
                && ENV_DENY_PREFIX.iter().any(|p| upper.starts_with(p)))
        {
            return Err(DepotError::new(format!("a depot may not set {k}")));
        }
        if upper == "LD_LIBRARY_PATH" {
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

/// The refusal an unknown placeholder earns, worded for the two people who
/// will read it: the player it lands on, who can do nothing about it, and the
/// publisher whose depot it names the fix for.
fn unknown_placeholder(name: &str) -> DepotError {
    let known = ARG_VARS.map(|v| format!("{{{v}}}")).join(", ");
    // `{servers}` for `{server}` is the observed failure — a build shipped
    // with the plural and every Play of it died here, as "a weird issue".
    // Naming the likely intent turns that dialog into a one-word depot fix.
    let hint = ARG_VARS
        .iter()
        .copied()
        .find(|v| name.strip_suffix('s') == Some(v) || v.strip_suffix('s') == Some(name))
        .map(|v| format!(" — probably {{{v}}}, and the fix belongs in the depot"))
        .unwrap_or_default();
    DepotError::new(format!(
        "launch arg uses unknown placeholder {{{name}}}; this launcher fills {known}{hint}"
    ))
}

/// Refuse argv placeholders this launcher will never fill.
///
/// The installer calls this so a bad launch command is refused while it is
/// still a download — the alternative is what `{servers}` actually did: a
/// build that installs, verifies clean, and then dies on Play, one player at
/// a time. Launching still re-checks through [`fill`], because a receipt is a
/// file on disk and can be edited after it is written.
pub fn check_args(args: &[String]) -> Result<()> {
    fill(args, &BTreeMap::new()).map(|_| ())
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
                    return Err(unknown_placeholder(name));
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
    // Streamer mode rides every launch from here, so no door can start a game
    // that shows what the launcher is hiding (`streamer.rs`).
    env.extend(crate::streamer::launch_env());
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
