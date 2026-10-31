//! `elo` — the desktop client's command half.
//!
//! Everything the GUI will do to a game, this does first and without a
//! display: install, verify, update, prune, launch. That ordering is not an
//! accident. A launcher whose only interface is a window cannot be tested on a
//! build machine, scripted by a player, or driven by an agent — and this
//! platform has agents as first-class players (`GATES.md` §0).

mod agent;
mod args;
mod assets;
mod credits;
mod chatkey;
mod dev;
mod friends;
mod mcp;
mod mods;
mod prompt;
mod relay;
mod send;
mod signer;
mod skins;
mod stage;
mod stage_chat;
mod stage_cli;
mod stage_ears;
mod stage_live;
mod stage_mcp;
mod stage_sources;
mod stage_voice;
mod stage_youtube;
mod trade;
mod voice;

// The account's address and signature both come through this trait, so the CLI
// and the window use one implementation rather than two that agree today.
use elo_broker::signer::Signer;
use elo_depot::session::{self, Ending, Session};
use elo_depot::publisher;
use elo_depot::dirs::{home_based, state_root};
use elo_depot::update::{Checked, UpdateState};
use elo_depot::{
    install, installed, launch, manifest, parse_depot, plan, superseded, uninstall, verify,
    Depot, DepotError, Manifest, Plan,
};
use elo_hive::relay::{note_for, Presence, Relay, Status, HEARTBEAT};
use elo_hive::voice::Voice;
use elo_hive::who::{self, NameKind};
use elo_hive::{clock, render, Hive};
use elo_net::Net;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
elo — the desktop client

  elo check                       report whether this machine can run a build
  elo games                       every title the origin serves a manifest for
  elo list                        every installed build, from its receipt
  elo status  <title>             is a newer build published?
  elo install <title>             install the native build for this machine
  elo update  <title>             install it only if it is newer
              [--branch <name>]   …from a beta branch the title publishes
  elo entitle <title>             a sold title's download grant: sign once with
                                   the wallet that holds a copy, kept for a day
  elo verify  <slug> [build]      re-hash an install against its receipt
  elo prune   <slug>              remove builds superseded by the newest
  elo uninstall <slug> <build>    remove one build
  elo play    <title>             start the installed build (or open the page)
  elo streamer [on|off]           streamer mode: games start with it on, and
                                   the window hides wallets and server addresses
  elo gamemode [on|off]           game mode: GameMode on Linux, above-normal
                                   priority on Windows, for every game started
  elo join    <link>              follow a friend's elo://join/… link
  elo servers <title>             the shards that title is serving, live
  elo digest  <depot>             the bytes32 a notary would commit
  elo chat [rooms|read|voice]     the Elo Pros chat rooms, read
  elo chat whoami                 the chat key this machine speaks with
  elo chat voice-new              make one (schnorr, kept here, never sent)
  elo chat link                   speak with your wallet's own chat key, so
                                   rooms show your profile name and picture
  elo chat unlink                 undo that — the key stays on this machine
  elo chat say <text>             say something in #general or --room
  elo chat presence               show as online until interrupted
                                  (`elo hive …` still works)
  elo who [handle]                one identity — name, face, and who checked
  elo friends                     your friends, and what each one is playing
  elo friends add <who>           ask a wallet (0x…) or a name to be friends
  elo friends accept|decline <who>    answer an ask
  elo friends cancel|remove|block|unblock <who>
  elo friends join <who>          join the server a friend is playing on
  elo friends invite <who>        invite a friend to the game you are in
  elo mods <game>                 the Workshop mods you subscribed to for a game
  elo mods sync <game>            download them, check each file's sha256, and
                                   remove the ones you dropped
  elo mods subscribe|unsubscribe <game> <mod>
  elo assets                      the asset store, in the game you are making:
                                   search, add, sync, update, credits — every
                                   file's sha256 checked, pinned in a lock
  elo assets init|check|publish   list your own: elo-asset.json from your
                                   files, checked, then listed (`elo assets help`)
  elo credits [buy ten|fifty]     your dev credits, and a pack paid in USDG
                                   from this wallet; they pay for hosting,
                                   Claude builds, buddies and the relay
  elo seat [--wallet ADDR]        the characters a wallet holds, and whether
                                   it may still mint — one call, no key
  elo seat card                   the collection: contract, supply, doors
  elo sessions                    what has been played, and what is running
  elo account                     the account on this machine (exit 3 if none)
  elo signin                      sign this wallet in to the store, no browser
  elo signin <code>               sign a browser tab in, with the code it shows
  elo profile                     your name and face; with --name, --face <png>,
                                   --kind agent|human, --harness, --model: change it
  elo agent init --name <name>    an agent from nothing: this machine's own key
                [--face <png>]     (made, words to --words-file, if there is
                [--words-file <p>] none), signed in, named, a face, chat linked
  elo account new                 make one — twelve recovery words, shown once;
                                   one address on Robinhood Chain, one on Solana
  elo account import              restore from recovery words, or adopt a key
  elo account solana              give an older account its Solana address,
                                   from its recovery words
  elo wallet [--wallet ADDR]      what this account holds on Robinhood Chain
                                   and Solana, read off each chain
  elo send <amount> <coin> <to>   send ETH, SOL or a coin: the address picks the
                                   chain; checked, shown, then the passphrase
  elo move <amount> <coin>        SOL/USDC to Robinhood Chain, ETH/USDG to
                                   Solana — to your own address there
  elo trade [pairs]               every market: price and 24h change
  elo trade quote|buy|sell|swap   quote or trade through the pools, signed here
  elo trade watch [pair…]         new trades as they happen (--wallet to follow)
  elo trade orders [add|run]      resting orders kept on this machine: limit,
                                   stops, trailing, brackets, DCA, TWAP; they
                                   fire only while `orders run` is running
  elo trade settings | log        signing settings, and what was signed
  elo skins                       your items in our collections, and listings
  elo skins sell <id> <price>     list one on OpenSea; it stays yours, and in
                                   game, until it sells (cancel|buy|accept|open)
  elo skins market                the floor and the best listings
  elo voice <game>                party voice: start or join a party, or talk
                                   in the game's open room (opens the page;
                                   `elo voice party|guild …`)
  elo account sign <message>      sign with it (EIP-191), for a game or a claim
  elo pair                        sign with the wallet in your BROWSER instead
                                   — prints a code to type at <host>/pair.html
  elo pair show                   which wallet this machine can reach
  elo pair forget                 drop the pairing
  elo signin  <code>              sign the website in with this account — type
                                   the code the browser shows, never a link
  elo publisher <title>           who signed this title's manifest, if anyone
  elo publisher <title> --statement   the exact bytes a publisher signs
  elo dev                         where this account stands on the store's desk
  elo dev edit <slug> --set k=v   edit your own listing's row
  elo dev image <slug> <file>     upload art for it, no commit needed
  elo dev post <slug> --title T   post a community update
  elo dev delegate <slug> <addr>  let your agent act for you, scoped
  elo dev help                    the whole desk
  elo prove <game> <server> <nonce>   sign in to a game server (SIWE, EIP-4361)
  elo mcp                         serve this machine's reads over MCP on stdio
  elo stage                       an agent's body on your screen, for OBS: a
                                   VRM or pictures, captions and a voice, driven
                                   by your agent (`elo stage mcp`) or by hand

<title> is a SLUG (`gates`), a path to a manifest, or an https url. A slug is
read from the origin at /api/launcher/manifest/<slug>; a local file wins over
the origin, so a title can be tested from a checkout. <depot> is a path or url.

  --games PATH        install root (default ~/.local/share/elo/games)
  --host URL          the origin to read (default https://elopros.com)
  --room NAME         which chat room, by name (gates) or id; omit for #general
  --limit N           how many messages
  --detach            start a game without watching it — no playtime recorded
  --status ONLINE     online | away, for chat presence
  --quiet             do not show as online in chat while playing
  --platform TAG      override this machine's platform tag
  --server ADDR       passed to the game as {server}
  --wallet ADDR       passed to the game as {wallet}; whose list, for `friends`
                      and `mods`
  --browser CMD       browser command; `{url}` marks where the url goes,
                      which is what app mode needs: 'chromium --app={url}'
  --json              machine-readable output
  --no-watch          after `signin`, exit instead of staying to approve
                      the page's signature requests
  --local             sign with the account on this machine, even when a
                      browser pairing is live (ELO_SIGNER=local: always)
  --no-reuse          download every file, even ones already on disk
  --allow-insecure    permit http manifests and depots (a local test server)
  --dry-run           say what would happen, do nothing
  --yes               replace a chat key, or link one, without asking
";

fn main() {
    // Before any key can be in memory: no core file, no same-user attach.
    for e in elo_vault::harden_process() {
        eprintln!("elo: {e}");
    }
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args::parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("elo: {}", shown(&e));
            std::process::exit(2);
        }
    };
    if parsed.verb.is_empty() || parsed.verb == "help" || parsed.verb == "-h"
        || parsed.has("--help")
    {
        print!("{USAGE}");
        return;
    }
    let parsed = args::normalize_link(parsed);
    match run(&parsed) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("elo: {}", shown(&e));
            std::process::exit(1);
        }
    }
}

/// The CLI's error sink, made fit for a terminal.
///
/// An error can carry text somebody else chose — a relay's refusal reason, an
/// origin's message, an `elo://` link a web page built — and an escape in it
/// is an instruction to the player's terminal (OSC 52 writes the clipboard).
/// The relay cleans its own words where they enter an error; this is the
/// backstop for every path that does not, line by line so a multi-line error
/// keeps its shape.
fn shown(e: &dyn std::fmt::Display) -> String {
    e.to_string().lines().map(render::safe_line).collect::<Vec<_>>().join("\n")
}

