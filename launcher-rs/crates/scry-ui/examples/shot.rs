//! Open a window so it can be looked at. `cargo run -p scry-ui --example shot -- games`
use fltk::{app, prelude::*};
use scry_ui::windows::{self, Row};

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "menu".into());
    let a = app::App::default();
    let mut w = match which.as_str() {
        "games" => windows::games(&[
            Row { slug: "gates".into(), build: "0.1.0-g607af0314".into(),
                  bytes: 75_829_730, digest: "0xf43be2a2".into(), stale: Some(false) },
            Row { slug: "gates".into(), build: "0.2.0-gdeadbee".into(),
                  bytes: 75_829_730, digest: "0xabc".into(), stale: Some(true) },
            Row { slug: "unreached".into(), build: "0.1.0".into(),
                  bytes: 4_100_000, digest: "0x0".into(), stale: None },
        ]),
        "empty" => windows::games(&[]),
        "about" => windows::about(),
        "store" => windows::store(&[], false),
        "store2" => windows::store(&[
            ("Gates".into(), "survival — 100-player shards. Wake with nothing on a hostile island.".into()),
            ("The Barrow".into(), "a three-room delve — fight, sneak, or leave".into()),
        ], true),
        // All four shard states are capturable, because a reader must be able
        // to tell them apart at a glance — and three of them are the ones that
        // get drawn wrong. `unreadable` in particular must never look like
        // `no shards up`.
        "servers" => windows::servers("gates", &windows::Shards::Unpublished).window,
        "servers2" => windows::servers("gates", &windows::Shards::Listed {
            url: "https://scry.moreright.xyz/depot/gates/servers.json".into(),
            rows: vec![
                shard("eu-1", "Gates EU 1", "game.moreright.xyz:61234",
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
            url: "https://scry.moreright.xyz/depot/gates/servers.json".into(),
            why: "could not reach it — dns error".into(),
            reachable: false,
        }).window,
        "servers4" => windows::servers("gates", &windows::Shards::Listed {
            url: "https://scry.moreright.xyz/depot/gates/servers.json".into(),
            rows: vec![],
        }).window,
        // Both states are capturable, because they are different windows: with
        // no account the two make/import doors are the point of the screen,
        // and with one they are deliberately absent.
        "account" => windows::account(None, "https://scry.moreright.xyz").window,
        "account2" => windows::account(
            Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"),
            "https://scry.moreright.xyz",
        )
        .window,
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
            &scry_ui::consent::Ask::new(
                "gates", "play",
                "scry play\naction: duel\nvow: vow_7f3a9c\nday: 2026-08-08\n\
                 detail: ETH up 5 by close",
                "settling round 41"),
            Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"), true).window,
        "consent2" => windows::consent(
            &scry_ui::consent::Ask::new("gates", "play", "scry play\naction: duel", ""),
            None, false).window,
        "consent3" => windows::consent(
            &scry_ui::consent::Ask::new(
                "gates", "store",
                &format!("scry store\n{}", "detail: a very long line that goes on \
                                            and on and has to wrap somewhere\n".repeat(40)),
                &"pay me ".repeat(60)),
            Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"), false).window,
        _ => windows::main_menu(true, "0.0.0-shot").window,
    };
    w.show();
    app::flush();
    a.run().unwrap();
}

/// A shard row, for the capture states above. Fields are spelled out rather
/// than defaulted so a capture cannot silently stop exercising one.
fn shard(id: &str, name: &str, addr: &str, players: Option<u32>, max_players: Option<u32>,
         map: Option<&str>) -> scry_depot::shardlist::Shard {
    scry_depot::shardlist::Shard {
        id: id.into(), name: name.into(), addr: addr.into(),
        players, max_players, map: map.map(str::to_string),
        ping_ms: None, status_url: None,
    }
}
