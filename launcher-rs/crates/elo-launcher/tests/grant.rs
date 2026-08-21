//! The grant rides only to the origin it belongs to.
//!
//! `ELO_GRANT` is a bearer token for one game's downloads. The one thing
//! that must never happen is the launcher mailing it to whatever host a
//! manifest names — so this drives the REAL `Net` against two local
//! listeners: one posing as the configured origin (the header must arrive)
//! and one posing as anywhere else (the header must not).
//!
//! One #[test] on purpose: the grant is env-configured and env is process
//! global, so splitting these cases into parallel tests would race.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use elo_net::Net;

/// Serve one HTTP request, return its raw head, answer 200 with `{}`.
fn one_request(listener: TcpListener) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 4096];
        let n = s.read(&mut buf).unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        let body = b"{}";
        let _ = write!(
            s,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let _ = s.write_all(body);
        head
    })
}

#[test]
fn grant_goes_to_the_origin_and_nowhere_else() {
    let origin_l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let origin = format!("http://127.0.0.1:{}", origin_l.local_addr().unwrap().port());
    let other_l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let other = format!("http://127.0.0.1:{}", other_l.local_addr().unwrap().port());

    // Env before Net::new() — the grant is read at construction.
    std::env::set_var("ELO_GRANT", "gates.0xabc.9999999999.deadbeef");
    std::env::set_var("ELO_GRANT_ORIGIN", &origin);
    std::env::set_var("NO_PROXY", "127.0.0.1"); // never tunnel the loopback
    let net = Net::new();

    let h = one_request(origin_l);
    let got = net.get_json(&format!("{origin}/launcher/depot/gates/0.1.0"));
    assert!(got.ok, "the origin answered: {}", got.why);
    let head = h.join().expect("join");
    assert!(
        head.to_lowercase().contains("x-elo-grant: gates.0xabc.9999999999.deadbeef"),
        "the grant header must reach the configured origin; request head was:\n{head}"
    );

    let h = one_request(other_l);
    let got = net.get_json(&format!("{other}/launcher/depot/gates/0.1.0"));
    assert!(got.ok, "the other host answered: {}", got.why);
    let head = h.join().expect("join");
    assert!(
        !head.to_lowercase().contains("x-elo-grant"),
        "the grant is a bearer token and must NOT be mailed to a foreign host; head:\n{head}"
    );

    std::env::remove_var("ELO_GRANT");
    std::env::remove_var("ELO_GRANT_ORIGIN");
}
