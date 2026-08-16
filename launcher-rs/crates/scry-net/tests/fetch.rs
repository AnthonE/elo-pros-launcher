//! The fetcher's tick counts bytes that LANDED, and lands them all.
//!
//! `Net::fetch_with` is what drives every download meter in the client, so its
//! two properties are pinned where they can fail a build rather than a
//! player's afternoon:
//!
//! * the ticks are the write loop's own count — monotonic, and the last one
//!   is the file's true size, because a meter that runs past its file or
//!   stops short of it is a launcher lying about a download;
//! * a file BIGGER than one chunk ticks more than once, which is the whole
//!   reason `fetch_with` exists — `scry_depot::install`'s progress callback
//!   fires per file, and a game is typically one huge pak.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

/// A tiny HTTP/1.1 server, hand-written for the same reason `told.rs` and
/// `transport.rs` write their own: a web framework in the dev-dependencies of
/// a launcher would grow the tree this workspace's policy keeps small.
struct Server {
    base: String,
    _stop: mpsc::Sender<()>,
}

fn serve_bytes(body: Vec<u8>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if rx.try_recv().is_ok() {
                return;
            }
            let Ok(mut stream) = stream else { continue };
            let _ = drain(&mut stream);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    });
    Server { base: format!("http://127.0.0.1:{port}"), _stop: tx }
}

fn drain(stream: &mut TcpStream) -> Option<()> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    Some(())
}

#[test]
fn the_ticks_climb_to_exactly_the_file_and_a_big_file_ticks_more_than_once() {
    // Bigger than one CHUNK, and deliberately not a multiple of it: the last
    // tick has to be the true size, not the last round number.
    let size = scry_depot::CHUNK * 2 + 12_345;
    let body: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    let server = serve_bytes(body.clone());

    let dir = std::env::temp_dir().join(format!("scry-fetch-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let dest = dir.join("pak0.bin");

    let mut ticks: Vec<u64> = Vec::new();
    scry_net::Net::new()
        .fetch_with(&format!("{}/pak0.bin", server.base), &dest, &mut |sofar| {
            ticks.push(sofar)
        })
        .expect("the fetch must succeed");

    // Monotonic — a meter driven by these can only move forward.
    assert!(ticks.windows(2).all(|w| w[0] < w[1]), "ticks went backwards: {ticks:?}");
    assert_eq!(ticks.last().copied(), Some(size as u64),
               "the last tick is the file's true size");
    assert!(ticks.len() >= 2,
            "a file bigger than one chunk must tick during the download, \
             not only at the end: {ticks:?}");
    // And the bytes on disk are the bytes that were served — the count was
    // measuring a real write, not narrating one.
    let got = std::fs::read(&dest).expect("the file must exist");
    assert_eq!(got, body, "the landed bytes differ from the served ones");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The plain `Fetcher` impl is `fetch_with` with a tick nobody reads — the
/// file still lands whole. One code path is the property this pins: a
/// divergence would mean the meter and the download could disagree.
#[test]
fn the_plain_fetcher_lands_the_same_bytes() {
    let body: Vec<u8> = (0..1000).map(|i| (i % 13) as u8).collect();
    let server = serve_bytes(body.clone());
    let dir = std::env::temp_dir().join(format!("scry-fetch-plain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let dest = dir.join("small.bin");
    scry_depot::Fetcher::fetch(
        &scry_net::Net::new(),
        &format!("{}/small.bin", server.base),
        &dest,
    )
    .expect("the fetch must succeed");
    assert_eq!(std::fs::read(&dest).expect("file"), body);
    let _ = std::fs::remove_dir_all(&dir);
}
