//! A friends list, read off the origin — `GET /api/friends/{wallet}`.
//!
//! Both halves of the client read it here: `elo friends` in a terminal and the
//! Friends window. One parser and one presence sentence, so the two cannot
//! disagree about whether someone is playing.
//!
//! **Presence is the origin's measurement, never ours.** It comes from the
//! shards' own rosters (`meter/playtime.py`), and the document says whether
//! that ledger could be read at all. When it could not, every friend's line is
//! `—`: "we could not look" is not "offline", which is the same trap
//! [`crate::Fetched`] exists for, one field down.
//!
//! **A join link is untrusted input, so it is re-checked here.** The origin
//! builds it from a shard row in `elo-depot`'s grammar; a link that grammar
//! refuses is dropped rather than carried, and the friend reads as playing
//! with no link known. Nothing past this module ever holds a link the parser
//! has not accepted.
//!
//! **The text a change signs is built here too** ([`sign_text`]), once, for
//! both halves: the CLI and the window each rebuild it from the change asked
//! for and sign only when the origin offered exactly those bytes.
//! `sdk/testdata/friends_texts.json` pins them against `meter/friends.py`.

use crate::{Fetched, Net};
use serde_json::{json, Map, Value};

/// The longest piece of origin text kept, in characters. A name or a server
/// name is a label in a row, and the window clips it anyway; this is the bound
/// on what a hostile document can make the client carry around.
pub const MAX_TEXT_CHARS: usize = 120;

/// The list as the origin answered it.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// Whose list this is, lowercased.
    pub wallet: String,
    pub friends: Vec<Friend>,
    /// The wallet's own AI buddies — agents it runs, listed with no ask and
    /// never the subject of a change. Empty from a meter that predates them.
    pub buddies: Vec<Friend>,
    /// Asks waiting on this wallet's answer.
    pub asks_in: Vec<Ask>,
    /// Asks this wallet made that nobody has answered yet.
    pub asks_out: Vec<Ask>,
    pub presence: Presence,
    pub limits: Limits,
}

/// Whether the presence ledger could be read. **`reachable: false` means every
/// friend's presence is unknown**, not that everyone is offline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub reachable: bool,
    pub why: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Limits {
    pub friends_max: Option<u64>,
    pub asks_out_max: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Friend {
    pub wallet: String,
    /// When the friendship began, unix seconds.
    pub since: Option<i64>,
    pub name: Option<String>,
    pub pic: Option<String>,
    pub agent: bool,
    pub online: bool,
    pub playing: Option<Playing>,
    /// The last time a shard's roster had them, unix seconds.
    pub last_online: Option<i64>,
    /// Set on a row from `buddies`: which of the wallet's agents this is.
    pub buddy: Option<Buddy>,
}

/// What marks a buddy row: the agent's id and kind, as the origin names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buddy {
    pub id: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playing {
    /// The title's slug.
    pub game: String,
    /// The title's name — the slug when the origin named none.
    pub title: String,
    /// The shard row's id, and its name when the list gives one.
    pub server: Option<String>,
    pub server_name: Option<String>,
    pub since: Option<i64>,
    /// `elo://join/<slug>/<addr>`, canonical, or `None` when no address is
    /// known — or when the one sent did not parse, which is the same thing to
    /// a player.
    pub join: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub wallet: String,
    pub at: Option<i64>,
    pub name: Option<String>,
}

impl Friend {
    /// What a row is titled: the name the origin gave, or the short wallet.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| short(&self.wallet))
    }

    /// The one presence sentence, for the terminal and the window alike.
    ///
    /// `—` when the ledger could not be read, whatever the row says: a row
    /// built from a ledger nobody reached carries no measurement.
    pub fn presence_line(&self, presence: &Presence, now: i64) -> String {
        if !presence.reachable {
            return "—".into();
        }
        if let Some(p) = &self.playing {
            return match p.server_name.as_ref().or(p.server.as_ref()) {
                Some(s) => format!("playing {} · {s}", p.title),
                None => format!("playing {}", p.title),
            };
        }
        if self.online {
            return "online".into();
        }
        match self.last_online {
            Some(at) => format!("last online {}", ago(at, now)),
            None => "offline".into(),
        }
    }

    /// The link to follow, when they are in a game and its address is known.
    pub fn join_link(&self) -> Option<&str> {
        self.playing.as_ref()?.join.as_deref()
    }
}

