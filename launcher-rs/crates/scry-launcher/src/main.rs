//! `scry` — the desktop client's command half.
//!
//! Everything the GUI will do to a game, this does first and without a
//! display: install, verify, update, prune, launch. That ordering is not an
//! accident. A launcher whose only interface is a window cannot be tested on a
//! build machine, scripted by a player, or driven by an agent — and this
//! platform has agents as first-class players (`GATES.md` §0).

mod args;
mod dev;
mod mcp;
mod prompt;

// The account's address and signature both come through this trait, so the CLI
// and the window use one implementation rather than two that agree today.
use scry_broker::signer::Signer;
use scry_depot::session::{self, Ending, Session};
use scry_depot::publisher;
use scry_depot::update::{Checked, UpdateState};
use scry_depot::{
    install, installed, launch, manifest, parse_depot, plan, superseded, uninstall, verify,
    Depot, DepotError, Manifest, Plan,
};
use scry_hive::relay::{note_for, Presence, Relay, Status, HEARTBEAT};
use scry_hive::voice::Voice;
use scry_hive::who::{self, NameKind};
use scry_hive::{clock, render, Hive};
use scry_net::Net;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
scry — the desktop client

  scry check                       report whether this machine can run a build
  scry games                       every title the origin serves a manifest for
  scry list                        every installed build, from its receipt
  scry status  <title>             is a newer build published?
  scry install <title>             install the native build for this machine
  scry update  <title>             install it only if it is newer
  scry verify  <slug> [build]      re-hash an install against its receipt
  scry prune   <slug>              remove builds superseded by the newest
  scry uninstall <slug> <build>    remove one build
  scry play    <title>             start the installed build (or open the page)
  scry join    <link>              follow a friend's scry://join/… link
  scry servers <title>             the shards that title is serving, live
  scry digest  <depot>             the bytes32 a notary would commit
  scry hive [rooms|read|voice]     the town's talk annex, read
  scry hive whoami                 the voice this machine speaks with
  scry hive voice-new              make one (schnorr, kept here, never sent)
  scry hive say <text>             speak in the town stream or --room
  scry hive presence               hold this voice online until interrupted
  scry who [handle]                one identity — name, face, and who checked
  scry sessions                    what has been played, and what is running
  scry account                     the account on this machine (exit 3 if none)
  scry account new                 make one — generated here, transmitted nowhere
  scry account import              adopt a key you already have
  scry account sign <message>      sign with it (EIP-191), for a game or a claim
  scry signin  <code>              sign the website in with this account — type
                                   the code the browser shows, never a link
  scry publisher <title>           who signed this title's manifest, if anyone
  scry publisher <title> --statement   the exact bytes a publisher signs
  scry dev                         where this account stands on the store's desk
  scry dev edit <slug> --set k=v   edit your own listing's row
  scry dev image <slug> <file>     upload art for it, no commit needed
  scry dev post <slug> --title T   post a community update
  scry dev delegate <slug> <addr>  let your agent act for you, scoped
  scry dev help                    the whole desk
  scry prove <game> <server> <nonce>   sign in to a game server (SIWE, EIP-4361)
  scry mcp                         serve this machine's reads over MCP on stdio

