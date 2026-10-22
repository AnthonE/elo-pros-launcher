//! A friends change from the window, end to end, against a REAL
//! `meter/friends.py` — run on this machine, never against production.
//!
//! The unit tests pin the refusal order and the text check with a keystore
//! that cannot unlock. What only a real server can say is whether the text
//! this client rebuilds, the signature it makes and the body it sends are the
//! ones `friends.py` verifies (`playauth.verify_wallet_play`). So this makes
//! two real accounts, has one ask and the other accept, and reads both lists
//! back.
//!
//! ```bash
//! # serve meter/friends.py on 127.0.0.1:3613 (the router under /api), e.g.
//! cd meter && ELO_FRIENDS_DIR=$(mktemp -d) SCRY_PLAYTIME_DIR=$(mktemp -d) \
//!   SCRY_FACES_DIR=$(mktemp -d) SCRY_SIG_CHAIN=off python3 -c \
//!   'import fastapi, uvicorn, friends; a = fastapi.FastAPI();
//!    a.include_router(friends.router, prefix="/api");
//!    uvicorn.run(a, host="127.0.0.1", port=3613)'
//! # then:
//! ELO_FRIENDS_ORIGIN=http://127.0.0.1:3613 \
//!   cargo test --release -p elo-ui --test friends_local -- --ignored
//! ```
//!
//! `--release` because the accounts' keystores are real scrypt. The origin
//! must be loopback: a test that could write a friendship to production is a
//! test that one day will.

use elo_broker::signer::Signer;
use elo_net::friends;
use elo_ui::friendsop::RealFriendsHttp;
use elo_ui::wiring::{self, Changed, FriendSigner, Shared};
use std::sync::{Arc, Mutex};

const PASS: &str = "test passphrase";

/// A fresh account in `dir`, locked, as the window finds one.
fn account(dir: &std::path::Path, name: &str) -> (Shared, String) {
    let mut a = elo_vault::LocalSigner::create(dir.join(format!("{name}.json")), PASS)
        .expect("a fresh account").0;
    let wallet = a
        .address()
        .expect("the account names its address")
        .to_ascii_lowercase();
    a.lock();
    (Arc::new(Mutex::new(a)), wallet)
}

/// One change, as the window makes it: the signer the window would pick, the
/// passphrase prompt answered, and the list that comes back.
fn change(signer: &Shared, origin: &str, op: &str, who: &str) -> friends::View {
    let mut via = FriendSigner::pick(signer, None);
    let prompted = std::cell::Cell::new(false);
    let ask = |_: &str, prompt: &str| {
        assert!(
            prompt.contains("action: friends"),
            "the prompt shows the text: {prompt}"
        );
        prompted.set(true);
        Some(wiring::Secret::new(PASS.to_string()))
    };
    let got = wiring::change_friend(&mut via, origin, op, who, &ask, &RealFriendsHttp)
        .unwrap_or_else(|e| panic!("the real friends.py refused {op}: {e}"));
    assert!(prompted.get(), "{op} asked for the passphrase");
    assert!(
        !signer.lock().unwrap().is_unlocked(),
        "a transient unlock relocks"
    );
    match got {
        Changed::Done(view) => view,
        Changed::HandedOff(url) => panic!("{op} was handed off to {url} with an account here"),
    }
}

fn read(origin: &str, wallet: &str) -> friends::View {
    let got = friends::read(&elo_net::Net::new(), origin, wallet);
    got.value
        .unwrap_or_else(|| panic!("the list did not read: {}", got.why))
}

#[test]
#[ignore = "needs a local friends.py — ELO_FRIENDS_ORIGIN=http://127.0.0.1:3613"]
fn an_ask_and_an_accept_land_on_a_real_friends_route() {
    let origin =
        std::env::var("ELO_FRIENDS_ORIGIN").expect("ELO_FRIENDS_ORIGIN names the local friends.py");
    assert!(
        origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:"),
        "loopback only — this writes a friendship: {origin}"
    );
    let dir = std::env::temp_dir().join(format!("elo-friends-local-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (ash, ash_wallet) = account(&dir, "ash");
    let (bo, bo_wallet) = account(&dir, "bo");

    // Ash asks Bo: the list that comes back carries the ask, and Bo's shows it.
    let after = change(&ash, &origin, "ask", &bo_wallet);
    assert_eq!(after.wallet, ash_wallet);
    assert!(
        after.asks_out.iter().any(|a| a.wallet == bo_wallet),
        "{after:?}"
    );
    assert!(after.friends.is_empty());
    let theirs = read(&origin, &bo_wallet);
    assert!(
        theirs.asks_in.iter().any(|a| a.wallet == ash_wallet),
        "{theirs:?}"
    );

    // Bo accepts — typed in capitals, as a paste might be.
    let shouted = ash_wallet.to_uppercase().replacen("0X", "0x", 1);
    let after = change(&bo, &origin, "accept", &shouted);
    assert!(
        after.friends.iter().any(|f| f.wallet == ash_wallet),
        "{after:?}"
    );
    assert!(after.asks_in.is_empty(), "the ask is answered");

    // Read back, both sides, with the client's own reader.
    let mine = read(&origin, &ash_wallet);
    assert!(
        mine.friends.iter().any(|f| f.wallet == bo_wallet),
        "{mine:?}"
    );
    assert!(mine.asks_out.is_empty());
    assert!(read(&origin, &bo_wallet)
        .friends
        .iter()
        .any(|f| f.wallet == ash_wallet));

    // A change the origin refuses arrives in its words, after the passphrase
    // check the meter cannot see: Bo has no ask left to accept.
    let mut via = FriendSigner::pick(&bo, None);
    let no_prompt = |_: &str, _: &str| -> Option<wiring::Secret> {
        panic!("prompted over a change the origin will not prepare")
    };
    let e = wiring::change_friend(
        &mut via,
        &origin,
        "accept",
        &ash_wallet,
        &no_prompt,
        &RealFriendsHttp,
    )
    .unwrap_err();
    assert!(e.contains("no ask"), "{e}");

    // And Remove, from the other side, ends it for both.
    let after = change(&ash, &origin, "remove", &bo_wallet);
    assert!(after.friends.is_empty(), "{after:?}");
    assert!(read(&origin, &bo_wallet).friends.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