impl Ask {
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| short(&self.wallet))
    }
}

/// Where a wallet's list is read.
pub fn url(host: &str, wallet: &str) -> String {
    format!("{}/api/friends/{}", host.trim_end_matches('/'), wallet.trim().to_ascii_lowercase())
}

/// Read a list on the long deadline — the terminal, where the person typed
/// the command and is waiting for exactly this.
pub fn read(net: &Net, host: &str, wallet: &str) -> Fetched<View> {
    decode(net.get_json(&url(host, wallet)))
}

/// The same read on the short deadline, for the window's UI thread.
pub fn read_quick(net: &Net, host: &str, wallet: &str) -> Fetched<View> {
    decode(net.get_json_quick(&url(host, wallet)))
}

/// A fetched document, parsed. **`reachable` survives**: a list we never got
/// is not an empty list, and a list we got and could not parse is neither.
pub fn decode(got: Fetched<Value>) -> Fetched<View> {
    match got.value {
        Some(v) => match parse(&v) {
            Ok(view) => Fetched::good(view),
            Err(e) => Fetched::bad(e),
        },
        None => Fetched {
            value: None,
            ok: got.ok,
            reachable: got.reachable,
            status: got.status,
            why: got.why,
        },
    }
}

/// Parse one answer. A document missing what every answer carries is refused
/// whole, the way a shard list with a bad address is: a half-read list drawn
/// as the list would be the client inventing the rest.
pub fn parse(v: &Value) -> Result<View, String> {
    let doc = v.as_object().ok_or("the friends list is not a JSON object")?;
    let wallet = wallet_at(doc, "wallet").ok_or("the friends list names no wallet")?;
    let friends = doc
        .get("friends")
        .and_then(Value::as_array)
        .ok_or("the friends list carries no `friends` array")?
        .iter()
        .enumerate()
        .map(|(i, row)| {
            friend(row)
                .map(|f| Friend { buddy: None, ..f })
                .ok_or_else(|| format!("friend {} is not a friend row", i + 1))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let buddies = buddies(doc)?;
    let presence = doc
        .get("presence")
        .and_then(Value::as_object)
        .ok_or("the friends list does not say whether presence was read")?;
    let presence = Presence {
        reachable: presence
            .get("reachable")
            .and_then(Value::as_bool)
            .ok_or("the friends list does not say whether presence was read")?,
        why: text(presence, "why"),
    };
    let limits = doc.get("limits").and_then(Value::as_object).map(|l| Limits {
        friends_max: l.get("friends_max").and_then(Value::as_u64),
        asks_out_max: l.get("asks_out_max").and_then(Value::as_u64),
    });
    Ok(View {
        wallet,
        friends,
        buddies,
        asks_in: asks(doc, "asks_in")?,
        asks_out: asks(doc, "asks_out")?,
        presence,
        limits: limits.unwrap_or_default(),
    })
}

fn friend(row: &Value) -> Option<Friend> {
    let row = row.as_object()?;
    let playing = match row.get("playing") {
        None | Some(Value::Null) => None,
        Some(p) => Some(playing(p.as_object()?)?),
    };
    Some(Friend {
        wallet: wallet_at(row, "wallet")?,
        since: row.get("since").and_then(Value::as_i64),
        name: text(row, "name"),
        pic: text(row, "pic"),
        agent: row.get("agent").and_then(Value::as_bool).unwrap_or(false),
        online: row.get("online").and_then(Value::as_bool).unwrap_or(false),
        playing,
        last_online: row.get("last_online").and_then(Value::as_i64),
        buddy: row.get("buddy").and_then(Value::as_object).map(|b| Buddy {
            id: text(b, "id").unwrap_or_default(),
            kind: text(b, "kind").unwrap_or_else(|| "buddy".into()),
        }),
    })
}

/// `buddies`: friend rows, each marked as one of the wallet's own agents.
/// Absent reads as none — a meter from before buddies — and present and
/// malformed is a refusal like any other row.
fn buddies(doc: &Map<String, Value>) -> Result<Vec<Friend>, String> {
    let Some(rows) = doc.get("buddies") else { return Ok(Vec::new()) };
    let rows = rows.as_array().ok_or("`buddies` is not a list")?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let f = friend(row).ok_or_else(|| format!("buddy {} is not a friend row", i + 1))?;
            let buddy = f.buddy.clone().unwrap_or(Buddy { id: String::new(), kind: "buddy".into() });
            Ok(Friend { agent: true, buddy: Some(buddy), ..f })
        })
        .collect()
}

