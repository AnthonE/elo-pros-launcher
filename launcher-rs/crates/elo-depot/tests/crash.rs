//! Crash dumps: which files in `<install>/crashes/` are offered, and the
//! report that carries one (`meter/bugs.py` is the rule it must pass).

use elo_depot::crash::{self, Crash, Format};
use std::time::{Duration, SystemTime};

fn crash<'a>(ended: &'a str) -> Crash<'a> {
    Crash { slug: "gates", build: "0.5.0-abcdef1", platform: "linux", ended, note: None,
            reported: "2026-11-02T12:00:00Z" }
}

fn touch(path: &std::path::Path, body: &[u8], at: SystemTime) {
    std::fs::write(path, body).unwrap();
    std::fs::File::options().write(true).open(path).unwrap().set_modified(at).unwrap();
}

#[test]
fn the_newest_dump_comes_first_and_only_dumps_are_offered() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let t0 = SystemTime::now() - Duration::from_secs(600);
    touch(&d.join("old.dmp"), b"MDMP\x01", t0);
    touch(&d.join("panic.txt"), b"thread 'main' panicked at a.rs:1", t0 + Duration::from_secs(60));
    touch(&d.join("core"), b"\x7fELF\x02\x01", t0 + Duration::from_secs(120));
    touch(&d.join("notes.md"), b"panicked", t0 + Duration::from_secs(120));
    touch(&d.join("earlier.dmp.sent"), b"MDMP", t0 + Duration::from_secs(180));
    std::fs::create_dir(d.join("sub.dmp")).unwrap();
    let got: Vec<String> = crash::find(d, None).iter().map(|x| x.name()).collect();
    assert_eq!(got, ["panic.txt", "old.dmp"]);
    let since: Vec<String> = crash::find(d, Some(t0 + Duration::from_secs(30)))
        .iter().map(|x| x.name()).collect();
    assert_eq!(since, ["panic.txt"], "a dump from before this run is not this run's");
    assert!(crash::find(&d.join("missing"), None).is_empty());
}

#[cfg(unix)]
#[test]
fn a_link_in_the_folder_is_never_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("secret.txt");
    std::fs::write(&outside, b"a private file").unwrap();
    let dir = tmp.path().join("crashes");
    std::fs::create_dir(&dir).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("panic.txt")).unwrap();
    assert!(crash::find(&dir, None).is_empty());
}

#[test]
fn a_sent_dump_is_renamed_and_not_offered_again() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("1.dmp"), b"MDMP").unwrap();
    let d = crash::find(tmp.path(), None).remove(0);
    let to = crash::mark_sent(&d).unwrap();
    assert!(to.ends_with("1.dmp.sent") && to.is_file());
    assert!(crash::find(tmp.path(), None).is_empty());
}

#[test]
fn a_dump_past_its_cap_is_listed_but_never_read() {
    let tmp = tempfile::tempdir().unwrap();
    let mut big = b"MDMP".to_vec();
    big.resize(crash::MINIDUMP_MAX as usize + 1, 0);
    std::fs::write(tmp.path().join("big.dmp"), &big).unwrap();
    let d = crash::find(tmp.path(), None).remove(0);
    assert!(d.too_big());
    assert!(d.read().is_err());
}

#[test]
fn the_body_is_the_report_the_dump_and_the_players_yes_and_nothing_else() {
    let dump = b"MDMP\x93\xa7rest";
    let b = crash::report_body(&crash("signal 11 (SIGSEGV)"), Format::Minidump, dump);
    let keys: Vec<&str> = b.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["dump", "game", "report"]);
    assert_eq!(b["game"], "gates");
    assert_eq!(b["dump"]["format"], "minidump");
    assert_eq!(b["dump"]["consent"], true);
    assert_eq!(b["dump"]["b64"], "TURNUJOncmVzdA==");
    let r = &b["report"];
    assert_eq!(r["format"], 1);
    assert_eq!(r["kind"], "crash");
    let fp = r["fingerprint"].as_str().unwrap();
    assert!(fp.len() == 16 && fp.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_eq!(r["build"]["version"], "0.5.0-abcdef1");
    assert_eq!(r["build"]["commit"], "abcdef1");
    assert_eq!(r["build"]["runtime"], "desktop");
    assert_eq!(r["read_first"], "README.md");
    assert!(r["issue_title"].as_str().unwrap().starts_with("[crash] gates"));
    assert!(r.get("machine").is_none(), "nothing about the machine rides along");
}

#[test]
fn one_crash_twice_is_one_board_row() {
    let a = crash::report(&crash("exit code 3"), Format::Minidump, b"MDMP1");
    let b = crash::report(&crash("exit code 3"), Format::Minidump, b"MDMP2");
    let c = crash::report(&crash("exit code 4"), Format::Minidump, b"MDMP1");
    assert_eq!(a["fingerprint"], b["fingerprint"]);
    assert_ne!(a["fingerprint"], c["fingerprint"]);
    let p = |t: &str| crash::report(&crash("exit code 101"), Format::Backtrace, t.as_bytes());
    assert_eq!(p("thread 'main' panicked at a.rs:1\n 0: x")["fingerprint"],
               p("\nthread 'main' panicked at a.rs:1\n 0: y")["fingerprint"]);
}

#[cfg(unix)]
#[test]
fn an_exit_is_abnormal_unless_it_succeeded() {
    let run = |s: &str| std::process::Command::new("sh").args(["-c", s]).status().unwrap();
    assert_eq!(crash::abnormal(&run("exit 0")), None);
    assert_eq!(crash::abnormal(&run("exit 3")).as_deref(), Some("exit code 3"));
    assert_eq!(crash::abnormal(&run("kill -SEGV $$")).as_deref(), Some("signal 11 (SIGSEGV)"));
}
