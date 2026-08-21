//! The keyless backends, and the property that makes them keyless.

use elo_broker::signer::{ArcaSigner, BrowserSigner, ExternalSigner, NoSigner, Signer};

/// `CLAUDE.md` invariant 7 — *we hold no user keys* — asserted against the
/// source rather than promised in a comment.
///
/// This is a **file-level property** and it is checkable because the four
/// backends here work by asking someone else: the browser, the arca, a program
/// the player named, or nobody. The moment `LocalSigner` lands this test must
/// change shape — the Python module made exactly that move and states it, so
/// the failure here is a prompt to state it again rather than to delete a
/// check.
#[test]
fn no_branch_in_this_file_reads_a_key() {
    let src = include_str!("../src/signer.rs");
    for needle in [
        "private_key", "privatekey", "secret_key", "seed_phrase",
        "mnemonic", "keystore", "from_bytes", "SigningKey",
    ] {
        // Prose may name the thing it forbids; code may not. Only lines that
        // are not comments are searched.
        for (n, line) in src.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("*") {
                continue;
            }
            assert!(
                !code.contains(needle),
                "signer.rs:{} reads like key material ({needle:?}): {code}\n\
                 Four backends here hold no key by construction. If this is \
                 LocalSigner arriving, this test must be re-scoped per-backend \
                 the way the Python module's was — not deleted.",
                n + 1
            );
        }
    }
}

#[test]
fn none_refuses_and_says_what_would_change_it() {
    let mut s = NoSigner;
    assert_eq!(s.kind(), "none");
    assert!(s.address().is_none());
    let e = s.sign("elo play\nx", "y").expect_err("none must not sign");
    assert!(e.reason.contains("Signing") || e.reason.contains("configured"),
            "the refusal should point somewhere: {}", e.reason);
    assert!(!e.refused_by_player, "the player did not refuse — nothing was asked");
}

/// A handoff is **not an error**, and the SDK distinguishes it. A game that
/// renders it as one tells the player something broke when nothing did.
#[test]
fn the_browser_hands_off_rather_than_failing() {
    let mut s = BrowserSigner {
        host: "https://elopros.com/".into(),
        wallet: Some("0xabc".into()),
    };
    let e = s.sign("elo play\nround 41", "settling").expect_err("browser never signs here");
    let url = e.handoff.expect("a browser signer must hand off");
    assert!(url.starts_with("https://elopros.com/play.html"),
            "play goes to the play page: {url}");
    assert!(url.contains("wallet=0xabc"));
    assert!(!e.refused_by_player, "a handoff is not a refusal by the player");
}

/// A family with no page goes to the wallet page — **a 404 in a browser is a
/// worse answer than a general page.**
#[test]
fn an_unplaceable_family_goes_somewhere_that_exists() {
    assert_eq!(BrowserSigner::page_for("elo play\n…"), "play.html");
    assert_eq!(BrowserSigner::page_for("elo review\n…"), "reviews.html");
    assert_eq!(BrowserSigner::page_for("elo braid\n…"), "hive.html");
    assert_eq!(BrowserSigner::page_for("elo somethingnew\n…"), "wallet.html");
    assert_eq!(BrowserSigner::page_for("not even an elo message"), "wallet.html");
}

/// The rule that stops an error travelling as a signature and failing far away
/// with a useless message.
#[test]
fn an_external_signer_must_print_something_shaped_like_a_signature() {
    let good = format!("0x{}", "ab".repeat(65));
    assert!(ExternalSigner::looks_like_a_signature(&good));
    assert!(ExternalSigner::looks_like_a_signature(&format!("  {good}\n")));

    for bad in [
        "",
        "error: no device found",
        "0xdeadbeef",                              // too short
        &format!("0x{}", "ab".repeat(64)),         // 64 bytes, not 65
        &format!("0x{}", "zz".repeat(65)),         // not hex
        &"ab".repeat(65),                          // no 0x
    ] {
        assert!(
            !ExternalSigner::looks_like_a_signature(bad),
            "{bad:?} must not pass as a signature"
        );
    }
}

/// The command is split without a shell. The string is the player's, but
/// running it through one would make every metacharacter in it live.
#[test]
fn the_signing_command_is_split_without_a_shell() {
    assert_eq!(ExternalSigner::argv("cast wallet sign").unwrap(),
               vec!["cast", "wallet", "sign"]);
    assert_eq!(ExternalSigner::argv("/opt/my signer/sign.sh").unwrap(),
               vec!["/opt/my", "signer/sign.sh"]);
    assert_eq!(ExternalSigner::argv("\"/opt/my signer/sign.sh\" --hex").unwrap(),
               vec!["/opt/my signer/sign.sh", "--hex"]);
    // A shell would treat these as operators; here they are argument text and
    // the program named first is the only thing that runs.
    assert_eq!(ExternalSigner::argv("sign.sh; rm -rf /").unwrap(),
               vec!["sign.sh;", "rm", "-rf", "/"]);
    assert!(ExternalSigner::argv("").is_err(), "an empty command is refused");
    assert!(ExternalSigner::argv("\"unclosed").is_err());
}

#[test]
fn an_external_signer_with_no_command_refuses_before_spawning_anything() {
    let mut s = ExternalSigner { command: "   ".into(), wallet: None };
    let e = s.sign("elo play\nx", "").expect_err("nothing to run");
    assert!(e.reason.contains("no signing command"), "{}", e.reason);
    assert!(s.status().contains("no command set"));
}

/// An external signer that really runs. Uses `printf` so the test needs no
/// fixture on disk and no shell.
#[test]
#[cfg(unix)]
fn an_external_signer_that_prints_a_signature_is_believed() {
    let sig = format!("0x{}", "cd".repeat(65));
    let mut s = ExternalSigner {
        command: format!("printf {sig}"),
        wallet: Some("0xfeed".into()),
    };
    let out = s.sign("elo play\nx", "").expect("a well-shaped signature is accepted");
    assert_eq!(out["signature"], serde_json::json!(sig));
    assert_eq!(out["backend"], serde_json::json!("external"));
}

/// And one that prints an error instead is refused HERE, where the message is
/// still useful.
#[test]
#[cfg(unix)]
fn an_external_signer_that_prints_an_error_is_refused_here() {
    let mut s = ExternalSigner {
        command: "printf no-device-found".into(),
        wallet: None,
    };
    let e = s.sign("elo play\nx", "").expect_err("that is not a signature");
    assert!(e.reason.contains("no-device-found"),
            "the refusal should quote what it got: {}", e.reason);
}

/// An arca that is not there answers with a reason a player can act on, not a
/// panic and not a silent `None`.
#[test]
fn a_missing_arca_says_so() {
    let s = ArcaSigner { socket_path: String::new() };
    assert_eq!(s.kind(), "arca");
    let status = s.status();
    assert!(!status.is_empty(), "an unreachable arca still owes a sentence");

    let s = ArcaSigner { socket_path: "/nonexistent/arca.sock".into() };
    assert!(s.address().is_none());
    assert!(s.status().len() > 8, "and a reason: {}", s.status());
}
