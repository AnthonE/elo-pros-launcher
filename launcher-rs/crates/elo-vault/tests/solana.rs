//! The Solana half, against a reference implementation.
//!
//! Every expected value below was produced by `bip_utils` (BIP-39 + SLIP-0010
//! at `m/44'/501'/0'/0'`) and `solders` (the Rust `solana-sdk` types, through
//! Python) — not by this crate. The test phrase's address,
//! `HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk`, is also what Phantom shows
//! for "abandon … about", which is the exit this is for: the words a holder
//! wrote down bring back the same Solana address in any wallet.

use elo_vault::solana::{self, base58_decode, base58_encode, SolAccount, Transaction};
use elo_vault::{LocalSigner, Phrase};

const ABOUT: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon \
                     abandon abandon about";
const PASS: &str = "test passphrase";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn key(s: &str) -> [u8; 32] {
    solana::parse_address(s).unwrap()
}

#[test]
fn the_words_derive_the_address_phantom_shows() {
    let cases = [
        (ABOUT, "37df573b3ac4ad5b522e064e25b63ea16bcbe79d449e81a0268d1047948bb445",
         "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"),
        ("legal winner thank year wave sausage worth useful legal winner thank yellow",
         "6987bdb06aa8a243a3019f41489ffa8e609c953a885a748d1849a8df760aa479",
         "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX"),
        ("zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote",
         "feafaa95d64a1a37f4e4dce7fd2ee764bbc1e8eef627d5aedaceee19f89f76ff",
         "4wX8yu9YmSe4mv9ZPtTeoF9pe6Ji4ScjuJEffS3sCKZ4"),
    ];
    for (words, secret, addr) in cases {
        let sol = Phrase::parse(words).unwrap().solana().unwrap();
        assert_eq!(hex(sol.secret().as_ref()), secret, "SLIP-0010 drifted for {words:.20}…");
        assert_eq!(sol.address(), addr);
    }
}

#[test]
fn an_unhardened_step_is_refused() {
    let seed = Phrase::parse(ABOUT).unwrap().seed();
    assert!(solana::derive(seed.as_ref(), &[44 | 0x8000_0000, 501]).is_err());
}

#[test]
fn base58_round_trips_and_keeps_leading_zeros() {
    // Bitcoin's alphabet vectors.
    assert_eq!(base58_encode(b"Hello World!"), "2NEpo7TZRRrLZSi2U");
    assert_eq!(base58_encode(&[0, 0, 0x28, 0x7f, 0xb4, 0xcd]), "11233QC4");
    assert_eq!(base58_encode(&[0u8; 32]), "11111111111111111111111111111111");
    assert_eq!(base58_decode("11233QC4").unwrap(), vec![0, 0, 0x28, 0x7f, 0xb4, 0xcd]);
    assert!(base58_decode("0OIl").is_err());
    let k = key(solana::TOKEN_PROGRAM);
    assert_eq!(base58_encode(&k), solana::TOKEN_PROGRAM);
}

#[test]
fn an_address_of_the_other_chain_says_so() {
    let e = solana::parse_address("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").unwrap_err();
    assert!(e.contains("Robinhood Chain"), "{e}");
    assert!(solana::parse_address("abc").is_err());
}

const TO: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const BLOCKHASH: &str = "EkSnNWid2cvwEVnVx9aBqawnmiCNiDgp3gUdkDPTKN1N";

