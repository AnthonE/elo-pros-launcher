//! Argument parsing, hand-rolled and tested.
//!
//! Deliberately not a crate — see the note in `Cargo.toml`. The one rule worth
//! stating: **an unknown flag is a refusal, never an ignore.** A caller that
//! believes it set something and was quietly ignored has been misled in the
//! dangerous direction, which is the same rule the broker's wire follows.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    pub verb: String,
    pub positional: Vec<String>,
    pub flags: BTreeMap<String, String>,
    /// Every occurrence of every value flag, in order.
    ///
    /// `flags` keeps the LAST, which is right for `--host` and wrong for
    /// `--set`: an edit is a set of fields and `--set band=x --set blurb=y` is
    /// the obvious way to write one. Keeping both maps costs a vector and means
    /// no caller has to know which kind of flag it is holding.
    pub repeats: BTreeMap<String, Vec<String>>,
    pub switches: Vec<String>,
}

/// Flags that take a value. Everything else with a `--` is a switch, and a
/// `--` that is neither is an error.
pub const VALUE_FLAGS: [&str; 20] = ["--games", "--host", "--platform", "--browser", "--server",
                                     "--wallet", "--room", "--limit", "--status",
                                     // the dev desk
                                     "--set", "--from", "--slot", "--title", "--body",
                                     "--body-file", "--kind", "--art", "--link", "--scope",
                                     "--days"];
pub const SWITCHES: [&str; 10] = ["--json", "--yes", "--allow-insecure", "--no-reuse", "--dry-run",
                                  "--detach", "--quiet", "--statement", "--no-watch",
                                  // Which signer signs. Only ever narrows: it forces the local
                                  // keystore over a live browser pairing, and there is no
                                  // `--paired` because pairing is already the deliberate act.
                                  "--local"];

pub fn parse(argv: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = argv.iter().peekable();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--").map(|_| a.as_str()) {
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (name, None),
            };
            if VALUE_FLAGS.contains(&name) {
                let value = match inline {
                    Some(v) => v,
                    None => it
                        .next()
                        .cloned()
                        .ok_or_else(|| format!("{name} needs a value"))?,
                };
                out.repeats
                    .entry(name.to_string())
                    .or_default()
                    .push(value.clone());
                out.flags.insert(name.to_string(), value);
            } else if SWITCHES.contains(&name) {
                if inline.is_some() {
                    return Err(format!("{name} takes no value"));
                }
                out.switches.push(name.to_string());
            } else {
                return Err(format!("unknown flag {name}"));
            }
        } else if out.verb.is_empty() {
            out.verb = a.clone();
        } else {
            out.positional.push(a.clone());
        }
    }
    Ok(out)
}

/// A url in the verb slot is a join link, not an unknown command.
///
/// **This is the whole of the desktop-handler seam.** When a player clicks
/// `elo://join/gates/host:port`, the OS runs
/// `elo elo://join/gates/host:port` — the url arrives as `argv[1]`, which is
/// where a verb lives. Rewriting it to `join <url>` here means the url handler
/// and a typed `elo join <url>` are one code path, and the verb table never
/// has to know a url can reach it.
///
/// Only the SHAPE is recognised. A malformed link is still handed to `join`,
/// so the refusal talks about links rather than reporting
/// `unknown command "elo://…"` — which names the wrong mistake to someone who
/// clicked something a friend sent them.
pub fn normalize_link(mut a: Args) -> Args {
    if elo_depot::deeplink::is_link(&a.verb) {
        a.positional.insert(0, std::mem::take(&mut a.verb));
        a.verb = "join".into();
    }
    a
}

/// What a game's `{placeholder}` argv is filled from.
///
/// Its own function because this is the seam that decides what the game is
/// *told*, and it had no test while it was three lines inside a match arm —
/// which is how Gates shipped for a while drawing an empty shard browser over
/// a live shard. `elo-depot`'s `ARG_VARS` says which names are legal; this
/// says where each one's value comes from, and the two halves are only correct
/// together.
///
/// The split is deliberate: **`server`, `wallet` and `host` are the player's**,
/// typed or chosen, and **`servers` is the title's** — the manifest already
/// names the list it serves, so asking a player to re-type a url the launcher
/// is holding would be a knob for nothing. A value that is absent is simply
/// not inserted; `launch::fill` writes the empty string for it, which every
/// game on this launcher reads as absence rather than as a bad value.
pub fn launch_values(a: &Args, servers_url: Option<&str>) -> BTreeMap<String, String> {
    // The title's half comes from `elo-depot`, which is where the GUI's two
    // launch sites read it from too — one rule, three doors.
    let mut values = elo_depot::launch::title_values(servers_url);
    for key in ["server", "wallet", "host"] {
        if let Some(v) = a.flag(&format!("--{key}")) {
            values.insert(key.into(), v.to_string());
        }
    }
    values
}

impl Args {
    pub fn has(&self, switch: &str) -> bool {
        self.switches.iter().any(|s| s == switch)
    }
    pub fn flag(&self, name: &str) -> Option<&str> {
        self.flags.get(name).map(String::as_str)
    }
    /// Every occurrence, in the order they were typed. Empty when absent.
    pub fn all(&self, name: &str) -> &[String] {
        self.repeats.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
}
