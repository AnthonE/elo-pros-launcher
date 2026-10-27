//! Press the buttons for real, with no hands.
//!
//!     xvfb-run -a cargo run -p elo-ui --example click
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
use elo_ui::wiring::{self, Told};
use elo_ui::windows;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

fn main() {
    let _app = app::App::default();
    let dir = std::env::temp_dir().join(format!("elo-click-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let keystore = dir.join("keystore.json");
    let mut failures = 0;

    // ── 1 · Make an account, by pressing the button ──────────────────────────
    let account_w = windows::account(None, "https://elopros.com");
    let signing_w = windows::signing("local", "no account", None, false);
    let signer: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&keystore)));
    let log = Rc::new(RefCell::new(Vec::new()));

    // The words the window showed, so the check that follows can be answered
    // the way a player with a pencil would.
    let seen = Rc::new(RefCell::new(Vec::new()));
    wiring::wire_account(
        &account_w,
        &signing_w,
        &signer,
        &keystore,
        wiring::quiz_answering_ask(Rc::clone(&seen),
                                   vec![Some("correct horse".into()), Some("correct horse".into())]),
        wiring::recording_tell(Rc::clone(&log)),
        wiring::recording_show(Rc::clone(&seen)),
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
    let mut reopened = elo_vault::LocalSigner::at(&keystore);
    let on_disk = {
        use elo_broker::signer::Signer;
        reopened.address().unwrap_or_default()
    };
    check(&mut failures, "the window shows the address the file holds",
          on_screen == on_disk && on_screen.starts_with("0x"));
    // And the file holds the account the twelve words derive, which is what
    // makes them a backup rather than a souvenir.
    let from_words = elo_vault::Phrase::parse(&seen.borrow().join(" "))
        .and_then(|p| p.account())
        .map(|a| a.address())
        .unwrap_or_default();
    check(&mut failures, "twelve words were shown, and they derive this account",
          seen.borrow().len() == 12 && from_words == on_disk);

    // And the door closes: this window never offers to replace a key.
    check(&mut failures, "make is deactivated afterwards", !make.active());
    check(&mut failures, "and its place is Edit profile, with no restart",
          !make.visible() && account_w.edit_profile.as_ref().is_some_and(|e| e.visible()));
    check(&mut failures, "a made account says it has no profile yet",
          account_w.profile_name.as_ref().is_some_and(|n| n.label().contains("no profile")));

    // ── 2 · The passphrase it wrote is the passphrase it was given ───────────
    check(&mut failures, "the written keystore opens with that passphrase",
          reopened.unlock("correct horse").is_ok());

    // ── 3 · Unlock, by pressing the button ───────────────────────────────────
    let signing2 = windows::signing("local", "locked", Some(&on_disk), true);
    let signer2: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&keystore)));
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
        use elo_broker::signer::Signer;
        check(&mut failures, "the account is unlocked in memory", signer2.lock().unwrap().is_unlocked());
        // And it can actually sign — the point of holding a key at all.
        let signed = signer2.lock().unwrap().sign("hello", "click example");
        check(&mut failures, "an unlocked account signs", signed.is_ok());
    }

    // ── 4 · A wrong passphrase refuses and does NOT unlock ───────────────────
    let signing3 = windows::signing("local", "locked", Some(&on_disk), true);
    let signer3: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&keystore)));
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
    let s2: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&ks2)));
    let never_shown: wiring::ShowWords = Rc::new(|_| false);
    wiring::wire_account(&aw, &sw, &s2, &ks2,
                         wiring::scripted_ask(vec![Some("correct horse".into())]),
                         wiring::recording_tell(Rc::new(RefCell::new(Vec::new()))),
                         never_shown);
    aw.make.clone().expect("make").do_callback();
    check(&mut failures, "cancelling the words writes no keystore", !ks2.exists());

    let aw = windows::account(None, "h");
    let sw = windows::signing("local", "none", None, false);
    let s2: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&ks2)));
    wiring::wire_account(&aw, &sw, &s2, &ks2,
                         wiring::scripted_ask(vec![None]),
                         wiring::recording_tell(Rc::new(RefCell::new(Vec::new()))),
                         wiring::recording_show(Rc::new(RefCell::new(Vec::new()))));
    aw.make.clone().expect("make").do_callback();
    check(&mut failures, "cancelling the check writes no keystore", !ks2.exists());

    // Words that were not the words: refused, and nothing on disk.
    let aw = windows::account(None, "h");
    let sw = windows::signing("local", "none", None, false);
    let s2: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&ks2)));
    let told = Rc::new(RefCell::new(Vec::new()));
    wiring::wire_account(&aw, &sw, &s2, &ks2,
                         wiring::scripted_ask(vec![Some("zoo zoo zoo".into()),
                                                   Some("correct horse".into()),
                                                   Some("correct horse".into())]),
                         wiring::recording_tell(Rc::clone(&told)),
                         wiring::recording_show(Rc::new(RefCell::new(Vec::new()))));
    aw.make.clone().expect("make").do_callback();
    check(&mut failures, "the wrong words write no keystore, and say so",
          !ks2.exists() && matches!(told.borrow().first(), Some((Told::Refused, m)) if m.contains("not the words")));

    // ── 6 · unlocking in the window is visible to the GAME DOOR ─────────────
    //
    // The door runs on another thread, which is the whole reason the account is
    // `Arc<Mutex<…>>`. If it held its own copy, a player could unlock in
    // Signing and every `prove` would still answer "locked" — a launcher that
    // unlocks in a window nothing else can see. Asserted through the real
    // `Host` impl, not by reading the field.
    {
        use elo_broker::signer::Signer as _;
        use elo_broker::Host;
        struct Door(wiring::Shared);
        impl Host for Door {
            fn address(&self) -> Option<String> { self.0.lock().ok().and_then(|s| s.address()) }
            fn signer_name(&self) -> String { "local".into() }
            fn host_url(&self) -> String { "https://x".into() }
            fn ask_consent(&mut self, _g: &str, _f: &str, _t: &str, _w: &str) -> Option<u32> { None }
            fn sign(&mut self, _t: &str, _w: &str)
                -> Result<serde_json::Value, elo_broker::protocol::Refusal> {
                Err(elo_broker::protocol::Refusal::new("no"))
            }
            fn prove(&mut self, m: &str)
                -> Result<serde_json::Value, elo_broker::protocol::Refusal> {
                let mut s = self.0.lock().map_err(|_| elo_broker::protocol::Refusal::new("busy"))?;
                if !s.is_unlocked() {
                    return Err(elo_broker::protocol::Refusal::new("the account is locked"));
                }
                s.sign(m, "prove identity")
            }
            fn title_url(&self, _s: &str, _w: elo_broker::What) -> Option<String> { None }
            fn open(&mut self, _u: &str) -> Result<(), elo_broker::protocol::Refusal> { Ok(()) }
        }

        let shared: wiring::Shared =
            Arc::new(Mutex::new(elo_vault::LocalSigner::at(&keystore)));
        let mut door = Door(Arc::clone(&shared));
        // A SIWE (EIP-4361) message. The address is bound INTO it, so it is
        // read off the keystore rather than being an unrelated string — a proof
        // composed for one account and signed by another cannot verify.
        let addr = elo_vault::LocalSigner::at(&keystore).address().unwrap_or_default();
        let msg = elo_broker::protocol::prove_message(
            "s.example", &addr, "gates", "n0nce4d8f1e2b7c",
            &elo_broker::protocol::now_iso8601());

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
            let who = elo_vault::recover_signer(&msg, sig).unwrap_or_default();
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
        // The kind the broker hands the prompt (`consent_kind`): family and
        // action, which is what a counted grant is keyed on.
        let ask = elo_ui::consent::Ask::new(
            "gates",
            "play duel",
            "elo play\naction: duel\nwallet: 0xabc\nday: 2026-08-08\ndetail: ETH up 5",
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
        for (i, (label, grant)) in elo_ui::consent::ALLOWANCES.iter().enumerate() {
            let win = windows::consent(&ask, Some(&on_disk), true);
            let answer = wiring::wire_consent(&win);
            win.allow[i].clone().do_callback();
            check(&mut failures, &format!("\"{label}\" grants {grant}"),
                  answer.get() == Some(*grant));
        }

        // A kind that is asked about every time offers one signature only —
        // "Allow 25" there would be a button that quietly means one.
        let dev = elo_ui::consent::Ask::new("gates", "dev", "elo dev\nact: edit\ngame: gates", "");
        let win = windows::consent(&dev, Some(&on_disk), true);
        check(&mut failures, "a one-at-a-time kind offers only \"Just this one\"",
              win.allow.len() == 1);

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
        use elo_broker::signer::Signer as _;
        use elo_broker::{protocol::Refusal, Host};
        use elo_ui::consent::{Ask, Doorbell};

        struct Door {
            signer: wiring::Shared,
            bell: Doorbell,
        }
        // The same two methods `elo-gui` implements. Copied rather than
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
            fn title_url(&self, _s: &str, _w: elo_broker::What) -> Option<String> { None }
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
            let mut s = elo_vault::LocalSigner::at(keystore);
            s.unlock(pass).expect("the keystore must open");
            let signer: wiring::Shared = Arc::new(Mutex::new(s));
            let bell = Doorbell::new();

            let (door_bell, hello, request) = (bell.clone(), hello.clone(), request.clone());
            let door = std::thread::spawn(move || {
                let mut host = Door { signer, bell: door_bell };
                let mut session = elo_broker::Session::new();
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
            "text": "elo play\naction: duel\nwallet: 0xabc\nday: 2026-08-08\ndetail: ETH up 5",
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
            // The kind, as the broker keys a grant on it: the family and the
            // action, both parsed from the message (`consent_kind`).
            assert_eq!(ask.family, "play duel", "the kind is parsed from the message");
            assert!(ask.text.contains("ETH up 5"), "the message must arrive verbatim");
            assert!(ask.why.contains("round 41"), "the game's reason must reach the player");
            Some(1)
        });
        check(&mut failures, "an allowed prompt signs",
              allowed.get("ok") == Some(&serde_json::json!(true)));
        let sig = allowed.get("signature").and_then(|s| s.as_str()).unwrap_or_default();
        let text = request["text"].as_str().unwrap();
        let who = elo_vault::recover_signer(text, sig).unwrap_or_default();
        check(&mut failures, "and the signature recovers to this machine's account",
              who == on_disk);
        check(&mut failures, "and the reply names the family that was approved",
              allowed.get("family").and_then(|f| f.as_str()) == Some("play"));
    }

    // ── 9 · a Games row after an update plays, and holds while it runs ──────
    // The row used to remember "Update" forever, so after updating, its Play
    // button downloaded the build again — while the player was in it.
    {
        let fake = Rc::new(FakeFront::default());
        let front: Rc<dyn wiring::Storefront> = fake.clone();
        let row = windows::Row {
            slug: "gates".into(),
            build: "0.1.0-gold".into(),
            bytes: 1,
            digest: "old".into(),
            stale: Some(true),
            name: None,
            icon: None,
            why: None,
        };
        let gw = windows::games(&[row]);
        let take_it: wiring::Then = Rc::new(|_, _| true);
        let refreshed = Rc::new(std::cell::Cell::new(0));
        let r2 = Rc::clone(&refreshed);
        wiring::wire_games(&gw, front, Rc::new(|_, _| {}), take_it, Rc::new(move || r2.set(r2.get() + 1)));
        let mut act = gw.rows[0].act.clone();
        check(&mut failures, "a stale row says Update", act.label() == "Update");
        act.do_callback();
        // The update runs as a job off this thread now; its answer lands on a
        // timer, so the loop has to turn for it.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while fake.plays.get() == 0 && std::time::Instant::now() < until {
            let _ = app::wait_for(0.05);
        }
        check(&mut failures, "Update installs once and Play now starts it",
              fake.installs.get() == 1 && fake.plays.get() == 1);
        check(&mut failures, "and the library is re-read after the update",
              refreshed.get() == 1);
        check(&mut failures, "while the game runs the button says Running and is off",
              act.label() == "Running" && !act.active());
        fake.live.set(false);
        let until = std::time::Instant::now() + std::time::Duration::from_millis(1600);
        while std::time::Instant::now() < until {
            let _ = app::wait_for(0.05);
        }
        check(&mut failures, "when the game exits the button is Play again",
              act.label() == "Play" && act.active());
        act.do_callback();
        check(&mut failures, "pressing Play after an update plays and never re-installs",
              fake.installs.get() == 1 && fake.plays.get() == 2);
    }

    // ── 10 · the wallet: read, review, passphrase, sent ─────────────────────
    //
    // The real Wallet controls against a chain that answers like Robinhood
    // Chain and records what it is handed. The account is LOCKED throughout:
    // a send asks its own passphrase whatever the session holds.
    {
        let sent: Arc<Mutex<Vec<String>>> = Arc::default();
        let desk = |sent: &Arc<Mutex<Vec<String>>>| {
            let sent = Arc::clone(sent);
            wiring::WalletDesk {
                chain: Arc::new(move || Box::new(ClickChain { sent: Arc::clone(&sent) })),
                listed: Arc::new(|| Err("offline".into())),
                recipients: dir.join("recipients.json"),
                open: Rc::new(|_| {}),
                sol_chain: Arc::new(|| Box::new(wiring::NoRpc)),
                ours: Arc::new(Vec::new),
                quote: Arc::new(|_, _, _, _| Err("offline".into())),
                follow: Arc::new(|_, _| Err("offline".into())),
            }
        };
        let pump = |done: &dyn Fn() -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !done() && std::time::Instant::now() < until {
                let _ = app::wait_for(0.05);
            }
        };
        let locked = || -> wiring::Shared { Arc::new(Mutex::new(elo_vault::LocalSigner::at(&keystore))) };
        let told = Rc::new(RefCell::new(Vec::new()));
        let reviewed: Rc<RefCell<Vec<String>>> = Rc::default();
        let offered: Rc<RefCell<Vec<String>>> = Rc::default();
        let review: wiring::Review = {
            let r = Rc::clone(&reviewed);
            Rc::new(move |lines, _notes| {
                r.borrow_mut().push(lines[0].1.clone());
                true
            })
        };
        let then: wiring::Then = {
            let o = Rc::clone(&offered);
            Rc::new(move |body, _| {
                o.borrow_mut().push(body.to_string());
                false
            })
        };

        let ww = windows::wallet(Some(&on_disk));
        let read = wiring::wire_wallet(&ww, &locked(), desk(&sent),
                                       wiring::scripted_ask(vec![Some("correct horse".into())]),
                                       wiring::recording_tell(Rc::clone(&told)),
                                       Rc::clone(&review), Rc::clone(&then));
        read();
        let text = || ww.balances.buffer().map(|b| b.text()).unwrap_or_default();
        pump(&|| !text().is_empty());
        check(&mut failures, "the wallet reads the chain and paints the balance",
              text().contains("1 ETH"));

        ww.amount.clone().set_value("0.25");
        ww.to.clone().set_value("0xF0109fC8DF283027b6285cc889F5aA624EaC1F55");
        ww.review.clone().do_callback();
        pump(&|| !offered.borrow().is_empty() || !told.borrow().is_empty());
        let raws = sent.lock().unwrap().clone();
        check(&mut failures, "Review shows what will be signed, read from the transaction",
              reviewed.borrow().first().map(String::as_str) == Some("0.25 ETH"));
        check(&mut failures, "a locked account sends with the passphrase asked for it",
              raws.len() == 1 && raws[0].starts_with("0x02"));
        check(&mut failures, "the player is told it went, with its hash",
              offered.borrow().first().is_some_and(|m| m.starts_with("Sent 0.25 ETH") && m.contains("0x")));
        check(&mut failures, "the recipient is remembered, and the form is cleared",
              elo_net::wallet::known_recipients(&dir.join("recipients.json")).len() == 1
                  && ww.amount.value().is_empty());

        // A wrong passphrase: refused, and nothing reaches the chain.
        let told2 = Rc::new(RefCell::new(Vec::new()));
        let ww2 = windows::wallet(Some(&on_disk));
        let read2 = wiring::wire_wallet(&ww2, &locked(), desk(&sent),
                                        wiring::scripted_ask(vec![Some("wrong horse".into())]),
                                        wiring::recording_tell(Rc::clone(&told2)),
                                        Rc::clone(&review), Rc::clone(&then));
        read2();
        pump(&|| ww2.balances.buffer().is_some_and(|b| !b.text().is_empty()));
        ww2.amount.clone().set_value("0.1");
        ww2.to.clone().set_value("0xF0109fC8DF283027b6285cc889F5aA624EaC1F55");
        ww2.review.clone().do_callback();
        pump(&|| !told2.borrow().is_empty());
        check(&mut failures, "a wrong passphrase sends nothing, and says so",
              sent.lock().unwrap().len() == 1
                  && matches!(told2.borrow().first(), Some((Told::Refused, m)) if m.contains("wrong passphrase")));

        // Cancel on the review: no passphrase is asked and nothing is sent.
        let asked = Rc::new(std::cell::Cell::new(0));
        let counting: wiring::Ask = {
            let a = Rc::clone(&asked);
            Rc::new(move |_, _| {
                a.set(a.get() + 1);
                Some(wiring::Secret::new("correct horse".into()))
            })
        };
        let declined = Rc::new(std::cell::Cell::new(false));
        let cancel: wiring::Review = {
            let d = Rc::clone(&declined);
            Rc::new(move |_, _| {
                d.set(true);
                false
            })
        };
        let ww3 = windows::wallet(Some(&on_disk));
        let read3 = wiring::wire_wallet(&ww3, &locked(), desk(&sent), counting,
                                        wiring::recording_tell(Rc::new(RefCell::new(Vec::new()))),
                                        cancel, Rc::clone(&then));
        read3();
        pump(&|| ww3.balances.buffer().is_some_and(|b| !b.text().is_empty()));
        ww3.amount.clone().set_value("0.1");
        ww3.to.clone().set_value("0xF0109fC8DF283027b6285cc889F5aA624EaC1F55");
        ww3.review.clone().do_callback();
        pump(&|| declined.get());
        check(&mut failures, "Cancel on the review asks no passphrase and sends nothing",
              declined.get() && asked.get() == 0 && sent.lock().unwrap().len() == 1);

        // A mistyped address never reaches the review.
        let told4 = Rc::new(RefCell::new(Vec::new()));
        let ww4 = windows::wallet(Some(&on_disk));
        let read4 = wiring::wire_wallet(&ww4, &locked(), desk(&sent), wiring::scripted_ask(vec![]),
                                        wiring::recording_tell(Rc::clone(&told4)),
                                        Rc::clone(&review), Rc::clone(&then));
        read4();
        pump(&|| ww4.balances.buffer().is_some_and(|b| !b.text().is_empty()));
        ww4.amount.clone().set_value("0.1");
        ww4.to.clone().set_value("0xf0109fC8DF283027b6285cc889F5aA624EaC1F55");
        let before = reviewed.borrow().len();
        ww4.review.clone().do_callback();
        pump(&|| !told4.borrow().is_empty());
        check(&mut failures, "a checksum typo is refused before the review",
              reviewed.borrow().len() == before
                  && matches!(told4.borrow().first(), Some((Told::Refused, m)) if m.contains("checksum")));
    }

    // ── 11 · the same account on Solana: read, review, passphrase, sent ────
    //
    // The account made from words in §1 has its Solana half. Robinhood Chain
    // is offline here; the Solana line still reads, and SOL sends on its own
    // chain with its own passphrase prompt.
    {
        let sol_sent: Arc<Mutex<Vec<String>>> = Arc::default();
        let reviewed: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
        let offered: Rc<RefCell<Vec<String>>> = Rc::default();
        let told = Rc::new(RefCell::new(Vec::new()));
        let desk = {
            let s = Arc::clone(&sol_sent);
            wiring::WalletDesk {
                chain: Arc::new(|| Box::new(wiring::NoRpc)),
                listed: Arc::new(|| Err("offline".into())),
                recipients: dir.join("recipients.json"),
                open: Rc::new(|_| {}),
                sol_chain: Arc::new(move || Box::new(ClickSolana { sent: Arc::clone(&s) })),
                ours: Arc::new(Vec::new),
                quote: Arc::new(|_, _, _, _| Err("offline".into())),
                follow: Arc::new(|_, _| Err("offline".into())),
            }
        };
        let review: wiring::Review = {
            let r = Rc::clone(&reviewed);
            Rc::new(move |lines, _| {
                r.borrow_mut().extend(lines.iter().cloned());
                true
            })
        };
        let then: wiring::Then = {
            let o = Rc::clone(&offered);
            Rc::new(move |body, _| {
                o.borrow_mut().push(body.to_string());
                false
            })
        };
        let signer: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&keystore)));
        let has_sol = signer.lock().unwrap().sol_address().is_some();
        check(&mut failures, "an account made from words has its Solana address too", has_sol);
        let ww = windows::wallet(Some(&on_disk));
        let read = wiring::wire_wallet(&ww, &signer, desk,
                                       wiring::scripted_ask(vec![Some("correct horse".into())]),
                                       wiring::recording_tell(Rc::clone(&told)), review, then);
        read();
        let text = || ww.balances.buffer().map(|b| b.text()).unwrap_or_default();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !text().contains("SOL") && std::time::Instant::now() < until {
            let _ = app::wait_for(0.05);
        }
        check(&mut failures, "the wallet shows both chains, and Solana's SOL",
              text().contains("on Robinhood Chain") && text().contains("on Solana") && text().contains("2 SOL"));
        check(&mut failures, "the Solana address is shown beside the 0x one",
              ww.sol_address.label() == signer.lock().unwrap().sol_address().unwrap_or_default());
        // ETH (Robinhood) is first; SOL (Solana) second
        ww.coin.clone().set_value(1);
        ww.amount.clone().set_value("0.25");
        ww.to.clone().set_value("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM");
        ww.review.clone().do_callback();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while offered.borrow().is_empty() && told.borrow().is_empty() && std::time::Instant::now() < until {
            let _ = app::wait_for(0.05);
        }
        let net = reviewed.borrow().iter().find(|(k, _)| k == "Network").map(|(_, v)| v.clone());
        check(&mut failures, "a SOL send is reviewed on Solana, read from the transaction",
              reviewed.borrow().first().map(|(_, v)| v.as_str()) == Some("0.25 SOL")
                  && net.as_deref() == Some("Solana"));
        let raws = sol_sent.lock().unwrap().clone();
        let landed = raws.first().and_then(|b| elo_vault::solana::b64_decode(b).ok())
            .and_then(|b| elo_vault::solana::Transaction::parse(&b).ok());
        check(&mut failures, "it is signed with the passphrase and sent to Solana",
              landed.is_some_and(|t| t.signatures[0] != [0u8; 64]));
        check(&mut failures, "the player is told it went",
              offered.borrow().first().is_some_and(|m| m.starts_with("Sent 0.25 SOL")));
        check(&mut failures, "a Solana recipient is remembered beside the 0x ones",
              elo_net::solana::known_recipients(&dir.join("recipients.json")).len() == 1
                  && elo_net::wallet::known_recipients(&dir.join("recipients.json")).len() == 1);
    }

    // An account adopted from a bare key has no Solana half; the Solana row's
    // Add takes the words that made it and writes one.
    {
        let words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let ks = dir.join("older").join("keystore.json");
        let secret = elo_vault::Phrase::parse(words).unwrap().account().unwrap().secret();
        let made = elo_vault::LocalSigner::import(&ks, "correct horse", secret.as_ref()).unwrap();
        let addr = elo_broker::signer::Signer::address(&made).unwrap();
        let signer: wiring::Shared = Arc::new(Mutex::new(elo_vault::LocalSigner::at(&ks)));
        let told = Rc::new(RefCell::new(Vec::new()));
        let desk = wiring::WalletDesk {
            chain: Arc::new(|| Box::new(wiring::NoRpc)),
            listed: Arc::new(|| Err("offline".into())),
            recipients: dir.join("older").join("recipients.json"),
            open: Rc::new(|_| {}),
            sol_chain: Arc::new(|| Box::new(wiring::NoRpc)),
            ours: Arc::new(Vec::new),
            quote: Arc::new(|_, _, _, _| Err("offline".into())),
            follow: Arc::new(|_, _| Err("offline".into())),
        };
        let ww = windows::wallet(Some(&addr));
        let read = wiring::wire_wallet(&ww, &signer, desk,
                                       wiring::scripted_ask(vec![Some(words.into()), Some("correct horse".into())]),
                                       wiring::recording_tell(Rc::clone(&told)),
                                       Rc::new(|_, _| false), Rc::new(|_, _| false));
        read();
        check(&mut failures, "an older account's Solana row offers Add", ww.sol_copy.label() == "Add…");
        ww.sol_copy.clone().do_callback();
        check(&mut failures, "Add writes the Solana address the words make, the one Phantom shows",
              signer.lock().unwrap().sol_address().as_deref() == Some("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk")
                  && matches!(told.borrow().first(), Some((Told::Done, m)) if m.contains("HAgk14Jp"))
                  && ww.sol_copy.label() == "Copy");
    }

    let _ = std::fs::remove_dir_all(&dir);
    if failures == 0 {
        println!("\nclick: every control does what it says");
    } else {
        println!("\nclick: {failures} FAILED");
        std::process::exit(1);
    }
}

