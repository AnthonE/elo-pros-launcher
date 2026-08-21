//! Argument parsing.
//!
//! The one rule worth a test: **an unknown flag is a refusal, never an
//! ignore.** A caller that believes it set something and was quietly ignored
//! has been misled in the dangerous direction — the same rule the broker's
//! wire follows, for the same reason.

// Shared with the binary, so the test target legitimately does not
// exercise every item in it.
#[allow(dead_code)]
#[path = "../src/args.rs"]
mod args;

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

#[test]
fn a_verb_and_positionals_are_read_in_order() {
    let a = args::parse(&argv("uninstall gates 0.1.0")).unwrap();
    assert_eq!(a.verb, "uninstall");
    assert_eq!(a.positional, ["gates", "0.1.0"]);
}

#[test]
fn value_flags_take_a_value_either_way() {
    let a = args::parse(&argv("install m.json --games /tmp/g --platform linux-aarch64")).unwrap();
    assert_eq!(a.flag("--games"), Some("/tmp/g"));
    assert_eq!(a.flag("--platform"), Some("linux-aarch64"));
    assert_eq!(a.positional, ["m.json"]);

    let a = args::parse(&argv("install m.json --games=/tmp/g")).unwrap();
    assert_eq!(a.flag("--games"), Some("/tmp/g"));
}

#[test]
fn a_browser_command_survives_being_one_argument() {
    // App mode is the reason `{url}` exists, and it must reach the launcher as
    // a single token rather than being split by the caller.
    let a = args::parse(&[
        "play".into(),
        "m.json".into(),
        "--browser".into(),
        "chromium --app={url}".into(),
    ])
    .unwrap();
    assert_eq!(a.flag("--browser"), Some("chromium --app={url}"));
}

#[test]
fn switches_are_recognised_and_take_no_value() {
    let a = args::parse(&argv("status m.json --json --dry-run")).unwrap();
    assert!(a.has("--json"));
    assert!(a.has("--dry-run"));
    assert!(!a.has("--yes"));
    assert!(args::parse(&argv("status m.json --json=yes")).is_err());
}

#[test]
fn an_unknown_flag_is_refused_never_ignored() {
    let err = args::parse(&argv("install m.json --sneaky")).unwrap_err();
    assert!(err.contains("unknown flag"), "{err}");
    // Near-misses are the dangerous ones: silently ignoring `--game` when the
    // flag is `--games` would install to the default root without saying so.
    assert!(args::parse(&argv("install m.json --game /tmp")).is_err());
}

#[test]
fn a_value_flag_with_no_value_is_refused() {
    let err = args::parse(&argv("install m.json --games")).unwrap_err();
    assert!(err.contains("needs a value"), "{err}");
}

#[test]
fn no_arguments_is_not_an_error() {
    let a = args::parse(&[]).unwrap();
    assert!(a.verb.is_empty());
}

// ── repeated value flags ─────────────────────────────────────────────────────
//
// `flags` keeps the LAST occurrence, which is right for `--host` and wrong for
// `--set`: an edit is a set of fields, and `--set a=1 --set b=2` is the obvious
// way to write one. Both maps are kept so no caller has to know which kind of
// flag it is holding.

#[test]
fn a_repeated_value_flag_keeps_every_occurrence_in_order() {
    let a = args::parse(&argv("dev edit gates --set blurb=x --set kind=y --set tags=z")).unwrap();
    assert_eq!(a.all("--set"), ["blurb=x", "kind=y", "tags=z"]);
}

#[test]
fn and_the_last_one_still_wins_for_flags_that_are_singular() {
    let a = args::parse(&argv("games --host http://a --host http://b")).unwrap();
    assert_eq!(a.flag("--host"), Some("http://b"));
    assert_eq!(a.all("--host"), ["http://a", "http://b"]);
}

#[test]
fn a_flag_that_was_never_given_is_an_empty_list_not_a_panic() {
    let a = args::parse(&argv("dev")).unwrap();
    assert!(a.all("--set").is_empty());
}

