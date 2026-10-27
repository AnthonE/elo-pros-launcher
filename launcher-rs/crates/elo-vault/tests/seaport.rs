//! Seaport 1.6 listings against the meter's `meter/seaport.py` and
//! `eth_account`: the same fixture hashes to the same digest, and the same key
//! signs it to the same bytes. If either side drifts, a listing signed here is
//! one the meter refuses (or worse, one it accepts for a different order).

use elo_vault::seaport::{self, Order};
use elo_broker::signer::Signer;
use elo_vault::LocalSigner;
use serde_json::{json, Value};

const PASS: &str = "test passphrase";

/// `meter/test_seaport.py`'s FIXTURE, as the typed data's `message`.
fn fixture(offerer: &str) -> Value {
    json!({
        "offerer": offerer,
        "zone": "0x0000000000000000000000000000000000000000",
        "offer": [{"itemType": 2, "token": "0x93f4965638f9c99dd53f13bf2b47358a6ff7630c",
                   "identifierOrCriteria": "7", "startAmount": "1", "endAmount": "1"}],
        "consideration": [
            {"itemType": 0, "token": "0x0000000000000000000000000000000000000000", "identifierOrCriteria": "0",
             "startAmount": "9900000000000000", "endAmount": "9900000000000000", "recipient": offerer},
            {"itemType": 0, "token": "0x0000000000000000000000000000000000000000", "identifierOrCriteria": "0",
             "startAmount": "100000000000000", "endAmount": "100000000000000",
             "recipient": "0x2222222222222222222222222222222222222222"}],
        "orderType": 0, "startTime": "1761200000", "endTime": "1761804800",
        "zoneHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "salt": "12345678901234567890",
        "conduitKey": "0x0000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f0000",
        "counter": "0",
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn the_fixture_hashes_as_the_meter_does() {
    let o = Order::from_json(&fixture("0x1111111111111111111111111111111111111111")).expect("parse");
    assert_eq!(hex(&o.hash()), "59bbec9e63b899859fb68946317693dce818331dbbe0db9f3e01930959106239", "order hash");
    assert_eq!(hex(&o.digest()), "684c1b3f444d48ef4fcc0967f69a554e1ac797ae99aab4aac1c81bb7eb5f76c4", "digest");
}

#[test]
fn a_listing_signs_to_eth_accounts_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let s = LocalSigner::import(&path, PASS, &[0x42u8; 32]).expect("import");
    let me = s.address().expect("address");
    assert_eq!(me, "0x17c5185167401eD00cF5F5b2fc97D9BBfDb7D025");
    let o = Order::from_json(&fixture(&me.to_lowercase())).expect("parse");
    assert_eq!(hex(&o.digest()), "83946ac22ca4011efca4e0a10e14e70c43d0a924d9ea8f570b7ca2411e3ea65c");
    let sig = s.sign_seaport_listing(PASS, &o).expect("sign");
    assert_eq!(sig, "0xe9ece31a0b2d99b74a34ecdb1a5a611b99ba84021b4d85d50640f7b661c7d3f4\
746a69705bc7ab3480065b776db4cc01d3ed7c9ecaf2f16322bbeb6c1fc02ee21c");
    assert!(s.sign_seaport_listing("wrong passphrase", &o).is_err(), "the passphrase is asked every time");
}

#[test]
fn the_vault_signs_only_a_listing_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let s = LocalSigner::import(&path, PASS, &[0x42u8; 32]).expect("import");
    let me = s.address().unwrap().to_lowercase();
    let refuse = |m: Value, why: &str| {
        let o = Order::from_json(&m).expect("parse");
        let e = s.sign_seaport_listing(PASS, &o).expect_err(why);
        assert!(!e.is_empty(), "{why}");
    };
    refuse(fixture("0x1111111111111111111111111111111111111111"), "another account's listing");
    let mut m = fixture(&me);
    m["consideration"][0]["recipient"] = json!("0x2222222222222222222222222222222222222222");
    refuse(m, "a listing that pays someone else first");
    let mut m = fixture(&me);
    m["consideration"][1]["itemType"] = json!(1);
    refuse(m, "a payment in a token");
    let mut m = fixture(&me);
    m["offer"][0]["startAmount"] = json!("2");
    m["offer"][0]["endAmount"] = json!("2");
    refuse(m, "more than one item");
    let mut m = fixture(&me);
    m["offer"] = json!([]);
    refuse(m, "an order that offers nothing");
}

#[test]
fn numbers_round_trip_through_256_bits() {
    for s in ["0", "1", "12345678901234567890",
              "115792089237316195423570985008687907853269984665640564039457584007913129639935"] {
        assert_eq!(seaport::word_dec(&seaport::dec_word(s).unwrap()), s);
    }
    assert!(seaport::dec_word("115792089237316195423570985008687907853269984665640564039457584007913129639936").is_err());
    assert!(seaport::dec_word("-1").is_err() && seaport::dec_word("1e18").is_err() && seaport::dec_word("").is_err());
}
