//! Against published vectors, because the assembly is the only thing here
//! that is ours.
//!
//! Every primitive is a RustCrypto call and those have their own suites. What
//! this crate wrote is *which bytes go into which primitive in what order* —
//! the EIP-191 prefix, the EIP-55 casing, the V3 MAC over the second half of
//! the derived key. Each of those has a published answer, so each is checked
//! against it rather than against itself.

use scry_vault::{eip191_digest, keystore, to_checksum, Account};

/// EIP-55's own examples. If the casing rule were subtly wrong every address
/// this client shows would be a valid address with the wrong capitalisation —
/// which most tools accept, so nothing would break until one did not.
#[test]
fn eip55_checksums_match_the_specs_examples() {
    let cases = [
        "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
        "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
        "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
        "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
    ];
    for want in cases {
        let raw = hex_bytes(want);
        assert_eq!(to_checksum(&raw), want, "EIP-55 casing drifted");
    }
}

fn hex_bytes(s: &str) -> Vec<u8> {
    let s = s.trim_start_matches("0x");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// The EIP-191 prefix, against a digest anyone can recompute.
///
/// `keccak256("\x19Ethereum Signed Message:\n12Hello World")` — the canonical
/// worked example. The prefix is what stops a signature over a *message* from
/// ever being valid over a *transaction*, so getting the length or the prefix
/// wrong is not cosmetic.
#[test]
fn the_eip191_prefix_is_the_published_one() {
    let d = eip191_digest(b"Hello World");
    assert_eq!(
        scry_vault_hex(&d),
        "a1de988600a42c4b4ab089b619297c17d53cffae5d5120d82d8a92d0bb3b78f2",
        "the personal_sign digest drifted — check the prefix and the length"
    );
}

fn scry_vault_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A round trip through the real KDF at the real work factor. Slow on purpose:
/// scrypt at n=262144 is the whole point, and a test that lowered it would be
/// testing a configuration nobody ships.
#[test]
fn a_keystore_round_trips_at_the_shipped_work_factor() {
    let acct = Account::generate().expect("entropy");
    let secret = acct.secret();
    let addr = acct.address();

    let doc = keystore::encrypt(&secret, "correct horse battery staple", &addr).expect("encrypt");
    assert_eq!(doc["version"], serde_json::json!(3));
    assert_eq!(doc["crypto"]["cipher"], serde_json::json!("aes-128-ctr"));
    assert_eq!(doc["crypto"]["kdf"], serde_json::json!("scrypt"));
    assert_eq!(doc["crypto"]["kdfparams"]["n"], serde_json::json!(262144));

    let back = keystore::decrypt(&doc, "correct horse battery staple").expect("decrypt");
    assert_eq!(back, secret, "the key did not survive the round trip");

    // And the recovered key is the same account, not merely the same bytes.
    let again = Account::from_secret(&back).expect("valid key");
    assert_eq!(again.address(), addr);
}

/// A wrong passphrase must say **wrong passphrase**, not "corrupt".
///
/// They are the same bytes and different sentences, and telling a player their
/// file is corrupt when they mistyped is how someone deletes the only copy of
/// their key.
#[test]
fn a_wrong_passphrase_says_so_and_never_returns_plaintext() {
    let acct = Account::generate().expect("entropy");
    let doc = keystore::encrypt(&acct.secret(), "right", &acct.address()).expect("encrypt");
    let e = keystore::decrypt(&doc, "wrong").expect_err("must not decrypt");
    assert!(e.to_lowercase().contains("passphrase"), "say what happened: {e}");
    assert!(!e.to_lowercase().contains("corrupt"), "it is not corrupt: {e}");
}

/// The MAC is checked BEFORE the plaintext is used. Asserted by tampering with
/// the ciphertext: the answer must be the MAC failing, not 32 wrong bytes
/// coming back as a key.
#[test]
fn a_tampered_ciphertext_fails_the_mac_rather_than_returning_a_key() {
    let acct = Account::generate().expect("entropy");
    let mut doc = keystore::encrypt(&acct.secret(), "pass", &acct.address()).expect("encrypt");
    let ct = doc["crypto"]["ciphertext"].as_str().unwrap().to_string();
    let mut bytes: Vec<char> = ct.chars().collect();
    bytes[0] = if bytes[0] == 'a' { 'b' } else { 'a' };
    doc["crypto"]["ciphertext"] = serde_json::json!(bytes.into_iter().collect::<String>());
    let e = keystore::decrypt(&doc, "pass").expect_err("a tampered file must not open");
    assert!(e.contains("MAC") || e.to_lowercase().contains("passphrase"), "{e}");
}

/// Anything not fully understood is refused rather than guessed. A best effort
/// here produces 32 bytes that are not the key, and the first symptom is a
/// signature nobody can verify.
#[test]
fn an_unsupported_document_is_refused_with_a_reason() {
    let acct = Account::generate().expect("entropy");
    let good = keystore::encrypt(&acct.secret(), "p", &acct.address()).expect("encrypt");

    let mut v1 = good.clone();
    v1["version"] = serde_json::json!(1);
    assert!(keystore::decrypt(&v1, "p").unwrap_err().contains("v3"));

    let mut cbc = good.clone();
    cbc["crypto"]["cipher"] = serde_json::json!("aes-128-cbc");
    assert!(keystore::decrypt(&cbc, "p").unwrap_err().contains("aes-128-ctr"));

    let mut pb = good.clone();
    pb["crypto"]["kdf"] = serde_json::json!("pbkdf2");
    let e = keystore::decrypt(&pb, "p").unwrap_err();
    assert!(e.contains("scrypt") && e.contains("geth"),
            "an unsupported kdf should say what CAN open it: {e}");
}

/// A signature this crate produces must be the shape the external signer's own
/// check accepts — the two halves of the client agreeing about what a
/// signature looks like.
#[test]
fn a_signature_is_65_bytes_of_hex_with_an_ethereum_v() {
    let acct = Account::generate().expect("entropy");
    let sig = acct.sign_message("scry play\nround 41").expect("sign");
    assert!(sig.starts_with("0x"));
    assert_eq!(sig.len(), 132, "65 bytes: r || s || v");
    let v = u8::from_str_radix(&sig[130..132], 16).expect("v is hex");
    assert!(v == 27 || v == 28, "ethereum's v is 27 or 28, got {v}");
    assert!(scry_broker_shape(&sig), "the external signer's own check must accept it");
}

/// The same predicate `ExternalSigner` applies, restated here so the two halves
/// cannot drift apart silently.
fn scry_broker_shape(s: &str) -> bool {
    let s = s.trim();
    s.len() == 132 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Two generated accounts are different. Cheap, and it is the assertion that
/// fails loudly if entropy is ever stubbed or seeded.
#[test]
fn generated_accounts_are_not_the_same_account() {
    let a = Account::generate().expect("entropy");
    let b = Account::generate().expect("entropy");
    assert_ne!(a.secret(), b.secret(), "the entropy source is not random");
    assert_ne!(a.address(), b.address());
}

/// One account renders as ONE string, locked or not.
///
/// Caught by a round trip and not by a test: V3 stores the address lowercase
/// and unprefixed, so a locked account read straight from the file rendered
/// `0x8e66…` while the same account unlocked rendered `0x8E66…`. A player
/// comparing that against their wallet sees a mismatch and cannot tell which
/// is theirs.
#[test]
fn a_locked_account_shows_the_same_address_as_an_unlocked_one() {
    use scry_broker::signer::Signer;
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("keystore.json");

    let created = match scry_vault::LocalSigner::create(&path, "pass") {
        Ok(s) => s,
        Err(e) => panic!("create: {e}"),
    };
    let unlocked_addr = created.address().expect("an unlocked account knows itself");

    let mut reopened = scry_vault::LocalSigner::at(&path);
    let locked_addr = reopened.address().expect("a locked account still shows who it is");
    assert_eq!(locked_addr, unlocked_addr, "locked and unlocked must agree");

    reopened.unlock("pass").expect("unlock");
    assert_eq!(reopened.address().unwrap(), unlocked_addr, "and unlocking changes nothing");

    // And it is the checksummed form, which is what a wallet will show.
    assert_ne!(unlocked_addr, unlocked_addr.to_lowercase(),
               "an address should be EIP-55 cased, not lowercased");
}

/// A locked signer refuses rather than signing with nothing.
#[test]
fn a_locked_account_refuses_to_sign() {
    use scry_broker::signer::Signer;
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("keystore.json");
    let _ = scry_vault::LocalSigner::create(&path, "pass").map_err(|e| panic!("create: {e}"));

    let mut locked = scry_vault::LocalSigner::at(&path);
    let e = locked.sign("scry play\nx", "y").expect_err("locked must not sign");
    assert!(e.reason.contains("locked"), "say why: {}", e.reason);
}

/// A keystore is the only copy of a key. Overwriting one on a stray click
/// would be unforgivable, so it is refused outright.
#[test]
fn creating_over_an_existing_keystore_is_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("keystore.json");
    let _ = scry_vault::LocalSigner::create(&path, "pass").map_err(|e| panic!("first: {e}"));
    let e = match scry_vault::LocalSigner::create(&path, "other") {
        Ok(_) => panic!("must not clobber an existing keystore"),
        Err(e) => e,
    };
    assert!(e.contains("refusing to overwrite"), "{e}");
}

/// **The V3 document carries every field a FOREIGN reader requires.**
///
/// This test exists because the round trip could not catch what was missing.
/// `encrypt` omitted `id`, every test here decrypted with `decrypt` in the same
/// crate, and `decrypt` never reads `id` — so the file round-tripped perfectly
/// while failing to open in `cast wallet`, geth or MetaMask, all of which share
/// an `eth-keystore` decoder that types the field as REQUIRED. The symptom was
/// `Failed to decrypt keystore: missing field 'id'`, which reads like a wrong
/// passphrase and is not one.
///
/// So this asserts the SHAPE against the spec rather than against ourselves.
/// The module header promises the account is not trapped here; that promise is
/// only worth what a reader outside this crate can open.
#[test]
fn the_document_has_every_field_a_foreign_reader_needs() {
    let doc = match scry_vault::encrypt(&[7u8; 32], "pass", "0x0000000000000000000000000000000000000000") {
        Ok(d) => d,
        Err(e) => panic!("encrypt: {e}"),
    };

    for field in ["version", "id", "address", "crypto"] {
        assert!(doc.get(field).is_some(), "V3 requires `{field}` — a foreign reader rejects the file without it");
    }
    let crypto = doc.get("crypto").expect("crypto");
    for field in ["cipher", "cipherparams", "ciphertext", "kdf", "kdfparams", "mac"] {
        assert!(crypto.get(field).is_some(), "V3 crypto requires `{field}`");
    }

    // A UUID, not merely a non-empty string: 8-4-4-4-12, version nibble 4 and
    // a 10xx variant. A reader that parses the field will reject a placeholder.
    let id = doc.get("id").and_then(|v| v.as_str()).expect("id is a string");
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12],
               "a UUID is 8-4-4-4-12: {id}");
    assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'), "hex and dashes only: {id}");
    assert!(parts[2].starts_with('4'), "version 4 UUID: {id}");
    assert!(matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b')),
            "RFC 4122 variant is 10xx: {id}");

    // Two documents must not share an id — it is generated, never constant.
    let other = scry_vault::encrypt(&[7u8; 32], "pass", "0x0000000000000000000000000000000000000000")
        .expect("second");
    assert_ne!(doc.get("id"), other.get("id"), "each keystore gets its own id");
}