fn playing(p: &Map<String, Value>) -> Option<Playing> {
    let game = text(p, "game");
    let title = text(p, "title").or_else(|| game.clone())?;
    Some(Playing {
        game: game.unwrap_or_else(|| title.clone()),
        title,
        server: text(p, "server"),
        server_name: text(p, "server_name"),
        since: p.get("since").and_then(Value::as_i64),
        join: p
            .get("join")
            .and_then(Value::as_str)
            .and_then(|l| elo_depot::deeplink::parse(l).ok())
            .map(|j| j.link()),
    })
}

/// `asks_in` / `asks_out`. Absent reads as none; present and malformed is a
/// refusal like any other row.
fn asks(doc: &Map<String, Value>, key: &str) -> Result<Vec<Ask>, String> {
    let Some(rows) = doc.get(key) else { return Ok(Vec::new()) };
    let rows = rows.as_array().ok_or_else(|| format!("`{key}` is not a list"))?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let row = row.as_object();
            row.and_then(|r| {
                Some(Ask {
                    wallet: wallet_at(r, "wallet")?,
                    at: r.get("at").and_then(Value::as_i64),
                    name: text(r, "name"),
                })
            })
            .ok_or_else(|| format!("{key} row {} is not an ask", i + 1))
        })
        .collect()
}

/// A wallet field, stored the way every comparison expects it: trimmed and
/// lowercased, so it is exactly `0x` and forty hex digits. [`is_wallet`] trims
/// before it checks, so keeping the untrimmed string would carry a padded
/// one — line breaks and all — into every row that prints or matches it.
fn wallet_at(m: &Map<String, Value>, key: &str) -> Option<String> {
    m.get(key)
        .and_then(Value::as_str)
        .filter(|w| is_wallet(w))
        .map(|w| w.trim().to_ascii_lowercase())
}

/// Origin text, trimmed, non-empty, and cut at [`MAX_TEXT_CHARS`] — by
/// characters, never bytes, so a name in any script cannot split a codepoint.
fn text(m: &Map<String, Value>, key: &str) -> Option<String> {
    let s = m.get(key)?.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    Some(s.chars().take(MAX_TEXT_CHARS).collect())
}