#[test]
fn a_sol_transfer_is_byte_for_byte_solders() {
    let sol = Phrase::parse(ABOUT).unwrap().solana().unwrap();
    let msg = solana::compile(&sol.public(), &[solana::system_transfer(&sol.public(), &key(TO), 1234)],
                              &key(BLOCKHASH));
    assert_eq!(hex(&msg), "01000103f036276246a75b9de3349ed42b15e232f6518fc20f5fcd4f1d64e81f9bd258f77e8c088760bfde1dddcf32c17f209b8242ee52aaf131facd88d0ea2c6d0b06f20000000000000000000000000000000000000000000000000000000000000000cc490e928cd2e3873bb343fc95da33179ca60f4dbf46c2c36e91299d55d4e6b901020200010c02000000d204000000000000");
    let want = "Aa+XrPD3b0UZtCahV286xP2i5zHiKG1Q49A0Wifkiee5S7psDEpCHQxeApp9FznBN1dIAvdQ5PY0zmO0QmJC3QcBAAED8DYnYkanW53jNJ7UKxXiMvZRj8IPX81PHWToH5vSWPd+jAiHYL/eHd3PMsF/IJuCQu5SqvEx+s2I0OosbQsG8gAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAzEkOkozS44c7s0P8ldozF5ymD02/RsLDbpEpnVXU5rkBAgIAAQwCAAAA0gQAAAAAAAA=";
    // Signed through the real door, from a keystore made from the words.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let signer = LocalSigner::restore(&path, PASS, &Phrase::parse(ABOUT).unwrap()).unwrap();
    assert_eq!(signer.sol_address().as_deref(), Some("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"));
    let mut tx = Transaction::unsigned(msg).unwrap();
    signer.sign_solana_transaction(PASS, &mut tx).unwrap();
    assert_eq!(solana::b64_encode(&tx.to_bytes()), want);
    // and the wire form reads back to the same transaction
    let back = Transaction::parse(&solana::b64_decode(want).unwrap()).unwrap();
    assert_eq!(back.to_bytes(), tx.to_bytes());
    assert_eq!(back.message.signers, 1);
}

#[test]
fn a_coin_send_with_its_token_account_is_byte_for_byte_solders() {
    let sol = Phrase::parse(ABOUT).unwrap().solana().unwrap();
    let (me, to, usdc) = (sol.public(), key(TO), key("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"));
    let tp = key(solana::TOKEN_PROGRAM);
    let src = solana::associated_token_address(&me, &usdc, &tp);
    let dst = solana::associated_token_address(&to, &usdc, &tp);
    let msg = solana::compile(&me, &[
        solana::create_ata_idempotent(&me, &to, &usdc, &tp),
        solana::transfer_checked(&src, &usdc, &dst, &me, 2_500_000, 6, &tp),
    ], &key(BLOCKHASH));
    assert_eq!(hex(&msg), "01000508f036276246a75b9de3349ed42b15e232f6518fc20f5fcd4f1d64e81f9bd258f740d2f27c461f29e9feba1ab8acd39496a9c65b0ae62a713f01f941a993bf55edd3ea8cf5acaca8cd05207512175c43cef54a5dd99ede20a16b55253738f397dc000000000000000000000000000000000000000000000000000000000000000006ddf6e1d765a193d9cbe146ceeb79ac1cb485ed5f5b37913a8cf5857eff00a97e8c088760bfde1dddcf32c17f209b8242ee52aaf131facd88d0ea2c6d0b06f28c97258f4e2489f1bb3d1029148e0d830b5a1399daff1084048e7bd8dbe9f859c6fa7af3bedbad3a3d65f36aabc97431b1bbe4c2d2f6e0e47ca60203452f5d61cc490e928cd2e3873bb343fc95da33179ca60f4dbf46c2c36e91299d55d4e6b902060600020507030401010404010702000a0ca02526000000000006");
}

