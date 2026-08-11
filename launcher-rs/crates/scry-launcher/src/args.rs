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
pub const SWITCHES: [&str; 9] = ["--json", "--yes", "--allow-insecure", "--no-reuse", "--dry-run",
                                 "--detach", "--quiet", "--statement", "--no-watch"];

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
/// `scry://join/gates/host:port`, the OS runs
/// `scry scry://join/gates/host:port` — the url arrives as `argv[1]`, which is
/// where a verb lives. Rewriting it to `join <url>` here means the url handler
/// and a typed `scry join <url>` are one code path, and the verb table never
/// has to know a url can reach it.
///
/// Only the SHAPE is recognised. A malformed link is still handed to `join`,
/// so the refusal talks about links rather than reporting
/// `unknown command "scry://…"` — which names the wrong mistake to someone who
/// clicked something a friend sent them.
pub fn normalize_link(mut a: Args) -> Args {
    if scry_depot::deeplink::is_link(&a.verb) {
        a.positional.insert(0, std::mem::take(&mut a.verb));
        a.verb = "join".into();
    }
    a
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
