//! The wallet half of the vault — the recovery phrase and transactions —
//! checked against `eth_account` and the vectors every wallet already knows.
//!
//! The constants below were produced by `eth_account` 0.14 (Python), outside
//! this crate: `Account.from_mnemonic` for the phrases and
//! `Account.sign_transaction` for the type-2 transactions. Matching them byte
//! for byte is what "typing these words into MetaMask gives the same account"
//! and "a node accepts what this signs" rest on.

use elo_broker::signer::Signer;
use elo_vault::phrase::{self, Phrase};
use elo_vault::tx::{self, Tx};
use elo_vault::{parse_address, Account, LocalSigner};

const PASS: &str = "test passphrase";

fn unhex(s: &str) -> Vec<u8> {
    let s = s.trim_start_matches("0x");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// (entropy, words, seed, key at m/44'/60'/0'/0/0, address)
const BIP39: [(&str, &str, &str, &str, &str); 7] = [
    (
        "00000000000000000000000000000000",
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4",
        "1ab42cc412b618bdea3a599e3c9bae199ebf030895b039e9db1e30dafb12b727",
        "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
    ),
    (
        "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
        "878386efb78845b3355bd15ea4d39ef97d179cb712b77d5c12b6be415fffeffe5f377ba02bf3f8544ab800b955e51fbff09828f682052a20faa6addbbddfb096",
        "33fa40f84e854b941c2b0436dd4a256e1df1cb41b9c1c0ccc8446408c19b8bf9",
        "0x58A57ed9d8d624cBD12e2C467D34787555bB1b25",
    ),
    (
        "ffffffffffffffffffffffffffffffff",
        "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
        "b6a6d8921942dd9806607ebc2750416b289adea669198769f2e15ed926c3aa92bf88ece232317b4ea463e84b0fcd3b53577812ee449ccc448eb45e6f544e25b6",
        "7af65ba4dd53f23495dcb04995e96f47c243217fc279f10795871b725cd009ae",
        "0xfc2077CA7F403cBECA41B1B0F62D91B5EA631B5E",
    ),
    (
        "9e885d952ad362caeb4efe34a8e91bd2",
        "ozone drill grab fiber curtain grace pudding thank cruise elder eight picnic",
        "e2f88a043776c828063d4c3c97173944d32cf847a925b6e40b0b8bd0b4bead3ba734bdda5250d4698b310a71c9934e1a48e562315ce22bf85f89459df0e73a6c",
        "82d1b15eb8f18be874762d6ad5a4e1476aa984bc5b14ecd994dc8f03ec11b7ef",
        "0xd01D6027860c2E2e37295d3dE4B5a5f46F68d52f",
    ),
    (
        "c0ba5a8e914111210f2bd131f3d5e08d",
        "scheme spot photo card baby mountain device kick cradle pact join borrow",
        "00e1e39a39e23735adab690d0ffbefda6cacc063b4a5fcc605de060bd370adc54c94370d966b9a3fae5ed16bd58a02224cbefca4146a083951b70be2a2ce3dcc",
        "a024991551147ee4db00d16b1022e97b950bb46485a12e79b484f2d65f7c63df",
        "0xc70eC8094CCE52B6574A5a2b7bf0c7D8288d77cC",
    ),
    (
        "0000000000000000000000000000000000000000000000000000000000000000",
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
        "408b285c123836004f4b8842c89324c1f01382450c0d439af345ba7fc49acf705489c6fc77dbd4e3dc1dd8cc6bc9f043db8ada1e243c4a0eafb290d399480840",
        "1053fae1b3ac64f178bcc21026fd06a3f4544ec2f35338b001f02d1d8efa3d5f",
        "0xF278cF59F82eDcf871d630F28EcC8056f25C1cdb",
    ),
    (
        "68a79eaca2324873eacc50cb9c6eca8cc68ea5d936f98787c60c7ebc74e6ce7c",
        "hamster diagram private dutch cause delay private meat slide toddler razor book happy fancy gospel tennis maple dilemma loan word shrug inflict delay length",
        "17e4b5661796eeff8904550f8572289317ece7c1cc1316469f8f4c986c1ffd7b9f4c3aeac3e1713ffc21fa33707d09d57a2ece358d72111ef7c7658e7b33f2d5",
        "d224f35c694af7f12be4ba18ac5a9e1ac774633200ab2302d23839cef0a12f03",
        "0x1dD9058722aBEE7Eb7c24B7270c5F88D342E2e90",
    ),
];

/// (nonce, max priority fee, max fee, gas, to, value, data, raw, hash)
#[allow(clippy::type_complexity)]
const TXS: [(u64, u128, u128, u64, &str, u128, &str, &str, &str); 4] = [
    (
        0, 0, 20000000, 21000,
        "0x3535353535353535353535353535353535353535",
        0,
        "",
        "02f86882123780808401312d008252089435353535353535353535353535353535353535358080c001a010708770d686d48daee2a619553ee2fadd3220e5c1967313f8ba6aa30169250ba014c6122018e55098b62386c8548730fa9b955ba9504a74e785ed15c847523c2e",
        "fb9eb5c8912a331251842dfc07208ea77489deecc642f2a72714d5da62a5d160",
    ),
    (
        7, 0, 20000000, 60000,
        "0xF0109fC8DF283027b6285cc889F5aA624EaC1F55",
        1000000000000000,
        "",
        "02f86f82123707808401312d0082ea6094f0109fc8df283027b6285cc889f5aa624eac1f5587038d7ea4c6800080c080a0fa3569c9d1b3ff6e75fc18592df5480cca04824cd176aef4a0d07c57add8524ba044d3d12948e227b285a8af07a98ac7e48cebc0950fef7f50c0fb47bc95b6b371",
        "59f150fe2a3e42cf225800217d7992d46a2eed6ea75559b0297617a82c305b7e",
    ),
    (
        300, 1000000000, 200000000000, 1000000,
        "0x0000000000000000000000000000000000000001",
        340282366920938463463374607431768211455,
        "",
        "02f88082123782012c843b9aca00852e90edd000830f424094000000000000000000000000000000000000000190ffffffffffffffffffffffffffffffff80c080a0936eb6fdddd967e9836b2624dcf9fe8c227b490af01225cf5e77021a8d024506a07bd2548b2f6354f107cc043ee69e9abbe99d6b68a5637c4b2fa77ad11b02000b",
        "05bf17115b00b5d3b588b43305e2d1a219f00bfd62b7fbd4aef604912276b072",
    ),
    (
        65535, 0, 12345678, 90000,
        "0xd6bd13a8a16e5249fa54961fc92eee0307e0f1b9",
        0,
        "a9059cbb000000000000000000000000f0109fc8df283027b6285cc889f5aa624eac1f55000000000000000000000000000000000000000000000000ad78ebc5ac620000",
        "02f8af82123782ffff8083bc614e83015f9094d6bd13a8a16e5249fa54961fc92eee0307e0f1b980b844a9059cbb000000000000000000000000f0109fc8df283027b6285cc889f5aa624eac1f55000000000000000000000000000000000000000000000000ad78ebc5ac620000c001a0a6f00f77829b8b98912c8364f26ef0df3dc17de7147ff8074b1b109309a564b2a02453b72d0d634e363e9fb3b317a4c4a5db3d414a0c0cab62fd8c8632ca073e9a",
        "2c691e225808bbf3b6bbdef14aed7879c242886058b9e0f046d165f998fdf08a",
    ),
];

/// The key the transaction vectors are signed with (`eth_account`'s own docs
/// example) and its address.
const TX_KEY: &str = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
const TX_FROM: &str = "0x2c7536E3605D9C16a7a3D7b1898e529396a65c23";

/// The wordlist is the official one. One changed word and the same twelve
/// words derive somebody else's account in every other wallet.
#[test]
fn the_wordlist_is_the_official_english_list() {
    use sha2::Digest;
    let words = phrase::wordlist();
    assert_eq!(words.len(), 2048);
    assert!(words.windows(2).all(|w| w[0] < w[1]), "sorted, so lookup can bisect");
    let canonical = words.join("\n") + "\n";
    assert_eq!(
        hex(&sha2::Sha256::digest(canonical.as_bytes())),
        "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda",
        "bip-0039/english.txt's published SHA-256"
    );
}

#[test]
fn words_seed_and_key_match_eth_account() {
    for (entropy, words, seed, key, address) in BIP39 {
        let p = Phrase::from_entropy(&unhex(entropy)).expect("entropy");
        assert_eq!(p.words().join(" "), words, "entropy {entropy} → words");
        assert_eq!(hex(p.seed().as_ref()), seed, "{words} → seed");
        let acct = p.account().expect("derive");
        assert_eq!(hex(acct.secret().as_ref()), key, "{words} → key");
        assert_eq!(acct.address(), address, "{words} → address");
        // And typed back in, they are the same phrase.
        let back = Phrase::parse(&words.to_uppercase().replace(' ', "   ")).expect("parse");
        assert_eq!(back.words().join(" "), words);
    }
}

/// Two addresses anyone can check in any wallet: the "abandon … about" phrase
/// and Hardhat's default "test … junk".
#[test]
fn the_phrases_every_wallet_knows_give_the_addresses_every_wallet_shows() {
    let abandon = Phrase::parse(&("abandon ".repeat(11) + "about")).expect("abandon");
    assert_eq!(abandon.account().unwrap().address(), "0x9858EfFD232B4033E47d90003D41EC34EcaEda94");
    let hardhat = Phrase::parse(&("test ".repeat(11) + "junk")).expect("hardhat");
    assert_eq!(hardhat.account().unwrap().address(), "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
}

#[test]
fn a_bad_phrase_is_refused_and_never_repeated() {
    let good = "legal winner thank year wave sausage worth useful legal winner thank yellow";
    // A word not on the list: say which position, never the word.
    let e = Phrase::parse(&good.replace("sausage", "sausagez")).unwrap_err();
    assert!(e.contains("Word 6"), "{e}");
    assert!(!e.contains("sausage"), "the error must not carry the input: {e}");
    // All real words, wrong checksum.
    let e = Phrase::parse(&good.replace("yellow", "zoo")).unwrap_err();
    assert!(e.contains("checksum"), "{e}");
    // Wrong count.
    assert!(Phrase::parse("legal winner thank").unwrap_err().contains("12 or 24"));
}

#[test]
fn new_phrases_are_twelve_words_and_differ() {
    let a = Phrase::generate().unwrap();
    let b = Phrase::generate().unwrap();
    assert_eq!(a.words().len(), 12);
    assert_ne!(a.words(), b.words(), "the entropy source is not random");
    // A generated phrase reads back as itself, checksum and all.
    assert_eq!(Phrase::parse(&a.words().join(" ")).unwrap().words(), a.words());
    assert!(!format!("{a:?}").contains(a.words()[0]), "Debug must not print the words");
}

#[test]
fn the_quiz_asks_three_different_words_and_checks_them_in_order() {
    let p = Phrase::parse(BIP39[3].1).unwrap();
    let q = p.quiz().unwrap();
    assert!(q[0] < q[1] && q[1] < q[2] && q[0] >= 1 && q[2] <= 12, "{q:?}");
    let w = p.words();
    let right = format!("{} {} {}", w[q[0] - 1], w[q[1] - 1], w[q[2] - 1]);
    assert!(p.check(&q, &right));
    assert!(p.check(&q, &format!("  {}  ", right.to_uppercase())), "case and spacing are forgiven");
    let swapped = format!("{} {} {}", w[q[1] - 1], w[q[0] - 1], w[q[2] - 1]);
    assert!(w[q[0] - 1] == w[q[1] - 1] || !p.check(&q, &swapped), "order matters");
    assert!(!p.check(&q, &format!("{} {}", w[q[0] - 1], w[q[1] - 1])), "all three");
}

#[test]
fn a_restored_keystore_is_the_phrases_account() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let p = Phrase::parse(BIP39[0].1).unwrap();
    let s = LocalSigner::restore(&path, PASS, &p).expect("restore");
    assert_eq!(s.address().as_deref(), Some(BIP39[0].4));
    let mut again = LocalSigner::at(&path);
    again.unlock(PASS).expect("unlock");
    assert_eq!(again.address().as_deref(), Some(BIP39[0].4));
}

#[test]
fn a_short_passphrase_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let e = LocalSigner::import(&path, "seven77", &[1u8; 32]).err().expect("refused");
    assert!(e.contains("at least 8"), "{e}");
    assert!(!path.exists(), "a refused passphrase must leave no file");
}