/// An imported key is the SAME account it was elsewhere.
///
/// The point of `import` is that a holder with a wallet already does not have
/// to abandon it, so the one thing that must be true is that the address does
/// not change in transit. Checked against a known secret and its known address
/// rather than against whatever we happened to produce.
#[test]
fn importing_a_known_key_keeps_its_address() {
    use scry_broker::signer::Signer;
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("keystore.json");

    // secp256k1 secret 0x01 — its address is a published constant, and this
    // is the EIP-55 casing `cast wallet address --private-key 0x…01` prints.
    // ⚠ The last character is a lowercase `f`: the checksum casing is derived
    // from a keccak hash, so it cannot be eyeballed and a hand-typed constant
    // gets it wrong. That is the whole reason to check against a foreign tool.
    let mut secret = [0u8; 32];
    secret[31] = 1;
    const KNOWN: &str = "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf";

    let imported = match scry_vault::LocalSigner::import(&path, "pass", &secret) {
        Ok(s) => s,
        Err(e) => panic!("import: {e}"),
    };
    assert_eq!(imported.address().as_deref(), Some(KNOWN), "an imported key keeps its address");

    // And it survives the file: reopen and unlock gives the same account.
    let mut reopened = scry_vault::LocalSigner::at(&path);
    reopened.unlock("pass").expect("unlock an imported keystore");
    assert_eq!(reopened.address().as_deref(), Some(KNOWN), "and the file holds that same key");
}

