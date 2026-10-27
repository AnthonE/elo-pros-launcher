//! `elo mods` — the Workshop mods a wallet subscribed to, on this machine.
//!
//! The origin half is `meter/workshop.py`: a public list of what a wallet
//! subscribed to in a game, every file with its GitHub release url, size and
//! sha256. The origin stores none of the bytes. Landing them is
//! `elo_depot::workshop` — the crate that already decides where a stranger's
//! bytes go on this disk — so this file reads, prints and asks; it never
//! writes a mod file itself.
//!
//! Reading needs no key, like a friends list. Subscribing is the friends
//! change's three beats (`friends.rs`): prepare, sign, submit, with the text
//! rebuilt here and signed only if the origin offered exactly that.

use crate::args::Args;
use crate::friends::{safe_text, told, whose_for};
use crate::signer;
use elo_broker::signer::Signer;
use elo_depot::install::human;
use elo_depot::workshop::{self, Outcome, Subscribed, Synced};
use elo_depot::DepotError;
use elo_hive::render::{clip, safe_line};
use elo_net::friends::{self as roster};
use elo_net::Net;
use serde_json::{json, Value};
use std::path::Path;

pub const USAGE: &str = "\
elo mods — Workshop mods you subscribed to, on this machine

  elo mods <game>                       what you subscribed to for that game,
                                        and anything held back, with why
  elo mods sync <game>                  download each mod's files, check each
                                        file's sha256, and remove the mods you
                                        dropped — prints the folder a game
                                        loads them from
  elo mods subscribe <game> <mod>       subscribe to a mod (one signature)
  elo mods unsubscribe <game> <mod>     unsubscribe (one signature)

  --wallet ADDR    whose subscriptions (default: ELO_WALLET, then this
                   machine's signer)
  --dry-run        sync: say what would be downloaded and removed;
                   subscribe: print the text that would be signed
  --json           machine-readable output

Files go to <mods folder>/<game>/<mod>/, and mods.json beside them lists the
mods whose every file checked, in the order you subscribed. The mods folder is
ELO_MODS, else ~/.local/share/elo/mods. A file whose bytes differ from what
the author published is deleted, never kept.
";

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let net = Net::new();
    match a.positional.first().map(String::as_str) {
        None => Err(DepotError::new("which game? — `elo mods <game>`, e.g. `elo mods gates`")),
        Some("help") => {
            print!("{USAGE}");
            Ok(0)
        }
        Some("sync") => sync(&net, a, host, json_out),
        Some(op @ ("subscribe" | "unsubscribe")) => change(&net, a, host, op, json_out),
        Some(_) => list(&net, a, host, json_out),
    }
}

/// The game at `positional[at]`, held to the store's slug rule — it becomes a
/// url path and a folder name.
fn game_at(a: &Args, at: usize) -> Result<String, DepotError> {
    let typed = a
        .positional
        .get(at)
        .ok_or_else(|| DepotError::new("which game? — a slug, like gates"))?;
    let game = typed.trim().to_ascii_lowercase();
    if !workshop::is_slug(&game) {
        return Err(DepotError::new(format!(
            "{} is not a game slug — lowercase letters, digits and '-'",
            safe_line(typed)
        )));
    }
    Ok(game)
}

/// One read of the subscribed list, with "we could not look" kept apart from
/// "it said no" — and nothing on disk changes on either.
fn fetch(net: &Net, host: &str, game: &str, wallet: &str) -> Result<Subscribed, DepotError> {
    let got = net.get_json_told(&format!("{host}/api/workshop/{game}/subscribed/{wallet}"));
    let doc = match got.value {
        Some(v) if got.ok => v,
        _ if !got.reachable => {
            return Err(DepotError::new(format!(
                "could not reach {host} — {}",
                safe_line(&got.why)
            )))
        }
        _ => {
            return Err(DepotError::new(format!(
                "the subscribed list did not read — {}",
                safe_line(&got.why)
            )))
        }
    };
    let sub = workshop::parse_subscribed(&doc).map_err(|e| {
        DepotError::new(format!("the subscribed list did not read — {}", safe_line(&e.to_string())))
    })?;
    if sub.game != game {
        return Err(DepotError::new(format!(
            "asked for {game}'s mods and the origin answered for {}",
            safe_line(&sub.game)
        )));
    }
    Ok(sub)
}

