//! Every fixture in sdk/testdata/ parses, and every signed text rebuilds byte
//! for byte. The same files pin the Python and JavaScript SDKs, Gates and the
//! launcher, and `meter/test_sdk_fixtures.py` pins them to the server.

use elo_sdk::models::{FriendsView, Items, Leaderboard, Ticket, TicketOf, Who};
use elo_sdk::payout::{check_doc, check_row};
use elo_sdk::{classify, play_message, texts, Look, PayoutFile};
use serde_json::Value;

fn fx(text: &str) -> Value {
    serde_json::from_str(text).expect("a fixture is JSON")
}

fn as_answer<T: serde::de::DeserializeOwned>(v: &Value) -> T {
    match classify::<T>(200, &v.to_string()) {
        Look::Answer(t) => t,
        other => panic!("expected an answer, got {}", other.kind()),
    }
}

#[test]
fn friends_texts_rebuild_byte_for_byte() {
    let f = fx(include_str!("../../testdata/friends_texts.json"));
    let ops = f["ops"].as_object().unwrap();
    assert_eq!(ops.len(), texts::OPS.len());
    for (op, key) in texts::OPS {
        assert_eq!(ops[op], key, "{op} takes {key}");
    }
    let cases = f["cases"].as_array().unwrap();
    assert_eq!(cases.len(), texts::OPS.len());
    for c in cases {
        let (me, op, other) = (
            c["wallet"].as_str().unwrap(),
            c["op"].as_str().unwrap(),
            c["other"].as_str().unwrap(),
        );
        let at = c["at"].as_u64().unwrap();
        assert_eq!(
            texts::day_of(at as i64),
            c["day"].as_str().unwrap(),
            "{op}: day"
        );
        assert_eq!(
            texts::friends_detail(op, other, at).unwrap(),
            c["detail"].as_str().unwrap(),
            "{op}: detail"
        );
        assert_eq!(
            texts::friends_sign_text(me, op, other, at).unwrap(),
            c["text"].as_str().unwrap(),
            "{op}: text"
        );
        // A checksummed wallet signs the same lowercase bytes.
        let upper = format!("0x{}", me[2..].to_ascii_uppercase());
        assert_eq!(
            texts::friends_sign_text(&upper, op, other, at).unwrap(),
            c["text"].as_str().unwrap()
        );
    }
}

#[test]
fn friends_view_parses_with_buddies() {
    let v: FriendsView = as_answer(&fx(include_str!("../../testdata/friends_view.json")));
    assert_eq!(v.count, 3);
    assert_eq!(v.friends.len(), 3);
    assert!(v.presence.reachable);
    assert_eq!(v.limits.friends_max, Some(250));
    let ash = &v.friends[0];
    assert_eq!(ash.label(), "ash");
    assert_eq!(
        ash.playing.as_ref().and_then(|p| p.join.as_deref()),
        Some("elo://join/gates/game.elopros.com:61234")
    );
    assert!(v.friends[1].agent && v.friends[1].playing.as_ref().unwrap().join.is_none());
    assert_eq!(v.friends[2].last_online, Some(1_760_990_000));
    assert_eq!(v.friends[2].label(), "0x4444…4444");
    assert!(v.friends.iter().all(|f| !f.is_buddy()));
    assert_eq!(v.buddies.len(), 1);
    let pip = &v.buddies[0];
    assert!(pip.is_buddy() && pip.agent && pip.since.is_none());
    assert_eq!(
        pip.buddy.as_ref().map(|b| (b.id.as_str(), b.kind.as_str())),
        Some(("pip", "buddy"))
    );
    assert_eq!(v.asks_in[0].name.as_deref(), Some("rook"));
    assert_eq!(v.asks_out[0].name, None);
}

#[test]
fn ticket_card_and_ownership() {
    let t = fx(include_str!("../../testdata/ticket.json"));
    let card: Ticket = as_answer(&t["ticketed"]);
    assert!(card.ticketed);
    assert_eq!(card.price_usd, Some(9.99));
    assert!(card.rails["eth"].open && !card.rails["elo"].open);
    assert_eq!(card.all_rails[0].rail_id, 1);
    assert!(card.all_rails[0].buyable);
    assert_eq!(card.free_mint.as_ref().and_then(|f| f.per_wallet), Some(1));
    let free: Ticket = as_answer(&t["free"]);
    assert!(!free.ticketed && free.what.is_some());

    let o = fx(include_str!("../../testdata/ticket_of.json"));
    let owned: TicketOf = as_answer(&o["owned"]);
    assert_eq!(
        (owned.entitled, owned.copies, owned.free_left),
        (Some(true), Some(1), Some(1))
    );
    let free_of: TicketOf = as_answer(&o["free"]);
    assert_eq!(free_of.entitled, Some(true));
    let down = &o["could_not_look"];
    let look: Look<TicketOf> = classify(
        down["status"].as_u64().unwrap() as u16,
        &down["body"].to_string(),
    );
    match look {
        Look::CouldNotLook(why) => assert!(why.contains("could not look")),
        other => panic!(
            "a failed chain read must be could-not-look, got {}",
            other.kind()
        ),
    }
}