fn run(a: &args::Args) -> Result<i32, DepotError> {
    let games = games_root(a);
    let platform = a
        .flag("--platform")
        .map(str::to_string)
        .unwrap_or_else(launch::platform_tag);
    let insecure = a.has("--allow-insecure");
    let host = a
        .flag("--host")
        .unwrap_or("https://elopros.com")
        .trim_end_matches('/')
        .to_string();
    let json_out = a.has("--json");
    // Every game this run starts is told (`elo_depot::streamer`).
    elo_depot::streamer::load(&state_root());
    elo_depot::gamemode::load(&state_root());

    match a.verb.as_str() {
        "streamer" => {
            match a.positional.first().map(String::as_str) {
                Some("on") => elo_depot::streamer::save(&state_root(), true)?,
                Some("off") => elo_depot::streamer::save(&state_root(), false)?,
                Some(other) => {
                    return Err(DepotError::new(format!("streamer: on or off, not {other:?}")))
                }
                None => {}
            }
            let on = elo_depot::streamer::on();
            if json_out {
                println!("{}", serde_json::json!({ "streamer": on }));
            } else if on {
                println!("streamer mode is on: games start with it on, and the window hides \
                          wallets and server addresses");
            } else {
                println!("streamer mode is off (`elo streamer on` to turn it on)");
            }
            Ok(0)
        }
        "gamemode" => {
            match a.positional.first().map(String::as_str) {
                Some("on") => elo_depot::gamemode::save(&state_root(), true)?,
                Some("off") => elo_depot::gamemode::save(&state_root(), false)?,
                Some(other) => {
                    return Err(DepotError::new(format!("gamemode: on or off, not {other:?}")))
                }
                None => {}
            }
            let on = elo_depot::gamemode::on();
            let effect = elo_depot::gamemode::effect();
            if json_out {
                let runner = match &effect {
                    elo_depot::gamemode::Effect::GameMode(p) => Some(p.display().to_string()),
                    _ => None,
                };
                println!("{}", serde_json::json!({
                    "gamemode": on, "does": effect.describe(), "runner": runner
                }));
            } else if on {
                println!("game mode is on: {}", effect.describe());
            } else {
                println!("game mode is off (`elo gamemode on` to turn it on)");
                println!("  turned on here: {}", effect.describe());
            }
            Ok(0)
        }
        "check" => {
            let net = Net::new();
            println!("elo launcher {}", env!("CARGO_PKG_VERSION"));
            println!("platform    {platform}");
            println!("games       {}", games.display());
            println!("installs    {}", installed(&games).len());
            // ⚠ This probed a HARD-CODED origin while every other verb honours
            // `--host`, so `elo check --host http://localhost:3600` reported on
            // production and looked healthy. A check that reads a different
            // machine than the one you asked about is worse than no check.
            let probe = net.get_json(&format!("{host}/health"));
            println!(
                "origin      {}",
                if probe.reachable { host.as_str() } else { &probe.why }
            );
            // ── is this client current? ──────────────────────────────────────
            //
            // A NOTICE, not a self-update, and the distinction is deliberate.
            // A binary that replaces itself is the one part of this client that
            // would have to be trusted absolutely — it would need its own
            // signature policy, its own rollback, and its own answer for what
            // happens when it dies mid-write. The package manager already owns
            // that job on Linux and does it better. So this says what it knows
            // and lets the holder act.
            //
            // Three answers, never two: current, a version differs, or **we
            // could not look** — which is the trap this crate exists around and
            // must never print as "you are up to date".
            let card = net.get_json(&format!("{host}/api/launcher"));
            let published = card
                .value
                .as_ref()
                .and_then(|v| v.get("client"))
                .and_then(|c| c.get("version"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let mine = env!("CARGO_PKG_VERSION");
            match (&published, card.reachable) {
                (Some(there), _) => match elo_depot::update::version_cmp(there, mine) {
                    std::cmp::Ordering::Equal => println!("client      {mine} — current"),
                    std::cmp::Ordering::Greater => {
                        println!("client      {mine} — {there} is published");
                        // The install line, not just the fact. Whoever reads
                        // this is one command away from acting on it.
                        println!("            {host}/download.html");
                    }
                    // A local build ahead of the shelf. Saying "up to date"
                    // here would be wrong in the other direction, and nagging
                    // a developer to downgrade would be worse.
                    std::cmp::Ordering::Less => {
                        println!("client      {mine} — ahead of the origin's {there}")
                    }
                },
                (None, true) => println!("client      {mine} — this origin publishes no client"),
                (None, false) => println!("client      {mine} — could not ask: {}", card.why),
            }
            // The account, on the health card — "how do I make a key" should
            // be answerable from the one command that reports everything
            // else, not from a doc. Reading it costs no passphrase: `at`
            // reads only the public address.
            let ks = elo_vault::keystore_path();
            match elo_vault::LocalSigner::at(&ks).address() {
                Some(addr) => println!("account     {addr}"),
                None => {
                    println!("account     none — `elo account new` makes one \
                              (optional; games play anonymously)");
                }
            }
            // ⚠ Which signer this machine REACHES is a different question from
            // whether it holds an account, and a paired player's answer to the
            // second one is "none" while the first is a working wallet. Two
            // lines, because one would have to lie about one of them.
            match signer::choose(a, &host) {
                Ok(sig) => println!("signer      {}", sig.line()),
                Err(_) => println!("signer      none — `elo pair` reaches the wallet \
                                    in your browser, `elo account new` makes one here"),
            }
            Ok(0)
        }

        // ── the account ──────────────────────────────────────────────────────
        //
        // The headless half of the Account window, and for a while the ONLY
        // half that worked: `elo-vault` could make a key from the day it
        // landed, and no path a user could reach ever asked it to — `create`
        // and `unlock` were called from tests and nowhere else. A client whose
        // only way to hold a key is a desktop window cannot hold one on a
        // server, in CI, or under an agent, which on a platform that calls
        // agents first-class players (`GATES.md` §0) is the wrong half to have
        // shipped first.
        //
        // ⚠ **Nothing in this verb touches the network.** The key is generated
        // here from the OS pool, encrypted here, and written here. elo the
        // service never sees it — `CLAUDE.md` invariant 7 is about custody, and
        // this is the holder's own machine holding the holder's own key.
        "account" => {
            let path = elo_vault::keystore_path();
            match a.positional.first().map(String::as_str).unwrap_or("show") {
                "show" => {
                    let signer = elo_vault::LocalSigner::at(&path);
                    if json_out {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "exists": signer.exists(),
                                "address": signer.address(),
                                "solana": signer.sol_address(),
                                "unreadable": signer.unreadable(),
                                "path": path.to_string_lossy(),
                            }))
                            .unwrap_or_default()
                        );
                        return Ok(if signer.exists() { 0 } else { 3 });
                    }
                    // A paired player has no account and still has a wallet,
                    // so this verb must not answer "none" and stop — that
                    // sends someone who is already set up off to make a key
                    // they do not need.
                    if let Some(p) = elo_net::pairing::load(&elo_vault::pairing_path()) {
                        println!("  a browser pairing is live: {} ({})", p.address, p.host);
                        println!("  `elo pair show` is that door; --local forces this one");
                    }
                    match signer.address() {
                        Some(addr) => {
                            println!("{addr}");
                            match (signer.sol_address(), signer.unreadable()) {
                                (Some(sa), _) => println!("{sa}\n  the same account on Robinhood Chain and on Solana"),
                                (None, Some(why)) => println!("  the Solana half is there and unreadable — {why}"),
                                (None, None) => println!("  no Solana address yet — `elo account solana` adds it"),
                            }
                            println!("  locked — a passphrase unlocks it for one command");
                            println!("  {}", path.display());
                            Ok(0)
                        }
                        // There and unreadable is not "no account": that
                        // sentence sends a player to make a new key over the
                        // one they have. Say what stopped the read instead.
                        None if signer.exists() => Err(DepotError::new(
                            signer.unreadable().unwrap_or("the keystore names no address").to_string(),
                        )),
                        None => {
                            // Not an error. A machine with no account is a
                            // normal machine and games play anonymously; 3 is
                            // this CLI's "unknown/absent", the same code
                            // `elo status` uses, so a script can branch.
                            println!("no account on this machine");
                            println!(
                                "  `elo account new` makes one · \
                                 `elo account import` adopts one you already have"
                            );
                            println!("  it would live at {}", path.display());
                            Ok(3)
                        }
                    }
                }

                // Twelve words first, checked, and only then a file. The words
                // go to stderr so `ADDR=$(elo account new)` captures the
                // address and nothing else, and they are never written by us.
                "new" => {
                    refuse_if_occupied(&path)?;
                    if elo_depot::streamer::on() {
                        return Err(DepotError::new(
                            "streamer mode is on, and the recovery words would be on \
                             stream — `elo streamer off` first",
                        ));
                    }
                    let phrase = elo_vault::Phrase::generate().map_err(DepotError::new)?;
                    show_words(&phrase);
                    let quiz = phrase.quiz().map_err(DepotError::new)?;
                    let answer = prompt::secret(&format!(
                        "to check you have them, type words {}, {} and {}: ",
                        quiz[0], quiz[1], quiz[2]
                    ))
                    .map_err(DepotError::new)?;
                    if !phrase.check(&quiz, &answer) {
                        return Err(DepotError::new(
                            "those are not the words — nothing was written. Run it again \
                             for a new set, and write them down this time",
                        ));
                    }
                    let pass = passphrase_twice()?;
                    let signer = elo_vault::LocalSigner::restore(&path, &pass, &phrase)
                        .map_err(DepotError::new)?;
                    report_new_account(&signer, &path, true);
                    Ok(0)
                }

                // The counterpart to `new`, and the reason both exist: a player
                // with a wallet already should not have to abandon it to use
                // this client.
                "import" => {
                    refuse_if_occupied(&path)?;
                    warn_if_echoing();
                    let raw = prompt::secret("your recovery words, or a private key (64 hex): ")
                        .map_err(DepotError::new)?;
                    let signer = if raw.split_whitespace().count() > 1 {
                        let phrase = elo_vault::Phrase::parse(&raw).map_err(DepotError::new)?;
                        let pass = passphrase_twice()?;
                        elo_vault::LocalSigner::restore(&path, &pass, &phrase)
                    } else {
                        let secret = parse_secret(&raw)?;
                        let pass = passphrase_twice()?;
                        elo_vault::LocalSigner::import(&path, &pass, secret.as_slice())
                    }
                    .map_err(DepotError::new)?;
                    report_new_account(&signer, &path, false);
                    Ok(0)
                }

                // Why this verb is the point of the whole command: an address
                // is a claim, and a signature is the only thing that makes it
                // mean anything. Without this the CLI could hold a key and
                // never use it.
                //
                // ⚠ It goes through `signer::choose` now, so it signs with a
                // paired BROWSER wallet as readily as with the account on this
                // box — the whole point of the seam. One thing changes for a
                // caller: a browser-relayed text must be a platform signable
                // (`elo <family>` first line), because that is the wall that
                // stops this door carrying somebody else's authorisation
                // grammar into a wallet with money in it. The origin says so
                // in words when it refuses, so `--local` is one flag away and
                // the reader is told which flag.
                "sign" => {
                    let message = need(a.positional.get(1), "a message to sign")?;
                    // Any text at all, so ELO_PASSPHRASE_FILE never answers here.
                    let mut signer = signer::choose(a, &host).map_err(DepotError::new)?.always_ask();
                    let out = signer
                        .sign(message, "elo account sign")
                        .map_err(|r| DepotError::new(r.reason))?;
                    if json_out {
                        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
                        return Ok(0);
                    }
                    println!(
                        "{}",
                        out.get("signature").and_then(Value::as_str).unwrap_or("")
                    );
                    println!(
                        "  signed by {}",
                        out.get("address").and_then(Value::as_str).unwrap_or("")
                    );
                    Ok(0)
                }

                // An account made before Solana, or adopted from a bare key,
                // gets its Solana half here: from the recovery words (which
                // must make THIS account's 0x address) or a Solana private key
                // as Phantom exports it. Nothing touches the network.
                "solana" => {
                    let mut signer = elo_vault::LocalSigner::at(&path);
                    if signer.address().is_none() {
                        return Err(DepotError::new("no account on this machine — `elo account new` makes \
                                                    one with both addresses"));
                    }
                    if let Some(sa) = signer.sol_address() {
                        println!("{sa}");
                        println!("  this account already has its Solana address");
                        return Ok(0);
                    }
                    warn_if_echoing();
                    let raw = prompt::secret("your recovery words, or a Solana private key: ")
                        .map_err(DepotError::new)?;
                    let pass = prompt::secret("this account's passphrase: ").map_err(DepotError::new)?;
                    let sa = signer.add_solana(&pass, &raw).map_err(DepotError::new)?;
                    drop(pass);
                    println!("{sa}");
                    println!("  your Solana address, encrypted with the same passphrase");
                    println!("  written to {}", signer.solana_path().display());
                    Ok(0)
                }

                other => Err(DepotError::new(format!(
                    "unknown account command {other:?} — try show, new, import, solana or sign"
                ))),
            }
        }

        // ── the wallet: balances off the chain, and a send ─────────────────
        "wallet" => send::show(a, &host, json_out),
        "send" => send::send(a, &host, json_out),
        "move" => send::move_coins(a, &host, json_out),
        "trade" => trade::run(a, &host, json_out),
        "skins" => skins::run(a, &host, json_out),
        // Workshop mods: the list a wallet subscribed to, and the files on
        // disk, checked. Writing them is `elo_depot::workshop`'s.
        "mods" => mods::run(a, &host, json_out),
        // Party voice: the page for a party, a guild or a game's open room.
        // The site's voice panel does the talking (voice.rs says why).
        "voice" => voice::run(a, &host, json_out),
        // An agent's body on your screen: a hub on 127.0.0.1, the stage OBS
        // loads, and `elo stage mcp` for the agent (stage.rs says why local).
        "stage" => {
            stage::set_origin(&host);
            stage_cli::run(a, &state_root(), json_out)
        }
        // The asset store: files a builder adds to a project, pinned in
        // elo-assets.lock. Writing them is `elo_depot::assets`'s.
        "assets" => assets::run(a, &host, json_out),
        // Dev credits: the balance, and a pack paid from this wallet over the
        // store's HTTP 402 door. The vault signs only a USDG pack (usdg.rs).
        "credits" => credits::run(a, &host, json_out),

        // ── pair: the wallet you already have, in the browser it lives in ───
        //
        // The mirror of `elo signin`, and the case that loses more players at
        // the door. `signin` says "you have a wallet here, but no browser";
        // this says "you have a browser wallet, and no wish for a second key
        // on disk." Same bridge — the origin — because a browser cannot knock
        // on a socket and this process cannot reach into a tab.
        //
        // ⚠ THE CODE IS PRINTED HERE AND TYPED THERE, never clicked. That is
        // `SIGN-IN.md` §1's rule mirrored, and it mirrors without weakening: a
        // code read off your own launcher and typed into your own browser
        // cannot be a stranger's. The phish it forbids is the other one's
        // reflection — a stranger's code, pasted into a chat as a free-mint
        // link, would point YOUR wallet at THEIR launcher's queue. There is no
        // `elo://pair/…` scheme on purpose; read meter/browser_signer.py's
        // header before adding one.
        //
        // What lands on disk afterwards is a code, a secret and an address.
        // None of the three can sign: every signature is asked for over the
        // wire and approved by a human in another window. A launcher paired
        // this way holds no key at all.
        "pair" => {
            let path = elo_vault::pairing_path();
            match a.positional.first().map(String::as_str) {
                Some("show") => {
                    // The one caller that must tell "expired" from "never
                    // paired" — everywhere else an expired pairing reads as
                    // none, so no verb has to remember to check.
                    let Some(p) = elo_net::pairing::load_even_if_expired(&path) else {
                        println!("no browser pairing on this machine.");
                        println!("  `elo pair` starts one; `elo account new` is the other door.");
                        return Ok(3);
                    };
                    let left = p.expires.saturating_sub(elo_net::pairing::now());
                    if json_out {
                        let mut v = p.to_json();
                        if let Some(m) = v.as_object_mut() {
                            // The secret is a bearer credential. It is on disk
                            // because this process needs it; it is not in
                            // stdout, where a pasted terminal log would carry
                            // it into a chat window.
                            m.remove("secret");
                            m.insert("seconds_left".into(), serde_json::json!(left));
                        }
                        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                        return Ok(if left == 0 { 3 } else { 0 });
                    }
                    println!("{}  ({})", p.address, p.host);
                    if left == 0 {
                        println!("  EXPIRED — `elo pair` claims a fresh code");
                        return Ok(3);
                    }
                    println!("  code {} · {}h {}m left", p.show_code(), left / 3600, (left % 3600) / 60);
                    println!("  every signature is still approved in the browser, one at a time");
                    Ok(0)
                }
                Some("forget") => {
                    let dropped = elo_net::pairing::forget(&path).map_err(DepotError::new)?;
                    println!(
                        "{}",
                        if dropped {
                            "forgotten — this machine reaches no browser wallet now"
                        } else {
                            "there was no pairing to forget"
                        }
                    );
                    Ok(0)
                }
                None => {
                    let net = Net::new();
                    let (code, secret, _) =
                        elo_net::pairing::start(&net, &host).map_err(DepotError::new)?;
                    let show = elo_net::pairing::show_code(&code);
                    // With --json, stdout carries exactly one document, so the
                    // instructions and the wait go to stderr, where a person
                    // running it still reads them.
                    let say = |line: &str| if json_out { eprintln!("{line}") } else { println!("{line}") };
                    say("open this in the browser your wallet lives in:");
                    say(&format!("    {host}/pair.html"));
                    say("and TYPE the code:");
                    say(&format!("    {show}"));
                    say("");
                    say("  nobody legitimate will ever send you one of these to enter.");
                    say("  waiting …");
                    let p = elo_net::pairing::await_claim(&net, &host, &code, &secret, || {
                        // A dot per poll, so a terminal shows the wait as a
                        // wait rather than as a hang. On stderr, which is
                        // unbuffered and never part of the --json document.
                        eprint!(".");
                    })
                    .map_err(DepotError::new)?;
                    say("");
                    elo_net::pairing::save(&path, &p).map_err(DepotError::new)?;
                    if json_out {
                        let mut v = p.to_json();
                        if let Some(m) = v.as_object_mut() {
                            // A bearer credential, kept off stdout — see `pair show`.
                            m.remove("secret");
                        }
                        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                        return Ok(0);
                    }
                    println!("paired with {}", p.address);
                    println!("  `elo entitle <game>` and `elo account sign` now ask that browser.");
                    println!("  it approves one message at a time; `elo pair forget` ends it.");
                    Ok(0)
                }
                Some(other) => Err(DepotError::new(format!(
                    "unknown pair command {other:?} — try `elo pair`, `elo pair show` \
                     or `elo pair forget`"
                ))),
            }
        }

        // ── the dev desk ─────────────────────────────────────────────────────
        //
        // The other side of the store from every verb above: those install and
        // play a title, this one SHIPS one. It lives in its own module because
        // it is a desk rather than a verb — prepare, sign, submit, seven times
        // over — and because the origin half (`meter/devdesk.py`) is where the
        // rules actually live, so this file should stay the dispatcher.
        "dev" => dev::run(a, &host, json_out),

        // ── prove, without a game in the middle ──────────────────────────────
        //
        // The same message the broker's `prove` verb composes, signed by the
        // same account, for the case with no window and no game process: an
        // agent playing headless, a CI job, a server. It exists so that side
        // never hand-assembles the string — one implementation
        // (`protocol::prove_message`) or the two drift and a server rejects a
        // signature that is perfectly good over slightly different bytes.
        //
        // ⚠ There is no consent here either, and for a stronger reason: the
        // operator typed this command. The trust boundary `prove` exists for is
        // between a GAME and the launcher, and there is no game here.
        "prove" => {
            let game = need(a.positional.first(), "a game slug")?;
            let server = need(a.positional.get(1), "the server to prove to")?;
            let nonce = need(a.positional.get(2), "the nonce that server issued")?;
            // The same refusals the broker applies, so a message this builds
            // can never contain a line the broker would have rejected.
            elo_broker::protocol::check_slug(game)
                .map_err(|r| DepotError::new(r.reason))?;
            elo_broker::protocol::check_domain(server)
                .map_err(|r| DepotError::new(r.reason))?;
            elo_broker::protocol::check_nonce(nonce)
                .map_err(|r| DepotError::new(r.reason))?;
            // ⚠ SIWE binds the address into the signed bytes, so unlike the old
            // format this cannot be composed before the account is known. The
            // keystore moved ahead of `--statement` for that reason: printing a
            // message with nobody's address in it would print something that can
            // never verify.
            let path = elo_vault::keystore_path();
            let mut signer = elo_vault::LocalSigner::at(&path);
            let (message, out) = if !signer.exists() {
                // A paired browser wallet signs instead, when the server is on
                // the pairing's own site: signed under that site's name, the
                // form a wallet's sign-in check passes and Gates' shards
                // accept (`Pairing::signs_for`). Anywhere else, the reason.
                let Some(p) = elo_net::pairing::load(&elo_vault::pairing_path()) else {
                    return Err(DepotError::new(format!(
                        "no account at {} — `elo account new` makes one, or \
                         `elo pair` uses the wallet in your browser",
                        path.display()
                    )));
                };
                let Some(page) = p.signs_for(server) else {
                    return Err(DepotError::new(format!(
                        "{server} is not on {}, so your browser wallet can't sign \
                         in there — the wallet would warn that it is a different \
                         site. `elo account new` makes an account here that can.",
                        p.site()
                    )));
                };
                let message = elo_broker::protocol::prove_message_for(
                    &page,
                    server,
                    &p.address,
                    game,
                    nonce,
                    &elo_broker::protocol::now_iso8601(),
                );
                if a.has("--statement") {
                    print!("{message}");
                    return Ok(0);
                }
                let mut s = elo_net::pairing::PairedBrowserSigner::new(p).telling(signer::tell);
                let out = s
                    .sign(&message, "sign in to the game server")
                    .map_err(|r| DepotError::new(r.reason))?;
                (message, out)
            } else {
                let address = signer
                    .address()
                    .ok_or_else(|| DepotError::new("the keystore names no address"))?;
                let message = elo_broker::protocol::prove_message(
                    server,
                    &address,
                    game,
                    nonce,
                    &elo_broker::protocol::now_iso8601(),
                );
                if a.has("--statement") {
                    print!("{message}");
                    return Ok(0);
                }
                warn_if_echoing();
                let pass = prompt::passphrase("passphrase: ").map_err(DepotError::new)?;
                signer.unlock(&pass).map_err(DepotError::new)?;
                let out = signer
                    .sign(&message, "prove identity")
                    .map_err(|r| DepotError::new(r.reason))?;
                (message, out)
            };
            if json_out {
                let mut v = out.clone();
                if let Some(m) = v.as_object_mut() {
                    m.insert("message".into(), Value::String(message.clone()));
                }
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(0);
            }
            println!("{}", out.get("signature").and_then(Value::as_str).unwrap_or(""));
            println!("  proving {} to {server}", out.get("address").and_then(Value::as_str).unwrap_or(""));
            Ok(0)
        }

        // ── entitle: the whole ticket dance, one command ─────────────────────
        //
        // Operator, 2026-08-08: "the launcher can make all of this easy we
        // decided you dont have to micromanage signing everytime." One
        // passphrase prompt (the vault's own, same as `account sign` — the
        // operator typed this command, so there is no per-action consent
        // theater), and the grant is cached beside the games for the day, so
        // install and update pick it up with no flags. The exact text to sign
        // comes FROM the origin — one implementation of the message, or the
        // two sides drift and a good signature verifies over the wrong bytes
        // (the prove_message lesson).
        "entitle" => {
            let slug = need(a.positional.first(), "a game slug")?;
            // ⚠ The address is the SIGNER's, not the keystore's, and that is
            // the whole of what changed here: with a browser pairing live this
            // entitles the wallet in the browser, which is the wallet that
            // actually bought the copy. Reading it off the keystore would have
            // asked the origin about an address nobody paid with.
            let mut signer = signer::choose(a, &host).map_err(DepotError::new)?;
            let address = signer
                .address()
                .ok_or_else(|| DepotError::new("the signer names no address"))?;
            let net = Net::new();
            let asked = net.get_json(&format!(
                "{host}/api/ticket/{slug}/message?wallet={address}"
            ));
            let Some(message) = asked
                .value
                .as_ref()
                .and_then(|v| v.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                return Err(DepotError::new(format!(
                    "could not fetch the text to sign — {}",
                    asked.why
                )));
            };
            let out = signer
                .sign(&message, &format!("download {slug}"))
                .map_err(|r| DepotError::new(r.reason))?;
            let sig = out
                .get("signature")
                .and_then(Value::as_str)
                .ok_or_else(|| DepotError::new("the signer returned no signature"))?;
            let resp = net.post_json(
                &format!("{host}/api/ticket/{slug}/entitle"),
                &serde_json::json!({ "wallet": address, "signature": sig }),
            );
            if resp.status == Some(402) {
                return Err(DepotError::new(format!(
                    "{address} holds no ticket for {slug}.\n\
                     buy the game (any wallet, any rail) and re-run:\n  \
                     {host}/game.html?slug={slug}\n  \
                     {host}/api/ticket/{slug} — the price and the contract"
                )));
            }
            let Some(v) = resp.value else {
                return Err(DepotError::new(format!("entitle failed — {}", resp.why)));
            };
            let grant = v.get("grant").and_then(Value::as_str).unwrap_or("");
            let expires = v.get("expires_at").and_then(Value::as_u64).unwrap_or(0);
            if grant.is_empty() {
                return Err(DepotError::new(format!(
                    "the origin sent no grant: {}",
                    v.get("error").and_then(Value::as_str).unwrap_or("no reason given")
                )));
            }
            save_grant(&games, slug, &host, grant, expires)?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(0);
            }
            println!("entitled — {address} holds a ticket for {slug}");
            println!("  the grant is cached for the day; install and update use it on their own");
            Ok(0)
        }

        // ── signin: the website reads as THIS machine's account ──────────────
        //
        // The launcher half of `meter/signin.py`'s device-code flow. The
        // browser shows a short code; the player TYPES it here — never clicks
        // it, which is the whole anti-phish property: a code that has to be
        // read off your own screen cannot be a stranger's, pasted into a chat
        // window. This side then composes the SIWE message ITSELF
        // (`protocol::signin_message` — the page contributes nothing but the
        // code), shows it, signs after one passphrase, and delivers the proof
        // to the origin. No session and no cookie exist afterwards: the
        // browser holds the proof, and every write still takes its own
        // signature.
        //
        // Then, unless `--no-watch`, it stays attached as the page's signer:
        // each queued signable is shown VERBATIM and answered y/N, the same
        // consent shape the broker gives a game — except here the passphrase
        // was typed once and the key stays unlocked for the watch, which is
        // rung 0's honest trade (`local.rs`: the process that draws the
        // prompt holds the key either way). Ctrl-C ends the watch; the
        // sign-in itself stays good.
        "signin" if a.positional.is_empty() => agent::signin(a, &host, json_out),
        "profile" => agent::profile(a, &host, json_out),
        "agent" => match a.positional.first().map(String::as_str) {
            Some("init") => agent::init(a, &host, &voice_dir()),
            _ => Err(DepotError::new("elo agent init --name <name> [--face <png>] [--words-file <path>]")),
        },
        "signin" => {
            let raw = need(a.positional.first(), "the code the website shows")?;
            let code: String = raw
                .to_ascii_uppercase()
                .chars()
                .filter(|c| *c != '-')
                .collect();
            // The code IS the EIP-4361 nonce, so the nonce rule is the shape
            // check — at least 8, alphanumeric only.
            elo_broker::protocol::check_nonce(&code).map_err(|r| {
                DepotError::new(format!(
                    "that is not a sign-in code — {}. The browser page shows \
                     one like ABCD-2345.",
                    r.reason
                ))
            })?;
            let domain = host
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(&host)
                .trim_end_matches('/')
                .to_string();
            let path = elo_vault::keystore_path();
            let mut signer = elo_vault::LocalSigner::at(&path);
            if !signer.exists() {
                return Err(DepotError::new(format!(
                    "no account at {} — `elo account new` makes one (or `import`). \
                     The account IS what the website signs in as.",
                    path.display()
                )));
            }
            let address = signer
                .address()
                .ok_or_else(|| DepotError::new("the keystore names no address"))?;

            // Look before the passphrase: a typo costs a request, not a
            // passphrase entry.
            let net = Net::new();
            let asked = net.get_json(&format!("{host}/api/signin/{code}/challenge"));
            if asked.value.is_none() {
                return Err(DepotError::new(match asked.status {
                    Some(404) => format!(
                        "{code} is not waiting to sign in — the code expired or \
                         was mistyped. The browser page shows a fresh one."
                    ),
                    Some(409) => "that code was already used — a sign-in code works once".into(),
                    _ if !asked.reachable => format!("could not reach {host} — {}", asked.why),
                    _ => format!("the origin refused the code — {}", asked.why),
                }));
            }

            let message = elo_broker::protocol::signin_message(
                &domain,
                &address,
                &code,
                &elo_broker::protocol::now_iso8601(),
            );
            // The person still reads this before the passphrase; with --json
            // it goes to stderr so stdout stays one document.
            let say = |line: &str| if json_out { eprintln!("{line}") } else { println!("{line}") };
            say("about to sign exactly this, and nothing else:");
            say("──");
            say(&message);
            say("──");
            warn_if_echoing();
            let pass = prompt::passphrase("passphrase: ").map_err(DepotError::new)?;
            signer.unlock(&pass).map_err(DepotError::new)?;
            let out = signer
                .sign(&message, "sign the website in")
                .map_err(|r| DepotError::new(r.reason))?;
            let sig = out
                .get("signature")
                .and_then(Value::as_str)
                .ok_or_else(|| DepotError::new("the signer returned no signature"))?;
            let resp = net.post_json_told(
                &format!("{host}/api/signin/{code}/proof"),
                &serde_json::json!({ "message": message, "signature": sig }),
            );
            let Some(v) = resp.value else {
                return Err(DepotError::new(format!("could not deliver the proof — {}", resp.why)));
            };
            if v.get("ok").and_then(Value::as_bool) != Some(true) {
                return Err(DepotError::new(format!(
                    "the origin refused the proof — {}",
                    v.get("error").and_then(Value::as_str).unwrap_or(&resp.why)
                )));
            }
            say(&format!("signed in — the browser showing {code} now reads as {address}"));
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            let approver = v.get("approver").and_then(Value::as_str).unwrap_or("");
            if a.has("--no-watch") || approver.is_empty() {
                return Ok(0);
            }

            // ── the watch: the page's signer, with the player in the loop ────
            println!(
                "\nwatching for signature requests from that page. Each one is \
                 shown in full\nand nothing is signed without a yes. Ctrl-C \
                 stops watching; the sign-in\nitself stays good."
            );
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let got =
                    net.get_json(&format!("{host}/api/signin/{code}/asks?approver={approver}"));
                let Some(feed) = got.value else {
                    if got.status == Some(404) {
                        println!("the pairing expired — done. `elo signin` a fresh code to reattach.");
                        return Ok(0);
                    }
                    // A transient network failure is a missed beat, not an exit.
                    continue;
                };
                let asks = feed
                    .get("asks")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for ask in asks {
                    let id = ask.get("id").and_then(Value::as_str).unwrap_or("");
                    let text = ask.get("text").and_then(Value::as_str).unwrap_or("");
                    let why = ask.get("why").and_then(Value::as_str).unwrap_or("");
                    if id.is_empty() || text.is_empty() {
                        continue;
                    }
                    // `why` first, through safe_line; the verbatim text last,
                    // right above the prompt (relay.rs says why). A text the
                    // terminal would draw as something else is refused unasked.
                    let shown = relay::ask_block(text, why);
                    let yes = match &shown {
                        Some(block) => {
                            println!("\n{block}");
                            prompt::confirm("sign it? [y/N] ").unwrap_or(false)
                        }
                        None => {
                            println!(
                                "\nthe page sent a text with control or direction \
                                 characters — refused without asking"
                            );
                            false
                        }
                    };
                    let body = if shown.is_none() {
                        serde_json::json!({
                            "approver": approver, "id": id,
                            "refusal": "the launcher cannot show this text as it would be signed",
                        })
                    } else if yes {
                        match signer.sign(text, "site relay") {
                            Ok(o) => serde_json::json!({
                                "approver": approver, "id": id,
                                "signature": o.get("signature").and_then(Value::as_str).unwrap_or(""),
                            }),
                            Err(r) => serde_json::json!({
                                "approver": approver, "id": id,
                                "refusal": format!("the launcher could not sign: {}", r.reason),
                            }),
                        }
                    } else {
                        serde_json::json!({
                            "approver": approver, "id": id,
                            "refusal": "the player said no",
                        })
                    };
                    let sent =
                        net.post_json_told(&format!("{host}/api/signin/{code}/answer"), &body);
                    let landed = sent
                        .value
                        .as_ref()
                        .and_then(|x| x.get("ok"))
                        .and_then(Value::as_bool)
                        == Some(true);
                    println!(
                        "{}",
                        if landed && yes {
                            "signed and delivered"
                        } else if landed {
                            "refused, and the page was told"
                        } else {
                            "could not deliver the answer — the page will see it lapse"
                        }
                    );
                }
            }
        }

        // ── who published a title ────────────────────────────────────────────
        //
        // Both directions in one verb, because they are the same string seen
        // from either end: `--statement` prints the exact bytes a publisher
        // signs, and the default checks a signature that already exists. A
        // verifier whose signable form nobody could produce would be a lock
        // with no key cut for it.
        //
        //     elo publisher gates --statement | xargs -0 elo account sign
        "publisher" => {
            let m = load_manifest(need(a.positional.first(), "a title")?, &host, READS_ONLY)?;
            if a.has("--statement") {
                // A statement `verify` will always refuse is not one to hand
                // a publisher to sign.
                if let Some(why) = publisher::ambiguous(&m) {
                    return Err(DepotError::new(why));
                }
                // No trailing newline: it is a message to be signed, and a
                // stray byte is a different signature.
                print!("{}", publisher::statement(&m));
                return Ok(0);
            }
            let check = publisher::verify(&m);
            if json_out {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "slug": m.slug,
                        "publisher": m.publisher,
                        "state": check.line(),
                        "signed": check.is_signed(),
                        "broken": check.is_broken(),
                    }))
                    .unwrap_or_default()
                );
            } else {
                println!("{:<16} {}", m.slug, check.line());
                if matches!(check, publisher::PublisherCheck::Unsigned) {
                    println!(
                        "  the manifest names {:?}, and nothing proves it — \
                         no signature is offered",
                        m.publisher
                    );
                }
            }
            // A broken signature is the only failing exit. Unsigned is the
            // expected state today and must not fail a script.
            Ok(if check.is_broken() { 1 } else { 0 })
        }

        "list" => {
            let rows = installed(&games);
            if json_out {
                let items: Vec<Value> = rows
                    .iter()
                    .map(|i| {
                        serde_json::json!({
                            "slug": i.slug, "build": i.build, "platform": i.platform,
                            "depot_digest": i.depot_digest, "bytes": i.total_bytes,
                            "path": i.path.to_string_lossy(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&items).unwrap_or_default());
                return Ok(0);
            }
            if rows.is_empty() {
                // The honest sentence, and it is not the same as "we could not
                // look" — this one IS a real count of a directory we read.
                println!("nothing installed in {}", games.display());
                return Ok(0);
            }
            for i in rows {
                println!(
                    "{:<16} {:<12} {:>10}  {}",
                    i.slug,
                    i.build,
                    elo_depot::install::human(i.total_bytes),
                    i.depot_digest
                );
            }
            Ok(0)
        }

        "digest" => {
            let raw = load_json(need(a.positional.first(), "a depot")?)?;
            println!("{}", elo_depot::digest(&raw)?);
            Ok(0)
        }

        // What the origin serves manifests for. This is the other half of
        // taking a slug instead of a url: a player who does not already know
        // the word "gates" has to be able to find it.
        "games" => {
            let net = Net::new();
            let got = net.get_json(&format!("{host}/api/launcher/manifests"));
            let Some(v) = got.value else {
                return Err(DepotError::new(if got.reachable {
                    got.why
                } else {
                    format!("could not reach {host} — {}", got.why)
                }));
            };
            // The depot root's own state, because a manifest whose native row
            // is empty because the origin could not look is not the same
            // manifest as one with nothing published. Read before either
            // output, so a script reading --json gets the same exit code.
            let depots_unread = v
                .get("depots")
                .and_then(|d| d.get("reachable"))
                .and_then(Value::as_bool)
                == Some(false);
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(if depots_unread { 3 } else { 0 });
            }
            let rows = v
                .get("manifests")
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_default();
            // An origin that serves no manifests and an origin we could not
            // read are different sentences, and only one of them is a fact
            // about the catalog.
            if rows.is_empty() {
                println!("{host} serves no manifests");
                return Ok(0);
            }
            for row in &rows {
                let Ok(mut m) = manifest::parse_manifest(row) else {
                    continue;
                };
                m.rebase(&host);
                let native = m.build_for(&platform, Some("native"));
                let state = match native {
                    Some(b) if b.published() => b
                        .version
                        .clone()
                        .unwrap_or_else(|| "published".into()),
                    _ if m.playable_now() => "browser".into(),
                    _ => "no build".into(),
                };
                println!("{:<16} {:<12} {:<10} {}", m.slug, m.state, state, m.name);
                // A BROKEN signature is shouted; an absent one is not
                // mentioned here at all. Nobody has signed a manifest yet, so
                // printing "unsigned" against every title would train a reader
                // to skim the one line that will ever matter.
                let check = publisher::verify(&m);
                if check.is_broken() {
                    println!("  ⚠ {}", check.line());
                }
            }
            if depots_unread {
                println!(
                    "\n! the origin could not read its depot root — {}",
                    v.pointer("/depots/why").and_then(Value::as_str).unwrap_or("no reason given")
                );
                println!("  a native row missing here means UNKNOWN, not unpublished.");
                return Ok(3);
            }
            Ok(0)
        }


        // ── the characters ───────────────────────────────────────────────────
        //
        // `/api/seat/of/{wallet}` answers "what does this wallet hold" and "may
        // it still mint" in ONE call, so this verb has no second half. That
        // shape is the origin's rather than a convenience invented here: a
        // client asking the two questions separately can render one against a
        // stale read of the other, and the mint answer is the half that moves.
        //
        // ⚠ The wallet is the SIGNER's address, not the keystore's — the same
        // reading `elo dev` takes, for the same reason. A paired player's
        // characters sit in the wallet in their browser, and a keystore read
        // would report an empty roster for a wallet holding a full one.
        // `--wallet` overrides both, because reading a roster needs no key at
        // all: this whole verb is unauthenticated.
        "seat" => {
            let net = Net::new();

            if a.positional.first().map(String::as_str) == Some("card") {
                let got = net.get_json(&format!("{host}/api/seat"));
                let Some(v) = got.value else {
                    return Err(DepotError::new(if got.reachable {
                        got.why
                    } else {
                        format!("could not reach {host} — {}", got.why)
                    }));
                };
                let deployed = v.get("deployed").and_then(Value::as_bool) == Some(true);
                if json_out {
                    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                    return Ok(if deployed { 0 } else { 3 });
                }
                if !deployed {
                    println!("{host} has no collection deployed");
                    return Ok(3);
                }
                let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("—").to_string();
                println!("collection  {} ({})", s("name"), s("symbol"));
                println!("contract    {}", s("contract"));
                // A supply we could not read and a supply of zero are different
                // sentences; only one of them is a fact about the collection.
                match v.get("supply") {
                    Some(sup) => {
                        let n = |k: &str| sup.get(k).and_then(Value::as_u64);
                        match (n("minted"), n("max")) {
                            (Some(m), Some(cap)) => println!("minted      {m} of {cap}"),
                            _ => println!("minted      unknown — the origin could not read it"),
                        }
                    }
                    None => println!("minted      unknown — the origin served no supply"),
                }
                for d in v.get("doors").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
                    let name = d.get("name").and_then(Value::as_str).unwrap_or("?");
                    let num = d.get("door").and_then(Value::as_u64).unwrap_or(0);
                    let open = d.get("open").and_then(Value::as_bool).unwrap_or(false);
                    let left = d.get("left").and_then(Value::as_u64);
                    println!(
                        "door {num:<7} {name:<10} {:<7} {}",
                        if open { "open" } else { "closed" },
                        match left {
                            Some(l) => format!("{l} left"),
                            None => "left unknown".into(),
                        }
                    );
                }
                return Ok(0);
            }

            let wallet = match a.flag("--wallet") {
                Some(w) => w.to_string(),
                None => match signer::choose(a, &host).ok().and_then(|s| s.address()) {
                    Some(addr) => addr,
                    None => {
                        return Err(DepotError::new(
                            "no wallet — pass `--wallet 0x…`, or `elo pair` to reach \
                             the wallet in your browser, or `elo account new` to make \
                             one here. Reading a roster needs no key, only an address."
                                .to_string(),
                        ))
                    }
                },
            };

            let got = net.get_json(&format!("{host}/api/seat/of/{wallet}"));
            let Some(v) = got.value else {
                return Err(DepotError::new(if got.reachable {
                    got.why
                } else {
                    format!("could not reach {host} — {}", got.why)
                }));
            };
            if json_out {
                // The same exit code the text below ends on: 3 when nothing is
                // deployed or a door's list has drifted from the chain's root.
                let deployed = v.get("deployed").and_then(Value::as_bool) == Some(true);
                let stale = v.get("eligible").and_then(Value::as_array).is_some_and(|es| {
                    es.iter().any(|e| {
                        e.get("root_matches_chain").and_then(Value::as_bool) == Some(false)
                    })
                });
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(if deployed && !stale { 0 } else { 3 });
            }
            if v.get("deployed").and_then(Value::as_bool) != Some(true) {
                println!("{host} has no collection deployed");
                return Ok(3);
            }

            let seats = v.get("seats").and_then(Value::as_array).cloned().unwrap_or_default();
            let holds = v.get("holds").and_then(Value::as_u64);
            println!("wallet      {wallet}");
            match holds {
                Some(n) => println!("holds       {n}"),
                // `holds` absent is "we could not count", never zero. An agent
                // that prints 0 here has invented the most damaging number in
                // the response.
                None => println!("holds       unknown — the origin could not count"),
            }
            for s in &seats {
                let id = s.get("token_id").and_then(Value::as_u64).unwrap_or(0);
                let tier = s.get("tier").and_then(Value::as_u64);
                let door = s.get("door").and_then(Value::as_str).unwrap_or("?");
                let live = s.get("activated").and_then(Value::as_bool).unwrap_or(false);
                println!(
                    "  #{id:<6} {:<8} {door:<10} {}",
                    match tier {
                        Some(t) => format!("tier {t}"),
                        None => "tier ?".into(),
                    },
                    if live { "unlocked" } else { "sealed" }
                );
            }

            // The other half of the same call. A door a wallet is in but has
            // spent is still printed — "you had three and took three" is the
            // answer to "may I mint", and silence would read as "we did not
            // check".
            let eligible = v.get("eligible").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut stale = false;
            if eligible.is_empty() {
                println!("eligible    no open door lists this wallet");
            } else {
                println!("eligible");
                for e in &eligible {
                    let num = e.get("door").and_then(Value::as_u64).unwrap_or(0);
                    let name = e.get("door_name").and_then(Value::as_str).unwrap_or("?");
                    let allow = e.get("allowance").and_then(Value::as_u64);
                    let took = e.get("already_taken").and_then(Value::as_u64);
                    let now = e.get("claimable_now").and_then(Value::as_u64);
                    let fmt = |o: Option<u64>| o.map(|n| n.to_string()).unwrap_or("?".into());
                    println!(
                        "  door {num} ({name})  allowance {}  taken {}  claimable now {}",
                        fmt(allow), fmt(took), fmt(now)
                    );
                    if e.get("window_closed").and_then(Value::as_bool) == Some(true) {
                        println!("    the window is closed");
                    }
                    // The one field to branch on. The door's list is rebuilt
                    // every Monday; when the served root and the chain's have
                    // drifted, the proof does not spend — it costs gas and
                    // reverts for a reason the receipt never names.
                    if e.get("root_matches_chain").and_then(Value::as_bool) == Some(false) {
                        stale = true;
                        println!("    ⚠ this list and the chain's root disagree — the proof \
                                  will NOT spend; re-read before minting");
                    }
                }
                println!("\n`elo seat card` is the collection; minting is prepared at");
                println!("{host}/api/seat/prepare?wallet={wallet}&door=<n> — your wallet signs it.");
            }
            if stale {
                return Ok(3);
            }
            Ok(0)
        }


        // `hive` was the verb until 2026-10-17; installed scripts still say it.
        "chat" | "hive" => {
            let net = Net::new();
            let hive = Hive::new(&net, &host);
            let limit: usize = a.flag("--limit").and_then(|v| v.parse().ok()).unwrap_or(20);
            match a.positional.first().map(String::as_str).unwrap_or("read") {
                "rooms" => {
                    let got = hive.channels();
                    let Some(c) = got.value else {
                        // Unreachable and empty are different sentences, and a
                        // chat client on a train is exactly where that matters.
                        return Err(DepotError::new(if got.reachable {
                            got.why
                        } else {
                            format!("could not reach chat — {}", got.why)
                        }));
                    };
                    println!("relay {}", c.relay.as_deref().unwrap_or("none posted"));
                    if c.channels.is_empty() {
                        println!("no chat rooms are posted");
                        return Ok(0);
                    }
                    for room in &c.channels {
                        println!(
                            "  #{:<14} {}",
                            render::clip(&render::safe_line(&room.name), 14),
                            render::clip(&render::safe_line(&room.about), 56)
                        );
                        println!("   {:<14} {}", "", room.id);
                    }
                    Ok(0)
                }
                "whoami" => {
                    let dir = voice_dir();
                    if !Voice::exists(&dir) {
                        println!("no chat key on this machine — `elo chat voice-new` makes one");
                        return Ok(0);
                    }
                    let v = Voice::load(&dir).map_err(DepotError::from)?;
                    println!("npub    {}", v.npub());
                    println!("key     {}", dir.join(elo_hive::voice::KEY_NAME).display());
                    // The public record, from the key's end. A failed look is
                    // said as one, never as "not linked".
                    let got = net.get_json_quick(&format!("{host}/api/hive/link/{}", v.npub()));
                    match got.value.as_ref().map(|r| r.get("wallet").and_then(Value::as_str)) {
                        Some(Some(w)) => println!("linked  {w} — rooms show its profile name and picture"),
                        Some(None) => println!("linked  no — `elo chat link` uses your wallet's own key"),
                        None => println!("linked  could not look — {}", render::safe_line(&got.why)),
                    }
                    // Said on the surface every time, not once in a doc. This
                    // key can speak as you and cannot touch anything else, and
                    // both halves of that matter.
                    println!("\nThis is a SPEAKING key, not a wallet — it holds no funds and signs");
                    println!("no transaction. It sits at 0600 unencrypted, so anyone who can read");
                    println!("your home directory can speak as you in chat. We never see it.");
                    Ok(0)
                }
                "link" | "unlink" => {
                    let unlink = a.positional.first().map(String::as_str) == Some("unlink");
                    chatkey::run(a, &host, &voice_dir(), unlink)
                }
                "voice-new" => {
                    let dir = voice_dir();
                    if Voice::exists(&dir) && !a.has("--yes") {
                        // Overwriting a voice loses the identity everything
                        // said under it, so it takes an explicit act.
                        return Err(DepotError::new(format!(
                            "a chat key already exists at {} — `--yes` replaces it, and \
                             everything said under the old npub stays under the old npub",
                            dir.join(elo_hive::voice::KEY_NAME).display()
                        )));
                    }
                    let v = Voice::generate().map_err(DepotError::from)?;
                    let path = v.save(&dir).map_err(DepotError::from)?;
                    println!("npub    {}", v.npub());
                    println!("saved   {}", path.display());
                    println!("\nBack it up as an nsec if you want this identity to survive this");
                    println!("machine. It is a speaking key: no funds, no transactions, and elo");
                    println!("never sees it. With a wallet, `elo chat link` speaks with the");
                    println!("wallet's own key instead, and rooms show your profile name.");
                    Ok(0)
                }
                "presence" => {
                    let v = Voice::load(&voice_dir()).map_err(DepotError::from)?;
                    let url = relay_url(&hive)?;
                    let status = match a.flag("--status").unwrap_or("online") {
                        "away" => Status::Away,
                        "online" => Status::Online,
                        other => return Err(DepotError::new(format!(
                            "unknown status {other:?} — online or away"))),
                    };
                    let mut held = Presence::hold(&url, v, status)
                        .map_err(DepotError::from)?;
                    println!("{} is {} on {url}", held.npub(), status.as_str());
                    // The socket IS the presence. `meter/hive_presence.py`:
                    // buzz clears it the instant the connection closes, so
                    // this loop existing is the feature — there is no beat we
                    // could send that would survive hanging up.
                    println!("holding the socket — every {}s. Ctrl-C to go offline.",
                             HEARTBEAT.as_secs());
                    loop {
                        std::thread::sleep(HEARTBEAT);
                        if let Err(e) = held.tick() {
                            return Err(DepotError::new(format!(
                                "presence dropped after {} beats: {e}", held.beats())));
                        }
                    }
                }
                "say" => {
                    let text = need(a.positional.get(1), "something to say")?;
                    let v = Voice::load(&voice_dir()).map_err(DepotError::from)?;
                    let rooms = channels_of(&hive)?;
                    let url = rooms.relay.clone().filter(|r| !r.is_empty())
                        .ok_or_else(|| DepotError::new("the house has posted no relay"))?;
                    let room = rooms.resolve(a.flag("--room")).map_err(DepotError::new)?;
                    let event = v
                        .sign(note_for(room.as_deref(), text, clock::now()))
                        .map_err(DepotError::from)?;
                    if a.has("--dry-run") {
                        println!("{}", serde_json::to_string_pretty(&event.to_json()).unwrap_or_default());
                        return Ok(0);
                    }
                    println!("speaking as {} on {url}", v.npub());
                    let mut relay = Relay::connect(&url, &v).map_err(DepotError::from)?;
                    let id = relay.publish(&event).map_err(DepotError::from)?;
                    relay.close();
                    // "Published" means a relay ruled true. Nothing else does —
                    // the HTTP path this replaced failed SILENTLY.
                    println!("published {id}");
                    Ok(0)
                }
                "voice" => {
                    let got = hive.outbox(limit);
                    let Some(v) = got.value else {
                        return Err(DepotError::new(if got.reachable {
                            got.why
                        } else {
                            format!("could not reach chat — {}", got.why)
                        }));
                    };
                    println!("the house's outbox — {} composed, {} queued", v.composed, v.queued);
                    let now = clock::now();
                    for n in &v.notes {
                        println!(
                            "{:>8}  {}{}",
                            clock::ago(n.created_at, now),
                            if n.carried { "  " } else { "· " },
                            render::clip(&render::safe_line(&n.content), 92)
                        );
                    }
                    // `carried` means a relay acknowledged it; otherwise the
                    // note is still waiting for the room, which is a state and
                    // not a failure.
                    println!("\n· = not yet carried by a relay");
                    Ok(0)
                }
                // `read` is the default verb, so an unknown word reads the
                // landing room rather than erroring — a chat client should not
                // punish a typo by refusing to show the room.
                _ => {
                    // a NAME is resolved here; no --room is left to the origin,
                    // which lands it in #general (an old origin: the town wire)
                    let asked = match a.flag("--room") {
                        Some(r) => channels_of(&hive)?.resolve(Some(r)).map_err(DepotError::new)?
                            .or_else(|| Some("town".into())),
                        None => None,
                    };
                    let got = hive.room(asked.as_deref(), limit);
                    let Some(r) = got.value else {
                        return Err(DepotError::new(if got.reachable {
                            got.why
                        } else {
                            format!("could not reach chat — {}", got.why)
                        }));
                    };
                    println!("{} · relay {}", r.scope, r.relay.as_deref().unwrap_or("none"));
                    if r.messages.is_empty() {
                        println!("nothing said here yet");
                        return Ok(0);
                    }
                    let now = clock::now();
                    // Oldest first, so a terminal reads top-to-bottom like a
                    // chat window rather than backwards like an API.
                    let mut rows = r.messages.clone();
                    rows.sort_by_key(|m| m.at);
                    // A speaker the room ties to a wallet (a linked key)
                    // wears the wallet's own profile name, as in
                    // the window and on the site. One look per wallet, a few
                    // wallets at most, and a failed look keeps the room's name.
                    let mut profiles: BTreeMap<String, Option<String>> = BTreeMap::new();
                    for w in rows.iter().filter(|m| !m.house).filter_map(|m| m.wallet.as_deref()) {
                        let w = w.to_lowercase();
                        if who::looks_like_wallet(&w) && profiles.len() < 12 {
                            profiles.entry(w.clone()).or_insert_with(|| {
                                who::resolve(&net, &host, &w).value
                                    .filter(|id| id.kind == NameKind::Declared)
                                    .map(|id| id.name)
                            });
                        }
                    }
                    for m in &rows {
                        // One resolver, the same one `elo who` uses.
                        let mut id = who::from_message(m, &host);
                        let named = m.wallet.as_deref()
                            .and_then(|w| profiles.get(&w.to_lowercase()).cloned().flatten());
                        if let Some(name) = named.filter(|n| !n.trim().is_empty() && !m.house) {
                            id.name = name;
                            id.kind = NameKind::Declared;
                        }
                        println!(
                            "{}",
                            render::chat_line(
                                &clock::ago(m.at, now),
                                &id.name,
                                id.kind.checked(),
                                &m.content,
                                100
                            )
                        );
                    }
                    println!("\n~ = a name the speaker gave themselves");
                    Ok(0)
                }
            }
        }

        "who" => {
            // The wallet IS the account, so a bare `elo who` resolves the
            // watched address. One name, one face, one place they come from.
            let handle = match a.positional.first() {
                Some(h) => h.clone(),
                None => a
                    .flag("--wallet")
                    .map(str::to_string)
                    .or_else(|| std::env::var("ELO_WALLET").ok())
                    .ok_or_else(|| DepotError::new(
                        "no handle, and no wallet is set — pass one, use --wallet, \
                         or set ELO_WALLET"))?,
            };
            who::check_handle(&handle).map_err(DepotError::from)?;
            let net = Net::new();
            let got = who::resolve(&net, &host, &handle);
            let Some(id) = got.value else {
                return Err(DepotError::new(if got.reachable {
                    got.why
                } else {
                    format!("could not reach the directory — {}", got.why)
                }));
            };
            if json_out {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "name": id.name, "name_kind": format!("{:?}", id.kind).to_lowercase(),
                    "checked": id.kind.checked(), "face": id.face, "wallet": id.wallet,
                    "npub": id.npub, "nip05": id.nip05,
                    "bare": id.bare, "matches": id.matches, "ambiguous": id.ambiguous(),
                })).unwrap_or_default());
                return Ok(0);
            }
            println!("name    {}", id.name);
            println!("        {}", id.kind.line());
            if id.face.is_empty() {
                println!("face    none set — a letter tile");
            } else {
                println!("face    {}", id.face);
            }
            for (label, v) in [("wallet", &id.wallet), ("npub", &id.npub),
                               ("nip05", &id.nip05)] {
                if let Some(v) = v {
                    println!("{label:<7} {v}");
                }
            }
            if id.ambiguous() {
                // Never quietly present one of several names as THE name.
                println!("\n⚠ {} people use this name, so this is one of them. Look", id.matches);
                println!("someone up by their wallet or npub to be sure.");
            }
            if id.bare {
                // Not an error and not an empty read: a bare wallet is a real
                // answer, and no account exists to require.
                println!("\nNothing is set for this address yet. It can still buy, play and be paid.");
            }
            if id.kind == NameKind::Declared {
                println!("\nThis name is self-declared. It is wallet-signed, so it is THEIRS,");
                println!("but nobody checked that it is true. A hive handle is held by one");
                println!("wallet and cannot be set by asserting it.");
            }
            Ok(0)
        }

        // ── friends ──────────────────────────────────────────────────────────
        //
        // Its own module for the same reason `dev` is: a list, a presence
        // line and seven signed changes are a desk, not a verb. What comes
        // back to this dispatcher is either an exit code or a friend's link —
        // and a link goes through `join` exactly as a pasted one does, which
        // then becomes `play`. Following a friend is not a third launch path.
        "friends" => match friends::run(a, &host, json_out)? {
            friends::Done::Code(code) => Ok(code),
            friends::Done::Join(link) => {
                let mut join = a.clone();
                join.verb = "join".into();
                join.positional = vec![link];
                // `--wallet` here named whose LIST to read, and `play` would
                // hand it to the game as `{wallet}` — joining from a friend's
                // list as that friend. Dropped, so the game asks this
                // launcher who is playing, as the window's Join does.
                join.flags.remove("--wallet");
                join.repeats.remove("--wallet");
                run(&join)
            }
        },

        "mcp" => {
            // Read-only by construction — see `mcp.rs`. Installing, launching
            // and speaking stay on the CLI where a person typed them.
            // `what=friends` defaults to the wallet `elo friends` reads, asked
            // per call so a pairing made while the server runs is the one read.
            let whose: mcp::Whose = {
                let (a, host) = (a.clone(), host.clone());
                Box::new(move || friends::whose(&a, &host).map_err(|e| e.to_string()))
            };
            mcp::run(a, &games, &state_root(), &host, &platform, whose)
                .map_err(|e| DepotError::new(format!("mcp: {e}")))?;
            Ok(0)
        }

        "sessions" => {
            let all = session::load(&state_root());
            if all.is_empty() {
                println!("nothing played yet");
                return Ok(0);
            }
            let totals = session::totals(&all);
            let unmeasured = session::unmeasured(&all);
            println!("── played ──");
            for (slug, secs) in &totals {
                let extra = unmeasured.get(slug).copied().unwrap_or(0);
                println!(
                    "{:<16} {:>10}{}",
                    slug,
                    clock::duration(*secs),
                    // Never fold an unwatched launch into the number. Saying
                    // "and 3 not measured" is the honest shape; adding a guess
                    // would make this client a source of truth, which wall 3
                    // says it is not.
                    if extra > 0 { format!("   (+{extra} not measured)") } else { String::new() }
                );
            }
            for slug in unmeasured.keys() {
                if !totals.contains_key(slug) {
                    println!("{:<16} {:>10}   ({} not measured)", slug, "—",
                             unmeasured[slug]);
                }
            }
            let running: Vec<&Session> = all
                .iter()
                .filter(|s| match s.ending {
                    Ending::Running { pid } => session::is_alive(pid),
                    _ => false,
                })
                .collect();
            if !running.is_empty() {
                println!("\n── running ──");
                let now = clock::now();
                for s in running {
                    println!(
                        "{:<16} {} for {}",
                        s.slug,
                        s.build,
                        clock::duration(now - s.started_at)
                    );
                }
            }
            Ok(0)
        }

        "status" => {
            let source = need(a.positional.first(), "a manifest")?;
            let (from, base) = manifest_source_on(source, &host, insecure, branch_flag(a)?)?;
            // A manifest we could not fetch is a status we could not check —
            // exit 3 and the same JSON shape as any other unknown, never a
            // bare error a script reads as a crash.
            let m = match read_manifest(source, &from, &base) {
                Ok(m) => m,
                Err(e) => {
                    let state = UpdateState::Unknown { why: e.to_string() };
                    if json_out {
                        println!("{}", serde_json::to_string_pretty(&state_json(&state)).unwrap_or_default());
                    } else {
                        println!("{source} — {}", shown(&state.line()));
                    }
                    return Ok(3);
                }
            };
            let checked = check_updates(&m, &platform, &games, insecure);
            if json_out {
                println!("{}", serde_json::to_string_pretty(&state_json(&checked.state)).unwrap_or_default());
            } else {
                println!("{} — {}", m.name, checked.state.line());
            }
            // A state we could not determine is not a success. Scripts and CI
            // read this, and "unknown" exiting 0 is how a stale build ships.
            Ok(match checked.state {
                UpdateState::Unknown { .. } => 3,
                UpdateState::Stale { .. } => 10,
                _ => 0,
            })
        }

        "install" | "update" => {
            let m = load_manifest_on(need(a.positional.first(), "a manifest")?, &host, insecure,
                                     branch_flag(a)?)?;
            use_cached_grant(&games, &m.slug, &host);
            let checked = check_updates(&m, &platform, &games, insecure);
            println!("{} — {}", m.name, checked.state.line());
            match &checked.state {
                UpdateState::Unknown { why } => {
                    // A 401 from the depot is the ticket door, not a fault:
                    // this title is sold, and the download needs a grant.
                    if why.contains("401") {
                        return Err(DepotError::new(format!(
                            "refusing to install: {why}\n\
                             this game is sold as a ticket — one command does the whole dance\n\
                             (sign with your launcher account, cache a day grant):\n  \
                             elo entitle {slug}\n\
                             then re-run `elo {verb} …`. No ticket yet? the price and the\n\
                             contract: {host}/api/ticket/{slug}",
                            why = why,
                            slug = m.slug,
                            host = host,
                            verb = a.verb
                        )));
                    }
                    return Err(DepotError::new(format!("refusing to install: {why}")))
                }
                UpdateState::Unpublished => return Ok(0),
                UpdateState::Current { .. } if a.verb == "update" => return Ok(0),
                _ => {}
            }
            let Some(depot) = checked.depot else {
                return Ok(0);
            };
            do_install(&depot, &games, a)?;
            Ok(0)
        }

        "verify" => {
            let slug = need(a.positional.first(), "a slug")?;
            let build = a.positional.get(1);
            let mut any = false;
            let mut bad = false;
            for i in installed(&games) {
                if &i.slug != slug || build.is_some_and(|b| &i.build != b) {
                    continue;
                }
                any = true;
                let v = verify(&i.path);
                println!("{} {}  {}", i.slug, i.build, v.line());
                for p in v.missing.iter().chain(&v.changed) {
                    println!("    {p}");
                }
                bad |= !v.ok;
            }
            if !any {
                return Err(DepotError::new(format!("{slug}: nothing installed")));
            }
            Ok(if bad { 1 } else { 0 })
        }

        "prune" => {
            let slug = need(a.positional.first(), "a slug")?;
            // By install TIME, never by name — a sha does not sort. Keeping
            // the wrong build here would delete the right one.
            let newest = elo_depot::newest_of(&games, slug)
                .ok_or_else(|| DepotError::new(format!("{slug}: nothing installed")))?;
            let old = superseded(&games, slug, &newest.build);
            if old.is_empty() {
                println!("{slug}: only build {} is installed", newest.build);
                return Ok(0);
            }
            for i in &old {
                println!(
                    "{} remove build {} ({})",
                    if a.has("--dry-run") { "would" } else { "  " },
                    i.build,
                    elo_depot::install::human(i.total_bytes)
                );
                if !a.has("--dry-run") {
                    uninstall(&i.path)?;
                }
            }
            Ok(0)
        }

        "uninstall" => {
            let slug = need(a.positional.first(), "a slug")?;
            let build = need(a.positional.get(1), "a build")?;
            let target = installed(&games)
                .into_iter()
                .find(|i| &i.slug == slug && &i.build == build)
                .ok_or_else(|| DepotError::new(format!("{slug} {build}: not installed")))?;
            uninstall(&target.path)?;
            println!("removed {} {}", slug, build);
            Ok(0)
        }

        "play" => {
            let m = load_manifest(need(a.positional.first(), "a manifest")?, &host, READS_ONLY)?;
            // `{servers}` comes from the MANIFEST, never from a flag: the shard
            // list is the title's to name and it has already named it in the
            // document we just loaded, so the game's own browser and the
            // Servers window cannot disagree about what is up.
            let values = args::launch_values(a, m.servers_url.as_deref());
            // Prefer an installed native build; fall back to the browser row;
            // refuse rather than guess when neither is published.
            //
            // The build installed MOST RECENTLY, not the last by name: a
            // build id's sha carries no order, and a name chosen to sort last
            // would otherwise win every Play. And only a build this manifest's
            // origin installed (`newest_from`): a slug is anyone's to write.
            if let Some(i) = elo_depot::newest_from(&games, &m.slug, &m.origin) {
                let l = launch::resolve(&i, &values, &BTreeMap::new())?;
                if a.has("--dry-run") {
                    println!("{}", l.command_line().join(" "));
                    return Ok(0);
                }
                let started = clock::now();
                let state = state_root();
                // Detached is the exception, not the default. A launcher that
                // lets go of the process can never know how long you played,
                // and guessing later would make this a source of truth — which
                // wall 3 says it is not. So the honest default is to watch.
                if a.has("--detach") {
                    let pid = launch::spawn(&l)?;
                    session::append(&state, Session {
                        slug: m.slug.clone(), build: i.build.clone(),
                        started_at: started, ending: Ending::Detached { pid },
                    })?;
                    println!("started {} build {} (pid {pid}, not watched — \
                              no playtime will be recorded)", m.slug, i.build);
                    return Ok(0);
                }
                let mut child = launch::spawn_supervised(&l)?;
                let pid = child.id();
                // Presence held for exactly as long as the game runs — which
                // is the whole of "in-game presence", and only possible
                // because `play` supervises. A detached launch cannot have it
                // for the same reason it cannot have playtime.
                let holding = hold_presence_while_playing(a, &host, &m.slug);
                session::append(&state, Session {
                    slug: m.slug.clone(), build: i.build.clone(),
                    started_at: started, ending: Ending::Running { pid },
                })?;
                println!("playing {} build {} (pid {pid})", m.slug, i.build);
                let status = child.wait().map_err(|e| DepotError::new(e.to_string()))?;
                let ended = clock::now();
                if let Some((stop, joiner)) = holding {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = joiner.join();
                }
                session::close(&state, &m.slug, &i.build, ended, status.code())?;
                println!("played {}", clock::duration(ended - started));
                // The game's exit code is the game's business and never this
                // process's verdict: a player quitting to the desktop is not a
                // launcher failure.
                return Ok(0);
            }
            let browser = m
                .build_for(&platform, Some("browser"))
                .and_then(|b| b.url.clone());
            match browser {
                Some(url) => {
                    let argv = elo_depot::browser_argv(&url, a.flag("--browser").unwrap_or(""))?;
                    if a.has("--dry-run") {
                        println!("{}", argv.join(" "));
                        return Ok(0);
                    }
                    std::process::Command::new(&argv[0])
                        .args(&argv[1..])
                        .spawn()
                        .map_err(|e| DepotError::new(format!("could not open a browser: {e}")))?;
                    println!("opened {url}");
                    Ok(0)
                }
                None => Err(DepotError::new(format!(
                    "{} has no published build to start",
                    m.name
                ))),
            }
        }

        // The shards a title is serving, with live counts.
        //
        // The CLI half of the Servers window, and it exists for a reason
        // beyond symmetry: this is the surface that can be run against a real
        // origin from a terminal, so "is the list up, and is it saying
        // anything" is answerable without a display.
        "servers" => {
            let m = load_manifest(need(a.positional.first(), "a manifest")?, &host, READS_ONLY)?;
            let net = Net::new();
            let Some(url) = m.servers_url.clone() else {
                // Dark, and it names the field that would light it — the same
                // discipline every dark panel in this client follows. Exit 0:
                // "this title publishes no shard list" is a true answer, not a
                // failure of the command.
                if json_out {
                    println!("{}", serde_json::json!({
                        "slug": m.slug, "servers_url": null, "servers": []
                    }));
                } else {
                    println!("{} publishes no shard list", m.name);
                    println!("  it would set `servers.url` in its manifest — the list is the \
                              game's to serve, not ours to invent");
                }
                return Ok(0);
            };

            let got = net.shards(&url);
            let Some(mut rows) = got.value else {
                // `reachable` is not `empty`, and this is exactly where that
                // distinction earns its keep: a laptop on a train must not
                // report that a title has no shards.
                return Err(DepotError::new(if got.reachable {
                    format!("{url}: {}", got.why)
                } else {
                    format!("could not reach {url} — {}", got.why)
                }));
            };

            // Live counts, polled from each shard rather than read out of the
            // document. See `shardlist.rs`: a count baked into a served file
            // is stale before anybody reads it. A shard that does not answer
            // keeps whatever the row said, which is usually nothing.
            for r in rows.iter_mut() {
                let Some(su) = r.status_url_to_poll(&url).map(str::to_string) else { continue };
                if let Some(s) = net.status(&su, &url).value {
                    r.apply_status(&s);
                }
            }

            if json_out {
                let out: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
                    "id": r.id, "name": r.name, "addr": r.addr,
                    "players": r.players, "max_players": r.max_players,
                    "map": r.map, "full": r.full(),
                    "link": elo_depot::deeplink::link_for(&m.slug, &r.addr).ok(),
                })).collect();
                println!("{}", serde_json::json!({
                    "slug": m.slug, "servers_url": url, "servers": out
                }));
                return Ok(0);
            }
            if rows.is_empty() {
                // A real answer, and not an error: nobody is running one.
                println!("{} lists no shards up", m.name);
                return Ok(0);
            }
            for r in &rows {
                let map = r.map.clone().unwrap_or_default();
                println!("{:<20} {:>9}  {:<28} {}",
                         truncate(&r.name, 20), r.population(), r.addr, map);
                // The invite, so a player can send someone here without
                // opening the window. One writer for the shared form.
                if let Ok(link) = elo_depot::deeplink::link_for(&m.slug, &r.addr) {
                    println!("{:<20} {}", "", link);
                }
            }
            Ok(0)
        }

        // A join link, from a friend or from the desktop's url handler.
        //
        // **This does not start anything itself — it becomes `play`.** A
        // second launch path would be a second place for playtime, presence
        // and session records to be got wrong, and the two would drift the
        // first time one of them was fixed. So the link resolves to a title
        // and an address, and the address goes in exactly where `--server`
        // already goes.
        "join" => {
            let link = need(a.positional.first(), "a join link")?;
            let j = elo_depot::deeplink::parse(link)?;
            println!("joining {} on {}", j.slug, j.addr);

            let mut play = a.clone();
            play.verb = "play".into();
            play.positional = vec![j.slug.clone()];
            // The link wins over a typed `--server`: resolving one is this
            // verb's entire purpose, so honouring a flag over it would make
            // `elo join <link> --server X` silently ignore the link.
            play.flags.insert("--server".into(), j.addr.clone());
            let had_it = elo_depot::newest_of(&games, &j.slug).is_some();
            run(&play).map_err(|e| {
                // `play` refuses an uninstalled title correctly but says
                // nothing about what to do next, and arriving from a link is
                // exactly when a player does not have the game yet. Only
                // then: an installed game that failed to start, or an origin
                // that did not answer, is not fixed by installing it.
                if had_it {
                    return e;
                }
                DepotError::new(format!(
                    "{e}\n  the link is good — install it first: elo install {}",
                    j.slug
                ))
            })
        }

        other => Err(DepotError::new(format!(
            "unknown command {other:?} — try `elo help`"
        ))),
    }
}

