//! Open a window so it can be looked at. `cargo run -p elo-ui --example shot -- games`
use fltk::{app, prelude::*};
use elo_ui::windows::{self, Price, Row, Shelf};

/// A library row. Spelled out rather than defaulted so a capture cannot
/// silently stop exercising a field.
fn row(slug: &str, build: &str, bytes: i64, stale: Option<bool>, name: Option<&str>) -> Row {
    Row {
        slug: slug.into(),
        build: build.into(),
        bytes,
        digest: "0xf43be2a2".into(),
        stale,
        name: name.map(str::to_string),
        icon: None,
        why: (stale.is_none()).then(|| "the origin was not reached".to_string()),
    }
}

/// A shelf row, with the REAL blurb the origin serves for Gates.
///
/// That is the point of this helper and not a detail: the old capture used a
/// hand-shortened blurb, which is exactly why the overrun never showed up in a
/// screenshot. The live one is 140 characters (`GET /api/launcher/manifests`)
/// and it is what ran off the window.
fn shelf(slug: &str, name: &str, blurb: &str, state: &str, installable: bool,
         installed: bool, price: Price) -> Shelf {
    Shelf {
        slug: slug.into(),
        name: name.into(),
        blurb: blurb.into(),
        state: state.into(),
        installable,
        installed,
        price,
        icon: None,
    }
}

