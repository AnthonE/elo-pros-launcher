//! A shard's status poll goes to the shard and nowhere else.
//!
//! l-net-client-ssrf#1: `shardlist::Shard::status_url_to_poll` holds the url
//! to the row's own host, and a redirect is the way round that — the shard
//! answers `302 Location: http://169.254.169.254/…` and the launcher follows.
//! A status endpoint answers for itself, so its poll follows no redirect.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use elo_net::Net;

/// Accept one connection (or give up after a few seconds), answer it with
/// `reply`, and hand back the request head it sent. `None` is "nobody came".
fn serve_once(l: TcpListener, reply: String) -> thread::JoinHandle<Option<String>> {
    l.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(3);
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
        Some(String::from_utf8_lossy(&buf[..n]).to_string())
    })
}

#[test]
fn a_status_poll_follows_no_redirect() {
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    let shard = TcpListener::bind("127.0.0.1:0").unwrap();
    let inside = TcpListener::bind("127.0.0.1:0").unwrap();
    let shard_url = format!("http://127.0.0.1:{}/status.json", shard.local_addr().unwrap().port());
    let inside_url = format!("http://127.0.0.1:{}/latest/meta-data/", inside.local_addr().unwrap().port());

    let s = serve_once(shard, format!(
        "HTTP/1.1 302 Found\r\nlocation: {inside_url}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    ));
    let body = r#"{"players":1,"max_players":2}"#;
    let i = serve_once(inside, format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{body}", body.len()
    ));
    let got = Net::new().status(&shard_url, "http://127.0.0.1:3600/api/launcher/servers/gates");
    assert!(s.join().unwrap().is_some(), "the shard itself is asked");
    assert!(i.join().unwrap().is_none(), "the redirect target was fetched");
    assert!(got.value.is_none(), "a redirected status is no count: {:?}", got.value);
}