fn do_install(depot: &Depot, games: &Path, a: &args::Args) -> Result<PathBuf, DepotError> {
    let p = if a.has("--no-reuse") {
        Plan::default()
    } else {
        plan(depot, games)
    };
    println!("build {} — {}", depot.build, p.line());
    // Never silent. This depot was packaged against a host the platform has
    // retired, and the files are coming from somewhere its document does not
    // name — which a player is entitled to be told BEFORE the bytes move, not
    // in a changelog. The digest printed on the way out is unaffected.
    if let Some(was) = &depot.healed_from {
        println!(
            "note: this build names its files on {}, which is retired and answers 410.\n\
             \x20     downloading from {} instead — every file is still checked against\n\
             \x20     the sha256 in the depot this origin served (its digest is below).",
            elo_depot::origin_of(was),
            elo_depot::origin_of(&depot.root)
        );
    }
    if a.has("--dry-run") {
        return Ok(games.join(&depot.slug).join(&depot.build));
    }
    let net = Net::new();
    let total = depot.files.len();
    let mut n = 0usize;
    let mut progress = |done: i64, all: i64, path: &str, reused: bool| {
        n += 1;
        let pct = if all > 0 { done * 100 / all } else { 100 };
        println!(
            "  [{n:>4}/{total}] {pct:>3}%  {}{}",
            path,
            if reused { "  (on disk)" } else { "" }
        );
    };
    let at = install(depot, games, &net, Some(&p), Some(&mut progress))?;
    println!("installed to {}", at.display());
    // Say the digest on the way out. It is the one number a player can take to
    // a block explorer, and a launcher that computes it and keeps it quiet has
    // wasted the only property no other storefront has.
    println!("depot digest {}", depot.digest()?);
    Ok(at)
}