fn list(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let game = game_at(a, 0)?;
    let me = whose_for(a, host, "whose mods?")?;
    let sub = fetch(net, host, &game, &me)?;
    let dir = workshop::game_dir(&elo_depot::dirs::mods_root(), &game)?;
    if json_out {
        let mut out = list_json(&sub);
        out["dir"] = json!(dir.display().to_string());
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(0);
    }
    for line in render(&sub, &dir) {
        println!("{line}");
    }
    Ok(0)
}

/// The list as terminal lines. Every string the origin wrote goes through
/// `safe_line`: a mod's author chooses its name, and a name is not allowed to
/// drive the terminal it is printed on.
pub fn render(sub: &Subscribed, dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let n = sub.mods.len();
    out.push(format!(
        "{} · {} · {n} mod{}",
        sub.game,
        safe_line(&roster::short(&sub.wallet)),
        if n == 1 { "" } else { "s" }
    ));
    if let Some(why) = &sub.why {
        out.push(format!("  {}", safe_line(why)));
    } else if !sub.open {
        out.push("  this workshop is closed to new subscriptions".into());
    }
    if n == 0 && sub.why.is_none() {
        out.push(format!("  nothing subscribed — elo mods subscribe {} <mod>", sub.game));
    }
    for m in &sub.mods {
        let bytes: i64 = m.files.iter().map(|f| f.size).sum();
        out.push(format!(
            "  {:<20} {} {}{}",
            clip(&m.id, 20),
            safe_line(&m.name),
            safe_line(&m.version),
            if m.kind.is_empty() { String::new() } else { format!(" · {}", safe_line(&m.kind)) }
        ));
        out.push(format!(
            "  {:<20} {} file{}, {} · github.com/{}/{}",
            "",
            m.files.len(),
            if m.files.len() == 1 { "" } else { "s" },
            human(bytes),
            m.owner,
            m.repo
        ));
    }
    out.extend(needs(sub));
    for (head, rows) in [("held back", &sub.withheld), ("not taken by this launcher", &sub.refused)] {
        if rows.is_empty() {
            continue;
        }
        out.push(String::new());
        out.push(head.into());
        for (id, why) in rows {
            out.push(format!("  {:<20} {}", clip(&safe_line(id), 20), safe_line(why)));
        }
    }
    if let Some(w) = &sub.warning {
        out.push(String::new());
        out.push(format!("⚠ {}", safe_line(w)));
    }
    if n > 0 {
        out.push(String::new());
        out.push(format!("`elo mods sync {}` puts them in {}", sub.game, dir.display()));
    }
    out
}

/// A mod that needs one this wallet is not subscribed to, said once per pair.
fn needs(sub: &Subscribed) -> Vec<String> {
    let have: Vec<&str> = sub.mods.iter().map(|m| m.id.as_str()).collect();
    sub.mods
        .iter()
        .flat_map(|m| {
            m.requires
                .iter()
                .filter(|r| !have.contains(&r.as_str()))
                .map(move |r| format!("  {} needs {r}, which you are not subscribed to", m.id))
        })
        .collect()
}