/// Import refuses to clobber, exactly as `create` does. A refusal that depends
/// on which verb you typed is a refusal a caller can route around.
#[test]
fn importing_over_an_existing_keystore_is_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("keystore.json");
    let _ = scry_vault::LocalSigner::create(&path, "pass").map_err(|e| panic!("first: {e}"));
    let e = match scry_vault::LocalSigner::import(&path, "other", &[9u8; 32]) {
        Ok(_) => panic!("import must not clobber an existing keystore"),
        Err(e) => e,
    };
    assert!(e.contains("refusing to overwrite"), "{e}");
}

/// **Signing and recovering are inverses, and that is the whole trust story.**
///
/// Until `recover_signer` existed this client could produce a signature and
/// verify none — which is why `publisher` in a manifest was a string somebody
/// typed. Checked as a round trip AND against tampering, because a recover
/// that returned the right answer for the right message and *also* for a wrong
/// one would pass a round-trip test and secure nothing.
#[test]
fn a_signature_recovers_to_the_account_that_made_it() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("keystore.json");
    let mut secret = [0u8; 32];
    secret[31] = 1;
    let signer = match scry_vault::LocalSigner::import(&path, "pass", &secret) {
        Ok(s) => s,
        Err(e) => panic!("import: {e}"),
    };
    let account = scry_vault::Account::from_secret(&secret).expect("account");
    let address = account.address();

    let message = "scry manifest\ngates\n0.1.0";
    let sig = account.sign_message(message).expect("sign");

    assert_eq!(scry_vault::recover_signer(message, &sig).as_deref(), Ok(address.as_str()),
               "a signature must recover to its own signer");

    // Belt and braces: the address the signer reports and the one recovered
    // from its signature are the same string, casing included.
    use scry_broker::signer::Signer;
    assert_eq!(signer.address().as_deref(), Some(address.as_str()));

    // ⚠ The negative half. A signature is only worth anything if it FAILS to
    // recover the signer for a message they did not sign.
    let tampered = scry_vault::recover_signer("scry manifest\ngates\n0.2.0", &sig)
        .expect("recovery still yields some key");
    assert_ne!(tampered, address, "a changed message must not recover the signer");
}

