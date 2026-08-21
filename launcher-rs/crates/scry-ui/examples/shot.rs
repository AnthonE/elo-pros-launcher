//! Open a window so it can be looked at. `cargo run -p scry-ui --example shot -- games`
use fltk::{app, prelude::*};
use scry_ui::windows::{self, Price, Row, Shelf};

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
        // Mid-download, posed exactly as `wiring::drive_meter` leaves it: the
        // meter standing where the status line stood, the byte count beside
        // it, the button already saying what it is doing. A capture rather
        // than only an assertion, because what a picture shows is whether the
        // count CLEARS the Update button and the fill reads against the well.
        "downloading" => {
            let g = windows::games(&[
                row("gates", "0.1.0-g607af0314", 75_829_730, Some(true), Some("Gates")),
                row("barrow", "0.3.0", 4_100_000, Some(false), Some("The Barrow")),
            ]);
            let r = &g.rows[0];
            let (mut bar, mut status) = (r.progress.clone(), r.status.clone());
            status.hide();
            bar.show();
            bar.set_value(37.0);
            bar.set_label("28.0 MB of 75.8 MB");
            let mut act = r.act.clone();
            act.deactivate();
            act.set_label("Updating…");
            g.window
        }
        "about" => windows::about(),
        "store" => windows::store(&[], false, "dns error: no such host").window,
        "store2" => windows::store(&[
            shelf("gates", "Gates", GATES_BLURB, "live", true, false, Price::Free),
            shelf("barrow", "The Barrow", "a three-room delve — fight, sneak, or leave",
                  "in-build", false, false, Price::Free),
            // Sample data, and the figure is arbitrary on purpose: a price is
            // whatever that title's dev posted, so a capture must not look
            // like the storefront has one number in it.
            shelf("sold", "A Title Its Dev Priced", GATES_BLURB, "live", true, false,
                  Price::Posted { line: "$14.99, paid in ETH".into() }),
            shelf("dark", "A Title We Could Not Price", GATES_BLURB, "live", true, false,
                  Price::Unknown { why: "could not reach the origin".into() }),
            shelf("mine", "Already Installed", GATES_BLURB, "live", true, true, Price::Free),
        ], true, "").window,
        // The passphrase prompt, in both shapes. `pass2` is the browser
        // pairing's — the whole message being signed rides in the prompt.
        "pass" => windows::passphrase("Unlock", "Passphrase:").window,
        "pass2" => windows::passphrase(
            "Sign the browser in",
            "About to sign exactly this, and nothing else:\n\n\
             scry.moreright.xyz wants to sign you in\naddress: 0x7E5F…95Bdf\n\
             code: ABCD2345\nissued: 2026-08-12T09:00:00Z\n\nPassphrase:").window,
        "notice" => windows::notice(windows::Note::Done, "Account created",
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf\n\n\
             The keystore is the only copy of this key.\n\
             Back it up. The passphrase cannot be reset.", false).window,
        "notice2" => windows::notice(windows::Note::Refused, "The game door did not open",
            "Games will still install and start — they will just play anonymously.\n\n\
             The usual cause is another copy of scry already running.\n\n\
             Nothing retries this while the program is open, so a restart is the fix.",
            true).window,
        "lock" => windows::lock("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "unlock the account on this machine to carry on").window,
        "lock2" => windows::lock("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "it locked itself while you were away — every signature resets that clock").window,
        // All four shard states are capturable, because a reader must be able
        // to tell them apart at a glance — and three of them are the ones that
        // get drawn wrong. `unreadable` in particular must never look like
        // `no shards up`.
        "servers" => windows::servers("gates", &windows::Shards::Unpublished).window,
        "servers2" => windows::servers("gates", &windows::Shards::Listed {
            url: "https://scry.moreright.xyz/api/launcher/servers/gates".into(),
            rows: vec![
                shard("us-east-1", "Gates US East 1", "game.moreright.xyz:61234",
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
            url: "https://scry.moreright.xyz/api/launcher/servers/gates".into(),
            why: "could not reach it — dns error".into(),
            reachable: false,
        }).window,
        "servers4" => windows::servers("gates", &windows::Shards::Listed {
            url: "https://scry.moreright.xyz/api/launcher/servers/gates".into(),
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
    // A capture instead of a session when SCRY_SHOT_OUT names a file: render
    // the window into an offscreen surface, write the pixels as a P6 PPM,
    // exit. `watchtower/art/make_client_shots.py` drives this under a headless
    // X and folds the PPM into the PNGs the download page and the .deb's
    // metainfo point at. A surface render rather than a screen read on
    // purpose: there is no map-then-read race to lose, no stride to guess,
    // and nothing else can be in frame — the capture IS the window's own
    // draw, which is the claim a client screenshot should make anyway.
    if let Ok(out) = std::env::var("SCRY_SHOT_OUT") {
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
         map: Option<&str>) -> scry_depot::shardlist::Shard {
    scry_depot::shardlist::Shard {
        id: id.into(), name: name.into(), addr: addr.into(),
        players, max_players, map: map.map(str::to_string),
        ping_ms: None, status_url: None,
    }
}
