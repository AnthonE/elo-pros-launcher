//! Linking the chat key and unlinking it, end to end, against a REAL `meter/hive.py` (with
//! `directory.py`, `faces.py` and `playauth.py`) on this machine — never
//! production.
//!
//! The unit tests pin the refusal order and the derivation vector. What only
//! the real server can say is whether the two signatures this client makes are
//! the ones `/hive/link` verifies, and whether a linked key then reads as its
//! wallet everywhere that names people: `/who` (the launcher's faces, the
//! site's profile) answers the wallet, with the wallet's own profile name.
//!
//! ```bash
//! # serve those four routers under /api on 127.0.0.1:3612, then:
//! ELO_LINK_ORIGIN=http://127.0.0.1:3612 \
//!   cargo test --release -p elo-ui --test chatlink_local -- --ignored
//! ```

use elo_broker::signer::Signer;
use elo_hive::voice::Voice;
use elo_ui::wiring::{self, Ask, Confirm, Shared};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "needs a local meter — ELO_LINK_ORIGIN=http://127.0.0.1:3612"]
fn a_launcher_links_its_wallets_chat_key_rooms_name_it_and_unlink_undoes_it() {
    let origin = std::env::var("ELO_LINK_ORIGIN").expect("ELO_LINK_ORIGIN names the local meter");
    assert!(
        origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:"),
        "loopback only — this writes a link and a profile: {origin}"
    );
    let dir = std::env::temp_dir().join(format!("elo-link-local-{}", std::process::id()));
    let voice_dir = dir.join("voice");
    std::fs::create_dir_all(&voice_dir).unwrap();
    let mut account = elo_vault::LocalSigner::create(dir.join("keystore.json"), "test passphrase")
        .expect("a fresh account").0;
    let wallet = account.address().unwrap().to_lowercase();
    account.lock();
    let signer: Shared = Arc::new(Mutex::new(account));
    let ask: Ask = Rc::new(|_, _| Some(wiring::Secret::new("test passphrase".into())));
    let confirm: Confirm = Rc::new(|_, body, _| {
        assert!(body.contains("chatkey"), "the confirmation shows the link text: {body}");
        true
    });

    // A profile name, so a linked key has something to be named by.
    let named = elo_ui::profile::Change { name: Some("Linker".into()), picture: None };
    wiring::set_profile(&signer, &origin, &named, &ask, &elo_ui::profile::RealFaceHttp)
        .expect("the profile name lands");

    // A random key already on this machine: it must be kept, not deleted.
    let old = Voice::generate().unwrap();
    old.save(&voice_dir).unwrap();

    let done = wiring::link_chat_key(&signer, &origin, &voice_dir, &ask, &confirm,
                                     &elo_hive::link::RealLinkHttp)
        .unwrap_or_else(|e| panic!("the real /hive/link refused: {e}"));
    assert!(done.starts_with("Linked."), "{done}");
    assert!(!signer.lock().unwrap().is_unlocked(), "the account relocks after both signatures");

    let now = Voice::load(&voice_dir).expect("the wallet's key is saved");
    assert_ne!(now.npub(), old.npub(), "the key on this machine is now the wallet's");
    assert!(voice_dir.join(elo_hive::link::retired_key_name(&old.npub())).is_file(),
            "the old key is kept beside it");

    // The public record, from both ends.
    let net = elo_net::Net::new();
    let by_wallet = net.get_json(&format!("{origin}/api/hive/link/{wallet}")).value.unwrap();
    assert_eq!(by_wallet["npub"], now.npub());

    // And every surface that names people: /who answers the npub as the wallet.
    let id = elo_hive::who::resolve(&net, &origin, &now.npub()).value.expect("/who answers");
    assert_eq!(id.wallet.as_deref().map(str::to_lowercase).as_deref(), Some(wallet.as_str()));
    assert_eq!(id.name, "Linker", "a linked key wears the wallet's own profile name");

    // And back: one signature, and the key reads as its own again — still here.
    let undone = wiring::unlink_chat_key(&signer, &origin, &ask, &confirm,
                                         &elo_hive::link::RealLinkHttp)
        .unwrap_or_else(|e| panic!("the real /hive/link refused the unlink: {e}"));
    assert!(undone.starts_with("Unlinked."), "{undone}");
    assert!(!signer.lock().unwrap().is_unlocked(), "the account relocks after unlinking");
    let by_wallet = net.get_json(&format!("{origin}/api/hive/link/{wallet}")).value.unwrap();
    assert!(by_wallet["npub"].is_null(), "{by_wallet}");
    let id = elo_hive::who::resolve(&net, &origin, &now.npub()).value.expect("/who answers");
    assert_eq!(id.wallet, None, "an unlinked key no longer answers as the wallet");
    assert_eq!(Voice::load(&voice_dir).unwrap().npub(), now.npub(), "the key stays on this machine");
    let e = wiring::unlink_chat_key(&signer, &origin, &ask, &confirm, &elo_hive::link::RealLinkHttp)
        .unwrap_err();
    assert!(e.contains("nothing to unlink"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