#[test]
fn items_held_unreadable_and_unconfigured() {
    let i = fx(include_str!("../../testdata/items_of.json"));
    let held: Items = as_answer(&i["held"]);
    assert_eq!(held.count, 3);
    assert_eq!(held.items.len(), 3);
    assert_eq!(held.items[0].token_id, Some(41));
    assert!(held.items[0].image.is_some());
    assert!(held.items[1]
        .derived
        .as_ref()
        .is_some_and(|d| d.get("float").is_some()));
    assert!(held.items[2].is_vault() && held.items[2].token_id.is_none());
    assert_eq!(held.balance_onchain, Some(2));
    let down: Look<Items> = classify(200, &i["could_not_look"].to_string());
    assert!(
        down.is_could_not_look(),
        "an index that timed out is not an empty wallet"
    );
    let dark: Items = as_answer(&i["not_configured"]);
    assert_eq!((dark.configured, dark.items.len()), (Some(false), 0));
}

#[test]
fn face_body_and_signed_texts() {
    let f = fx(include_str!("../../testdata/face.json"));
    let set: elo_sdk::models::Face = as_answer(&f["set"]);
    assert_eq!(set.name.as_deref(), Some("ash"));
    assert!(set.picture.as_deref().unwrap().starts_with("/api/face/"));
    assert_eq!(set.guild.as_ref().map(|g| g.tag.as_str()), Some("OWL"));
    let unset: elo_sdk::models::Face = as_answer(&f["unset"]);
    assert_eq!(unset.name, None);
    assert_eq!(unset.guild, None);
    for c in f["cases"].as_array().unwrap() {
        let g = |k: &str| c["fields"].get(k).and_then(Value::as_str);
        let detail = texts::face_detail(
            c["at"].as_u64().unwrap(),
            g("name"),
            g("picture_sha256"),
            g("kind"),
            g("harness"),
            g("model"),
        );
        assert_eq!(detail, c["detail"].as_str().unwrap());
        let text = play_message(
            texts::FACE_ACTION,
            c["wallet"].as_str().unwrap(),
            &detail,
            c["day"].as_str(),
        );
        assert_eq!(text, c["text"].as_str().unwrap());
    }
}

#[test]
fn guild_tags_and_a_wallets_guilds() {
    let g = fx(include_str!("../../testdata/guilds.json"));
    let tags: elo_sdk::models::GuildTags = as_answer(&g["tags"]);
    let owl = tags
        .of("0xB2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2")
        .expect("a wearer reads, any case");
    assert_eq!(
        (owl.tag.as_str(), owl.color.as_deref(), owl.role.as_deref()),
        ("OWL", Some("teal"), Some("member"))
    );
    assert!(tags
        .of("0xc3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3")
        .is_none());
    assert!(tags
        .tags
        .contains_key("0xc3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3"));
    let of: elo_sdk::models::GuildsOf = as_answer(&g["of"]);
    assert_eq!(of.worn.as_deref(), Some("OWL"));
    assert!(!of.hidden);
    assert_eq!(of.guilds[0].emblem.shape.as_deref(), Some("hex"));
    assert_eq!(
        (of.guilds[0].members, of.guilds[0].role.as_deref()),
        (2, Some("member"))
    );
}

#[test]
fn leaderboard_published_and_not() {
    let b = fx(include_str!("../../testdata/leaderboard.json"));
    let pub_: Leaderboard = as_answer(&b["published"]);
    assert!(pub_.published && pub_.boards.len() == 3);
    let island = &pub_.boards[0];
    assert_eq!(island.sort_key, "island");
    assert_eq!(island.score_of(&island.entries[0]), Some(1200.0));
    assert_eq!(
        island.entries[1].harness.as_deref(),
        Some("claude-code/2.1")
    );
    assert!(pub_.boards[2].is_final);
    let agents: Leaderboard = as_answer(&b["agents"]);
    assert_eq!(agents.kind.as_deref(), Some("agent"));
    let none: Look<Leaderboard> = classify(200, &b["unpublished"].to_string());
    assert!(none.is_could_not_look());
}