fn check_updates(m: &Manifest, platform: &str, games: &Path, insecure: bool) -> Checked {
    let net = Net::new();
    elo_depot::check(m, platform, games, &net, insecure)
}

fn state_json(state: &UpdateState) -> Value {
    match state {
        UpdateState::NotInstalled { published } => {
            serde_json::json!({"state": "not-installed", "published": published})
        }
        UpdateState::Current { build, digest } => {
            serde_json::json!({"state": "current", "build": build, "depot_digest": digest})
        }
        UpdateState::Stale { from_build, to_build, from_digest, to_digest, plan } => {
            serde_json::json!({
                "state": "stale", "from": from_build, "to": to_build,
                "from_digest": from_digest, "to_digest": to_digest,
                "fetch_bytes": plan.fetch_bytes, "reuse_bytes": plan.reuse_bytes,
                "reused_files": plan.reuse.len(),
            })
        }
        UpdateState::Unpublished => serde_json::json!({"state": "unpublished"}),
        // `reachable: false` rides in the payload rather than being inferred
        // from a null, so a consumer cannot read this as "no update".
        UpdateState::Unknown { why } => {
            serde_json::json!({"state": "unknown", "reachable": false, "why": why})
        }
    }
}

/// Where sessions and other launcher state live. Separate from the games root
/// because XDG says so and because a player pointing `--games` at an external
/// drive should not lose their playtime with it.
/// Where the hive speaking key lives — its own directory at 0700, beside the
/// state rather than inside the games root.
/// Hold hive presence for the life of a game, on its own thread.
///
/// Returns the stop flag and the thread, or `None` when there is nothing to
/// hold with — no voice, no relay, `--quiet`. **A failure here never stops the
/// game.** Presence is a nicety; a launcher that refused to start a title
/// because a chat relay was unreachable would be trading the product for the
/// garnish.
type Holding = (
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
);