#[test]
fn token_accounts_match_find_program_address() {
    let me = key("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk");
    let usdc = key("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
    let (tp, t22) = (key(solana::TOKEN_PROGRAM), key(solana::TOKEN_2022_PROGRAM));
    let cases = [
        (me, tp, "5N3f1tj9v1vc5TUZ8S7mCAnVmjVKrfnzXWhxLaxyZAgt"),
        (me, t22, "52DxsD9Yt7q6329gb9yCZwVL5tTXrjKRh2h9Ce4BqSKb"),
        (key(TO), tp, "FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B"),
        (key(TO), t22, "GdjpegrtGwU3pgtzPivYVViSA8rmGL248qBVKzsrU3DD"),
    ];
    for (owner, prog, want) in cases {
        assert_eq!(base58_encode(&solana::associated_token_address(&owner, &usdc, &prog)), want);
    }
}

#[test]
fn wallets_are_on_the_curve_and_program_addresses_are_not() {
    assert!(solana::is_on_curve(&key("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk")));
    assert!(solana::is_on_curve(&key(TO)));
    assert!(!solana::is_on_curve(&key("5N3f1tj9v1vc5TUZ8S7mCAnVmjVKrfnzXWhxLaxyZAgt")));
    assert!(!solana::is_on_curve(&key("GdjpegrtGwU3pgtzPivYVViSA8rmGL248qBVKzsrU3DD")));
}

#[test]
fn a_v0_transaction_from_elsewhere_is_signed_in_its_slot() {
    // solders' VersionedTransaction over a MessageV0, signed by the test key.
    let want = "017bbd87c066914dbc201d57f05a9b32380c1199e17092b27a6733d4b058456019d8aba34acc86a2c0b806f47019ef144042b617bc0c44cb1895cdf2e0b1691e068001000103f036276246a75b9de3349ed42b15e232f6518fc20f5fcd4f1d64e81f9bd258f77e8c088760bfde1dddcf32c17f209b8242ee52aaf131facd88d0ea2c6d0b06f20000000000000000000000000000000000000000000000000000000000000000cc490e928cd2e3873bb343fc95da33179ca60f4dbf46c2c36e91299d55d4e6b901020200010c02000000050000000000000000";
    let mut unsigned = unhex(want);
    unsigned[1..65].fill(0);
    let mut tx = Transaction::parse(&unsigned).unwrap();
    assert_eq!(tx.message.version, Some(0));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let signer = LocalSigner::restore(&path, PASS, &Phrase::parse(ABOUT).unwrap()).unwrap();
    signer.sign_solana_transaction(PASS, &mut tx).unwrap();
    assert_eq!(hex(&tx.to_bytes()), want);
}

#[test]
fn a_transaction_for_someone_else_is_not_signed() {
    let other = Phrase::parse("legal winner thank year wave sausage worth useful legal winner thank yellow")
        .unwrap().solana().unwrap();
    let msg = solana::compile(&other.public(), &[solana::system_transfer(&other.public(), &key(TO), 1)],
                              &key(BLOCKHASH));
    let dir = tempfile::tempdir().unwrap();
    let signer = LocalSigner::restore(dir.path().join("keystore.json"), PASS,
                                      &Phrase::parse(ABOUT).unwrap()).unwrap();
    let mut tx = Transaction::unsigned(msg).unwrap();
    let e = signer.sign_solana_transaction(PASS, &mut tx).unwrap_err();
    assert!(e.contains("not asking your Solana account"), "{e}");
    assert!(tx.signatures.iter().all(|s| *s == [0u8; 64]));
}

#[test]
fn a_transaction_needing_a_second_signer_is_not_half_signed() {
    let sol = Phrase::parse(ABOUT).unwrap().solana().unwrap();
    let other = [5u8; 32];
    // `other` must sign too (it is the `from` of a second transfer)
    let msg = solana::compile(&sol.public(), &[
        solana::system_transfer(&sol.public(), &key(TO), 1),
        solana::system_transfer(&other, &key(TO), 1),
    ], &key(BLOCKHASH));
    let dir = tempfile::tempdir().unwrap();
    let signer = LocalSigner::restore(dir.path().join("keystore.json"), PASS,
                                      &Phrase::parse(ABOUT).unwrap()).unwrap();
    let mut tx = Transaction::unsigned(msg).unwrap();
    assert_eq!(tx.message.signers, 2);
    assert!(signer.sign_solana_transaction(PASS, &mut tx).unwrap_err().contains("another signer"));
    assert!(tx.signatures.iter().all(|s| *s == [0u8; 64]));
}

#[test]
fn a_wrong_passphrase_signs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let signer = LocalSigner::restore(dir.path().join("keystore.json"), PASS,
                                      &Phrase::parse(ABOUT).unwrap()).unwrap();
    let me = solana::parse_address(&signer.sol_address().unwrap()).unwrap();
    let mut tx = Transaction::unsigned(solana::compile(&me, &[solana::system_transfer(&me, &key(TO), 1)],
                                                       &key(BLOCKHASH))).unwrap();
    assert!(signer.sign_solana_transaction("not the passphrase", &mut tx).is_err());
}