/// `0x` and forty hex digits. Enough to route a string as an address, never
/// enough to trust one.
pub fn is_wallet(s: &str) -> bool {
    let s = s.trim();
    s.len() == 42 && s.starts_with("0x") && s[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// `0xabcd…1234`. The same cut `elo_hive::who::short` makes; it lives here too
/// because `elo-hive` reads through this crate and cannot be read from it.
pub fn short(address: &str) -> String {
    if address.len() <= 16 || !address.is_ascii() {
        return address.to_string();
    }
    format!("{}…{}", &address[..6], &address[address.len() - 4..])
}

/// `"3m ago"`. The twin of `elo_hive::clock::ago`, for the same reason as
/// [`short`]; `elo-launcher`'s friends tests hold the two to one answer.
pub fn ago(at: i64, now: i64) -> String {
    let d = now - at;
    if d < 0 {
        return format!("in {}", span(-d));
    }
    if d < 5 {
        return "just now".into();
    }
    format!("{} ago", span(d))
}

fn span(d: i64) -> String {
    match d {
        d if d < 60 => format!("{d}s"),
        d if d < 3600 => format!("{}m", d / 60),
        d if d < 86_400 => format!("{}h", d / 3600),
        d if d < 86_400 * 365 => format!("{}d", d / 86_400),
        d => format!("{}y", d / (86_400 * 365)),
    }
}

/// The list as JSON, for `elo friends --json` and `elo mcp`'s friends read —
/// one shape for both. Rebuilt from the parsed list rather than echoed, and
/// every string the origin wrote goes through `clean` (`elo_hive::render::
/// safe_line`, which this crate cannot reach): a friend chooses their own name,
/// and a name is not allowed to drive whatever prints it.
pub fn view_json(view: &View, now: i64, clean: fn(&str) -> String) -> Value {
    let text = |s: &Option<String>| s.as_deref().map(clean);
    let row = |f: &Friend| {
        let mut r = json!({
            "wallet": f.wallet,
            "since": f.since,
            "name": text(&f.name),
            "pic": text(&f.pic),
            "agent": f.agent,
            "online": f.online,
            "playing": f.playing.as_ref().map(|p| json!({
                "game": clean(&p.game),
                "title": clean(&p.title),
                "server": text(&p.server),
                "server_name": text(&p.server_name),
                "since": p.since,
                "join": p.join,
            })),
            "last_online": f.last_online,
            "presence": clean(&f.presence_line(&view.presence, now)),
        });
        if let Some(b) = &f.buddy {
            r["buddy"] = json!({"id": clean(&b.id), "kind": clean(&b.kind)});
        }
        r
    };
    let asks = |a: &[Ask]| {
        a.iter()
            .map(|x| json!({"wallet": x.wallet, "at": x.at, "name": text(&x.name)}))
            .collect::<Vec<_>>()
    };
    json!({
        "wallet": view.wallet,
        "count": view.friends.len(),
        "friends": view.friends.iter().map(row).collect::<Vec<_>>(),
        "buddies": view.buddies.iter().map(row).collect::<Vec<_>>(),
        "asks_in": asks(&view.asks_in),
        "asks_out": asks(&view.asks_out),
        "presence": {"reachable": view.presence.reachable, "why": text(&view.presence.why)},
        "limits": {
            "friends_max": view.limits.friends_max,
            "asks_out_max": view.limits.asks_out_max,
        },
    })
}

// ── changes ──────────────────────────────────────────────────────────────────

/// The changes a wallet signs: the op sent, and the arg that names the other
/// wallet — `meter/friends.OPS`, the contract's names in one table.
pub const OPS: [(&str, &str); 8] = [
    ("ask", "to"),
    ("accept", "from"),
    ("decline", "from"),
    ("cancel", "to"),
    ("remove", "wallet"),
    ("block", "wallet"),
    ("unblock", "wallet"),
    ("invite", "to"),
];

/// The arg an op takes, or `None` when it is not a friends change.
pub fn op_key(op: &str) -> Option<&'static str> {
    OPS.iter().find(|(o, _)| *o == op).map(|(_, key)| *key)
}

/// The detail a friends change signs: compact JSON, keys sorted — the bytes
/// `meter/friends.detail_of` writes. Written out by hand because every part
/// is fixed ASCII (an op and an arg name from [`OPS`], a checked lowercase
/// wallet, an integer), so there is nothing to escape and no map order to
/// trust.
pub fn detail(op: &str, key: &str, other: &str, at: u64) -> String {
    format!(r#"{{"args":{{"{key}":"{other}"}},"at":{at},"op":"{op}"}}"#)
}

/// The exact text a friends change signs: `meter/playauth.play_message`, with
/// the day keyed to the signed `at` (`playauth.day_of`). Rebuilt by the client
/// rather than taken from the origin, so the signer is only ever asked for the
/// change that was asked for. `None` for an `at` no clock can hold.
pub fn sign_text(me: &str, op: &str, key: &str, other: &str, at: u64) -> Option<String> {
    let day = day(i64::try_from(at).ok()?);
    Some(format!(
        "elo play\naction: friends\nwallet: {me}\nday: {day}\ndetail: {}",
        detail(op, key, other, at)
    ))
}

/// `"2025-10-20"` — the UTC day of a unix time, as `playauth.day_of` keys a
/// signed text. The twin of `elo_hive::clock::day`, for [`short`]'s reason;
/// `elo-launcher`'s friends tests hold the two to one answer.
pub fn day(at: i64) -> String {
    // Howard Hinnant's civil-from-days, public domain.
    let z = at.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ME: &str = "0x00000000000000000000000000000000000000aa";
    const ASH: &str = "0x00000000000000000000000000000000000000bb";
    const BO: &str = "0x00000000000000000000000000000000000000cc";

    fn doc(friends: Value, reachable: bool) -> Value {
        json!({
            "wallet": ME, "count": friends.as_array().map(Vec::len).unwrap_or(0),
            "friends": friends,
            "asks_in": [{"wallet": BO, "at": 1_760_000_000, "name": "Bo"}],
            "asks_out": [],
            "presence": {"reachable": reachable, "why": if reachable { Value::Null }
                         else { json!("the playtime ledger did not read") }},
            "limits": {"friends_max": 200, "asks_out_max": 50},
        })
    }

    fn ash_playing(join: Value) -> Value {
        json!({
            "wallet": ASH, "since": 1_759_000_000, "name": "Ash", "pic": null,
            "agent": false, "online": true,
            "playing": {"game": "gates", "title": "Gates", "server": "eu-1",
                        "server_name": "Gates EU", "since": 1_760_000_100,
                        "join": join},
            "last_online": 1_760_000_200,
        })
    }

    #[test]
    fn a_good_answer_reads_into_rows_with_presence_and_a_link() {
        let v = parse(&doc(json!([ash_playing(json!("elo://join/gates/eu-1.example.test:61234"))]),
                           true))
            .unwrap();
        assert_eq!(v.wallet, ME);
        assert_eq!(v.friends.len(), 1);
        let ash = &v.friends[0];
        assert_eq!(ash.label(), "Ash");
        assert_eq!(ash.join_link(), Some("elo://join/gates/eu-1.example.test:61234"));
        assert_eq!(ash.presence_line(&v.presence, 1_760_000_300), "playing Gates · Gates EU");
        assert_eq!(v.asks_in.len(), 1);
        assert_eq!(v.asks_in[0].label(), "Bo");
        assert!(v.asks_out.is_empty());
        assert_eq!(v.limits.friends_max, Some(200));
    }

    #[test]
    fn presence_that_was_not_read_is_a_dash_and_never_offline() {
        let v = parse(&doc(json!([ash_playing(Value::Null)]), false)).unwrap();
        assert!(!v.presence.reachable);
        assert_eq!(v.presence.why.as_deref(), Some("the playtime ledger did not read"));
        assert_eq!(v.friends[0].presence_line(&v.presence, 0), "—");
    }

    #[test]
    fn each_presence_state_has_its_own_sentence() {
        let p = Presence { reachable: true, why: None };
        let mut f = Friend {
            wallet: ASH.into(), since: None, name: None, pic: None, agent: false,
            online: false, playing: None, last_online: None, buddy: None,
        };
        assert_eq!(f.presence_line(&p, 0), "offline");
        assert_eq!(f.label(), "0x0000…00bb", "no name falls back to the short wallet");
        f.last_online = Some(1_000);
        assert_eq!(f.presence_line(&p, 1_000 + 3 * 3600), "last online 3h ago");
        f.online = true;
        assert_eq!(f.presence_line(&p, 0), "online");
        f.playing = Some(Playing {
            game: "gates".into(), title: "Gates".into(), server: Some("eu-1".into()),
            server_name: None, since: None, join: None,
        });
        assert_eq!(f.presence_line(&p, 0), "playing Gates · eu-1",
                   "the row id stands in when the list names no server");
        assert_eq!(f.join_link(), None);
    }

    #[test]
    fn a_link_the_grammar_refuses_is_dropped_not_carried() {
        // An identity segment is the phishing primitive `deeplink` exists to
        // refuse; a link that reached the window with one would be followable.
        for bad in [json!("elo://join/gates/eu-1.example.test:61234/0xabc"),
                    json!("https://evil.example/join"),
                    json!("elo://join/../etc:1"),
                    json!(42)] {
            let v = parse(&doc(json!([ash_playing(bad.clone())]), true)).unwrap();
            assert_eq!(v.friends[0].join_link(), None, "{bad} must not survive");
            assert!(v.friends[0].playing.is_some(), "they are still playing: {bad}");
        }
    }

    #[test]
    fn an_empty_list_is_a_real_answer() {
        let mut d = doc(json!([]), true);
        d["asks_in"] = json!([]);
        let v = parse(&d).unwrap();
        assert!(v.friends.is_empty() && v.asks_in.is_empty() && v.asks_out.is_empty());
        // Absent ask lists read as none.
        let mut d = doc(json!([]), true);
        d.as_object_mut().unwrap().remove("asks_in");
        d.as_object_mut().unwrap().remove("asks_out");
        d.as_object_mut().unwrap().remove("limits");
        let v = parse(&d).unwrap();
        assert!(v.asks_in.is_empty());
        assert_eq!(v.limits, Limits::default());
    }

    #[test]
    fn a_malformed_answer_is_refused_whole() {
        let mut cases = vec![
            ("not an object", json!([1, 2])),
            ("no wallet", { let mut d = doc(json!([]), true);
                            d.as_object_mut().unwrap().remove("wallet"); d }),
            ("no friends", { let mut d = doc(json!([]), true);
                             d["friends"] = json!({}); d }),
            ("no presence", { let mut d = doc(json!([]), true);
                              d.as_object_mut().unwrap().remove("presence"); d }),
            ("presence not a bool", { let mut d = doc(json!([]), true);
                                      d["presence"] = json!({"reachable": "yes"}); d }),
            ("a row with no wallet", doc(json!([{"name": "Ash"}]), true)),
            ("a row whose wallet is not one", doc(json!([{"wallet": "ash"}]), true)),
            ("a row that is a string", doc(json!(["0xabc"]), true)),
            ("playing that names no game", doc(json!([{"wallet": ASH, "playing": {}}]), true)),
        ];
        let mut d = doc(json!([]), true);
        d["asks_in"] = json!([{"name": "no wallet"}]);
        cases.push(("an ask with no wallet", d));
        for (what, v) in cases {
            assert!(parse(&v).is_err(), "{what} must be refused");
        }
    }

    #[test]
    fn decoding_keeps_reachable_apart_from_unreadable() {
        let lost = decode(Fetched::unreachable("dns error"));
        assert!(!lost.reachable && !lost.ok && lost.value.is_none());
        assert_eq!(lost.why, "dns error");

        let refused = decode(Fetched::status(404));
        assert!(refused.reachable && !refused.ok);
        assert_eq!(refused.status, Some(404));

        let garbled = decode(Fetched::good(json!({"wallet": 7})));
        assert!(garbled.reachable && !garbled.ok, "read, and not a list: {}", garbled.why);

        let good = decode(Fetched::good(doc(json!([]), true)));
        assert!(good.ok && good.value.unwrap().friends.is_empty());
    }

    #[test]
    fn wallets_are_lowercased_and_text_is_bounded() {
        let mut row = ash_playing(Value::Null);
        row["wallet"] = json!(ASH.to_ascii_uppercase().replacen("0X", "0x", 1));
        row["name"] = json!("W".repeat(500));
        let v = parse(&doc(json!([row]), true)).unwrap();
        assert_eq!(v.friends[0].wallet, ASH);
        assert_eq!(v.friends[0].name.as_ref().unwrap().chars().count(), MAX_TEXT_CHARS);
        assert_eq!(url("https://elo.test/", &ASH.to_ascii_uppercase().replacen("0X", "0x", 1)),
                   format!("https://elo.test/api/friends/{ASH}"));
    }

    #[test]
    fn a_padded_wallet_is_stored_trimmed() {
        // Whitespace `is_wallet` trims past: CR, LF, VT and U+2028. Kept, they
        // printed as line breaks and made a listed friend "not on your list".
        let pad = |w: &str| json!(format!("\r\n\u{b}{w}\u{2028}"));
        let mut row = ash_playing(Value::Null);
        row["wallet"] = pad(ASH);
        let mut d = doc(json!([row]), true);
        d["wallet"] = pad(ME);
        d["asks_in"][0]["wallet"] = pad(BO);
        let v = parse(&d).unwrap();
        assert_eq!(v.wallet, ME);
        assert_eq!(v.friends[0].wallet, ASH);
        assert_eq!(v.asks_in[0].wallet, BO);
        assert_eq!(short(&v.friends[0].wallet), "0x0000…00bb");
    }

    /// The shape `meter/friends.py` serves, buddies included — the same file
    /// the meter's own suite writes against.
    const VIEW: &str = include_str!("../../../../sdk/testdata/friends_view.json");

    #[test]
    fn the_fixture_view_reads_with_its_buddies_marked() {
        let v = parse(&serde_json::from_str(VIEW).unwrap()).unwrap();
        assert_eq!(v.friends.len(), 3);
        assert!(v.friends.iter().all(|f| f.buddy.is_none()), "a friend is not a buddy");
        assert_eq!(v.buddies.len(), 1);
        let pip = &v.buddies[0];
        assert_eq!(pip.label(), "Pip");
        assert!(pip.agent, "a buddy is an agent");
        assert_eq!(pip.buddy, Some(Buddy { id: "pip".into(), kind: "buddy".into() }));
        assert_eq!(pip.join_link(), Some("elo://join/gates/game.elopros.com:61234"));
        assert_eq!(v.asks_in[0].label(), "rook");
        assert_eq!(v.asks_out[0].wallet, "0x7777777777777777777777777777777777777777");
        assert_eq!(v.limits.friends_max, Some(250));

        // The JSON view carries them through, marked, for the CLI and MCP.
        let out = view_json(&v, 1_761_003_600, |s| s.to_string());
        assert_eq!(out["buddies"][0]["buddy"]["id"], "pip");
        assert_eq!(out["buddies"][0]["presence"], "playing Gates · Gates US East 1");
        assert!(out["friends"][0].get("buddy").is_none());
        assert_eq!(out["count"], 3);
    }

    #[test]
    fn buddies_are_optional_and_a_bad_one_is_refused() {
        let mut d = doc(json!([]), true);
        assert!(parse(&d).unwrap().buddies.is_empty(), "an older meter sends none");
        // A row with no marker is still a buddy: the list it came in says so.
        d["buddies"] = json!([{"wallet": BO, "name": "Pip"}]);
        let v = parse(&d).unwrap();
        assert_eq!(v.buddies[0].buddy.as_ref().map(|b| b.kind.as_str()), Some("buddy"));
        assert!(v.buddies[0].agent);
        for bad in [json!({}), json!([{"name": "no wallet"}]), json!(["0xabc"])] {
            d["buddies"] = bad.clone();
            assert!(parse(&d).is_err(), "{bad} must be refused");
        }
    }

    /// Every case `meter/friends.py` wrote into `sdk/testdata/friends_texts.json`
    /// (`playauth.play_message('friends', wallet, detail_of(…), day_of(at))`),
    /// rebuilt here byte for byte.
    #[test]
    fn every_fixture_text_is_rebuilt_byte_for_byte() {
        let fx: Value =
            serde_json::from_str(include_str!("../../../../sdk/testdata/friends_texts.json"))
                .unwrap();
        let ops = fx["ops"].as_object().unwrap();
        assert_eq!(ops.len(), OPS.len(), "one table, the meter's");
        for (op, key) in OPS {
            assert_eq!(ops[op], key, "{op} takes `{key}` on both sides");
        }
        let cases = fx["cases"].as_array().unwrap();
        assert_eq!(cases.len(), OPS.len(), "a case per op");
        for c in cases {
            let s = |k: &str| c[k].as_str().unwrap();
            let (op, at) = (s("op"), c["at"].as_u64().unwrap());
            let key = op_key(op).unwrap();
            assert_eq!(c["args"][key], s("other"), "{op}");
            assert_eq!(detail(op, key, s("other"), at), s("detail"), "{op}");
            assert_eq!(day(at as i64), s("day"), "{op}");
            assert_eq!(sign_text(s("wallet"), op, key, s("other"), at).unwrap(), s("text"), "{op}");
        }
        assert_eq!(op_key("add"), None, "`add` is the CLI's word; the wire says `ask`");
    }
}