fn hold_presence_while_playing(a: &args::Args, host: &str, slug: &str) -> Option<Holding> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    if a.has("--quiet") {
        return None;
    }
    let dir = voice_dir();
    if !Voice::exists(&dir) {
        return None;
    }
    let voice = Voice::load(&dir).ok()?;
    let net = Net::new();
    let url = relay_url(&Hive::new(&net, host)).ok()?;

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let slug = slug.to_string();
    let joiner = std::thread::spawn(move || {
        // Connected here, not on the play path: a slow relay must never stand
        // between the game and `child.wait`, which is where playtime comes from.
        let mut held = match Presence::hold(&url, voice, Status::Online) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("elo: chat presence unavailable ({}) — playing anyway", shown(&e));
                return;
            }
        };
        println!("chat: {} online while you play {slug}", held.npub());
        // Woken often, beats rarely: the relay's TTL is three heartbeats, so
        // sleeping the whole interval would make quitting wait for it.
        let tick = std::time::Duration::from_millis(500);
        let mut waited = std::time::Duration::ZERO;
        while !flag.load(Ordering::Relaxed) {
            std::thread::sleep(tick);
            waited += tick;
            if waited >= HEARTBEAT {
                waited = std::time::Duration::ZERO;
                if held.tick().is_err() {
                    // The socket died. Nothing to recover and nothing to say
                    // — the game is still running, which is what matters.
                    return;
                }
            }
        }
        held.release();
    });
    Some((stop, joiner))
}