const GATES_BLURB: &str = "Wake with nothing on a hostile island. Gather, craft, build, \
                           raid, lose it all, again. 100-player shards on a public server \
                           that is up now.";

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "menu".into());
    let a = app::App::default();
    let mut w = match which.as_str() {
        "games" => windows::games(&[
            row("gates", "0.1.0-g607af0314", 75_829_730, Some(false), Some("Gates")),
            row("gates", "0.2.0-gdeadbee", 75_829_730, Some(true), Some("Gates")),
            row("unreached", "0.1.0", 4_100_000, None, None),
        ]).window,
        "empty" => windows::games(&[]).window,
        // A library longer than any window it could be shown in — the state
        // the shelf was drawing off the bottom edge until 2026-08-13, and the
        // reason `chrome::shelf` scrolls. Worth a capture rather than only a
        // test: what a picture shows and an assertion cannot is that the
        // scrollbar is REACHABLE and the footer is still on screen under it.
        "games2" => windows::games(&(0..15)
            .map(|i| row("gates", "0.1.0-g607af0314", 75_829_730, Some(false),
                         Some(&format!("Title {i}"))))
            .collect::<Vec<_>>()).window,
        "about" => windows::about(),
        "store" => windows::store(&[], false, "dns error: no such host").window,
        "store2" => windows::store(&[
            shelf("gates", "Gates", GATES_BLURB, "live", true, false, Price::Free),
            shelf("barrow", "The Barrow", "a three-room delve — fight, sneak, or leave",
                  "in-build", false, false, Price::Free),
            shelf("sold", "A Title With A Price", GATES_BLURB, "live", true, false,
                  Price::Posted { line: "$10.00, paid in ETH".into() }),
            shelf("dark", "A Title We Could Not Price", GATES_BLURB, "live", true, false,
                  Price::Unknown { why: "could not reach the origin".into() }),
            shelf("mine", "Already Installed", GATES_BLURB, "live", true, true, Price::Free),
        ], true, "").window,
        // The passphrase prompt, in both shapes. `pass2` is the browser
        // pairing's — the whole message being signed rides in the prompt.
        "wallet" => {
            // Painted from a fixed read, so the capture says the same thing
            // whenever it is taken and touches no chain.
            let w = windows::wallet(Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"));
            let junk = elo_net::wallet::Coin {
                sym: "JUNK".into(),
                token: Some([0xd6; 20]),
                decimals: 18,
                what: "Gates's coin".into(),
            };
            let elo = elo_net::wallet::Coin { sym: "ELO".into(), what: "the platform's".into(), ..junk.clone() };
            let snap = elo_net::wallet::Snapshot {
                coins: vec![
                    (elo_net::wallet::Coin::eth(), Ok(1_964_833_011_588_000)),
                    (elo, Ok(0)),
                    (junk, Ok(12_500_000_000_000_000_000)),
                ],
                warning: None,
                note: None,
            };
            elo_ui::wiring::paint_wallet(&w, &snap, &std::cell::RefCell::new(Vec::new()));
            let (mut a, mut t) = (w.amount.clone(), w.to.clone());
            a.set_value("2.5");
            t.set_value("0xF0109fC8DF283027b6285cc889F5aA624EaC1F55");
            w.window
        }
        "wallet2" => windows::wallet(None).window,
        "review" => windows::send_review(
            &[("Send".into(), "12.5 JUNK".into()),
              ("To".into(), "0x F010 9fC8 DF28 3027 b628 5cc8 89F5 aA62 4EaC 1F55".into()),
              ("From".into(), "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".into()),
              ("Network".into(), "Robinhood Chain (4663)".into()),
              ("Fee".into(), "at most 0.000015561873092 ETH".into())],
            &["You have not sent to this address from this launcher before. Check every \
               character against where you got it.".into(),
              "⚠ This address looks like 0xF0103a7cE5a7c6E85Dc4c6A2A5A6D0c3a19E1F55, which you \
               have sent to before, but it is a different address. Scammers plant lookalikes \
               in your history. Do not send unless you checked every character.".into()],
        ).window,
        // BIP-39's own published test words, never a real account's.
        "words" => windows::recovery_words(&"legal winner thank year wave sausage worth \
                                              useful legal winner thank yellow"
            .split(' ').collect::<Vec<_>>()).window,
        "pass" => windows::passphrase("Unlock", "Passphrase:").window,
        "pass2" => windows::passphrase(
            "Sign the browser in",
            "About to sign exactly this, and nothing else:\n\n\
             elopros.com wants to sign you in\naddress: 0x7E5F…95Bdf\n\
             code: ABCD2345\nissued: 2026-08-12T09:00:00Z\n\nPassphrase:").window,
        "notice" => windows::notice(windows::Note::Done, "Account created",
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf\n\n\
             The keystore is the only copy of this key.\n\
             Back it up. The passphrase cannot be reset.", None).window,
        "notice2" => windows::notice(windows::Note::Refused, "The game door did not open",
            "Games will still install and start — they will just play anonymously.\n\n\
             The usual cause is another copy of elo already running.\n\n\
             Nothing retries this while the program is open, so a restart is the fix.",
            Some("Restart elo now")).window,
        // The install notice, with the step after it ON it. Operator,
        // 2026-08-23: *"when we install a game it would be nice if it just
        // showed a play button."* The body used to end with directions to
        // another window; the button is those directions, taken.
        "notice3" => windows::notice(windows::Note::Done, "Done",
            "Gates 0.5.0-g6e4103edb is installed.\n\n\
             /home/you/.local/share/elo/games/gates/0.5.0-g6e4103edb\n\n\
             depot digest\n\
             0xf577e208150f846c59127ca9d3cef3ece17f5b5ba023c8c451224bfee150492f\n\n\
             It is in Games, and Play now starts it.",
            Some("Play now")).window,
        "lock" => windows::lock("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "unlock the account on this machine, or use the wallet in your browser").window,
        "lock2" => windows::lock("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "it locked itself while you were away — playing or signing resets that clock").window,
        // All four shard states are capturable, because a reader must be able
        // to tell them apart at a glance — and three of them are the ones that
        // get drawn wrong. `unreadable` in particular must never look like
        // `no shards up`.
        "servers" => windows::servers("gates", &windows::Shards::Unpublished).window,
        "servers2" => windows::servers("gates", &windows::Shards::Listed {
            url: "https://elopros.com/api/launcher/servers/gates".into(),
            rows: vec![
                shard("us-east-1", "Gates US East 1", "game.elopros.com:61234",
                      Some(47), Some(100), Some("island 20260731")),
                // No count at all: this shard publishes no status endpoint,
                // and `?` is the honest render.
                shard("us-1", "Gates US 1", "us.example.test:61234", None, None, None),
                // A measured zero. Not the same as `?`, and it must not look
                // like it.
                shard("dev", "Dev shard", "dev.example.test:4433", Some(0), Some(100),
                      Some("island 1")),
                // Full: drawn, gold, and not pressable — hiding it would make
                // a busy server look like one that does not exist.
                shard("full", "Gates EU 2", "eu2.example.test:61234",
                      Some(100), Some(100), Some("island 20260731")),
            ],
        }).window,
        "servers3" => windows::servers("gates", &windows::Shards::Unreadable {
            url: "https://elopros.com/api/launcher/servers/gates".into(),
            why: "could not reach it — dns error".into(),
            reachable: false,
        }).window,
        "servers4" => windows::servers("gates", &windows::Shards::Listed {
            url: "https://elopros.com/api/launcher/servers/gates".into(),
            rows: vec![],
        }).window,
        // Friends in each state a player can meet. The list goes through the
        // same parser the window's caller uses, so a capture cannot draw a
        // shape the origin could never have sent.
        "friends" => windows::friends(
            &windows::Friends::Listed { view: friends_view(true, true) }, SHOT_NOW).window,
        "friends2" => windows::friends(
            &windows::Friends::Listed { view: friends_view(false, true) }, SHOT_NOW).window,
        "friends3" => windows::friends(&windows::Friends::Unreadable {
            url: "https://elopros.com/api/friends/0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
                .into(),
            why: "could not reach it — dns error".into(),
            reachable: false,
        }, SHOT_NOW).window,
        "friends4" => windows::friends(
            &windows::Friends::Listed { view: friends_view(true, false) }, SHOT_NOW).window,
        "friends5" => windows::friends(&windows::Friends::NoWallet, SHOT_NOW).window,
        // Both states are capturable, because they are different windows: with
        // no account the two make/import doors are the point of the screen,
        // and with one they are deliberately absent.
        "account" => windows::account(None, "https://elopros.com").window,
        "account2" => windows::account(
            Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"),
            "https://elopros.com",
        )
        .window,
        // The pairing row with a code IN it — the state that only exists after
        // a press, and the one worth a picture: a code a player has to carry
        // to a browser must be legible, selectable, and sitting next to the
        // two buttons that move it. Hidden until minted, so a capture has to
        // put it there.
        "account3" => {
            let a = windows::account(
                Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"),
                "https://elopros.com",
            );
            if let Some(mut r) = a.pair_row.clone() {
                r.code.set_value("ABCD-2345");
                r.show();
            }
            a.window
        }
        // No account here, and a browser wallet paired: the well names the
        // paired wallet, because that is who games are told is playing.
        "account4" => {
            let a = windows::account(None, "https://elopros.com");
            elo_ui::wiring::show_pairing(&a, "0x24445EFddB08d4938E3E3627042B2Cf4063d9092");
            a.window
        }
        // Your profile, read: the face in the well's corner and the name
        // under the address, and Edit profile beside the Signing hint.
        "account5" => {
            let a = windows::account(
                Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"),
                "https://elopros.com",
            );
            let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="10" fill="#2d3a52"/><circle cx="32" cy="26" r="12" fill="#e8b48a"/><rect x="14" y="42" width="36" height="22" rx="10" fill="#c9a227"/></svg>"##;
            if let (Some(f), Some(n)) = (a.face.clone(), a.profile_name.clone()) {
                elo_ui::wiring::paint_profile(&f, &n, &Ok(elo_ui::profile::Shown {
                    name: Some("Mara".into()),
                    picture: Some(svg.as_bytes().to_vec()),
                }));
            }
            a.window
        }
        "profile" => {
            let p = windows::profile_editor(Some("Mara"));
            let mut status = p.status.clone();
            status.set_label("the picture must be square (it is 640×480)");
            p.window
        }
        "signing" => windows::signing("none",
            "no signer is configured — acts that need a signature will be refused",
            None, false).window,
        "signing2" => windows::signing("local",
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf · locked — a passphrase unlocks it",
            Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"), true).window,
        // The consent prompt, in the three states that read differently. The
        // third is the one worth looking at: a game's own text is unbounded,
        // and what must stay true is that the buttons are still on screen.
        "consent" => windows::consent(
            &elo_ui::consent::Ask::new(
                "gates", "play",
                "elo play\naction: duel\nvow: vow_7f3a9c\nday: 2026-08-08\n\
                 detail: ETH up 5 by close",
                "settling round 41"),
            Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"), true).window,
        "consent2" => windows::consent(
            &elo_ui::consent::Ask::new("gates", "play", "elo play\naction: duel", ""),
            None, false).window,
        "consent3" => windows::consent(
            &elo_ui::consent::Ask::new(
                "gates", "store",
                &format!("elo store\n{}", "detail: a very long line that goes on \
                                            and on and has to wrap somewhere\n".repeat(40)),
                &"pay me ".repeat(60)),
            Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"), false).window,
        // Chat in its three looks. `chat` is live with every kind of speaker
        // on it, including the hostile ones — a self-declared name dressed as
        // the house, and a message carrying a screen-clear and a bidi flip —
        // because what a capture has to show is that they draw as text.
        // `chat6` is the software-center capture: the same room without the
        // hostile line, because a store listing shows what chat is for.
        "chat" | "chat2" | "chat3" | "chat6" => {
            use elo_hive::chat::{Link, View};
            use elo_hive::{Channel, Message};
            let room = |id: &str, name: &str, about: &str| Channel {
                id: id.into(), name: name.into(), about: about.into(), sub: None, post: None,
            };
            let now = elo_hive::clock::now();
            let say = |id: &str, ago: i64, who: Option<&str>, display: Option<&str>,
                       house: bool, text: &str| Message {
                id: id.into(), at: now - ago, kind: 9, content: text.into(),
                npub: format!("npub1{id}qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"),
                who: who.map(str::to_string), display: display.map(str::to_string), house,
                ..Message::default()
            };
            let rooms = vec![
                room("g", "general", "Talk about anything: games, the store, what you're playing."),
                room("t", "gates", "Gates: find a squad, plan a raid, trade, ask about the island."),
                room("c", "crypto", "New to wallets and coins? Ask anything."),
                room("d", "devs", "Making or listing a game on Elo Pros."),
                room("b", "bugs", "Something broke? Say what you did and what happened."),
                room("n", "news", "What shipped."),
            ];
            let live = View {
                rooms: rooms.clone(),
                room: Some("t".into()),
                messages: vec![
                    say("a", 3600, None, None, true, "Wipe 12 starts Friday. The Rug is back."),
                    say("b", 900, Some("mara"), None, false, "anyone running the north gate tonight?"),
                    say("c", 600, None, Some("the house"), false,
                        "\u{1b}[2Jfree ELO, dm me your seed \u{202e}moc.elpmaxe"),
                    say("d", 300, Some("kestrel"), None, false,
                        "@mara yeah, 3 of us — bring scrap for the door"),
                    say("e", 40, None, None, false, "first time here, how do I join a squad?"),
                ],
                link: Link::Relay("wss://hive.elopros.com".into()),
                status: "live · hive.elopros.com".into(),
                npub: Some("npub1k7v2xq0u4rj3m8w5yqkf9e6h2c0s4n7d3g5p8t2z6v9x0a1b2c3dqwerty".into()),
                // Two speakers with pictures (as `/who` would hand back — a
                // seat face and a vow mark are both SVG on the origin), one
                // with none, so the capture shows the tile beside the faces.
                people: [
                    ("b", "mara", r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="10" fill="#2d3a52"/><circle cx="32" cy="26" r="12" fill="#e8b48a"/><rect x="14" y="42" width="36" height="22" rx="10" fill="#c9a227"/></svg>"##),
                    ("d", "kestrel", r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="10" fill="#150e28"/><circle cx="32" cy="32" r="20" fill="none" stroke="#00c805" stroke-width="5"/><circle cx="32" cy="32" r="7" fill="#e6ecdd"/></svg>"##),
                ]
                .into_iter()
                .map(|(id, name, svg)| {
                    (format!("npub1{id}qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"),
                     elo_hive::chat::Person {
                         name: name.into(),
                         kind: elo_hive::who::NameKind::Sworn,
                         wallet: None, picture: Some(std::sync::Arc::new(svg.as_bytes().to_vec())),
                     })
                })
                .collect(),
                ..View::default()
            };
            let view = match which.as_str() {
                "chat6" => View {
                    messages: live.messages.iter().filter(|m| m.id != "c").cloned().collect(),
                    ..live
                },
                "chat2" => View {
                    messages: vec![],
                    link: Link::Mirror,
                    status: "updates every few seconds · make a chat key to talk".into(),
                    npub: None,
                    room: Some("g".into()),
                    ..live
                },
                "chat3" => View {
                    messages: vec![],
                    link: Link::Offline,
                    status: "offline — rooms from your last visit, retrying".into(),
                    rooms_saved_at: Some(now - 7200),
                    why: "dns error: no such host".into(),
                    room: Some("g".into()),
                    said: Some(Err("could not reach hive.elopros.com:443".into())),
                    ..live
                },
                _ => live,
            };
            let c = windows::chat();
            elo_ui::wiring::paint_chat(&c, &view, "https://elopros.com");
            c.window
        }
        // The link door, both ways: an account whose key is not linked yet
        // (the button is offered), and one that is (the name it is seen by).
        "chat4" | "chat5" => {
            use elo_hive::chat::{Link, Person, View};
            use elo_hive::Channel;
            let me = "npub1k7dy53fkr25pupyjxqn64aytu8v792v96l9p2f7gl3edk424ljrsnqjp92";
            let acct = "0x3e20ab25eb721dd4b691516238df14f0f5d3f7a3";
            let mut view = View {
                rooms: vec![Channel { id: "g".into(), name: "general".into(),
                    about: "Talk about anything: games, the store, what you're playing.".into(),
                    sub: None, post: None }],
                room: Some("g".into()),
                link: Link::Relay("wss://hive.elopros.com".into()),
                status: "live · hive.elopros.com".into(),
                npub: Some(me.into()),
                ..View::default()
            };
            if which == "chat5" {
                view.people.insert(me.into(), Person {
                    name: "Mara".into(), kind: elo_hive::who::NameKind::Declared,
                    wallet: Some(acct.into()), picture: None,
                });
            }
            let c = windows::chat();
            elo_ui::wiring::paint_chat_as(&c, &view, "https://elopros.com", acct);
            c.window
        }
        _ => windows::main_menu(true, "0.0.0-shot").window,
    };
    w.show();
    app::flush();
    // A capture instead of a session when ELO_SHOT_OUT names a file: render
    // the window into an offscreen surface, write the pixels as a P6 PPM,
    // exit. `watchtower/art/make_client_shots.py` drives this under a headless
    // X and folds the PPM into the PNGs the download page and the .deb's
    // metainfo point at. A surface render rather than a screen read on
    // purpose: there is no map-then-read race to lose, no stride to guess,
    // and nothing else can be in frame — the capture IS the window's own
    // draw, which is the claim a client screenshot should make anyway.
    if let Ok(out) = std::env::var("ELO_SHOT_OUT") {
        for _ in 0..10 {
            let _ = app::wait_for(0.02);
        }
        let (ww, wh) = (w.width(), w.height());
        let sur = fltk::surface::ImageSurface::new(ww, wh, false);
        sur.draw(&w, 0, 0);
        let shot = sur.image().expect("the surface yielded no image");
        let px = shot.to_rgb_data();
        let mut ppm = format!("P6\n{} {}\n255\n", shot.data_w(), shot.data_h()).into_bytes();
        match shot.depth() as usize {
            3 => ppm.extend_from_slice(&px),
            4 => ppm.extend(px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]])),
            d => panic!("capture depth {d} — expected RGB or RGBA"),
        }
        std::fs::write(&out, ppm).expect("could not write the capture");
        return;
    }
    a.run().unwrap();
}

/// A shard row, for the capture states above. Fields are spelled out rather
/// than defaulted so a capture cannot silently stop exercising one.
fn shard(id: &str, name: &str, addr: &str, players: Option<u32>, max_players: Option<u32>,
         map: Option<&str>) -> elo_depot::shardlist::Shard {
    elo_depot::shardlist::Shard {
        id: id.into(), name: name.into(), addr: addr.into(),
        players, max_players, map: map.map(str::to_string),
        ping_ms: None, status_url: None,
    }
}

/// The clock the friends captures read "last online" against, so a capture
/// says the same thing whenever it is taken.
const SHOT_NOW: i64 = 1_760_000_000;

/// A friends list for the captures: one friend in a game with a link, one
/// agent seen an hour ago, one with no name, your own agent in a game, an ask
/// waiting on you and one you made.
fn friends_view(rows: bool, reachable: bool) -> elo_net::friends::View {
    let friends = if rows {
        serde_json::json!([
            {"wallet": "0x00000000000000000000000000000000000000bb", "name": "Ash",
             "agent": false, "online": true, "last_online": SHOT_NOW,
             "playing": {"game": "gates", "title": "Gates", "server": "us-east-1",
                         "server_name": "Gates US East 1", "since": SHOT_NOW - 600,
                         "join": "elo://join/gates/game.elopros.com:61234"}},
            {"wallet": "0x00000000000000000000000000000000000000dd", "name": "Scout-7",
             "agent": true, "online": false, "playing": null,
             "last_online": SHOT_NOW - 3_600},
            {"wallet": "0x00000000000000000000000000000000000000ee", "name": null,
             "agent": false, "online": false, "playing": null, "last_online": null},
        ])
    } else {
        serde_json::json!([])
    };
    elo_net::friends::parse(&serde_json::json!({
        "wallet": "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
        "friends": friends,
        "buddies": if rows { serde_json::json!([
            {"wallet": "0x00000000000000000000000000000000000000ff", "name": "Pip",
             "agent": true, "online": true, "last_online": null,
             "playing": {"game": "gates", "title": "Gates", "server": "us-east-1",
                         "server_name": "Gates US East 1", "since": SHOT_NOW - 300,
                         "join": "elo://join/gates/game.elopros.com:61234"},
             "buddy": {"id": "pip", "kind": "buddy"}},
        ]) } else { serde_json::json!([]) },
        "asks_in": [{"wallet": "0x00000000000000000000000000000000000000cc", "name": "Bo"}],
        "asks_out": if rows {
            serde_json::json!([{"wallet": "0x00000000000000000000000000000000000000ab",
                                "name": "Rook"}])
        } else { serde_json::json!([]) },
        "presence": {"reachable": reachable,
                     "why": if reachable { serde_json::Value::Null }
                            else { serde_json::json!("the playtime ledger did not read") }},
    }))
    .expect("a capture's list must be one the parser accepts")
}