#[test]
fn the_dev_desks_flags_take_values_rather_than_being_unknown() {
    // Each of these was an `unknown flag` refusal until the desk landed, which
    // is the failure mode this list exists to prevent.
    for flag in ["--set", "--from", "--slot", "--title", "--body", "--body-file",
                 "--kind", "--art", "--link", "--scope", "--days"] {
        let line = format!("dev post gates {flag} value");
        let a = args::parse(&argv(&line))
            .unwrap_or_else(|e| panic!("{flag} should take a value: {e}"));
        assert_eq!(a.flag(flag), Some("value"), "{flag}");
    }
}

// ── join links ───────────────────────────────────────────────────────────────
//
// The desktop hands a url handler its url as `argv[1]`, which lands in the
// verb slot. These pin that the handler and a typed `elo join` are one path.

#[test]
fn a_url_in_the_verb_slot_becomes_a_join() {
    let a = args::normalize_link(
        args::parse(&argv("elo://join/gates/game.elopros.com:61234")).unwrap(),
    );
    assert_eq!(a.verb, "join");
    assert_eq!(a.positional, ["elo://join/gates/game.elopros.com:61234"]);
}

#[test]
fn a_typed_join_is_left_exactly_as_it_was() {
    // The two must arrive at the verb identically, or the handler path and the
    // typed path can drift.
    let typed = args::normalize_link(
        args::parse(&argv("join elo://join/gates/h:1")).unwrap(),
    );
    let clicked = args::normalize_link(
        args::parse(&argv("elo://join/gates/h:1")).unwrap(),
    );
    assert_eq!(typed, clicked);
}

#[test]
fn a_malformed_link_still_reaches_join_rather_than_unknown_command() {
    // It must be refused as a bad LINK. Falling through to the verb table
    // would report `unknown command "elo://nonsense"` to somebody who clicked
    // something a friend sent them.
    let a = args::normalize_link(args::parse(&argv("elo://nonsense")).unwrap());
    assert_eq!(a.verb, "join");
    assert_eq!(a.positional, ["elo://nonsense"]);
}

#[test]
fn an_ordinary_verb_is_untouched() {
    for line in ["play gates", "games", "install gates --games /tmp/g"] {
        let before = args::parse(&argv(line)).unwrap();
        assert_eq!(args::normalize_link(before.clone()), before, "{line}");
    }
}

#[test]
fn flags_survive_the_rewrite() {
    // A url handler invocation carries no flags, but a typed one does, and the
    // rewrite must not eat them.
    let a = args::normalize_link(
        args::parse(&argv("elo://join/gates/h:1 --games /tmp/g --dry-run")).unwrap(),
    );
    assert_eq!(a.verb, "join");
    assert_eq!(a.flag("--games"), Some("/tmp/g"));
    assert!(a.has("--dry-run"));
}

#[test]
fn a_titles_shard_list_reaches_its_argv_and_a_players_flags_do_not_reach_it() {
    // The defect this gates: Gates draws its own shard browser, the list was
    // published, the manifest named it — and nothing put it on the argv, so
    // every launch outside the Servers window drew an empty list over a live
    // shard. `{servers}` is the TITLE's to name, so it comes from the manifest
    // and there is no flag for it.
    let a = args::parse(&argv("play gates --server game.example:61234 --wallet 0xabc")).unwrap();

    let v = args::launch_values(&a, Some("https://elo.example/api/launcher/servers/gates"));
    assert_eq!(v.get("server").map(String::as_str), Some("game.example:61234"));
    assert_eq!(v.get("wallet").map(String::as_str), Some("0xabc"));
    assert_eq!(
        v.get("servers").map(String::as_str),
        Some("https://elo.example/api/launcher/servers/gates")
    );

    // A title that publishes no list inserts nothing rather than an empty
    // string — `launch::fill` is what turns an absent value into `""`, and
    // deciding it twice is how the two would drift.
    let v = args::launch_values(&a, None);
    assert!(!v.contains_key("servers"));

    // And no flag can forge one: `--servers` is not a flag this parser knows,
    // so it is refused rather than quietly ignored.
    assert!(args::parse(&argv("play gates --servers https://evil.example/list")).is_err());
}