/// The origin's room list, or why it could not be read.
fn channels_of(hive: &Hive) -> Result<elo_hive::Channels, DepotError> {
    let got = hive.channels();
    got.value.ok_or_else(|| DepotError::new(if got.reachable {
        got.why
    } else {
        format!("could not reach chat — {}", got.why)
    }))
}

/// The relay the house posted. Never typed by us — one place, so a fork or a
/// local origin moves it for every verb at once.
fn relay_url(hive: &Hive) -> Result<String, DepotError> {
    channels_of(hive)?
        .relay
        .filter(|r| !r.is_empty())
        .ok_or_else(|| DepotError::new("the house has posted no relay"))
}

/// The chat key's directory. One definition, in `elo-hive`, so the window and
/// this CLI find the same voice (`elo_hive::voice::default_dir`).
fn voice_dir() -> PathBuf {
    elo_hive::voice::default_dir()
}

fn games_root(a: &args::Args) -> PathBuf {
    if let Some(p) = a.flag("--games") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("ELO_GAMES") {
        return PathBuf::from(p);
    }
    home_based("XDG_DATA_HOME", ".local/share")
        .join("elo")
        .join("games")
}

/// Missing arguments say what was wanted, not "usage:". A player reading a
/// terminal is owed the same sentence a window would have shown them.
fn need<'a>(v: Option<&'a String>, what: &str) -> Result<&'a String, DepotError> {
    v.ok_or_else(|| DepotError::new(format!("needs {what}")))
}