/// Readable by its owner only from the moment it exists, in a directory only
/// its owner can list.
#[cfg(unix)]
#[test]
fn the_keystore_is_private_from_birth() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new").join("keystore.json");
    LocalSigner::import(&path, PASS, &[1u8; 32]).expect("import");
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
}

fn vector_tx(i: usize) -> Tx {
    let (nonce, prio, max, gas, to, value, data, _, _) = TXS[i];
    let mut t = [0u8; 20];
    t.copy_from_slice(&unhex(to));
    Tx { nonce, max_priority_fee: prio, max_fee: max, gas, to: t, value, data: unhex(data) }
}

/// Byte for byte against `eth_account`: zero nonce and value, a plain send,
/// the largest u128 value, and an ERC-20 transfer.
#[test]
fn transactions_match_eth_account_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let s = LocalSigner::import(&path, PASS, &unhex(TX_KEY)).expect("import");
    assert_eq!(s.address().as_deref(), Some(TX_FROM));
    // One decrypt per transaction is the design, so this is slow by scrypt.
    for (i, v) in TXS.iter().enumerate() {
        let signed = s.sign_transaction(PASS, &vector_tx(i)).expect("sign");
        assert_eq!(hex(&signed.raw), v.7, "raw tx {i}");
        assert_eq!(hex(&signed.hash), v.8, "tx hash {i}");
    }
}