#[test]
fn who_parses() {
    let w = fx(include_str!("../../testdata/who_fixture.json"));
    let who: Who = as_answer(&w["who"]);
    assert!(who.authoritative);
    let found = who.found.unwrap();
    assert_eq!(found.agent.as_deref(), Some("Moss"));
    assert_eq!(found.nip05.as_deref(), Some("moss@elopros.com"));
    assert_eq!(
        found.profile.as_ref().and_then(|p| p.name.as_deref()),
        Some("Moss")
    );
    assert!(
        found.rest.contains_key("reach"),
        "what is not named lands in `rest`"
    );
}

#[test]
fn payout_files_good_and_bad() {
    let good = fx(include_str!("../../testdata/payout.json"));
    assert!(check_doc(&good).is_empty(), "{:?}", check_doc(&good));
    let file: PayoutFile = serde_json::from_value(good.clone()).unwrap();
    assert!(file.validate().is_empty());
    assert_eq!(file.payouts[0].reasons, vec!["EXTRACTED 120".to_string()]);
    let back: PayoutFile = serde_json::from_str(&file.to_json()).unwrap();
    assert_eq!(back, file);

    let bad = fx(include_str!("../../testdata/payout_bad.json"));
    for b in bad["payouts"].as_array().unwrap() {
        let why = b["why"].as_str().unwrap();
        let got = check_row(&b["row"]).expect_err(why);
        assert!(got.starts_with(why), "{got} should start with {why}");
    }
    for b in bad["files"].as_array().unwrap() {
        assert!(!check_doc(&b["doc"]).is_empty(), "{} is refused", b["why"]);
    }

    let dir = std::env::temp_dir().join(format!("elo-sdk-payout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(file.file_name("island"));
    file.write_atomic(&path).unwrap();
    let read: PayoutFile = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(read, file);
    let refused = PayoutFile::new(0).write_atomic(&dir.join("bad.json"));
    assert!(refused.is_err() && !dir.join("bad.json").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn buddy_texts_read_back_as_the_change_asked_for() {
    let f = fx(include_str!("../../testdata/buddy_texts.json"));
    let ops: Vec<&str> = f["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_str().unwrap())
        .collect();
    assert_eq!(ops, texts::BUDDY_OPS, "the ops are the server's");
    for c in f["cases"].as_array().unwrap() {
        let w = c["wallet"].as_str().unwrap();
        let got = texts::buddy_change(c["text"].as_str().unwrap(), w).expect("a buddy text");
        assert_eq!(got.day, c["day"].as_str().unwrap());
        assert_eq!(got.op, c["op"].as_str().unwrap());
        assert_eq!(got.at, c["at"].as_u64().unwrap());
        assert_eq!(got.args, c["args"], "{}", c["op"]);
        assert!(texts::same_args(&c["asked"], &got.args), "{}", c["asked"]);
        let upper = format!("0x{}", w[2..].to_ascii_uppercase());
        assert_eq!(
            texts::read_play(c["text"].as_str().unwrap(), texts::BUDDY_ACTION, &upper),
            Some((
                c["day"].as_str().unwrap().to_string(),
                c["detail"].as_str().unwrap().to_string()
            )),
            "a checksummed wallet reads the same text"
        );
    }
    for c in f["refused"].as_array().unwrap() {
        let got = texts::buddy_change(c["text"].as_str().unwrap(), c["wallet"].as_str().unwrap());
        assert!(
            !got.is_some_and(|g| g.op == c["op"] && texts::same_args(&c["asked"], &g.args)),
            "{}",
            c["why"]
        );
    }
}

#[test]
fn board_and_entitle_texts_rebuild_byte_for_byte() {
    let f = fx(include_str!("../../testdata/board_texts.json"));
    for c in f["cases"].as_array().unwrap() {
        let (w, q, day) = (
            c["wallet"].as_str().unwrap(),
            c["quest_id"].as_str().unwrap(),
            c["day"].as_str().unwrap(),
        );
        if c["op"] == "claim" {
            assert_eq!(
                texts::claim_text(w, q, Some(day)),
                c["text"].as_str().unwrap()
            );
            continue;
        }
        #[cfg(feature = "http")]
        {
            let (url, n) = (c["url"].as_str().unwrap(), c["n"].as_u64().unwrap());
            assert_eq!(
                texts::submit_detail(q, url, n),
                c["detail"].as_str().unwrap()
            );
            assert_eq!(
                texts::submit_text(w, q, url, n, Some(day)),
                c["text"].as_str().unwrap()
            );
        }
    }
    let f = fx(include_str!("../../testdata/entitle_texts.json"));
    for c in f["cases"].as_array().unwrap() {
        assert_eq!(
            texts::entitle_text(
                c["game"].as_str().unwrap(),
                c["wallet"].as_str().unwrap(),
                c["day"].as_str()
            ),
            c["text"].as_str().unwrap()
        );
    }
}
