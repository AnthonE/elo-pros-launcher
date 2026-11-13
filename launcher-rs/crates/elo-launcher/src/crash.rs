//! `elo report-crash`, and the offer `elo play` makes when its game crashes.
//!
//! A game this launcher starts is told where to write a crash dump
//! (`ELO_CRASH_DIR`, which is `<install>/crashes/`; `elo_depot::crash`). When
//! the game ends abnormally and left one there, the player is told what the
//! file holds and asked, then and there, whether to send it with a bug report
//! to the game's developer. The answer defaults to no, and nothing is sent
//! without a yes typed at that moment: `elo play` asks only on a terminal, and
//! otherwise prints the command that would ask (`elo report-crash <title>`).
//! `--yes` on `report-crash` is the player's yes given on the command line,
//! for an agent playing on its own machine.
//!
//! What goes: the dump, and a report naming the build, the platform and how
//! the game ended, plus a note when the player writes one (`--body`). Nothing
//! else about the machine or the player. A sent dump is renamed `<name>.sent`
//! and stays on this machine.

use crate::args::Args;
use elo_depot::crash::{self, Crash, Dump};
use elo_depot::error::DepotError;
use elo_depot::install::Install;
use elo_depot::session;
use elo_hive::clock;
use elo_net::Net;
use serde_json::Value;
use std::io::IsTerminal;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

/// How long the meter keeps a dump before deleting it (`meter/bugs.py`
/// `DUMP_DAYS`). Said before the player chooses; the answer then names the date.
pub const KEPT_DAYS: u32 = 30;

fn human(bytes: u64) -> String {
    elo_depot::install::human(bytes as i64)
}

/// What the player reads before they choose.
pub fn disclosure(slug: &str, d: &Dump) -> String {
    format!(
        "{slug} left a crash dump: {}  ({}, {})\n\
         It holds {}.\n\
         Sending it gives that file, with a report of the build, the platform and how the game \
         ended, to the game's developer; we keep it {KEPT_DAYS} days, then delete it. Nothing \
         else on this machine is sent.",
        d.path.display(),
        d.format.as_str(),
        human(d.bytes),
        d.format.contains()
    )
}

/// How the last watched session of this build ended, if the dump came from it.
fn ended_for(slug: &str, build: &str, dump: &Dump) -> String {
    let at = dump.modified.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let last = session::load(&elo_depot::dirs::state_root())
        .into_iter()
        .filter(|s| s.slug == slug && s.build == build && s.started_at <= at)
        .max_by_key(|s| s.started_at);
    match last.map(|s| s.ending) {
        Some(session::Ending::Measured { code: Some(c), .. }) if c != 0 => format!("exit code {c}"),
        Some(session::Ending::Measured { code: None, .. }) => "a signal".into(),
        _ => "a crash".into(),
    }
}

/// Send one dump with its report. The caller has the player's yes.
fn send(host: &str, install: &Install, dump: &Dump, ended: &str, note: Option<&str>)
    -> Result<Value, DepotError> {
    let platform = crash::platform_word()
        .ok_or_else(|| DepotError::new("crash reports are taken from Linux, Windows and macOS"))?;
    let data = dump.read()?;
    let reported = elo_broker::protocol::iso8601(clock::now());
    let c = Crash { slug: &install.slug, build: &install.build, platform, ended, note,
                    reported: &reported };
    let body = crash::report_body(&c, dump.format, &data);
    let got = Net::new().post_json_told(&format!("{host}/api/bugs"), &body);
    if !got.ok {
        return Err(DepotError::new(if got.reachable {
            format!("the report was refused: {}", got.why)
        } else {
            format!("could not reach {host}: {}", got.why)
        }));
    }
    let answer = got.value.unwrap_or(Value::Null);
    if answer.get("dump") == Some(&Value::Bool(true)) {
        crash::mark_sent(dump)?;
    }
    Ok(answer)
}

fn said(answer: &Value, dump: &Dump) -> String {
    let id = answer.get("id").and_then(Value::as_str).unwrap_or("?");
    match answer.get("dump_expires").and_then(Value::as_str) {
        Some(until) => format!(
            "sent: report {id}, with the dump (kept until {until}); {} is now {}{}",
            dump.name(), dump.name(), crash::SENT_SUFFIX
        ),
        None => format!("sent: report {id}; the dump was not kept (the game's dump space is full)"),
    }
}

/// `elo report-crash <slug> [build] [--body TEXT] [--yes] [--dry-run] [--json]`
pub fn run(a: &Args, host: &str, games: &Path, json_out: bool) -> Result<i32, DepotError> {
    let slug = a
        .positional
        .first()
        .ok_or_else(|| DepotError::new("which game? elo report-crash <slug>"))?;
    let install = match a.positional.get(1) {
        Some(b) => elo_depot::installed(games)
            .into_iter()
            .find(|i| &i.slug == slug && &i.build == b),
        None => elo_depot::newest_of(games, slug),
    }
    .ok_or_else(|| DepotError::new(format!("{slug}: nothing installed")))?;
    let dir = crash::dir(&install.path);
    let Some(dump) = crash::find(&dir, None).into_iter().next() else {
        return Err(DepotError::new(format!(
            "{slug} build {}: no crash dump in {} — a game writes one there when it crashes, \
             if it has a crash handler (it is told the folder as {})",
            install.build,
            dir.display(),
            crash::ENV
        )));
    };
    let ended = ended_for(slug, &install.build, &dump);
    eprintln!("{}", disclosure(slug, &dump));
    if a.has("--dry-run") {
        println!("would send {} ({}) as a crash report for {slug} build {} that ended with {ended}; \
                  nothing was sent", dump.name(), dump.format.as_str(), install.build);
        return Ok(0);
    }
    let yes = a.has("--yes")
        || crate::prompt::confirm("Send it? [y/N] ").map_err(DepotError::new)?;
    if !yes {
        println!("not sent");
        return Ok(0);
    }
    let answer = send(host, &install, &dump, &ended, a.flag("--body"))?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&answer).unwrap_or_default());
    } else {
        println!("{}", said(&answer, &dump));
    }
    Ok(0)
}

/// After `elo play` watched its game end with `ended` (`crash::abnormal`):
/// offer the dump it left, if any. Never sends without a yes typed now.
pub fn after_play(host: &str, install: &Install, started_at: i64, ended: &str) {
    // A dump written during this run; a second's slack for coarse clocks.
    let since = UNIX_EPOCH + Duration::from_secs(started_at.saturating_sub(1).max(0) as u64);
    let dir = crash::dir(&install.path);
    let Some(dump) = crash::find(&dir, Some(since)).into_iter().next() else {
        println!("{} ended with {ended}; it left no crash dump in {}", install.slug, dir.display());
        return;
    };
    println!("{} ended with {ended}.", install.slug);
    eprintln!("{}", disclosure(&install.slug, &dump));
    if !std::io::stdin().is_terminal() {
        println!("to send it: elo report-crash {}", install.slug);
        return;
    }
    match crate::prompt::confirm("Send it? [y/N] ") {
        Ok(true) => match send(host, install, &dump, ended, None) {
            Ok(answer) => println!("{}", said(&answer, &dump)),
            Err(e) => println!("{e}; `elo report-crash {}` tries again", install.slug),
        },
        _ => println!("not sent; `elo report-crash {}` asks again", install.slug),
    }
}