/// The list as JSON, rebuilt from what parsed — every origin string through
/// `safe_line`, the way `elo friends --json` does it.
fn list_json(sub: &Subscribed) -> Value {
    let pairs = |rows: &[(String, String)]| {
        rows.iter()
            .map(|(id, why)| json!({"mod": safe_line(id), "why": safe_line(why)}))
            .collect::<Vec<_>>()
    };
    json!({
        "game": sub.game, "wallet": safe_line(&sub.wallet), "open": sub.open,
        "runs_code": sub.runs_code,
        "warning": sub.warning.as_deref().map(safe_line),
        "why": sub.why.as_deref().map(safe_line),
        "count": sub.mods.len(),
        "mods": sub.mods.iter().map(|m| json!({
            "id": m.id, "name": safe_line(&m.name), "kind": safe_line(&m.kind),
            "version": safe_line(&m.version), "requires": m.requires,
            "author": {"owner": m.owner, "repo": m.repo},
            "license": m.license.as_deref().map(safe_line),
            "files": m.files.iter().map(|f| json!({
                "name": f.name, "url": f.url, "sha256": f.sha256, "size": f.size,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "withheld": pairs(&sub.withheld),
        "refused": pairs(&sub.refused),
    })
}

fn sync(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let game = game_at(a, 1)?;
    let me = whose_for(a, host, "whose mods?")?;
    let sub = fetch(net, host, &game, &me)?;
    let dry = a.has("--dry-run");
    // stderr, so a `--json` caller's stdout stays one document.
    let mut tell = |m: &workshop::Mod, f: &workshop::ModFile| {
        eprintln!("  fetching {}/{} ({})", m.id, f.name, human(f.size));
    };
    let done = workshop::sync(&sub, &elo_depot::dirs::mods_root(), net, dry, &mut tell)?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&sync_json(&sub, &done)).unwrap_or_default());
    } else {
        for line in sync_lines(&sub, &done) {
            println!("{line}");
        }
    }
    let bad = done.bad();
    if bad > 0 {
        eprintln!(
            "elo: {bad} file{} did not check out — nothing unchecked was kept",
            if bad == 1 { "" } else { "s" }
        );
        return Ok(1);
    }
    Ok(0)
}

/// What a sync did, a line a file.
pub fn sync_lines(sub: &Subscribed, done: &Synced) -> Vec<String> {
    let mut out = Vec::new();
    let n = done.mods.len();
    out.push(format!(
        "{} · {n} mod{} → {}",
        sub.game,
        if n == 1 { "" } else { "s" },
        done.dir.display()
    ));
    for (m, d) in sub.mods.iter().zip(&done.mods) {
        out.push(format!("  {} {}", m.id, safe_line(&m.version)));
        for f in &d.files {
            let said = match &f.outcome {
                Outcome::Kept => "already here".to_string(),
                Outcome::Fetched => format!("downloaded, {}, sha256 checked", human(f.size)),
                Outcome::WouldFetch => format!("would download, {}", human(f.size)),
                Outcome::Mismatch(why) => format!("NOT KEPT — {}", safe_line(why)),
                Outcome::Failed(why) => format!("not downloaded — {}", safe_line(why)),
            };
            out.push(format!("    {:<24} {said}", clip(&f.name, 24)));
        }
    }
    out.extend(needs(sub));
    let removing = if done.dry_run { "would remove" } else { "removed" };
    for id in &done.removed {
        out.push(format!("  {removing} {id} — no longer subscribed"));
    }
    for (head, rows) in [("held back", &sub.withheld), ("not taken by this launcher", &sub.refused)] {
        for (id, why) in rows {
            out.push(format!("  {head}: {} — {}", safe_line(id), safe_line(why)));
        }
    }
    if let Some(w) = &sub.warning {
        out.push(format!("⚠ {}", safe_line(w)));
    }
    if done.dry_run {
        out.push("nothing was downloaded or removed — drop --dry-run to do it".into());
    } else {
        out.push(format!(
            "a game loads them from {} — {} lists the ones that checked",
            done.dir.display(),
            workshop::INDEX_NAME
        ));
    }
    out
}

fn sync_json(sub: &Subscribed, done: &Synced) -> Value {
    let outcome = |o: &Outcome| -> (&str, Option<String>) {
        match o {
            Outcome::Kept => ("kept", None),
            Outcome::Fetched => ("fetched", None),
            Outcome::WouldFetch => ("would_fetch", None),
            Outcome::Mismatch(why) => ("mismatch", Some(safe_line(why))),
            Outcome::Failed(why) => ("failed", Some(safe_line(why))),
        }
    };
    json!({
        "game": sub.game, "wallet": safe_line(&sub.wallet), "dry_run": done.dry_run,
        "dir": done.dir.display().to_string(),
        "index": (!done.dry_run).then(|| done.index.display().to_string()),
        "mods": done.mods.iter().map(|d| json!({
            "id": d.id, "dir": d.dir.display().to_string(), "complete": d.complete(),
            "files": d.files.iter().map(|f| {
                let (state, why) = outcome(&f.outcome);
                json!({"name": f.name, "path": f.path.display().to_string(), "size": f.size,
                       "outcome": state, "why": why})
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "removed": done.removed,
        "withheld": sub.withheld.iter().map(|(id, why)| json!({"mod": safe_line(id), "why": safe_line(why)}))
            .collect::<Vec<_>>(),
        "refused": sub.refused.iter().map(|(id, why)| json!({"mod": safe_line(id), "why": safe_line(why)}))
            .collect::<Vec<_>>(),
        "warning": sub.warning.as_deref().map(safe_line),
        "bad": done.bad(),
    })
}

/// The detail a subscription signs: compact JSON, keys sorted — the bytes
/// `meter/workshop.detail_of` writes. By hand, as `elo_net::friends::detail`
/// is, because every part is checked ASCII (an op, a slug, a mod id, an
/// integer) and there is nothing to escape.
pub fn detail(op: &str, game: &str, id: &str, at: u64) -> String {
    format!(r#"{{"at":{at},"game":"{game}","mod":"{id}","op":"{op}"}}"#)
}

/// The exact text a subscription signs: `meter/playauth.play_message` with
/// action `workshop`, the day keyed to the signed `at`. `None` for an `at` no
/// clock can hold.
pub fn sign_text(me: &str, op: &str, game: &str, id: &str, at: u64) -> Option<String> {
    let day = roster::day(i64::try_from(at).ok()?);
    Some(format!(
        "elo play\naction: workshop\nwallet: {me}\nday: {day}\ndetail: {}",
        detail(op, game, id, at)
    ))
}

/// prepare → sign → submit, for one subscription.
fn change(net: &Net, a: &Args, host: &str, op: &str, json_out: bool) -> Result<i32, DepotError> {
    let game = game_at(a, 1)?;
    let typed = a
        .positional
        .get(2)
        .ok_or_else(|| DepotError::new(format!("{op} to which mod? — its id, from `elo mods {game}` or the game's workshop")))?;
    let id = typed.trim().to_ascii_lowercase();
    if !workshop::is_mod_id(&id) {
        return Err(DepotError::new(format!(
            "{} is not a mod id — lowercase letters, digits and '-'",
            safe_line(typed)
        )));
    }

    // Made as the wallet that signs, never as a typed one; a `--wallet`
    // naming someone else is refused rather than quietly overridden.
    let mut signer = signer::choose(a, host).map_err(DepotError::new)?;
    let me = signer
        .address()
        .filter(|w| roster::is_wallet(w))
        .ok_or_else(|| DepotError::new("the signer names no address"))?
        .trim()
        .to_ascii_lowercase();
    if let Some(w) = a.flag("--wallet") {
        if w.trim().to_ascii_lowercase() != me {
            return Err(DepotError::new(format!(
                "--wallet names {}, and this machine signs as {me} — drop --wallet",
                safe_line(w)
            )));
        }
    }

    let prepared = told(net.post_json_told(
        &format!("{host}/api/workshop/prepare"),
        &json!({"wallet": me, "op": op, "game": game, "mod": id}),
    ))?;
    let offered = prepared
        .get("sign_this")
        .and_then(Value::as_str)
        .ok_or_else(|| DepotError::new("the origin returned no text to sign"))?;
    let at = prepared
        .get("at")
        .and_then(Value::as_u64)
        .ok_or_else(|| DepotError::new("the origin returned no `at` to sign"))?;
    // What gets signed is what was typed: rebuilt here from the op, game and
    // mod and the origin's `at`, and refused before the signer is asked if
    // the origin offered anything else.
    let message = sign_text(&me, op, &game, &id, at)
        .filter(|m| m == offered)
        .ok_or_else(|| {
            DepotError::new(
                "the origin prepared a different change from the one asked for — nothing was signed",
            )
        })?;
    if a.has("--dry-run") {
        if json_out {
            let out = json!({"op": op, "game": game, "mod": id, "at": at, "sign_this": safe_text(&message)});
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        } else {
            println!("would sign:\n{}", safe_text(&message));
            println!("\nnothing was sent — drop --dry-run to do it");
        }
        return Ok(0);
    }

    let out = signer
        .sign(&message, &format!("workshop: {op} {game}/{id}"))
        .map_err(|r| DepotError::new(r.reason))?;
    let signature = out
        .get("signature")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| DepotError::new("the signer returned no signature"))?;
    let done = told(net.post_json_told(
        &format!("{host}/api/workshop/act"),
        &json!({"wallet": me, "op": op, "game": game, "mod": id, "at": at, "signature": signature}),
    ))?;
    let subscribed: Vec<String> = done
        .get("subscribed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|m| workshop::is_mod_id(m))
        .map(str::to_string)
        .collect();
    if json_out {
        let out = json!({"ok": true, "op": op, "game": game, "mod": id, "subscribed": subscribed});
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(0);
    }
    if op == "subscribe" {
        println!("subscribed to {id} in {game} — `elo mods sync {game}` downloads it");
    } else {
        println!("unsubscribed from {id} in {game} — `elo mods sync {game}` removes its files");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args;
    use std::path::PathBuf;

    const ME: &str = "0x00000000000000000000000000000000000000aa";

    fn argv(parts: &[&str]) -> args::Args {
        args::parse(&parts.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    fn sub(mods: Value) -> Subscribed {
        workshop::parse_subscribed(&json!({
            "game": "gates", "wallet": ME, "open": true, "runs_code": true,
            "mods": mods, "withheld": [{"mod": "old-run", "why": "taken down"}],
            "warning": "this mod runs code in the game; the game runs it, not us, and nobody reviewed it",
        }))
        .unwrap()
    }

    fn desert(name: &str) -> Value {
        json!({"id": "desert-run", "name": name, "kind": "map", "version": "1.2.0",
               "requires": ["base-pack"], "author": {"owner": "ash", "repo": "desert-run"},
               "files": [{"name": "map.pak",
                          "url": "https://github.com/ash/desert-run/releases/download/v1.2.0/map.pak",
                          "sha256": "ab".repeat(32), "size": 2048}]})
    }

    /// The bytes `meter/playauth.play_message("workshop", me, detail_of(op,
    /// game, mod, at), day_of(at))` returns for these inputs.
    #[test]
    fn the_text_signed_is_the_meters_text() {
        assert_eq!(
            sign_text(ME, "subscribe", "gates", "desert-run", 1_760_000_000).unwrap(),
            "elo play\naction: workshop\nwallet: 0x00000000000000000000000000000000000000aa\n\
             day: 2025-10-09\ndetail: {\"at\":1760000000,\"game\":\"gates\",\"mod\":\"desert-run\",\
             \"op\":\"subscribe\"}"
        );
        assert!(sign_text(ME, "unsubscribe", "gates", "x", 86_400).unwrap().contains("\nday: 1970-01-02\n"));
        assert_eq!(sign_text(ME, "subscribe", "gates", "x", u64::MAX), None);
    }

    #[test]
    fn a_game_and_a_mod_are_held_to_their_alphabets() {
        assert_eq!(game_at(&argv(&["mods", "Gates"]), 0).unwrap(), "gates");
        assert_eq!(game_at(&argv(&["mods", "sync", "gates"]), 1).unwrap(), "gates");
        for bad in ["../x", "gates/../../etc", "ga tes", "", "-x"] {
            assert!(game_at(&argv(&["mods", "sync", bad]), 1).is_err(), "{bad:?}");
        }
        assert!(game_at(&argv(&["mods", "sync"]), 1).is_err());
    }

    #[test]
    fn the_list_says_what_is_subscribed_held_back_and_why() {
        let lines = render(&sub(json!([desert("Desert Run")])), &PathBuf::from("/m/gates")).join("\n");
        assert!(lines.contains("gates · 0x0000…00aa · 1 mod\n"), "{lines}");
        assert!(lines.contains("desert-run") && lines.contains("Desert Run 1.2.0 · map"), "{lines}");
        assert!(lines.contains("1 file, 2.0 KB · github.com/ash/desert-run"), "{lines}");
        assert!(lines.contains("desert-run needs base-pack"), "{lines}");
        assert!(lines.contains("held back") && lines.contains("taken down"), "{lines}");
        assert!(lines.contains("runs code in the game"), "{lines}");
        assert!(lines.contains("`elo mods sync gates` puts them in /m/gates"), "{lines}");

        let empty = render(&sub(json!([])), &PathBuf::from("/m/gates")).join("\n");
        assert!(empty.contains("0 mods") && empty.contains("elo mods subscribe gates"), "{empty}");
    }

    #[test]
    fn a_mods_name_cannot_drive_the_terminal() {
        let hostile = sub(json!([desert("Desert\x1b[2J\x1b]52;c;aGk=\x07\r\nforged")]));
        for line in render(&hostile, &PathBuf::from("/m")) {
            assert!(!line.contains('\x1b') && !line.contains('\r') && !line.contains('\x07'),
                    "{line:?}");
        }
        let v = list_json(&hostile).to_string();
        assert!(!v.contains("\\u001b"), "{v}");
    }
}
