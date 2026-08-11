//! Sessions and playtime.
//!
//! The load-bearing property is not "does it add up" — it is that **an
//! unwatched launch never becomes a number**. A launcher that guesses when a
//! detached game ended is inventing data, and `LAUNCHER.md` wall 3 is that
//! this client is not a source of truth.

use scry_depot::session::{self, Ending, Session};

fn started(slug: &str, at: i64, ending: Ending) -> Session {
    Session {
        slug: slug.into(),
        build: "0.1.0".into(),
        started_at: at,
        ending,
    }
}

#[test]
fn a_measured_session_yields_playtime_and_the_others_do_not() {
    let m = started("gates", 1000, Ending::Measured { ended_at: 4600, seconds: 3600, code: Some(0) });
    let r = started("gates", 1000, Ending::Running { pid: 42 });
    let d = started("gates", 1000, Ending::Detached { pid: 42 });

    assert_eq!(m.playtime(), Some(3600));
    // `None` is not zero. Zero would be a claim that they played for no time;
    // None is the truth, which is that nobody watched.
    assert_eq!(r.playtime(), None);
    assert_eq!(d.playtime(), None);
    assert!(r.is_running() && !d.is_running() && !m.is_running());
}

#[test]
fn totals_count_only_what_was_watched_and_say_how_much_was_not() {
    let all = vec![
        started("gates", 0, Ending::Measured { ended_at: 3600, seconds: 3600, code: Some(0) }),
        started("gates", 0, Ending::Measured { ended_at: 1800, seconds: 1800, code: Some(0) }),
        started("gates", 0, Ending::Detached { pid: 7 }),
        started("gates", 0, Ending::Detached { pid: 8 }),
        started("barrow", 0, Ending::Running { pid: 9 }),
    ];
    let totals = session::totals(&all);
    assert_eq!(totals.get("gates"), Some(&5400));
    // A title that has only ever been detached or is still running has no
    // total at all, rather than a total of zero.
    assert_eq!(totals.get("barrow"), None);
    assert_eq!(session::unmeasured(&all).get("gates"), Some(&2));
    assert_eq!(session::unmeasured(&all).get("barrow"), None);
}

#[test]
fn a_session_round_trips_through_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let rows = vec![
        started("gates", 100, Ending::Measured { ended_at: 200, seconds: 100, code: Some(0) }),
        started("gates", 300, Ending::Running { pid: 12345 }),
        started("barrow", 400, Ending::Detached { pid: 999 }),
    ];
    session::save(tmp.path(), &rows).unwrap();
    assert_eq!(session::load(tmp.path()), rows);
}

#[test]
fn a_missing_log_is_empty_and_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(session::load(tmp.path()).is_empty());
    // …and so is a corrupt one. Unlike a network read, this really is
    // "nothing there": we looked at a local file and it was not usable.
    std::fs::write(session::sessions_path(tmp.path()), b"{ not json").unwrap();
    assert!(session::load(tmp.path()).is_empty());
}

#[test]
fn closing_a_session_measures_it() {
    let tmp = tempfile::tempdir().unwrap();
    session::append(tmp.path(), started("gates", 1000, Ending::Running { pid: 5 })).unwrap();

    assert!(session::close(tmp.path(), "gates", "0.1.0", 4600, Some(0)).unwrap());
    let all = session::load(tmp.path());
    assert_eq!(all[0].playtime(), Some(3600));

    // Closing again finds nothing open, and says so rather than inventing a
    // second session or double-counting the first.
    assert!(!session::close(tmp.path(), "gates", "0.1.0", 9999, Some(0)).unwrap());
    assert_eq!(session::totals(&session::load(tmp.path())).get("gates"), Some(&3600));
}

#[test]
fn closing_matches_the_newest_open_session_for_that_build() {
    // A player can have two builds installed and start either, and can start
    // the same build twice.
    let tmp = tempfile::tempdir().unwrap();
    session::append(tmp.path(), started("gates", 100, Ending::Running { pid: 1 })).unwrap();
    session::append(tmp.path(), started("gates", 500, Ending::Running { pid: 2 })).unwrap();
    let mut other = started("gates", 700, Ending::Running { pid: 3 });
    other.build = "0.2.0".into();
    session::append(tmp.path(), other).unwrap();

    session::close(tmp.path(), "gates", "0.1.0", 900, Some(0)).unwrap();
    let all = session::load(tmp.path());
    assert!(all[0].is_running(), "the older 0.1.0 session stays open");
    assert_eq!(all[1].playtime(), Some(400), "the newer one closed");
    assert!(all[2].is_running(), "0.2.0 is untouched");
}

#[test]
fn a_clock_that_went_backwards_never_produces_negative_playtime() {
    let tmp = tempfile::tempdir().unwrap();
    session::append(tmp.path(), started("gates", 5000, Ending::Running { pid: 5 })).unwrap();
    session::close(tmp.path(), "gates", "0.1.0", 1000, Some(0)).unwrap();
    assert_eq!(session::load(tmp.path())[0].playtime(), Some(0));
}

#[test]
fn the_log_is_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let rows: Vec<Session> = (0..session::MAX_SESSIONS as i64 + 50)
        .map(|i| started("gates", i, Ending::Measured { ended_at: i + 1, seconds: 1, code: Some(0) }))
        .collect();
    session::save(tmp.path(), &rows).unwrap();
    let back = session::load(tmp.path());
    assert_eq!(back.len(), session::MAX_SESSIONS);
    // The OLDEST are dropped, so "what did I play recently" survives.
    assert_eq!(back.last().unwrap().started_at, rows.last().unwrap().started_at);
}

#[test]
fn a_dead_pid_is_not_alive_and_our_own_is() {
    assert!(!session::is_alive(0));
    #[cfg(unix)]
    {
        assert!(session::is_alive(std::process::id()));
        // A pid that cannot exist on Linux (max is 2^22 by default).
        assert!(!session::is_alive(0x7fff_fffe));
    }
}