/// Fit a string into a column, by CHARACTERS.
///
/// Not by bytes: a shard name arrives off the network and may be any UTF-8, so
/// a byte slice can land mid-codepoint and panic. A launcher must not be
/// crashable by a title naming a shard in Japanese.
fn truncate(s: &str, cols: usize) -> String {
    if s.chars().count() <= cols {
        return s.to_string();
    }
    // The ellipsis costs a column, so the cut is one short of the budget.
    s.chars().take(cols.saturating_sub(1)).collect::<String>() + "…"
}

// ── the account's helpers ────────────────────────────────────────────────────

/// Never write over a keystore. The vault refuses this too, and it is checked
/// here as well so the refusal arrives **before** a passphrase is typed rather
/// than after — being asked to invent a passphrase and only then told the
/// command was never going to work reads as a bug in the client.
fn refuse_if_occupied(path: &Path) -> Result<(), DepotError> {
    if path.is_file() {
        return Err(DepotError::new(format!(
            "{} already holds an account, and this never overwrites a keystore — \
             it may be the only copy of a key. Move that file aside if you really \
             mean to replace it.",
            path.display()
        )));
    }
    // The vault refuses this too (`LocalSigner::write_new`); asked here as
    // well so the refusal comes BEFORE a player has typed a key into a
    // prompt whose answer was always going to be no.
    let sol = elo_vault::solana_path_for(path);
    if sol.exists() {
        return Err(DepotError::new(format!(
            "{} is still there from an account whose keystore went — move it somewhere \
             safe first: it may be the only copy of a Solana key.",
            sol.display()
        )));
    }
    Ok(())
}

/// Say so when the passphrase is about to be painted on the screen.
///
/// A holder who does not know their key is being echoed cannot decide not to
/// type it — and on a shared terminal, a recorded session or a screen share,
/// that decision is the whole of their protection.
fn warn_if_echoing() {
    if prompt::echo_is_visible() {
        eprintln!(
            "! this terminal will show what you type — nothing here can hide it on \
             this platform yet"
        );
    }
}

fn passphrase_twice() -> Result<elo_vault::Zeroizing<String>, DepotError> {
    let pass = prompt::secret(&format!(
        "a passphrase for this account (at least {} characters): ",
        elo_vault::local::MIN_PASSPHRASE
    ))
    .map_err(DepotError::new)?;
    let again = prompt::secret("type it again: ").map_err(DepotError::new)?;
    if pass != again {
        // Nothing was written, and saying so is the point: the alternative
        // leaves a holder wondering whether a half-made keystore is on disk.
        return Err(DepotError::new("those two did not match — nothing was written"));
    }
    Ok(pass)
}