#[test]
fn the_solana_half_shows_while_locked_and_keeps_its_case() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    LocalSigner::restore(&path, PASS, &Phrase::parse(ABOUT).unwrap()).unwrap();
    let reopened = LocalSigner::at(&path);
    assert!(!reopened.is_unlocked());
    assert_eq!(reopened.sol_address().as_deref(), Some("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"));
    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("keystore.solana.json")).unwrap()).unwrap();
    assert_eq!(doc["path"], "m/44'/501'/0'/0'");
    assert_eq!(doc["chain"], "solana");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("keystore.solana.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn an_older_account_gains_solana_only_from_its_own_words() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let phrase = Phrase::parse(ABOUT).unwrap();
    // an account as it was made before the Solana half existed
    let secret = phrase.account().unwrap().secret();
    let mut signer = LocalSigner::import(&path, PASS, secret.as_ref()).unwrap();
    assert_eq!(signer.sol_address(), None);

    let e = signer.add_solana(PASS, "legal winner thank year wave sausage worth useful legal winner thank yellow")
        .unwrap_err();
    assert!(e.contains("different account"), "{e}");
    assert!(signer.add_solana("wrong passphrase", ABOUT).is_err());
    assert_eq!(signer.add_solana(PASS, ABOUT).unwrap(), "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk");
    assert!(signer.add_solana(PASS, ABOUT).unwrap_err().contains("already"));
    assert_eq!(LocalSigner::at(&path).sol_address().as_deref(),
               Some("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"));
}

#[test]
fn a_phantom_export_is_accepted_and_a_damaged_one_is_not() {
    let sol = Phrase::parse(ABOUT).unwrap().solana().unwrap();
    let mut pair = sol.secret().to_vec();
    pair.extend_from_slice(&sol.public());
    let exported = base58_encode(&pair);
    assert_eq!(SolAccount::from_typed(&exported).unwrap().address(), sol.address());
    pair[63] ^= 1;
    assert!(SolAccount::from_typed(&base58_encode(&pair)).is_err());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let mut signer = LocalSigner::import(&path, PASS, &[7u8; 32]).unwrap();
    assert_eq!(signer.add_solana(PASS, &exported).unwrap(), sol.address());
}

#[test]
fn a_leftover_solana_half_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    std::fs::write(dir.path().join("keystore.solana.json"), "{}").unwrap();
    let e = LocalSigner::restore(&path, PASS, &Phrase::parse(ABOUT).unwrap()).err().unwrap();
    assert!(e.contains("only copy"), "{e}");
    assert!(!path.exists(), "nothing is written when the Solana half is refused");
}

/// The same leftover refuses an IMPORT, which writes no Solana half of its
/// own. Let through, the old account's Solana address read back as the new
/// key's — a player who moved `keystore.json` aside, as the overwrite refusal
/// tells them to, would be shown someone else's address as theirs.
#[test]
fn an_import_beside_a_leftover_solana_half_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    LocalSigner::restore(&path, PASS, &Phrase::parse(ABOUT).unwrap()).unwrap();
    std::fs::rename(&path, dir.path().join("moved-aside.json")).unwrap();

    let e = LocalSigner::import(&path, PASS, &[7u8; 32]).err().expect("refused");
    assert!(e.contains("keystore.solana.json"), "the refusal names the file: {e}");
    assert!(e.contains("only copy"), "{e}");
    assert!(!path.exists(), "nothing is written when the leftover is refused");
}

/// A Solana half that is there and cannot be read is not "no Solana address
/// yet" — that sentence sends the player to make a second one.
#[test]
fn an_unreadable_solana_half_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    LocalSigner::import(&path, PASS, &[7u8; 32]).unwrap();
    // A directory where the file should be: there, and unreadable as a file.
    std::fs::create_dir(dir.path().join("keystore.solana.json")).unwrap();

    let mut signer = LocalSigner::at(&path);
    assert!(signer.unreadable().is_some_and(|w| w.contains("cannot read")), "{:?}", signer.unreadable());
    let me = [1u8; 32];
    let mut tx = Transaction::unsigned(solana::compile(&me, &[solana::system_transfer(&me, &key(TO), 1)],
                                                       &key(BLOCKHASH))).unwrap();
    let e = signer.sign_solana_transaction(PASS, &mut tx).unwrap_err();
    assert!(e.contains("cannot read") && !e.contains("yet"), "{e}");
    let e = signer.add_solana(PASS, ABOUT).unwrap_err();
    assert!(e.contains("cannot be read"), "{e}");
}

#[test]
fn base64_round_trips() {
    for n in 0..8 {
        let b: Vec<u8> = (0..n as u8).map(|i| i.wrapping_mul(37)).collect();
        assert_eq!(solana::b64_decode(&solana::b64_encode(&b)).unwrap(), b);
    }
    assert_eq!(solana::b64_encode(b"elo"), "ZWxv");
}
