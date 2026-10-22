//! A profile change, end to end, against a REAL `meter/faces.py` — run on this
//! machine, never against production.
//!
//! The unit tests pin the refusal order with a keystore that cannot unlock.
//! What only a real server can say is whether the text this client signs, the
//! signature it makes and the body it sends are the ones `faces.py` verifies
//! (`playauth.verify_wallet_play`). So this makes a real account, signs, and
//! reads the profile back.
//!
//! ```bash
//! # serve meter/faces.py on 127.0.0.1:3611 (the router under /api), then:
//! ELO_FACE_ORIGIN=http://127.0.0.1:3611 \
//!   cargo test --release -p elo-ui --test profile_local -- --ignored
//! ```
//!
//! `--release` because the account's keystore is real scrypt. The origin must
//! be loopback: a test that could write a profile to production is a test
//! that one day will.

use elo_broker::signer::Signer;
use elo_ui::profile::{self, Change, RealFaceHttp};
use elo_ui::wiring::{self, Ask, Shared};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for b in bytes {
        c ^= *b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend((body.len() as u32).to_be_bytes());
    let mut typed = kind.to_vec();
    typed.extend_from_slice(body);
    out.extend(&typed);
    out.extend(crc32(&typed).to_be_bytes());
}

/// A real, decodable `size`×`size` RGBA PNG: zlib stored blocks, no
/// compression library needed.
fn png(size: u32) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..size {
        raw.push(0); // filter: none
        for x in 0..size {
            raw.extend([(x * 4) as u8, (y * 4) as u8, 0x80, 0xff]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        z.extend((b.len() as u16).to_le_bytes());
        z.extend((!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for x in &raw {
        a = (a + *x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend(((b << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(size.to_be_bytes());
    ihdr.extend(size.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
#[ignore = "needs a local faces.py — ELO_FACE_ORIGIN=http://127.0.0.1:3611"]
fn a_signed_profile_change_lands_on_a_real_faces_route() {
    let origin = std::env::var("ELO_FACE_ORIGIN").expect("ELO_FACE_ORIGIN names the local faces.py");
    assert!(
        origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:"),
        "loopback only — this writes a profile: {origin}"
    );
    let dir = std::env::temp_dir().join(format!("elo-profile-local-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut account = elo_vault::LocalSigner::create(dir.join("keystore.json"), "test passphrase")
        .expect("a fresh account").0;
    let wallet = account.address().expect("the account names its address");
    account.lock();
    let signer: Shared = Arc::new(Mutex::new(account));
    let ask: Ask = Rc::new(|_, _| Some(wiring::Secret::new("test passphrase".into())));

    let picture = png(64);
    assert_eq!(profile::png_error(&picture), None, "the fixture passes the client's own rules");
    let change = Change { name: Some("Tester".into()), picture: Some(picture.clone()) };
    let done = wiring::set_profile(&signer, &origin, &change, &ask, &RealFaceHttp)
        .unwrap_or_else(|e| panic!("the real faces.py refused the change: {e}"));
    assert!(done.contains("Tester"), "{done}");
    assert!(!signer.lock().unwrap().is_unlocked(), "a transient unlock relocks");

    let shown = profile::read(&RealFaceHttp, &origin, &wallet).expect("read it back");
    assert_eq!(shown.name.as_deref(), Some("Tester"));
    assert_eq!(shown.picture.as_deref(), Some(picture.as_slice()), "the same bytes come back");

    // A second change with no new picture keeps the picture and moves the name.
    let again = Change { name: Some("Tester Two".into()), picture: None };
    wiring::set_profile(&signer, &origin, &again, &ask, &RealFaceHttp).expect("a rename");
    let shown = profile::read(&RealFaceHttp, &origin, &wallet).unwrap();
    assert_eq!(shown.name.as_deref(), Some("Tester Two"));
    assert!(shown.picture.is_some(), "a field left out keeps its value");
    let _ = std::fs::remove_dir_all(&dir);
}