/// 32 bytes from hex, and **no input ever reaches an error message** — the
/// input here is a private key, and a client that echoed a bad one into a
/// terminal, a log or a bug report would leak exactly what it was handed.
fn parse_secret(raw: &str) -> Result<elo_vault::Zeroizing<[u8; 32]>, DepotError> {
    let hex = raw.trim();
    let hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    if hex.len() != 64 {
        return Err(DepotError::new(format!(
            "a private key is 64 hex characters (32 bytes); that was {}",
            hex.len()
        )));
    }
    // Bytes checked before slicing: a multibyte character across a cut
    // would panic rather than say "not hexadecimal".
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(DepotError::new("that is not hexadecimal — nothing was written"));
    }
    let mut out = elo_vault::Zeroizing::new([0u8; 32]);
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| DepotError::new("that is not hexadecimal — nothing was written"))?;
    }
    Ok(out)
}

fn report_new_account(signer: &elo_vault::LocalSigner, path: &Path, made: bool) {
    println!("{}", signer.address().unwrap_or_default());
    match signer.sol_address() {
        Some(sa) => println!("{sa}\n  one account: the first address is on Robinhood Chain, the second on Solana"),
        None => println!("  a bare key has no recovery words, so no Solana address — `elo account solana` \
                          takes a Solana private key"),
    }
    println!("  written to {}", path.display());
    // The sentences a holder needs BEFORE they rely on this, not after they
    // have lost it. There is no reset and no copy anywhere else — that is what
    // self-custody means and it is the part that surprises.
    if made {
        println!("  your recovery words bring this account back in any wallet; keep them offline.");
        println!("  the account is empty until something is sent to it.");
    } else {
        println!("  ⚠ if you imported a bare key, this file is its only copy here. Back it up.");
    }
    println!("  ⚠ the passphrase cannot be reset — nobody else has it, including us.");
}

/// The words, numbered, to stderr. Three columns of four, so a holder copies
/// them in order without counting.
fn show_words(phrase: &elo_vault::Phrase) {
    let w = phrase.words();
    let rows = w.len().div_ceil(3);
    eprintln!();
    eprintln!("  your recovery words — write them down, in order, on paper:");
    eprintln!();
    for r in 0..rows {
        let mut line = String::from("   ");
        for c in 0..3 {
            let i = c * rows + r;
            if let Some(word) = w.get(i) {
                line.push_str(&format!(" {:>2}. {:<10}", i + 1, word));
            }
        }
        eprintln!("{line}");
    }
    eprintln!();
    eprintln!("  anyone with these words has the account. We never see them, and");
    eprintln!("  elo cannot show them again. no screenshot, no cloud notes, no chat.");
    eprintln!();
}

fn load_json(source: &str) -> Result<Value, DepotError> {
    if source.starts_with("https://") || source.starts_with("http://") {
        let net = Net::new();
        let got = net.get_json(source);
        return match got.value {
            Some(v) => Ok(v),
            None if got.reachable => Err(DepotError::new(got.why)),
            None => Err(DepotError::new(format!("could not reach {source} — {}", got.why))),
        };
    }
    let raw = std::fs::read_to_string(source)
        .map_err(|e| DepotError::new(format!("{source}: {e}")))?;
    serde_json::from_str(&raw).map_err(|e| DepotError::new(format!("{source}: {e}")))
}

/// A manifest named as a **slug**, a path, or a url.
///
/// `elo install gates` is the shape a player types, and until this existed
/// the only shape that worked was a full url to a JSON file — which meant the
/// client could install a game only if you already knew where the game's
/// manifest was published. That is a storefront you cannot shop in.
///
/// Resolution order, and it is deliberately "local first":
///
/// 1. a scheme (`https://`, `http://`) → fetched as given,
/// 2. a path that EXISTS on this disk → read as a file. A file wins over the
///    origin so a title can be tested from a checkout without the origin
///    knowing anything about it,
/// 3. a bare slug (`[A-Za-z0-9_-]+`) → `{host}/api/launcher/manifest/{slug}`,
/// 4. anything else → an error naming all three, rather than a fetch of
///    something that looks like a url and is not.
///
/// Whatever it came from, path-absolute urls inside it are rebased onto the
/// origin that served it (`Manifest::rebase`) — an origin cannot write its own
/// public name into a download url safely, so it writes a path and the client,
/// which knows where it asked, completes it.
/// For [`load_manifest`] from a verb that installs nothing the manifest names
/// (`play`, `servers`, `publisher`): an http origin is a local test server
/// there, and reading it moves no bytes onto the disk.
const READS_ONLY: bool = true;

fn load_manifest(source: &str, host: &str, insecure: bool) -> Result<Manifest, DepotError> {
    load_manifest_on(source, host, insecure, None)
}

/// [`load_manifest`], on a beta branch when one is named.
fn load_manifest_on(
    source: &str,
    host: &str,
    insecure: bool,
    branch: Option<&str>,
) -> Result<Manifest, DepotError> {
    let (from, base) = manifest_source_on(source, host, insecure, branch)?;
    read_manifest(source, &from, &base)
}

/// A title's beta branch, as `--branch` names it: lowercase letters, digits
/// and `-`, the same alphabet the origin's `published.json` keys use. Anything
/// else is a refusal here rather than a query string we did not mean to send.
fn branch_flag(a: &args::Args) -> Result<Option<&str>, DepotError> {
    match a.flag("--branch") {
        None => Ok(None),
        Some(b)
            if !b.is_empty()
                && b.len() <= 32
                && b.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') =>
        {
            Ok(Some(b))
        }
        Some(b) => Err(DepotError::new(format!(
            "--branch {b:?} is not a branch name: lowercase letters, digits and '-'"
        ))),
    }
}

/// Where a manifest is read from, and the origin its relative urls complete
/// against. Every refusal here is about what was typed, not about the network,
/// so a caller can tell "that is not a title" from "could not look".
///
/// A `branch` is the origin's answer to
/// `/api/launcher/manifest/<slug>?branch=<name>`, so it only means something for
/// a bare slug: a url or a file is already exactly the manifest asked for.
fn manifest_source_on(source: &str, host: &str, insecure: bool, branch: Option<&str>)
                      -> Result<(String, String), DepotError> {
    let is_url = source.starts_with("https://") || source.starts_with("http://");
    let is_slug = !source.is_empty()
        && source
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    if branch.is_some() && (is_url || Path::new(source).is_file()) {
        return Err(DepotError::new(
            "--branch goes with a title's slug: a url or a file is already one exact manifest",
        ));
    }
    let (from, base) = if is_url {
        (source.to_string(), elo_depot::origin_of(source))
    } else if Path::new(source).is_file() {
        // A file's relative urls have no origin to complete them against, so
        // the host flag is the only sensible base — and it is the one the rest
        // of this command is already reading.
        (source.to_string(), host.to_string())
    } else if is_slug {
        let query = branch.map(|b| format!("?branch={b}")).unwrap_or_default();
        (
            format!("{host}/api/launcher/manifest/{source}{query}"),
            host.to_string(),
        )
    } else {
        return Err(DepotError::new(format!(
            "{source:?} is not a slug, a file, or an https url. Try `elo games` \
             to see what {host} serves."
        )));
    };

    // The manifest names the depot, and the depot names every hash — so a
    // manifest read over http lets whoever is on that hop pick them. Held to
    // the same flag as an http depot.
    if from.starts_with("http://") && !insecure {
        return Err(DepotError::new(format!(
            "{from} is http — a manifest must be https. Pass --allow-insecure \
             for a local test server."
        )));
    }
    Ok((from, base))
}

/// Fetch (or read) and parse the manifest [`manifest_source_on`] located.
fn read_manifest(source: &str, from: &str, base: &str) -> Result<Manifest, DepotError> {
    let is_url = source.starts_with("https://") || source.starts_with("http://");
    let mut m = manifest::parse_manifest(&load_json(from)?)?;
    m.rebase(base);
    if !is_url && Path::new(source).is_file() {
        // Rebased against the host, but it did not come from there: a file
        // claiming the host's slug must not be filed as the host's build.
        let at = std::fs::canonicalize(source).unwrap_or_else(|_| PathBuf::from(source));
        m.origin = format!("file:{}", at.display());
    }
    Ok(m)
}

/// ── the grant cache — `elo entitle` writes it, install/update read it ──────
/// One small file beside the games: {slug: {grant, expires, origin}}. A grant
/// is day-lived and worthless to anyone but its wallet's downloads, so a
/// plain file is the right amount of ceremony. Hand-off to the network layer
/// is the same env `ELO_GRANT` a user can set by hand — one mechanism, and
/// the origin-scoping in elo-net rides along for free.
fn grants_path(games: &Path) -> PathBuf {
    games.join("grants.json")
}

fn save_grant(games: &Path, slug: &str, origin: &str, grant: &str, expires: u64) -> Result<(), DepotError> {
    let path = grants_path(games);
    let mut all: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(m) = all.as_object_mut() {
        m.insert(slug.to_string(), serde_json::json!({
            "grant": grant, "expires": expires, "origin": origin }));
    }
    // A bearer for a paid download, so 0600 from the first byte — the keystore's
    // writer, not `fs::write` at the umask's 0644 beside a 0755 games root.
    let body = serde_json::to_string_pretty(&all).unwrap_or_default();
    elo_vault::write_private(&path, body.as_bytes(), false)
        .map_err(|e| DepotError::new(format!("{}: {e}", path.display())))
}

/// Load a cached, unexpired grant for this slug into the env elo-net reads.
/// An env the user set themselves always wins; a stale or missing cache is a
/// silent no-op — the depot will say 401 in plain words if a grant was needed.
fn use_cached_grant(games: &Path, slug: &str, host: &str) {
    if std::env::var("ELO_GRANT").is_ok_and(|g| !g.is_empty()) {
        return;
    }
    let Some(all) = std::fs::read_to_string(grants_path(games))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        return;
    };
    let Some(row) = all.get(slug) else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if row.get("expires").and_then(Value::as_u64).unwrap_or(0) <= now {
        return; // expired: `elo entitle <slug>` mints a fresh one
    }
    if let Some(g) = row.get("grant").and_then(Value::as_str) {
        std::env::set_var("ELO_GRANT", g);
        std::env::set_var(
            "ELO_GRANT_ORIGIN",
            row.get("origin").and_then(Value::as_str).unwrap_or(host),
        );
    }
}

// Kept so the unused-import lint does not hide a real gap: these are the two
// entry points a GUI will call and the CLI does not.
#[allow(dead_code)]
fn _unused(raw: &Value) -> Result<Depot, DepotError> {
    parse_depot(raw, false)
}

#[cfg(test)]
mod tests {
    /// `--branch` takes a branch's own alphabet and nothing else, and only
    /// goes with a slug: a url or a file is already one exact manifest.
    #[test]
    fn a_branch_is_a_name_and_goes_with_a_slug() {
        let parse = |v: &[&str]| super::args::parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        let a = parse(&["install", "gates", "--branch", "beta-2"]);
        assert_eq!(super::branch_flag(&a).unwrap(), Some("beta-2"));
        assert_eq!(super::branch_flag(&parse(&["install", "gates"])).unwrap(), None);
        for bad in ["Beta", "beta?x=1", "../x", ""] {
            assert!(super::branch_flag(&parse(&["install", "gates", "--branch", bad])).is_err(), "{bad:?}");
        }
        let e = super::load_manifest_on("https://example.com/m.json", "https://elopros.com", false, Some("beta"))
            .err()
            .expect("a url with a branch is refused");
        assert!(e.to_string().contains("slug"), "{e}");
    }

    /// The backstop keeps an error's lines and drops what would drive the
    /// terminal: an OSC 52 clipboard write and a screen clear.
    #[test]
    fn an_error_cannot_drive_the_terminal() {
        let e = "the relay refused this key: \x1b]52;c;MHhB\x07\x1b[2Jok\n  try again";
        let got = super::shown(&e);
        assert!(!got.contains('\x1b') && !got.contains('\x07'), "{got:?}");
        assert_eq!(got.lines().count(), 2, "a multi-line error keeps its lines: {got:?}");
        assert!(got.ends_with("  try again"), "{got:?}");
    }

    /// The grant cache is a bearer for a paid download: another user on the
    /// machine must not be able to read it, so it is 0600 and never widened.
    #[cfg(unix)]
    #[test]
    fn the_grant_cache_is_the_holders_alone() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("tmp");
        let games = root.path().join("games");
        super::save_grant(&games, "gates", "https://elopros.com", "g.1", u64::MAX).expect("save");
        let path = super::grants_path(&games);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "grants.json is a bearer");
        // A second grant keeps the first and the mode.
        super::save_grant(&games, "other", "https://elopros.com", "g.2", u64::MAX).expect("save");
        let all: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(all["gates"]["grant"], "g.1");
        assert_eq!(all["other"]["grant"], "g.2");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