#[test]
fn a_transaction_needs_the_passphrase_every_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let mut s = LocalSigner::import(&path, PASS, &unhex(TX_KEY)).expect("import");
    // Unlocked for games does not mean unlocked for money.
    assert!(s.is_unlocked());
    let e = s.sign_transaction("wrong passphrase", &vector_tx(1)).unwrap_err();
    assert!(e.contains("wrong passphrase"), "{e}");
    // Locked is fine: the passphrase given is what opens it.
    s.lock();
    assert!(s.sign_transaction(PASS, &vector_tx(1)).is_ok());
}

/// A keystore whose clear-text address is not its key's would sign "from"
/// an address nobody was shown.
#[test]
fn a_keystore_naming_another_address_signs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    LocalSigner::import(&path, PASS, &unhex(TX_KEY)).expect("import");
    let mut doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    doc["address"] = serde_json::json!("f39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    std::fs::write(&path, doc.to_string()).unwrap();
    let e = LocalSigner::at(&path).sign_transaction(PASS, &vector_tx(1)).unwrap_err();
    assert!(e.contains("not the address it names"), "{e}");
}

#[test]
fn erc20_calldata_round_trips_and_nothing_else_reads_as_a_transfer() {
    let to = parse_address("0xF0109fC8DF283027b6285cc889F5aA624EaC1F55").unwrap();
    let data = tx::erc20_transfer(&to, 12_500_000_000_000_000_000);
    assert_eq!(hex(&data), TXS[3].6, "the same calldata eth_account signed");
    assert_eq!(tx::read_erc20_transfer(&data), Some((to, 12_500_000_000_000_000_000)));
    let mut other = data.clone();
    other[0] = 0x09; // approve-shaped, or anything else
    assert_eq!(tx::read_erc20_transfer(&other), None);
    assert_eq!(tx::read_erc20_transfer(&data[..67]), None);
}

#[test]
fn a_mistyped_checksummed_address_is_refused() {
    let good = "0xF0109fC8DF283027b6285cc889F5aA624EaC1F55";
    assert!(parse_address(good).is_ok());
    assert!(parse_address(&good.to_lowercase()).is_ok(), "no checksum to check");
    // One letter's case flipped: the shape of a single changed character.
    let bad = good.replacen('F', "f", 1);
    assert!(parse_address(&bad).unwrap_err().contains("checksum"));
    assert!(parse_address("0x1234").is_err());
}

#[test]
fn signing_a_message_and_a_transaction_use_the_same_account() {
    let a = Account::from_secret(&unhex(TX_KEY)).unwrap();
    assert_eq!(a.address(), TX_FROM);
}

/// Once hardened, another same-user process cannot attach to this one.
#[cfg(target_os = "linux")]
#[test]
fn hardening_blocks_attach() {
    let failed = elo_vault::harden_process();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(elo_vault::harden::attach_blocked(), Some(true));
}