/// A storefront that counts what it was asked to do and says a game is
/// running from the moment it is played until `live` is cleared.
#[derive(Default)]
struct FakeFront {
    installs: std::cell::Cell<u32>,
    plays: std::cell::Cell<u32>,
    live: std::cell::Cell<bool>,
}

impl wiring::Storefront for FakeFront {
    fn install(&self, _: &str) -> Result<String, String> {
        self.installs.set(self.installs.get() + 1);
        Ok("updated".into())
    }
    fn verify(&self, _: &str, _: &str) -> Result<String, String> {
        Ok("verified".into())
    }
    fn play(&self, _: &str) -> Result<String, String> {
        self.plays.set(self.plays.get() + 1);
        self.live.set(true);
        Ok("started".into())
    }
    fn running(&self, _: &str) -> bool {
        self.live.get()
    }
    fn open(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn page_url(&self, _: &str) -> String {
        String::new()
    }
}

/// A chain that answers like Robinhood Chain and keeps what it is sent.
struct ClickChain {
    sent: Arc<Mutex<Vec<String>>>,
}

/// Solana, answering like mainnet for a plain SOL send, and recording what it
/// is handed.
struct ClickSolana {
    sent: Arc<Mutex<Vec<String>>>,
}

impl elo_net::chain::Rpc for ClickSolana {
    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        use serde_json::json;
        Ok(match method {
            "getBalance" => json!({"value": 2_000_000_000u64}),
            "getTokenAccountsByOwner" => json!({"value": []}),
            "getAccountInfo" => json!({"value": {"owner": "11111111111111111111111111111111", "executable": false}}),
            "getLatestBlockhash" => json!({"value": {"blockhash": "EkSnNWid2cvwEVnVx9aBqawnmiCNiDgp3gUdkDPTKN1N"}}),
            "getMinimumBalanceForRentExemption" => json!(890_880),
            "simulateTransaction" => json!({"value": {"err": null, "accounts": [{"lamports": 1_749_995_000u64}],
                                                      "preBalances": [2_000_000_000u64], "postBalances": [1_749_995_000u64]}}),
            "sendTransaction" => {
                let raw = params[0].as_str().unwrap_or_default().to_string();
                self.sent.lock().map_err(|_| "poisoned")?.push(raw);
                json!("5ig1ature")
            }
            other => return Err(format!("no answer for {other}")),
        })
    }
}

impl elo_net::chain::Rpc for ClickChain {
    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        use serde_json::json;
        Ok(match method {
            "eth_chainId" => json!("0x1237"),
            "eth_getBalance" => json!("0xde0b6b3a7640000"),
            "eth_getTransactionCount" => json!("0x0"),
            "eth_getBlockByNumber" => json!({"baseFeePerGas": "0x989680"}),
            "eth_maxPriorityFeePerGas" => json!("0x0"),
            "eth_estimateGas" => json!("0x5208"),
            "eth_getCode" => json!("0x"),
            "eth_sendRawTransaction" => {
                let raw = params[0].as_str().unwrap_or_default().to_string();
                self.sent.lock().map_err(|_| "poisoned")?.push(raw);
                json!("0x00")
            }
            other => return Err(format!("no answer for {other}")),
        })
    }
}

fn check(failures: &mut i32, what: &str, ok: bool) {
    println!("  {}  {what}", if ok { "ok  " } else { "FAIL" });
    if !ok {
        *failures += 1;
    }
}