/// Both `v` conventions are accepted, and the transaction one is refused.
#[test]
fn the_recovery_byte_accepts_wallets_and_refuses_eip155() {
    let mut secret = [0u8; 32];
    secret[31] = 7;
    let account = scry_vault::Account::from_secret(&secret).expect("account");
    let address = account.address();
    let sig = account.sign_message("hello").expect("sign");

    // As produced: v is 27 or 28.
    assert_eq!(scry_vault::recover_signer("hello", &sig).as_deref(), Ok(address.as_str()));

    // The same signature with the raw 0/1 recovery id still works — refusing
    // it would reject valid signatures for a reason no caller could guess.
    let bare = sig.trim_start_matches("0x").to_string();
    let v = u8::from_str_radix(&bare[128..130], 16).expect("v");
    let raw_v = format!("{}{:02x}", &bare[..128], v - 27);
    assert_eq!(scry_vault::recover_signer("hello", &raw_v).as_deref(), Ok(address.as_str()));

    // ⚠ EIP-155's 35+2*chain_id is a TRANSACTION encoding and must not be
    // taken here: accepting it would let a signature authorising a transfer be
    // replayed as a signature over a document.
    let tx_v = format!("{}{:02x}", &bare[..128], 37u8);
    assert!(scry_vault::recover_signer("hello", &tx_v).is_err(),
            "an EIP-155 recovery byte must be refused");

    // And a truncated signature says the shape rather than panicking.
    assert!(scry_vault::recover_signer("hello", "0xdeadbeef").is_err());
}
