//! `elo friends` — who you play with, and where they are right now.
//!
//! The origin half is `meter/friends.py`; the document is parsed by
//! `elo_net::friends`, which the Friends window reads too, so the terminal and
//! the window say the same sentence about the same friend.
//!
//! Reading needs no key: a list is public, like playtime. Every change is the
//! dev desk's three beats (`dev.rs`) — **prepare, sign, submit** — so a
//! refusal arrives in the origin's own words before anything is signed. The
//! text signed is rebuilt here (`elo_net::friends::sign_text`, the builder the
//! Friends window uses too) and must match the one the origin offers, so a
//! prepared text is never signed on the origin's word.
//!
//! Following a friend is following their link. `join` hands the link back to
//! the dispatcher, which runs it through `elo join` exactly as a pasted one,
//! and that becomes `play` — no third launch path.

use crate::args::Args;
use crate::signer;
use elo_broker::signer::Signer;
use elo_depot::DepotError;
use elo_hive::render::{clip, safe_line};
use elo_hive::{clock, who};
use elo_net::friends::{self as roster, Friend, View};
use elo_net::{Fetched, Net};
use serde_json::{json, Value};

pub const USAGE: &str = "\
elo friends — who you play with, and where they are

  elo friends                      your friends, and what each one is playing
  elo friends add <who>            ask a wallet (0x…) or a name to be friends
  elo friends accept <who>         say yes to an ask
  elo friends decline <who>        say no
  elo friends cancel <who>         take back your own ask
  elo friends remove <who>         end a friendship
  elo friends block <who>          remove them and refuse their asks
  elo friends unblock <who>
  elo friends join <who>           join the server a friend is playing on

  --wallet ADDR    whose list to read (default: ELO_WALLET, then this
                   machine's signer)
  --dry-run        prepare a change and print what would be signed
  --json           machine-readable output
";

/// What the verb came to. `Join` carries a link the parser accepted, for the
/// dispatcher to run as `elo join <link>`.
pub enum Done {
    Code(i32),
    Join(String),
}

/// The words typed, and the op each one sends. `add` is the terminal's word
/// for the wire's `ask`; the arg each op takes is `elo_net::friends::OPS`, the
/// table the Friends window signs from too.
pub const WORDS: [(&str, &str); 7] = [
    ("add", "ask"),
    ("accept", "accept"),
    ("decline", "decline"),
    ("cancel", "cancel"),
    ("remove", "remove"),
    ("block", "block"),
    ("unblock", "unblock"),
];

/// `(op, arg)` for a typed word, or `None` when it is not a change.
pub fn op_for(word: &str) -> Option<(&'static str, &'static str)> {
    let op = WORDS.iter().find(|(w, _)| *w == word).map(|(_, op)| *op)?;
    Some((op, roster::op_key(op)?))
}

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<Done, DepotError> {
    let net = Net::new();
    match a.positional.first().map(String::as_str) {
        None => list(&net, a, host, json_out).map(Done::Code),
        Some("help") => {
            print!("{USAGE}");
            Ok(Done::Code(0))
        }
        Some("join") => join(&net, a, host).map(Done::Join),
        Some(word) => match op_for(word) {
            Some((op, key)) => change(&net, a, host, word, op, key, json_out).map(Done::Code),
            None => Err(DepotError::new(format!(
                "unknown friends command {:?} — `elo friends help` lists them",
                safe_line(word)
            ))),
        },
    }
}

/// Whose list to read: `--wallet`, then `ELO_WALLET`, then the address this
/// machine signs as. Reading the signer's address costs no passphrase.
pub fn whose(a: &Args, host: &str) -> Result<String, DepotError> {
    let typed = a
        .flag("--wallet")
        .map(str::to_string)
        .or_else(|| std::env::var("ELO_WALLET").ok().filter(|w| !w.trim().is_empty()));
    let wallet = match typed {
        Some(w) => w,
        None => signer::choose(a, host)
            .ok()
            .and_then(|s| s.address())
            .ok_or_else(|| {
                DepotError::new(
                    "whose friends? — pass --wallet 0x…, set ELO_WALLET, or give this \
                     machine a wallet (`elo pair` or `elo account new`)",
                )
            })?,
    };
    if !roster::is_wallet(&wallet) {
        return Err(DepotError::new(format!(
            "{} is not a wallet — 0x and forty hex digits",
            safe_line(&wallet)
        )));
    }
    Ok(wallet.trim().to_ascii_lowercase())
}

/// One read, with "we could not look" kept apart from "it said no".
fn fetch(net: &Net, host: &str, wallet: &str) -> Result<View, DepotError> {
    let got = roster::read(net, host, wallet);
    match got.value {
        Some(v) => Ok(v),
        None if !got.reachable => Err(DepotError::new(format!(
            "could not reach {host} — {}",
            safe_line(&got.why)
        ))),
        None => Err(DepotError::new(format!(
            "the friends list did not read — {}",
            safe_line(&got.why)
        ))),
    }
}

fn list(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let me = whose(a, host)?;
    let view = fetch(net, host, &me)?;
    let now = clock::now();
    if json_out {
        println!("{}", serde_json::to_string_pretty(&view_json(&view, now)).unwrap_or_default());
        return Ok(0);
    }
    for line in render(&view, now) {
        println!("{line}");
    }
    Ok(0)
}

/// The list as terminal lines. Every string the origin wrote goes through
/// `safe_line` first: a friend chooses their own name, and a name is not
/// allowed to drive the terminal it is printed on.
pub fn render(view: &View, now: i64) -> Vec<String> {
    let mut out = Vec::new();
    let n = view.friends.len();
    out.push(format!(
        "{} · {n} friend{}",
        safe_line(&roster::short(&view.wallet)),
        if n == 1 { "" } else { "s" }
    ));
    if !view.presence.reachable {
        out.push(format!(
            "  presence could not be read{}",
            view.presence
                .why
                .as_deref()
                .map(|w| format!(" — {}", safe_line(w)))
                .unwrap_or_default()
        ));
    }
    if view.friends.is_empty() {
        out.push("  no friends yet — elo friends add <wallet or name>".into());
    }
    let row = |out: &mut Vec<String>, f: &Friend| {
        out.push(format!(
            "  {:<20} {}",
            clip(&safe_line(&label(f)), 20),
            safe_line(&f.presence_line(&view.presence, now))
        ));
        out.push(format!("  {:<20} {}", "", safe_line(&f.wallet)));
        // A link off a ledger nobody read points at a place nobody measured.
        if let Some(link) = f.join_link().filter(|_| view.presence.reachable) {
            out.push(format!("  {:<20} {link}", ""));
        }
    };
    for f in &view.friends {
        row(&mut out, f);
    }
    // Your own agents: listed with no ask, and nothing here changes them.
    if !view.buddies.is_empty() {
        out.push(String::new());
        out.push("your agents".into());
        for b in &view.buddies {
            row(&mut out, b);
        }
    }
    for (head, asks, hint) in [
        ("asking you", &view.asks_in, "elo friends accept <wallet>"),
        ("you asked", &view.asks_out, "elo friends cancel <wallet>"),
    ] {
        if asks.is_empty() {
            continue;
        }
        out.push(String::new());
        out.push(format!("{head} — {hint}"));
        for ask in asks {
            out.push(format!(
                "  {:<20} {}",
                clip(&safe_line(&ask.label()), 20),
                safe_line(&ask.wallet)
            ));
        }
    }
    out
}

/// A row's title, with agents marked: an agent is a player here, and saying
/// which is which is the whole of the courtesy.
fn label(f: &Friend) -> String {
    if f.agent || f.buddy.is_some() {
        format!("{} (agent)", f.label())
    } else {
        f.label()
    }
}

/// The list as JSON — `elo_net::friends::view_json`, the shape `elo mcp`
/// answers with too, every origin string through `safe_line`.
fn view_json(view: &View, now: i64) -> Value {
    roster::view_json(view, now, safe_line)
}

/// A wallet or a name, as `(wallet, how to call them)`. A wallet is taken as
/// typed; a name goes through the register, the same lookup `elo who` makes.
fn wallet_of(net: &Net, host: &str, handle: &str) -> Result<(String, String), DepotError> {
    let h = handle.trim();
    if roster::is_wallet(h) {
        return Ok((h.to_ascii_lowercase(), roster::short(h)));
    }
    who::check_handle(h).map_err(DepotError::from)?;
    let got = who::resolve(net, host, h);
    let Some(id) = got.value else {
        return Err(DepotError::new(if got.reachable {
            safe_line(&got.why)
        } else {
            format!("could not reach the register — {}", safe_line(&got.why))
        }));
    };
    match id.wallet.filter(|w| roster::is_wallet(w)) {
        Some(w) => Ok((w.trim().to_ascii_lowercase(), id.name)),
        None => Err(DepotError::new(format!(
            "no wallet goes by {} — use their 0x address",
            safe_line(h)
        ))),
    }
}

/// prepare → sign → submit, for one change.
fn change(
    net: &Net,
    a: &Args,
    host: &str,
    word: &str,
    op: &str,
    key: &str,
    json_out: bool,
) -> Result<i32, DepotError> {
    let handle = a
        .positional
        .get(1)
        .ok_or_else(|| DepotError::new(format!("{word} who? — a wallet (0x…) or a name")))?;
    let (other, called) = wallet_of(net, host, handle)?;
    let called = safe_line(&called);

    // A change is signed, so it is made as the wallet that signs — never as a
    // typed one. A `--wallet` naming someone else is refused rather than
    // quietly overridden.
    let mut signer = signer::choose(a, host).map_err(DepotError::new)?;
    let me = signer
        .address()
        .filter(|w| roster::is_wallet(w))
        .ok_or_else(|| DepotError::new("the signer names no address"))?
        .trim()
        .to_ascii_lowercase();
    if let Some(typed) = a.flag("--wallet") {
        if typed.trim().to_ascii_lowercase() != me {
            return Err(DepotError::new(format!(
                "--wallet names {}, and this machine signs as {me} — drop --wallet",
                safe_line(typed)
            )));
        }
    }
    if other == me {
        return Err(DepotError::new("that is your own wallet"));
    }

    let prepared = told(net.post_json_told(
        &format!("{host}/api/friends/prepare"),
        &json!({"wallet": me, "op": op, "args": {key: other}}),
    ))?;
    let offered = prepared
        .get("sign_this")
        .and_then(Value::as_str)
        .ok_or_else(|| DepotError::new("the origin returned no text to sign"))?;
    let at = prepared
        .get("at")
        .and_then(Value::as_u64)
        .ok_or_else(|| DepotError::new("the origin returned no `at` to sign"))?;
    // What gets signed is what was asked for. The text is rebuilt here from
    // the typed change and the origin's `at`, and an origin that offers any
    // other text — another action, another wallet, a block for an ask — is
    // refused before the signer is asked. The act then sends the same args.
    let message = roster::sign_text(&me, op, key, &other, at)
        .filter(|m| m == offered)
        .ok_or_else(|| {
            DepotError::new(
                "the origin prepared a different change from the one asked for — nothing was signed",
            )
        })?;
    let args = json!({key: other});
    if a.has("--dry-run") {
        if json_out {
            let out = json!({"op": op, "wallet": other, "at": at, "sign_this": safe_text(&message)});
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        } else {
            println!("would sign:\n{}", safe_text(&message));
            println!("\nnothing was sent — drop --dry-run to do it");
        }
        return Ok(0);
    }

    let out = signer
        .sign(&message, &format!("friends: {word} {}", roster::short(&other)))
        .map_err(|r| DepotError::new(r.reason))?;
    let signature = out
        .get("signature")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| DepotError::new("the signer returned no signature"))?;
    let done = told(net.post_json_told(
        &format!("{host}/api/friends/act"),
        &json!({"wallet": me, "op": op, "args": args, "at": at, "signature": signature}),
    ))?;
    let view = done.get("view").and_then(|v| roster::parse(v).ok());
    if json_out {
        // Rebuilt from the parsed list rather than echoed, so every string in
        // it went through the same sanitiser the terminal lines do.
        let out = json!({
            "ok": true, "op": op, "wallet": other,
            "view": view.as_ref().map(|v| view_json(v, clock::now())),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(0);
    }
    let now_friends = view.is_some_and(|v| v.friends.iter().any(|f| f.wallet == other));
    println!("{}", done_line(word, &called, now_friends));
    if word == "add" {
        // Said at the moment it becomes true for this person, once.
        println!("  friends lists are public, like playtime");
    }
    Ok(0)
}

/// What happened, in a line. `add` reads two ways because an ask to someone
/// who already asked you is a friendship at once.
pub fn done_line(word: &str, called: &str, now_friends: bool) -> String {
    match word {
        "add" if now_friends => format!("you and {called} are friends now"),
        "add" => format!("asked {called}"),
        "accept" => format!("you and {called} are friends now"),
        "decline" => format!("declined {called}"),
        "cancel" => format!("took back your ask to {called}"),
        "remove" => format!("{called} is off your list"),
        "block" => format!("blocked {called}"),
        "unblock" => format!("unblocked {called}"),
        _ => "done".into(),
    }
}

/// A refusal in the origin's own words, made safe to print.
fn told(got: Fetched<Value>) -> Result<Value, DepotError> {
    if got.ok {
        return Ok(got.value.unwrap_or(Value::Null));
    }
    if !got.reachable {
        return Err(DepotError::new(format!(
            "could not reach the origin — {}",
            safe_line(&got.why)
        )));
    }
    Err(DepotError::new(safe_line(&got.why)))
}

/// A multi-line text from the origin, each line made safe on its own — the
/// text to sign has real newlines, and only those survive.
fn safe_text(s: &str) -> String {
    s.lines().map(safe_line).collect::<Vec<_>>().join("\n")
}

/// The link to follow for a friend, or why there is none.
fn join(net: &Net, a: &Args, host: &str) -> Result<String, DepotError> {
    let handle = a
        .positional
        .get(1)
        .ok_or_else(|| DepotError::new("join whom? — a friend's wallet or name"))?;
    let me = whose(a, host)?;
    let view = fetch(net, host, &me)?;
    let f = pick(net, host, &view, handle)?;
    let called = safe_line(&f.label());
    if !view.presence.reachable {
        return Err(DepotError::new(format!(
            "could not tell where {called} is — {}",
            safe_line(view.presence.why.as_deref().unwrap_or("presence did not read"))
        )));
    }
    let Some(p) = &f.playing else {
        return Err(DepotError::new(format!("{called} is not in a game right now")));
    };
    match &p.join {
        Some(link) => Ok(link.clone()),
        None => Err(DepotError::new(format!(
            "{called} is playing {}, and no join link is known for that server",
            safe_line(&p.title)
        ))),
    }
}

/// One friend off the list, by wallet or name. A name is matched against the
/// list first, so a friend is found by the name the list shows; a name the
/// list does not carry goes through the register.
fn pick<'v>(net: &Net, host: &str, view: &'v View, handle: &str) -> Result<&'v Friend, DepotError> {
    let h = handle.trim();
    let wallet = if roster::is_wallet(h) {
        h.to_ascii_lowercase()
    } else {
        let named: Vec<&Friend> = view
            .friends
            .iter()
            .filter(|f| f.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(h)))
            .collect();
        match named.len() {
            1 => return Ok(named[0]),
            0 => wallet_of(net, host, h)?.0,
            n => {
                return Err(DepotError::new(format!(
                    "{n} friends go by {} — use the wallet",
                    safe_line(h)
                )))
            }
        }
    };
    view.friends.iter().find(|f| f.wallet == wallet).ok_or_else(|| {
        DepotError::new(format!("{} is not on your friends list", safe_line(h)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args;
    use elo_net::friends::{Playing, Presence};

    fn argv(parts: &[&str]) -> args::Args {
        args::parse(&parts.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    fn friend(name: Option<&str>, wallet: &str) -> Friend {
        Friend {
            wallet: wallet.into(), since: None, name: name.map(str::to_string), pic: None,
            agent: false, online: false, playing: None, last_online: None, buddy: None,
        }
    }

    const ME: &str = "0x00000000000000000000000000000000000000aa";
    const ASH: &str = "0x00000000000000000000000000000000000000bb";

    fn view(friends: Vec<Friend>, reachable: bool) -> View {
        View {
            wallet: ME.into(), friends, buddies: vec![], asks_in: vec![], asks_out: vec![],
            presence: Presence { reachable, why: (!reachable).then(|| "ledger down".into()) },
            limits: Default::default(),
        }
    }

    #[test]
    fn every_word_maps_to_the_contracts_op_and_arg() {
        assert_eq!(op_for("add"), Some(("ask", "to")));
        assert_eq!(op_for("accept"), Some(("accept", "from")));
        assert_eq!(op_for("decline"), Some(("decline", "from")));
        assert_eq!(op_for("cancel"), Some(("cancel", "to")));
        assert_eq!(op_for("remove"), Some(("remove", "wallet")));
        assert_eq!(op_for("block"), Some(("block", "wallet")));
        assert_eq!(op_for("unblock"), Some(("unblock", "wallet")));
        // `ask` is the wire's word and `join` is not a change.
        assert_eq!(op_for("ask"), None);
        assert_eq!(op_for("join"), None);
    }

    #[test]
    fn the_typed_wallet_wins_and_must_be_a_wallet() {
        let a = argv(&["friends", "--wallet", &ASH.to_ascii_uppercase().replacen("0X", "0x", 1)]);
        assert_eq!(whose(&a, "https://elo.test").unwrap(), ASH);
        let a = argv(&["friends", "--wallet", "ash"]);
        assert!(whose(&a, "https://elo.test").is_err());
        // The subverb and its target are positionals, in order.
        let a = argv(&["friends", "add", ASH, "--dry-run"]);
        assert_eq!(a.positional, ["add", ASH]);
        assert!(a.has("--dry-run"));
    }

    #[test]
    fn the_terminal_lines_say_where_each_friend_is() {
        let mut ash = friend(Some("Ash"), ASH);
        ash.online = true;
        ash.playing = Some(Playing {
            game: "gates".into(), title: "Gates".into(), server: Some("eu-1".into()),
            server_name: Some("Gates EU".into()), since: None,
            join: Some("elo://join/gates/eu-1.example.test:61234".into()),
        });
        let lines = render(&view(vec![ash.clone()], true), 0).join("\n");
        assert!(lines.contains("1 friend\n"), "{lines}");
        assert!(lines.contains("Ash"), "{lines}");
        assert!(lines.contains("playing Gates · Gates EU"), "{lines}");
        assert!(lines.contains("elo://join/gates/eu-1.example.test:61234"), "{lines}");

        // Presence nobody could read draws a dash and says why, once.
        let lines = render(&view(vec![ash], false), 0).join("\n");
        assert!(lines.contains("presence could not be read — ledger down"), "{lines}");
        assert!(!lines.contains("playing"), "an unread ledger must not say playing: {lines}");
        assert!(!lines.contains("offline"), "an unread ledger must not say offline: {lines}");
        assert!(!lines.contains("elo://"), "nor offer a place nobody measured: {lines}");
    }

    #[test]
    fn an_empty_list_says_how_to_add_one() {
        let lines = render(&view(vec![], true), 0).join("\n");
        assert!(lines.contains("0 friends"), "{lines}");
        assert!(lines.contains("elo friends add"), "{lines}");
    }

    #[test]
    fn a_name_cannot_drive_the_terminal() {
        let hostile = friend(Some("Ash\x1b[2J\x1b]52;c;aGk=\x07\r\nforged"), ASH);
        let lines = render(&view(vec![hostile], true), 0).join("\n");
        assert!(!lines.contains('\x1b') && !lines.contains('\r') && !lines.contains('\x07'),
                "{lines:?}");
    }

    #[test]
    fn a_wallet_cannot_drive_the_terminal_either() {
        // The parser trims a wallet; a row built any other way still prints
        // one line per line.
        let mut v = view(vec![friend(Some("Ash"), &format!("\r\n{ASH}\x0b"))], true);
        v.asks_in.push(roster::Ask { wallet: format!("{ASH}\r\nforged"), at: None, name: None });
        for line in render(&v, 0) {
            assert!(!line.contains('\r') && !line.contains('\n') && !line.contains('\x0b'),
                    "{line:?}");
        }
    }

    /// The bytes `meter/playauth.play_message(ACTION, me, detail_of(op, args,
    /// at), day_of(at))` returns for these inputs, copied from the meter.
    #[test]
    fn the_text_signed_is_the_meters_text() {
        assert_eq!(
            roster::sign_text(ME, "ask", "to", ASH, 1_760_000_000).unwrap(),
            "elo play\naction: friends\nwallet: 0x00000000000000000000000000000000000000aa\n\
             day: 2025-10-09\ndetail: {\"args\":{\"to\":\"0x00000000000000000000000000000000000000bb\"},\
             \"at\":1760000000,\"op\":\"ask\"}"
        );
        // One `at` either side of midnight is two days: the day is the `at`'s.
        assert!(roster::sign_text(ME, "block", "wallet", ASH, 86_399).unwrap().contains("\nday: 1970-01-01\n"));
        assert!(roster::sign_text(ME, "block", "wallet", ASH, 86_400).unwrap().contains("\nday: 1970-01-02\n"));
        assert_eq!(roster::sign_text(ME, "ask", "to", ASH, u64::MAX), None);
    }

    #[test]
    fn a_name_on_the_list_picks_that_friend_and_two_of_them_do_not() {
        let net = Net::new();
        let v = view(vec![friend(Some("Ash"), ASH)], true);
        // No register round trip: an unroutable host proves the list answered.
        assert_eq!(pick(&net, "http://127.0.0.1:9", &v, "ash").unwrap().wallet, ASH);
        assert_eq!(pick(&net, "http://127.0.0.1:9", &v, ASH).unwrap().wallet, ASH);
        assert!(pick(&net, "http://127.0.0.1:9", &v, ME).is_err(), "not a friend");
        let twice = view(vec![friend(Some("Ash"), ASH), friend(Some("ASH"), ME)], true);
        assert!(pick(&net, "http://127.0.0.1:9", &twice, "ash").is_err());
    }

    #[test]
    fn each_change_reads_as_what_happened() {
        assert_eq!(done_line("add", "Ash", false), "asked Ash");
        assert_eq!(done_line("add", "Ash", true), "you and Ash are friends now");
        assert_eq!(done_line("block", "Ash", false), "blocked Ash");
        for (word, _) in WORDS {
            assert_ne!(done_line(word, "Ash", false), "done", "{word} needs its own line");
        }
    }

    /// The window and the terminal share `elo_net::friends::ago`; the chat
    /// feed uses `elo_hive::clock::ago`. Two copies, held to one answer — and
    /// the same for `day`, which keys every signed friends text.
    #[test]
    fn the_two_clocks_agree() {
        let now = 1_760_000_000;
        for d in [-7_200, -1, 0, 4, 5, 59, 60, 3_599, 3_600, 86_399, 86_400,
                  86_400 * 364, 86_400 * 365, 86_400 * 800] {
            assert_eq!(roster::ago(now - d, now), clock::ago(now - d, now), "at {d}s");
        }
        for at in [-86_401, -1, 0, 86_399, 86_400, 951_782_400, 1_709_164_800,
                   1_761_003_600, 4_102_444_800, 253_402_300_799] {
            assert_eq!(roster::day(at), clock::day(at), "at {at}");
        }
    }

    #[test]
    fn your_agents_are_listed_apart_and_marked() {
        let mut v = view(vec![friend(Some("Ash"), ASH)], true);
        let mut pip = friend(Some("Pip"), ME);
        pip.agent = true;
        pip.buddy = Some(roster::Buddy { id: "pip".into(), kind: "buddy".into() });
        v.buddies.push(pip);
        let lines = render(&v, 0).join("\n");
        assert!(lines.contains("1 friend\n"), "a buddy is not counted as a friend: {lines}");
        assert!(lines.contains("your agents"), "{lines}");
        assert!(lines.contains("Pip (agent)"), "{lines}");
        let json = view_json(&v, 0);
        assert_eq!(json["buddies"][0]["buddy"]["id"], "pip");
        assert_eq!(json["count"], 1);
    }
}
