//! `elo voice` — party voice, opened in the browser.
//!
//! The origin half is `meter/voicechat.py`: tickets into the LiveKit server we
//! run, for a party (a lobby), a guild, or a game's open room. The voice panel
//! is the site's (`watchtower/js/voice.js`), so this verb finds the right page
//! and opens it. The client links no WebRTC stack of its own: LiveKit's Rust
//! SDK brings libwebrtc, tens of megabytes of C++ that `deny.toml`'s policy
//! would have to make room for, and a browser already carries one. Your audio
//! goes through our voice server either way, never straight to another player.

use crate::args::Args;
use elo_depot::DepotError;
use elo_net::Net;
use serde_json::{json, Value};

pub const USAGE: &str = "\
elo voice — party voice: your party, your guild, or a game's open room

  elo voice                       whether voice is on, and the rooms
  elo voice <game>                the game's page: start or join a party,
                                  and its open room for finding people
  elo voice party <id or link>    your party's page, with its voice
  elo voice guild <TAG>           your guild's page, with its voice

  --json           machine-readable output

The page opens in your browser, which asks for the microphone when you join.
";

/// Where a target's voice lives on the site: the page to open.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Game(String),
    Party { slug: String, id: String },
    Guild(String),
}

fn slug_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
}

fn tag_ok(s: &str) -> bool {
    (2..=5).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// A party's id from a bare id or a party link (`…/games/<slug>/?party=<id>`).
pub fn party_id(raw: &str) -> Option<String> {
    let id = match raw.split_once("party=") {
        Some((_, rest)) => rest.split(['&', '#']).next().unwrap_or(""),
        None => raw.trim(),
    };
    let ok = (16..=32).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then(|| id.to_string())
}

/// The page for a target. `host` is the origin, with no trailing slash.
pub fn page_for(host: &str, t: &Target) -> String {
    match t {
        Target::Game(slug) => format!("{host}/games/{slug}/"),
        Target::Party { slug, id } => format!("{host}/games/{slug}/?party={id}"),
        Target::Guild(tag) => format!("{host}/g/{tag}"),
    }
}

fn card(net: &Net, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let got = net.get_json_told(&format!("{host}/api/voice"));
    let Some(v) = got.value.filter(|_| got.ok) else {
        return Err(DepotError::new(format!("couldn't read the voice card: {}", got.why)));
    };
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(0);
    }
    if v.get("on").and_then(Value::as_bool) == Some(true) {
        println!("voice is on.");
    } else {
        println!("voice isn't on yet.");
    }
    if let Some(rooms) = v.get("rooms").and_then(Value::as_object) {
        for (room, what) in rooms {
            println!("  {room:<22} {}", what.as_str().unwrap_or(""));
        }
    }
    println!("\n`elo voice <game>` opens a game's page; `elo voice help` for the rest.");
    Ok(0)
}

fn open(target: &Target, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let url = page_for(host, target);
    if json_out {
        println!("{}", json!({ "url": url }));
    } else {
        println!("  {url}");
    }
    elo_depot::launch::open_page(&url)
        .map_err(|e| DepotError::new(format!("couldn't open a browser: {e}")))?;
    Ok(0)
}

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let net = Net::new();
    let host = host.trim_end_matches('/');
    match a.positional.first().map(String::as_str) {
        None => card(&net, host, json_out),
        Some("help") => {
            print!("{USAGE}");
            Ok(0)
        }
        Some("party") => {
            let raw = a.positional.get(1).ok_or_else(|| DepotError::new("which party? — `elo voice party <id or link>`"))?;
            let id = party_id(raw).ok_or_else(|| DepotError::new("that isn't a party id or link"))?;
            let got = net.get_json_told(&format!("{host}/api/lobbies/{id}"));
            let lob = got.value.filter(|_| got.ok).ok_or_else(|| DepotError::new(got.why))?;
            let slug = lob.get("game").and_then(Value::as_str).unwrap_or_default().to_string();
            if !slug_ok(&slug) {
                return Err(DepotError::new("the origin named no game for that party"));
            }
            open(&Target::Party { slug, id }, host, json_out)
        }
        Some("guild") => {
            let tag = a.positional.get(1).map(|t| t.to_ascii_uppercase()).unwrap_or_default();
            if !tag_ok(&tag) {
                return Err(DepotError::new("a guild tag is 2 to 5 letters or digits — `elo voice guild <TAG>`"));
            }
            open(&Target::Guild(tag), host, json_out)
        }
        Some(game) => {
            let slug = game.to_ascii_lowercase();
            if !slug_ok(&slug) {
                return Err(DepotError::new(format!("{game:?} isn't a game's name on the store, like gates")));
            }
            open(&Target::Game(slug), host, json_out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages() {
        let h = "https://elopros.com";
        assert_eq!(page_for(h, &Target::Game("gates".into())), "https://elopros.com/games/gates/");
        assert_eq!(page_for(h, &Target::Guild("WOLF".into())), "https://elopros.com/g/WOLF");
        assert_eq!(
            page_for(h, &Target::Party { slug: "gates".into(), id: "abcdefghijklmnop".into() }),
            "https://elopros.com/games/gates/?party=abcdefghijklmnop"
        );
    }

    #[test]
    fn party_ids() {
        assert_eq!(party_id("abcdefghijklmnop").as_deref(), Some("abcdefghijklmnop"));
        assert_eq!(
            party_id("https://elopros.com/games/gates/?party=Ab_cd-efghijklmn0&x=1").as_deref(),
            Some("Ab_cd-efghijklmn0")
        );
        assert_eq!(party_id("short"), None);
        assert_eq!(party_id("abcdefghijklmnop'; rm"), None);
    }

    #[test]
    fn names() {
        assert!(slug_ok("gates") && slug_ok("before-colony"));
        assert!(!slug_ok("Gates") && !slug_ok("-x") && !slug_ok("a/b"));
        assert!(tag_ok("WOLF") && !tag_ok("W") && !tag_ok("wolf"));
    }
}
