//! Press the buttons for real, with no hands.
//!
//!     xvfb-run -a cargo run -p scry-ui --example click
//!
//! # Why this exists
//!
//! A screenshot proves a control is PAINTED. It cannot prove the control does
//! anything, and that distinction is the whole of the defect this example was
//! written for: every window drew correctly while the client contained exactly
//! one callback in the entire program, so the Account window was six labels and
//! two buttons that did nothing.
//!
//! So this drives the real `wiring::wire_account` / `wire_signing` callbacks
//! through `do_callback()`, with [`wiring::scripted_ask`] standing in for the
//! modal dialogs, and asserts against the DISK: a keystore that exists, opens,
//! and holds the address the window claims. Nothing here is a mock of the thing
//! under test — only of the human.
//!
//! It needs a display, which is why it is an example and not a `#[test]`: the
//! unit tests in `wiring.rs` cover what can be checked without one.

use fltk::{app, prelude::*};
use scry_ui::wiring::{self, Told};
use scry_ui::windows;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

fn main() {
    let _app = app::App::default();
    let dir = std::env::temp_dir().join(format!("scry-click-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let keystore = dir.join("keystore.json");
    let mut failures = 0;

    // ── 1 · Make an account, by pressing the button ──────────────────────────
    let account_w = windows::account(None, "https://scry.moreright.xyz");
    let signing_w = windows::signing("local", "no account", None, false);
    let signer: wiring::Shared = Arc::new(Mutex::new(scry_vault::LocalSigner::at(&keystore)));
    let log = Rc::new(RefCell::new(Vec::new()));

    wiring::wire_account(
        &account_w,
        &signing_w,
        &signer,
        &keystore,
        wiring::scripted_ask(vec![Some("correct horse".into()), Some("correct horse".into())]),
        wiring::recording_tell(Rc::clone(&log)),
    );

    let mut make = account_w.make.clone().expect("the make button must exist with no account");
    make.do_callback();

    check(&mut failures, "the keystore exists on disk", keystore.is_file());
    let told = log.borrow().first().cloned();
    check(&mut failures, "the player was told it worked",
          matches!(told, Some((Told::Done, _))));

    // The address on screen is the address in the file — the window is not
    // allowed to claim an account the disk does not hold.
    let on_screen = account_w.address.label();
    let mut reopened = scry_vault::LocalSigner::at(&keystore);
    let on_disk = {
        use scry_broker::signer::Signer;
        reopened.address().unwrap_or_default()
    };
    check(&mut failures, "the window shows the address the file holds",
          on_screen == on_disk && on_screen.starts_with("0x"));

    // And the door closes: this window never offers to replace a key.
    check(&mut failures, "make is deactivated afterwards", !make.active());

    // ── 2 · The passphrase it wrote is the passphrase it was given ───────────
    check(&mut failures, "the written keystore opens with that passphrase",
          reopened.unlock("correct horse").is_ok());

    // ── 3 · Unlock, by pressing the button ───────────────────────────────────
    let signing2 = windows::signing("local", "locked", Some(&on_disk), true);
    let signer2: wiring::Shared = Arc::new(Mutex::new(scry_vault::LocalSigner::at(&keystore)));
    let log2 = Rc::new(RefCell::new(Vec::new()));
    wiring::wire_signing(
        &signing2,
        &signer2,
        wiring::scripted_ask(vec![Some("correct horse".into())]),
        wiring::recording_tell(Rc::clone(&log2)),
    );
    let mut unlock = signing2.unlock.clone().expect("unlock exists when unlockable");
    unlock.do_callback();
    {
        use scry_broker::signer::Signer;
        check(&mut failures, "the account is unlocked in memory", signer2.lock().unwrap().is_unlocked());
        // And it can actually sign — the point of holding a key at all.
        let signed = signer2.lock().unwrap().sign("hello", "click example");
        check(&mut failures, "an unlocked account signs", signed.is_ok());
    }

    // ── 4 · A wrong passphrase refuses and does NOT unlock ───────────────────
    let signing3 = windows::signing("local", "locked", Some(&on_disk), true);
    let signer3: wiring::Shared = Arc::new(Mutex::new(scry_vault::LocalSigner::at(&keystore)));
    let log3 = Rc::new(RefCell::new(Vec::new()));
    wiring::wire_signing(
        &signing3,
        &signer3,
        wiring::scripted_ask(vec![Some("wrong".into())]),
        wiring::recording_tell(Rc::clone(&log3)),
    );
    signing3.unlock.clone().expect("unlock").do_callback();
    // `is_unlocked` is inherent to `LocalSigner`, so unlike the blocks above
    // this one needs no `Signer` trait in scope.
    check(&mut failures, "a wrong passphrase leaves it locked",
          !signer3.lock().unwrap().is_unlocked());
    let said = log3.borrow().first().cloned();
    check(&mut failures, "a wrong passphrase is REFUSED, and says wrong rather than corrupt",
          matches!(&said, Some((Told::Refused, m)) if m.contains("passphrase") && !m.contains("corrupt")));

    // ── 5 · Cancelling writes nothing ────────────────────────────────────────
    let dir2 = dir.join("cancelled");
    let ks2 = dir2.join("keystore.json");
    let aw = windows::account(None, "h");
    let sw = windows::signing("local", "none", None, false);
    let s2: wiring::Shared = Arc::new(Mutex::new(scry_vault::LocalSigner::at(&ks2)));
    wiring::wire_account(&aw, &sw, &s2, &ks2,
                         wiring::scripted_ask(vec![None]),
                         wiring::recording_tell(Rc::new(RefCell::new(Vec::new()))));
    aw.make.clone().expect("make").do_callback();
    check(&mut failures, "cancelling the dialog writes no keystore", !ks2.exists());

    // ── 6 · unlocking in the window is visible to the GAME DOOR ─────────────
    //
    // The door runs on another thread, which is the whole reason the account is
    // `Arc<Mutex<…>>`. If it held its own copy, a player could unlock in
    // Signing and every `prove` would still answer "locked" — a launcher that
    // unlocks in a window nothing else can see. Asserted through the real
    // `Host` impl, not by reading the field.
    {
        use scry_broker::signer::Signer as _;
        use scry_broker::Host;
        struct Door(wiring::Shared);
        impl Host for Door {
            fn address(&self) -> Option<String> { self.0.lock().ok().and_then(|s| s.address()) }
            fn signer_name(&self) -> String { "local".into() }
            fn host_url(&self) -> String { "https://x".into() }
            fn ask_consent(&mut self, _g: &str, _f: &str, _t: &str, _w: &str) -> Option<u32> { None }
            fn sign(&mut self, _t: &str, _w: &str)
                -> Result<serde_json::Value, scry_broker::protocol::Refusal> {
                Err(scry_broker::protocol::Refusal::new("no"))
            }
            fn prove(&mut self, m: &str)
                -> Result<serde_json::Value, scry_broker::protocol::Refusal> {
                let mut s = self.0.lock().map_err(|_| scry_broker::protocol::Refusal::new("busy"))?;
                if !s.is_unlocked() {
                    return Err(scry_broker::protocol::Refusal::new("the account is locked"));
                }
                s.sign(m, "prove identity")
            }
            fn title_url(&self, _s: &str, _w: scry_broker::What) -> Option<String> { None }
            fn open(&mut self, _u: &str) -> Result<(), scry_broker::protocol::Refusal> { Ok(()) }
        }

        let shared: wiring::Shared =
            Arc::new(Mutex::new(scry_vault::LocalSigner::at(&keystore)));
        let mut door = Door(Arc::clone(&shared));
        // A SIWE (EIP-4361) message. The address is bound INTO it, so it is
        // read off the keystore rather than being an unrelated string — a proof
        // composed for one account and signed by another cannot verify.
        let addr = scry_vault::LocalSigner::at(&keystore).address().unwrap_or_default();
        let msg = scry_broker::protocol::prove_message(
            "s.example", &addr, "gates", "n0nce4d8f1e2b7c",
            &scry_broker::protocol::now_iso8601());

        check(&mut failures, "a locked account cannot prove", door.prove(&msg).is_err());

        // Now unlock through the WINDOW, as a player would.
        let sw = windows::signing("local", "locked", None, true);
        wiring::wire_signing(&sw, &shared,
                             wiring::scripted_ask(vec![Some("correct horse".into())]),
                             wiring::recording_tell(Rc::new(RefCell::new(Vec::new()))));
        sw.unlock.clone().expect("unlock").do_callback();

        let proved = door.prove(&msg);
        check(&mut failures, "unlocking in the window lets the game door prove",
              proved.is_ok());
        if let Ok(v) = proved {
            let sig = v.get("signature").and_then(|s| s.as_str()).unwrap_or_default();
            let who = scry_vault::recover_signer(&msg, sig).unwrap_or_default();
            check(&mut failures, "and the proof recovers to this machine's address",
                  Some(who.as_str()) == door.address().as_deref());
        }
    }

    // ── 7 · the consent window, pressed for real ────────────────────────────
    //
    // The one window a GAME can cause to open, and the only place a signature
    // is ever approved. `wiring::wire_consent` is what a press means; the
    // nested event loop around it is the only part a test cannot drive, which
    // is exactly why the meaning was split out of it.
    {
        let ask = scry_ui::consent::Ask::new(
            "gates",
            "play",
            "scry play\naction: duel\nvow: vow_x\nday: 2026-08-08\ndetail: ETH up 5",
            "settling round 41",
        );

        // Refusing answers None — and the window closes, or the door thread
        // waits forever on a prompt nobody can dismiss.
        let win = windows::consent(&ask, Some(&on_disk), true);
        let answer = wiring::wire_consent(&win);
        win.refuse.clone().do_callback();
        check(&mut failures, "Refuse answers no", answer.get().is_none());
        check(&mut failures, "Refuse closes the prompt", !win.window.shown());

        // Each allowance grants its own number, in the order the table names.
        for (i, (label, grant)) in scry_ui::consent::ALLOWANCES.iter().enumerate() {
            let win = windows::consent(&ask, Some(&on_disk), true);
            let answer = wiring::wire_consent(&win);
            win.allow[i].clone().do_callback();
            check(&mut failures, &format!("\"{label}\" grants {grant}"),
                  answer.get() == Some(*grant));
        }

        // Nothing is granted by the window merely existing. The player has to
        // press something, and a prompt that timed out into a grant would be
        // the one bug this whole path exists to prevent.
        let win = windows::consent(&ask, Some(&on_disk), true);
        let answer = wiring::wire_consent(&win);
        check(&mut failures, "an unanswered prompt has granted nothing",
              answer.get().is_none());
    }

    // ── 8 · a game asks, a player answers, across the real thread boundary ──
    //
    // The whole path: the real broker parsing a real `sign` request, the real
    // `Consent` counter, the real `Doorbell` crossing to another thread and
    // back, and a real key signing at the end. Only the human is stood in for.
    {
        use scry_broker::signer::Signer as _;
        use scry_broker::{protocol::Refusal, Host};
        use scry_ui::consent::{Ask, Doorbell};

        struct Door {
            signer: wiring::Shared,
            bell: Doorbell,
        }
        // The same two methods `scry-gui` implements. Copied rather than
        // shared because an integration test cannot reach into a `[[bin]]` —
        // and a copy is only worth something if the original behaves the same
        // way, which is why the binary carries its own `mod tests` holding it
        // to these refusals rather than leaving this as the only check.
        impl Host for Door {
            fn address(&self) -> Option<String> { self.signer.lock().ok().and_then(|s| s.address()) }
            fn signer_name(&self) -> String { "local".into() }
            fn host_url(&self) -> String { "https://x".into() }
            fn ask_consent(&mut self, g: &str, f: &str, t: &str, w: &str) -> Option<u32> {
                self.bell.ask(Ask::new(g, f, t, w))
            }
            fn sign(&mut self, t: &str, w: &str) -> Result<serde_json::Value, Refusal> {
                let mut s = self.signer.lock().map_err(|_| Refusal::new("busy"))?;
                if !s.is_unlocked() {
                    return Err(Refusal::new("the account is locked — unlock it in Signing"));
                }
                s.sign(t, w)
            }
            fn title_url(&self, _s: &str, _w: scry_broker::What) -> Option<String> { None }
            fn open(&mut self, _u: &str) -> Result<(), Refusal> { Ok(()) }
        }

        /// Run one `sign` request the way the client does: the broker on a
        /// door thread, the player on this one.
        ///
        /// The sleep is not a timing assumption — the loop pumps first and
        /// checks second, so the only thing a slow thread costs is another
        /// pass. It is the same arrangement as the real timeout, at a tighter
        /// interval because nothing here is waiting on a human.
        fn drive(
            keystore: &std::path::Path,
            pass: &str,
            hello: &serde_json::Value,
            request: &serde_json::Value,
            decide: impl Fn(&Ask) -> Option<u32>,
        ) -> serde_json::Value {
            let mut s = scry_vault::LocalSigner::at(keystore);
            s.unlock(pass).expect("the keystore must open");
            let signer: wiring::Shared = Arc::new(Mutex::new(s));
            let bell = Doorbell::new();

            let (door_bell, hello, request) = (bell.clone(), hello.clone(), request.clone());
            let door = std::thread::spawn(move || {
                let mut host = Door { signer, bell: door_bell };
                let mut session = scry_broker::Session::new();
                session.handle(&mut host, &hello);
                session.handle(&mut host, &request)
            });

            loop {
                bell.pump(&mut |ask| decide(ask));
                if door.is_finished() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            door.join().expect("the door thread panicked")
        }

        let request = serde_json::json!({
            "op": "sign",
            "text": "scry play\naction: duel\nvow: vow_x\nday: 2026-08-08\ndetail: ETH up 5",
            "why": "settling round 41",
        });
        let hello = serde_json::json!({"op": "hello", "game": "gates", "protocol": 1});

        // A refusal reaches the game as a refusal, and no signature comes out.
        let refused = drive(&keystore, "correct horse", &hello, &request, |_| None);
        check(&mut failures, "a refused prompt refuses the game",
              refused.get("ok") == Some(&serde_json::json!(false)));
        check(&mut failures, "and no signature is produced",
              refused.get("signature").is_none());
        check(&mut failures, "and the game is told it was the player",
              refused.get("refused_by").and_then(|v| v.as_str()) == Some("the player"));

        // A grant reaches the signer, and the signature recovers to this
        // machine's account — the door signed with the key the window holds.
        let allowed = drive(&keystore, "correct horse", &hello, &request, |ask| {
            // The prompt got the game's own words, both of them.
            assert_eq!(ask.game, "gates", "hello's name must reach the prompt");
            assert_eq!(ask.family, "play", "the family is parsed from the message");
            assert!(ask.text.contains("ETH up 5"), "the message must arrive verbatim");
            assert!(ask.why.contains("round 41"), "the game's reason must reach the player");
            Some(1)
        });
        check(&mut failures, "an allowed prompt signs",
              allowed.get("ok") == Some(&serde_json::json!(true)));
        let sig = allowed.get("signature").and_then(|s| s.as_str()).unwrap_or_default();
        let text = request["text"].as_str().unwrap();
        let who = scry_vault::recover_signer(text, sig).unwrap_or_default();
        check(&mut failures, "and the signature recovers to this machine's account",
              who == on_disk);
        check(&mut failures, "and the reply names the family that was approved",
              allowed.get("family").and_then(|f| f.as_str()) == Some("play"));
    }

    // ── 9 · Update becomes Play by being pressed — never Update twice ───────
    //
    // Operator, 2026-08-15: *"after i update and hit play it just updates
    // again."* The verb was decided when the window was wired and moved into
    // the callback, so the button a landed update relabels **Play** still ran
    // the update — every press re-downloaded the build it already had, and
    // nothing ever launched. The storefront here counts what each press
    // actually did, which is the one thing the relabelled button cannot show.
    {
        use std::cell::Cell;

        struct Counting {
            installs: Rc<Cell<u32>>,
            plays: Rc<Cell<u32>>,
            /// The row's meter and the status line it stands in for, cloned in
            /// so the MID-download state can be recorded: the meter being up
            /// while bytes move is exactly what no after-the-fact assertion
            /// can see, because `meter_rest` runs before install returns.
            bar: fltk::misc::Progress,
            status: fltk::frame::Frame,
            mid: Rc<Cell<bool>>,
        }
        impl wiring::Storefront for Counting {
            fn install(&self, _slug: &str, tick: wiring::Meter<'_>) -> Result<String, String> {
                self.installs.set(self.installs.get() + 1);
                // What a real install does, in miniature: a first chunk, a
                // half-way chunk, the last one.
                tick(1, 100);
                tick(50, 100);
                self.mid.set(
                    self.bar.visible()
                        && !self.status.visible()
                        && (self.bar.value() - 50.0).abs() < f64::EPSILON,
                );
                tick(100, 100);
                Ok("gates 0.2.0 is installed.".into())
            }
            fn verify(&self, _slug: &str, _build: &str) -> Result<String, String> {
                Ok("verified".into())
            }
            fn play(&self, _slug: &str) -> Result<String, String> {
                self.plays.set(self.plays.get() + 1);
                Ok("running".into())
            }
            fn open(&self, _url: &str) -> Result<(), String> {
                Ok(())
            }
            fn page_url(&self, _slug: &str) -> String {
                "https://x.example".into()
            }
        }

        // A row whose build is measured stale, so the window offers Update.
        let stale = || windows::Row {
            slug: "gates".into(),
            build: "0.1.0".into(),
            bytes: 75_829_730,
            digest: "0xold".into(),
            stale: Some(true),
            name: None,
            icon: None,
            why: None,
        };

        let (installs, plays) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(0u32)));
        let (refreshed, mid) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(false)));
        let games_w = windows::games(&[stale()]);
        let after = {
            let refreshed = Rc::clone(&refreshed);
            Rc::new(move || refreshed.set(refreshed.get() + 1)) as Rc<dyn Fn()>
        };
        wiring::wire_games(
            &games_w,
            Rc::new(Counting {
                installs: Rc::clone(&installs),
                plays: Rc::clone(&plays),
                bar: games_w.rows[0].progress.clone(),
                status: games_w.rows[0].status.clone(),
                mid: Rc::clone(&mid),
            }),
            wiring::recording_tell(Rc::new(RefCell::new(Vec::new()))),
            after,
        );

        let mut act = games_w.rows[0].act.clone();
        let bar = games_w.rows[0].progress.clone();
        check(&mut failures, "a stale row's button says Update", act.label() == "Update");
        check(&mut failures, "and its meter is down until bytes move", !bar.visible());

        act.do_callback();
        check(&mut failures, "pressing Update installs, and does not play",
              installs.get() == 1 && plays.get() == 0);
        check(&mut failures, "mid-download the meter stands where the status line stood",
              mid.get());
        check(&mut failures, "a landed update leaves the full meter standing",
              bar.visible() && (bar.value() - 100.0).abs() < f64::EPSILON
                  && bar.label().contains(" of "));
        check(&mut failures, "a landed update relabels the button Play",
              act.label() == "Play");
        check(&mut failures, "and asks the library to re-read itself",
              refreshed.get() == 1);

        // The press that was the bug: the button says Play, so it must PLAY.
        act.do_callback();
        check(&mut failures, "pressing that Play plays — not a second update",
              plays.get() == 1 && installs.get() == 1);
        check(&mut failures, "and the row says Running", act.label() == "Running");

        // ── 10 · a failed download gives the row back ────────────────────────
        //
        // The meter stands in for the status line while bytes move, so a
        // download that DIES has to hand the line back — its sentence ("an
        // update is published") is still true. A meter left standing at 4%
        // under a Failed button would be the window keeping the player's one
        // status line as a souvenir.
        struct Failing;
        impl wiring::Storefront for Failing {
            fn install(&self, _slug: &str, tick: wiring::Meter<'_>) -> Result<String, String> {
                tick(4, 100); // the meter went up and moved...
                Err("the origin dropped the connection".into())
            }
            fn verify(&self, _slug: &str, _build: &str) -> Result<String, String> {
                Ok("verified".into())
            }
            fn play(&self, _slug: &str) -> Result<String, String> {
                Ok("running".into())
            }
            fn open(&self, _url: &str) -> Result<(), String> {
                Ok(())
            }
            fn page_url(&self, _slug: &str) -> String {
                "https://x.example".into()
            }
        }

        let failing_w = windows::games(&[stale()]);
        wiring::wire_games(
            &failing_w,
            Rc::new(Failing),
            wiring::recording_tell(Rc::new(RefCell::new(Vec::new()))),
            Rc::new(|| {}),
        );
        let mut act = failing_w.rows[0].act.clone();
        let bar = failing_w.rows[0].progress.clone();
        let status = failing_w.rows[0].status.clone();
        act.do_callback();
        check(&mut failures, "a failed download puts the meter away",
              !bar.visible() && bar.value() == 0.0);
        check(&mut failures, "and gives the status line back", status.visible());
        check(&mut failures, "and the button says Failed", act.label() == "Failed");

        // Failed retries the verb that failed: the next press must try the
        // UPDATE again, not launch the stale build under a player who just
        // asked for the new one.
        act.do_callback();
        check(&mut failures, "pressing Failed retries the update",
              act.label() == "Failed");
    }

    let _ = std::fs::remove_dir_all(&dir);
    if failures == 0 {
        println!("\nclick: every control does what it says");
    } else {
        println!("\nclick: {failures} FAILED");
        std::process::exit(1);
    }
}

fn check(failures: &mut i32, what: &str, ok: bool) {
    println!("  {}  {what}", if ok { "ok  " } else { "FAIL" });
    if !ok {
        *failures += 1;
    }
}
