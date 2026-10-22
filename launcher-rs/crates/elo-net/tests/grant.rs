//! The ticket grant reaches the origin and nothing else.
//!
//! `X-Elo-Grant` is a day-lived bearer token for one title's downloads, and a
//! manifest names whatever hosts it likes. Two ways it used to leave:
//!
//! * **A prefix.** The scope was `url.starts_with(origin)`, so
//!   `https://elopros.com.evil.tld/…` and `https://elopros.com@evil.tld/…`
//!   both counted as the origin. Locally the same bug reads as a port prefix:
//!   an origin of `http://127.0.0.1:4` matched a listener on port 45678.
//! * **A redirect.** ureq strips `Authorization` and `Cookie` on a redirect
//!   and nothing else, so a header set for the first url rode to the next.
//!
//! One test function, because `Net::new` reads the environment and tests in
//! one binary run on parallel threads.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use elo_depot::Fetcher;
use elo_net::Net;

const GRANT: &str = "gates.0xabc.9999999999.deadbeef";

/// Accept one connection (or give up after a few seconds), answer it with
/// `reply`, and hand back the request head it sent. `None` is "nobody came".
fn serve_once(l: TcpListener, reply: String) -> thread::JoinHandle<Option<String>> {
    l.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(5);
        let mut s = loop {
            match l.accept() {
                Ok((s, _)) => break s,
                Err(_) if Instant::now() < until => thread::sleep(Duration::from_millis(10)),
                Err(_) => return None,
            }
        };
        s.set_nonblocking(false).unwrap();
        let mut buf = [0u8; 8192];
        let n = s.read(&mut buf).unwrap_or(0);
        let _ = s.write_all(reply.as_bytes());
        Some(String::from_utf8_lossy(&buf[..n]).to_lowercase())
    })
}

fn ok_json() -> String {
    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\
     connection: close\r\n\r\n{}"
        .to_string()
}

fn redirect(to: &str) -> String {
    format!("HTTP/1.1 302 Found\r\nlocation: {to}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
}

fn carries_grant(head: &Option<String>) -> bool {
    head.as_deref().is_some_and(|h| h.contains(&format!("x-elo-grant: {GRANT}")))
}

#[test]
fn the_grant_reaches_the_origin_and_no_other_host() {
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    std::env::set_var("ELO_GRANT", GRANT);

    // ── a host that only shares the origin's text ──────────────────────────
    let evil_l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = evil_l.local_addr().unwrap().port().to_string();
    let lookalike = format!("http://127.0.0.1:{}", &port[..1]);
    std::env::set_var("ELO_GRANT_ORIGIN", &lookalike);
    let net = Net::new();

    let h = serve_once(evil_l.try_clone().unwrap(), ok_json());
    let _ = net.get_json(&format!("http://127.0.0.1:{port}/depot.json"));
    let head = h.join().unwrap();
    assert!(head.is_some(), "the fetch must still happen");
    assert!(!carries_grant(&head), "a port that only starts with the origin's: {head:?}");

    let h = serve_once(evil_l.try_clone().unwrap(), ok_json());
    let dest = std::env::temp_dir().join(format!("elo-grant-{}", std::process::id()));
    let _ = net.fetch_sized(&format!("http://127.0.0.1:{port}/files/gates"), &dest, 2);
    let _ = std::fs::remove_file(&dest);
    let head = h.join().unwrap();
    assert!(!carries_grant(&head), "the depot file fetch, too: {head:?}");

    // Userinfo: `<origin>@<elsewhere>` starts with the origin's text and
    // connects to elsewhere.
    let h = serve_once(evil_l, ok_json());
    let _ = net.get_json(&format!("{lookalike}@127.0.0.1:{port}/depot.json"));
    let head = h.join().unwrap();
    assert!(!carries_grant(&head), "userinfo is not the origin: {head:?}");

    // ── a redirect off the origin, and one that stays on it ───────────────
    let origin_l = TcpListener::bind("127.0.0.1:0").unwrap();
    let evil_l = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://127.0.0.1:{}", origin_l.local_addr().unwrap().port());
    let evil = format!("http://localhost:{}", evil_l.local_addr().unwrap().port());
    std::env::set_var("ELO_GRANT_ORIGIN", &origin);
    let net = Net::new();

    let o = serve_once(origin_l.try_clone().unwrap(), redirect(&format!("{evil}/stolen")));
    let e = serve_once(evil_l, ok_json());
    let got = net.get_json(&format!("{origin}/api/launcher/depot/gates/1"));
    let first = o.join().unwrap();
    let second = e.join().unwrap();
    assert!(carries_grant(&first), "the origin itself gets the grant: {first:?}");
    assert!(second.is_some(), "the redirect is still followed");
    assert!(!carries_grant(&second), "the grant followed a redirect off the origin: {second:?}");
    assert!(got.ok, "and the read still succeeds: {}", got.why);

    // A redirect within the origin keeps it, so a depot route that moves is
    // not a 401. Both hops land on the same listener, one after the other.
    let both = thread::spawn(move || {
        let a = serve_once(origin_l.try_clone().unwrap(), redirect("/api/launcher/depot/gates/2"))
            .join()
            .unwrap();
        let b = serve_once(origin_l, ok_json()).join().unwrap();
        (a, b)
    });
    let got = net.get_json(&format!("{origin}/api/launcher/depot/gates/1"));
    let (a, b) = both.join().unwrap();
    assert!(carries_grant(&a), "same origin, first hop: {a:?}");
    assert!(b.as_deref().is_some_and(|h| h.contains("/api/launcher/depot/gates/2")), "{b:?}");
    assert!(carries_grant(&b), "same origin, second hop: {b:?}");
    assert!(got.ok, "{}", got.why);

    std::env::remove_var("ELO_GRANT");
    std::env::remove_var("ELO_GRANT_ORIGIN");
}
