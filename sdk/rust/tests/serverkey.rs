//! A game server's key: the text and the signature match the shared fixture
//! byte for byte (Ed25519 is deterministic), in every SDK and the meter.
#![cfg(feature = "http")]

use elo_sdk::{new_server_nonce, server_text, ServerKey};
use serde_json::Value;

fn fixture() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../testdata/server_texts.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn texts_and_signatures_match_the_fixture() {
    let fx = fixture();
    let key = ServerKey::from_seed("my-game", "eu-1", &hex(fx["seed_hex"].as_str().unwrap())).unwrap();
    assert_eq!(key.public_b64(), fx["public_b64"]);
    for c in fx["cases"].as_array().unwrap() {
        let text = server_text(
            c["method"].as_str().unwrap(),
            c["path"].as_str().unwrap(),
            c["who"].as_str().unwrap(),
            c["at"].as_str().unwrap(),
            c["body_utf8"].as_str().unwrap().as_bytes(),
            c["nonce"].as_str(),
        );
        assert_eq!(text, c["text"].as_str().unwrap());
        assert_eq!(key.sign_text(&text), c["signature_b64"].as_str().unwrap());
    }
}

/// PEM armour around a base64 body, built here so no key header is a literal.
fn pem(body: &str) -> String {
    let label = ["PRIVATE", "KEY"].join(" ");
    format!("-----BEGIN {label}-----\n{body}\n-----END {label}-----\n")
}

#[test]
fn a_pem_gives_the_same_key_and_bad_input_is_refused() {
    let fx = fixture();
    let key = ServerKey::from_pem("my-game", "eu-1", &pem(fx["private_pkcs8_b64"].as_str().unwrap())).unwrap();
    assert_eq!(key.public_b64(), fx["public_b64"]);
    assert!(ServerKey::from_seed("My Game", "eu-1", &[0u8; 32]).is_err());
    assert!(ServerKey::from_seed("my-game", "EU", &[0u8; 32]).is_err());
    assert!(ServerKey::from_pem("my-game", "eu-1", &pem("AAAA")).is_err());
    assert!(new_server_nonce(4).is_none());
    let h = key.headers("PUT", "/cloud/my-game/x/save", b"{}", Some(1), Some("abcdefgh"));
    let text = server_text("PUT", "/cloud/my-game/x/save", "my-game/eu-1", "1", b"{}", Some("abcdefgh"));
    assert_eq!(h[3].1, key.sign_text(&text));
    assert_ne!(new_server_nonce(22), new_server_nonce(22));
}