<title> is a SLUG (`gates`), a path to a manifest, or an https url. A slug is
read from the origin at /api/launcher/manifest/<slug>; a local file wins over
the origin, so a title can be tested from a checkout. <depot> is a path or url.

  --games PATH        install root (default ~/.local/share/scry/games)
  --host URL          the origin to read (default https://scry.moreright.xyz)
  --room ID           which hive room; omit for the town stream
  --limit N           how many messages
  --detach            start a game without watching it — no playtime recorded
  --status ONLINE     online | away, for hive presence
  --quiet             do not hold hive presence while playing
  --platform TAG      override this machine's platform tag
  --server ADDR       passed to the game as {server}
  --wallet ADDR       passed to the game as {wallet}
  --browser CMD       browser command; `{url}` marks where the url goes,
                      which is what app mode needs: 'chromium --app={url}'
  --json              machine-readable output
  --no-watch          after `signin`, exit instead of staying to approve
                      the page's signature requests
  --no-reuse          download every file, even ones already on disk
  --allow-insecure    permit an http depot root (a local test server)
  --dry-run           say what would happen, do nothing
  --yes               do not ask before removing anything
";

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args::parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("scry: {e}");
            std::process::exit(2);
        }
    };
    if parsed.verb.is_empty() || parsed.verb == "help" || parsed.verb == "--help" {
        print!("{USAGE}");
        return;
    }
    let parsed = args::normalize_link(parsed);
    match run(&parsed) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("scry: {e}");
            std::process::exit(1);
        }
    }
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
        .unwrap_or("https://scry.moreright.xyz")
        .trim_end_matches('/')
        .to_string();
    let json_out = a.has("--json");

    match a.verb.as_str() {
        "check" => {
            let net = Net::new();
            println!("scry launcher {}", env!("CARGO_PKG_VERSION"));
            println!("platform    {platform}");
            println!("games       {}", games.display());
            println!("installs    {}", installed(&games).len());
            // ⚠ This probed a HARD-CODED origin while every other verb honours
            // `--host`, so `scry check --host http://localhost:3600` reported on
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
                (Some(there), _) => match scry_depot::update::version_cmp(there, mine) {
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
            let ks = scry_vault::keystore_path();
            match scry_vault::LocalSigner::at(&ks).address() {
                Some(addr) => println!("account     {addr}"),
                None => {
                    println!("account     none — `scry account new` makes one \
                              (optional; games play anonymously)");
                }
            }
            Ok(0)
        }

        // ── the account ──────────────────────────────────────────────────────
        //
        // The headless half of the Account window, and for a while the ONLY
        // half that worked: `scry-vault` could make a key from the day it
        // landed, and no path a user could reach ever asked it to — `create`
        // and `unlock` were called from tests and nowhere else. A client whose
        // only way to hold a key is a desktop window cannot hold one on a
        // server, in CI, or under an agent, which on a platform that calls
        // agents first-class players (`GATES.md` §0) is the wrong half to have
        // shipped first.
        //
        // ⚠ **Nothing in this verb touches the network.** The key is generated
        // here from the OS pool, encrypted here, and written here. scry the
        // service never sees it — `CLAUDE.md` invariant 7 is about custody, and
        // this is the holder's own machine holding the holder's own key.
        "account" => {
            let path = scry_vault::keystore_path();
            match a.positional.first().map(String::as_str).unwrap_or("show") {
                "show" => {
                    let signer = scry_vault::LocalSigner::at(&path);
                    if json_out {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "exists": signer.exists(),
                                "address": signer.address(),
                                "path": path.to_string_lossy(),
                            }))
                            .unwrap_or_default()
                        );
                        return Ok(if signer.exists() { 0 } else { 3 });
                    }
                    match signer.address() {
                        Some(addr) => {
                            println!("{addr}");
                            println!("  locked — a passphrase unlocks it for one command");
                            println!("  {}", path.display());
                            Ok(0)
                        }
                        None => {
                            // Not an error. A machine with no account is a
                            // normal machine and games play anonymously; 3 is
                            // this CLI's "unknown/absent", the same code
                            // `scry status` uses, so a script can branch.
                            println!("no account on this machine");
                            println!(
                                "  `scry account new` makes one · \
                                 `scry account import` adopts one you already have"
                            );
                            println!("  it would live at {}", path.display());
                            Ok(3)
                        }
                    }
                }

                "new" => {
                    refuse_if_occupied(&path)?;
                    warn_if_echoing();
                    let pass = passphrase_twice()?;
                    let signer = scry_vault::LocalSigner::create(&path, &pass)
                        .map_err(DepotError::new)?;
                    report_new_account(&signer, &path);
                    Ok(0)
                }

                // The counterpart to `new`, and the reason both exist: a player
                // with a wallet already should not have to abandon it to use
                // this client.
                "import" => {
                    refuse_if_occupied(&path)?;
                    warn_if_echoing();
                    let raw = prompt::secret("the private key to adopt (64 hex): ")
                        .map_err(DepotError::new)?;
                    let secret = parse_secret(&raw)?;
                    let pass = passphrase_twice()?;
                    let signer = scry_vault::LocalSigner::import(&path, &pass, &secret)
                        .map_err(DepotError::new)?;
                    report_new_account(&signer, &path);
                    Ok(0)
                }

                // Why this verb is the point of the whole command: an address
                // is a claim, and a signature is the only thing that makes it
                // mean anything. Without this the CLI could hold a key and
                // never use it.
                "sign" => {
                    let message = need(a.positional.get(1), "a message to sign")?;
                    let mut signer = scry_vault::LocalSigner::at(&path);
                    if !signer.exists() {
                        return Err(DepotError::new(format!(
                            "no account at {} — `scry account new` makes one",
                            path.display()
                        )));
                    }
                    warn_if_echoing();
                    let pass = prompt::secret("passphrase: ").map_err(DepotError::new)?;
                    // A wrong passphrase is a WRONG PASSPHRASE and never
                    // "corrupt" — the vault draws that line and this must not
                    // blur it back by rewording the error.
                    signer.unlock(&pass).map_err(DepotError::new)?;
                    let out = signer
                        .sign(message, "scry account sign")
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

                other => Err(DepotError::new(format!(
                    "unknown account command {other:?} — try show, new, import or sign"
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
            scry_broker::protocol::check_slug(game)
                .map_err(|r| DepotError::new(r.reason))?;
            scry_broker::protocol::check_domain(server)
                .map_err(|r| DepotError::new(r.reason))?;
            scry_broker::protocol::check_nonce(nonce)
                .map_err(|r| DepotError::new(r.reason))?;
            // ⚠ SIWE binds the address into the signed bytes, so unlike the old
            // format this cannot be composed before the account is known. The
            // keystore moved ahead of `--statement` for that reason: printing a
            // message with nobody's address in it would print something that can
            // never verify.
            let path = scry_vault::keystore_path();
            let mut signer = scry_vault::LocalSigner::at(&path);
            if !signer.exists() {
                return Err(DepotError::new(format!(
                    "no account at {} — `scry account new` makes one",
                    path.display()
                )));
            }
            let address = signer
                .address()
                .ok_or_else(|| DepotError::new("the keystore names no address"))?;
            let message = scry_broker::protocol::prove_message(
                server,
                &address,
                game,
                nonce,
                &scry_broker::protocol::now_iso8601(),
            );
            if a.has("--statement") {
                print!("{message}");
                return Ok(0);
            }
            warn_if_echoing();
            let pass = prompt::secret("passphrase: ").map_err(DepotError::new)?;
            signer.unlock(&pass).map_err(DepotError::new)?;
            let out = signer
                .sign(&message, "prove identity")
                .map_err(|r| DepotError::new(r.reason))?;
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
            let path = scry_vault::keystore_path();
            let mut signer = scry_vault::LocalSigner::at(&path);
            if !signer.exists() {
                return Err(DepotError::new(format!(
                    "no account at {} — `scry account new` makes one (or `import`)",
                    path.display()
                )));
            }
            let address = signer
                .address()
                .ok_or_else(|| DepotError::new("the keystore names no address"))?;
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
            warn_if_echoing();
            let pass = prompt::secret("passphrase: ").map_err(DepotError::new)?;
            signer.unlock(&pass).map_err(DepotError::new)?;
            let out = signer
                .sign(&message, "scry entitle")
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
            println!("entitled — {address} holds a ticket for {slug}");
            println!("  the grant is cached for the day; install and update use it on their own");
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
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
        "signin" => {
            let raw = need(a.positional.first(), "the code the website shows")?;
            let code: String = raw
                .to_ascii_uppercase()
                .chars()
                .filter(|c| *c != '-')
                .collect();
            // The code IS the EIP-4361 nonce, so the nonce rule is the shape
            // check — at least 8, alphanumeric only.
            scry_broker::protocol::check_nonce(&code).map_err(|r| {
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
            let path = scry_vault::keystore_path();
            let mut signer = scry_vault::LocalSigner::at(&path);
            if !signer.exists() {
                return Err(DepotError::new(format!(
                    "no account at {} — `scry account new` makes one (or `import`). \
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

            let message = scry_broker::protocol::signin_message(
                &domain,
                &address,
                &code,
                &scry_broker::protocol::now_iso8601(),
            );
            println!("about to sign exactly this, and nothing else:");
            println!("──");
            println!("{message}");
            println!("──");
            warn_if_echoing();
            let pass = prompt::secret("passphrase: ").map_err(DepotError::new)?;
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
            println!("signed in — the browser showing {code} now reads as {address}");
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
                        println!("the pairing expired — done. `scry signin` a fresh code to reattach.");
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
                    println!("\nthe page asks for a signature over exactly this:");
                    println!("──");
                    println!("{text}");
                    println!("──");
                    if !why.is_empty() {
                        // The page's own stated reason — a claim, and drawn as
                        // one, the same rule the broker's consent prompt holds.
                        println!("(the page says: {why})");
                    }
                    let yes = prompt::confirm("sign it? [y/N] ").unwrap_or(false);
                    let body = if yes {
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
        //     scry publisher gates --statement | xargs -0 scry account sign
        "publisher" => {
            let m = load_manifest(need(a.positional.first(), "a title")?, &host)?;
            if a.has("--statement") {
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
                    scry_depot::install::human(i.total_bytes),
                    i.depot_digest
                );
            }
            Ok(0)
        }

        "digest" => {
            let raw = load_json(need(a.positional.first(), "a depot")?)?;
            println!("{}", scry_depot::digest(&raw)?);
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
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(0);
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
            // The depot root's own state, because a manifest whose native row
            // is empty because the origin could not look is not the same
            // manifest as one with nothing published.
            if let Some(d) = v.get("depots") {
                if d.get("reachable").and_then(|r| r.as_bool()) == Some(false) {
                    println!(
                        "\n! the origin could not read its depot root — {}",
                        d.get("why").and_then(|w| w.as_str()).unwrap_or("no reason given")
                    );
                    println!("  a native row missing here means UNKNOWN, not unpublished.");
                    return Ok(3);
                }
            }
            Ok(0)
        }


        "hive" => {
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
                            format!("could not reach the hive — {}", got.why)
                        }));
                    };
                    println!("relay {}", c.relay.as_deref().unwrap_or("none posted"));
                    if c.channels.is_empty() {
                        println!("the hive posted no rooms");
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
                        println!("no voice on this machine — `scry hive voice-new` makes one");
                        return Ok(0);
                    }
                    let v = Voice::load(&dir).map_err(DepotError::from)?;
                    println!("npub    {}", v.npub());
                    println!("key     {}", dir.join(scry_hive::voice::KEY_NAME).display());
                    // Said on the surface every time, not once in a doc. This
                    // key can speak as you and cannot touch anything else, and
                    // both halves of that matter.
                    println!("\nThis is a SPEAKING key, not a wallet — it holds no funds and signs");
                    println!("no transaction. It sits at 0600 unencrypted, so anyone who can read");
                    println!("your home directory can speak as you in the hive. scry never sees it.");
                    Ok(0)
                }
                "voice-new" => {
                    let dir = voice_dir();
                    if Voice::exists(&dir) && !a.has("--yes") {
                        // Overwriting a voice loses the identity everything
                        // said under it, so it takes an explicit act.
                        return Err(DepotError::new(format!(
                            "a voice already exists at {} — `--yes` replaces it, and \
                             everything said under the old npub stays under the old npub",
                            dir.join(scry_hive::voice::KEY_NAME).display()
                        )));
                    }
                    let v = Voice::generate().map_err(DepotError::from)?;
                    let path = v.save(&dir).map_err(DepotError::from)?;
                    println!("npub    {}", v.npub());
                    println!("saved   {}", path.display());
                    println!("\nBack it up as an nsec if you want this identity to survive this");
                    println!("machine. It is a speaking key: no funds, no transactions, and scry");
                    println!("never sees it. Bind it to a wallet later with the hive page if you");
                    println!("want a sworn name rather than a bare key.");
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
                    let url = relay_url(&hive)?;
                    let room = a.flag("--room");
                    let event = v
                        .sign(note_for(room, text, clock::now()))
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
                            format!("could not reach the hive — {}", got.why)
                        }));
                    };
                    println!("the beekeeper's outbox — {} composed, {} queued", v.composed, v.queued);
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
                // town stream rather than erroring — a chat client should not
                // punish a typo by refusing to show the room.
                _ => {
                    let got = hive.room(a.flag("--room"), limit);
                    let Some(r) = got.value else {
                        return Err(DepotError::new(if got.reachable {
                            got.why
                        } else {
                            format!("could not reach the hive — {}", got.why)
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
                    for m in &rows {
                        // One resolver, the same one `scry who` uses.
                        let id = who::from_message(m, &host);
                        println!(
                            "{}",
                            render::chat_line(
                                &clock::ago(m.at, now),
                                &id.name,
                                id.kind.sworn(),
                                &m.content,
                                100
                            )
                        );
                    }
                    println!("\n~ = a name nobody checked; blank = sworn on the register");
                    Ok(0)
                }
            }
        }

        "who" => {
            // The wallet IS the account, so a bare `scry who` resolves the
            // watched address. One name, one face, one place they come from.
            let handle = match a.positional.first() {
                Some(h) => h.clone(),
                None => a
                    .flag("--wallet")
                    .map(str::to_string)
                    .or_else(|| std::env::var("SCRY_WALLET").ok())
                    .ok_or_else(|| DepotError::new(
                        "no handle, and no wallet is set — pass one, use --wallet, \
                         or set SCRY_WALLET"))?,
            };
            who::check_handle(&handle).map_err(DepotError::from)?;
            let net = Net::new();
            let got = who::resolve(&net, &host, &handle);
            let Some(id) = got.value else {
                return Err(DepotError::new(if got.reachable {
                    got.why
                } else {
                    format!("could not reach the register — {}", got.why)
                }));
            };
            if json_out {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "name": id.name, "name_kind": format!("{:?}", id.kind).to_lowercase(),
                    "sworn": id.kind.sworn(), "face": id.face, "wallet": id.wallet,
                    "npub": id.npub, "vow_id": id.vow_id, "nip05": id.nip05,
                    "bare": id.bare, "vows": id.vows, "ambiguous": id.ambiguous(),
                })).unwrap_or_default());
                return Ok(0);
            }
            println!("name    {}", id.name);
            println!("        {}", id.kind.line());
            println!("face    {}", id.face);
            for (label, v) in [("wallet", &id.wallet), ("npub", &id.npub),
                               ("vow", &id.vow_id), ("nip05", &id.nip05)] {
                if let Some(v) = v {
                    println!("{label:<7} {v}");
                }
            }
            if id.ambiguous() {
                // Never quietly present one of several names as THE name.
                println!("\n⚠ This wallet swore {} vows, so it answers to {} names. The", id.vows, id.vows);
                println!("account is the WALLET — reputation and the house hang off the address,");
                println!("not off any one vow — but the wallet has no name or face of its own");
                println!("yet, so the one above is whichever vow the register picked.");
            }
            if id.bare {
                // Not an error and not an empty read: a bare wallet is a real
                // answer, and the rule is that it still gets a page.
                println!("\nNothing is sworn to this address yet. It still has a name and a");
                println!("face — the register derives both — and no account exists to require.");
            }
            if id.kind == NameKind::Declared {
                println!("\nThis name is self-declared. It is wallet-signed, so it is THEIRS,");
                println!("but nobody checked that it is true. A sworn name lives on the");
                println!("register and cannot be set by asserting it.");
            }
            Ok(0)
        }

        "mcp" => {
            // Read-only by construction — see `mcp.rs`. Installing, launching
            // and speaking stay on the CLI where a person typed them.
            mcp::run(a, &games, &state_root(), &host, &platform)
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
            let m = load_manifest(need(a.positional.first(), "a manifest")?, &host)?;
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
            let m = load_manifest(need(a.positional.first(), "a manifest")?, &host)?;
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
                             scry entitle {slug}\n\
                             then re-run `scry {verb} …`. No ticket yet? the price and the\n\
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
            let newest = installed(&games)
                .into_iter().rfind(|i| &i.slug == slug)
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
                    scry_depot::install::human(i.total_bytes)
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
            let m = load_manifest(need(a.positional.first(), "a manifest")?, &host)?;
            // `{servers}` comes from the MANIFEST, never from a flag: the shard
            // list is the title's to name and it has already named it in the
            // document we just loaded, so the game's own browser and the
            // Servers window cannot disagree about what is up.
            let values = args::launch_values(a, m.servers_url.as_deref());
            // Prefer an installed native build; fall back to the browser row;
            // refuse rather than guess when neither is published.
            //
            // `next_back`, not `find` — builds sort by name and a title can
            // have several installed at once, so the first match is the OLDEST.
            // Starting the build a player just updated away from is the kind of
            // bug that reads as "the patch did nothing".
            if let Some(i) = installed(&games).into_iter().rfind(|i| i.slug == m.slug) {
                let l = launch::resolve(&i, &values, &BTreeMap::new())?;
                if a.has("--dry-run") {
                    println!("{}", l.argv.join(" "));
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
                    let argv = scry_depot::browser_argv(&url, a.flag("--browser").unwrap_or(""))?;
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
            let m = load_manifest(need(a.positional.first(), "a manifest")?, &host)?;
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
                let Some(su) = r.status_url.clone() else { continue };
                if let Some(s) = net.status(&su).value {
                    r.apply_status(&s);
                }
            }

            if json_out {
                let out: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
                    "id": r.id, "name": r.name, "addr": r.addr,
                    "players": r.players, "max_players": r.max_players,
                    "map": r.map, "full": r.full(),
                    "link": scry_depot::deeplink::link_for(&m.slug, &r.addr).ok(),
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
                if let Ok(link) = scry_depot::deeplink::link_for(&m.slug, &r.addr) {
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
            let j = scry_depot::deeplink::parse(link)?;
            println!("joining {} on {}", j.slug, j.addr);

            let mut play = a.clone();
            play.verb = "play".into();
            play.positional = vec![j.slug.clone()];
            // The link wins over a typed `--server`: resolving one is this
            // verb's entire purpose, so honouring a flag over it would make
            // `scry join <link> --server X` silently ignore the link.
            play.flags.insert("--server".into(), j.addr.clone());
            run(&play).map_err(|e| {
                // `play` refuses an uninstalled title correctly but says
                // nothing about what to do next, and arriving from a link is
                // exactly when a player does not have the game yet.
                DepotError::new(format!(
                    "{e}\n  the link is good — install it first: scry install {}",
                    j.slug
                ))
            })
        }

        other => Err(DepotError::new(format!(
            "unknown command {other:?} — try `scry help`"
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
    scry_depot::check(m, platform, games, &net, insecure)
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
    let mut held = match Presence::hold(&url, voice, Status::Online) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("scry: hive presence unavailable ({e}) — playing anyway");
            return None;
        }
    };
    println!("hive: {} online while you play {slug}", held.npub());

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let joiner = std::thread::spawn(move || {
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

/// The relay the house posted. Never typed by us — one place, so a fork or a
/// local origin moves it for every verb at once.
fn relay_url(hive: &Hive) -> Result<String, DepotError> {
    let got = hive.channels();
    let Some(c) = got.value else {
        return Err(DepotError::new(if got.reachable {
            got.why
        } else {
            format!("could not reach the hive — {}", got.why)
        }));
    };
    c.relay
        .filter(|r| !r.is_empty())
        .ok_or_else(|| DepotError::new("the house has posted no relay"))
}

fn voice_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SCRY_VOICE_DIR") {
        return PathBuf::from(p);
    }
    state_root().join("voice")
}

fn state_root() -> PathBuf {
    if let Ok(p) = std::env::var("SCRY_STATE") {
        return PathBuf::from(p);
    }
    home_based("XDG_STATE_HOME", ".local/state").join("scry")
}

/// The XDG dir on unix, `%LOCALAPPDATA%` on Windows — and never a path rooted
/// at `/` because an env var was missing.
///
/// ⚠ **This is why it is a function.** Both callers read `$HOME` directly and
/// `unwrap_or_default()`'d it, which is empty on Windows — `HOME` is not set
/// there, `USERPROFILE` is. That produced `/.local/share/scry/games`: an
/// absolute path at the filesystem root, which is the wrong place on unix with
/// no `HOME` and unwritable nonsense on Windows. An empty base is now a real
/// failure rather than a silent root-relative write, and it falls back to the
/// current directory so a game lands somewhere the caller can find rather than
/// somewhere they cannot.
fn home_based(xdg_var: &str, unix_tail: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let _ = (xdg_var, unix_tail);
        for var in ["LOCALAPPDATA", "APPDATA", "USERPROFILE"] {
            if let Ok(p) = std::env::var(var) {
                if !p.is_empty() {
                    return PathBuf::from(p);
                }
            }
        }
        PathBuf::from(".")
    }
    #[cfg(not(windows))]
    {
        if let Ok(p) = std::env::var(xdg_var) {
            if !p.is_empty() {
                return PathBuf::from(p);
            }
        }
        match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => PathBuf::from(h).join(unix_tail),
            _ => PathBuf::from("."),
        }
    }
}

fn games_root(a: &args::Args) -> PathBuf {
    if let Some(p) = a.flag("--games") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("SCRY_GAMES") {
        return PathBuf::from(p);
    }
    home_based("XDG_DATA_HOME", ".local/share")
        .join("scry")
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

fn passphrase_twice() -> Result<String, DepotError> {
    let pass = prompt::secret("a passphrase for this account: ").map_err(DepotError::new)?;
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
fn parse_secret(raw: &str) -> Result<[u8; 32], DepotError> {
    let hex = raw.trim();
    let hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    if hex.len() != 64 {
        return Err(DepotError::new(format!(
            "a private key is 64 hex characters (32 bytes); that was {}",
            hex.len()
        )));
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| DepotError::new("that is not hexadecimal — nothing was written"))?;
    }
    Ok(out)
}

fn report_new_account(signer: &scry_vault::LocalSigner, path: &Path) {
    println!("{}", signer.address().unwrap_or_default());
    println!("  written to {}", path.display());
    // The two sentences a holder needs BEFORE they rely on this file, not after
    // they have lost it. There is no reset here and there is no copy anywhere
    // else — that is what self-custody means and it is the part that surprises.
    println!("  ⚠ this file is the only copy of the key. Back it up.");
    println!("  ⚠ the passphrase cannot be reset — nobody else has it, including scry.");
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
/// `scry install gates` is the shape a player types, and until this existed
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
fn load_manifest(source: &str, host: &str) -> Result<Manifest, DepotError> {
    let is_url = source.starts_with("https://") || source.starts_with("http://");
    let is_slug = !source.is_empty()
        && source
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    let (from, base) = if is_url {
        (source.to_string(), origin_of(source))
    } else if Path::new(source).is_file() {
        // A file's relative urls have no origin to complete them against, so
        // the host flag is the only sensible base — and it is the one the rest
        // of this command is already reading.
        (source.to_string(), host.to_string())
    } else if is_slug {
        (
            format!("{host}/api/launcher/manifest/{source}"),
            host.to_string(),
        )
    } else {
        return Err(DepotError::new(format!(
            "{source:?} is not a slug, a file, or an https url. Try `scry games` \
             to see what {host} serves."
        )));
    };

    let mut m = manifest::parse_manifest(&load_json(&from)?)?;
    m.rebase(&base);
    Ok(m)
}

/// ── the grant cache — `scry entitle` writes it, install/update read it ──────
/// One small file beside the games: {slug: {grant, expires, origin}}. A grant
/// is day-lived and worthless to anyone but its wallet's downloads, so a
/// plain file is the right amount of ceremony. Hand-off to the network layer
/// is the same env `SCRY_GRANT` a user can set by hand — one mechanism, and
/// the origin-scoping in scry-net rides along for free.
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
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| DepotError::new(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&all).unwrap_or_default())
        .map_err(|e| DepotError::new(format!("{}: {e}", path.display())))
}

/// Load a cached, unexpired grant for this slug into the env scry-net reads.
/// An env the user set themselves always wins; a stale or missing cache is a
/// silent no-op — the depot will say 401 in plain words if a grant was needed.
fn use_cached_grant(games: &Path, slug: &str, host: &str) {
    if std::env::var("SCRY_GRANT").is_ok_and(|g| !g.is_empty()) {
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
        return; // expired: `scry entitle <slug>` mints a fresh one
    }
    if let Some(g) = row.get("grant").and_then(Value::as_str) {
        std::env::set_var("SCRY_GRANT", g);
        std::env::set_var(
            "SCRY_GRANT_ORIGIN",
            row.get("origin").and_then(Value::as_str).unwrap_or(host),
        );
    }
}

/// `scheme://host[:port]` of a url, with no url crate. Everything after the
/// third `/` is dropped, which is all a base needs to be.
fn origin_of(url: &str) -> String {
    let Some(rest) = url.find("://").map(|i| i + 3) else {
        return url.trim_end_matches('/').to_string();
    };
    match url[rest..].find('/') {
        Some(i) => url[..rest + i].to_string(),
        None => url.trim_end_matches('/').to_string(),
    }
}

// Kept so the unused-import lint does not hide a real gap: these are the two
// entry points a GUI will call and the CLI does not.
#[allow(dead_code)]
fn _unused(raw: &Value) -> Result<Depot, DepotError> {
    parse_depot(raw, false)
}
