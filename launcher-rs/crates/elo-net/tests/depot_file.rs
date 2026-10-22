//! A depot file is read to its declared size and not a byte past it.
//!
//! `fetch_sized` used the declared size only for a deadline and `io::copy`'d
//! the body uncapped, so a file the depot called 4 bytes landed at whatever the
//! host streamed until the deadline — and the hash check that refuses it runs
//! only after the whole stream is on the player's disk.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use elo_depot::Fetcher;
use elo_net::Net;

fn host(sent: usize) -> (String, thread::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/files/gates", l.local_addr().unwrap().port());
    let t = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = s.read(&mut buf);
        let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-length: {sent}\r\nconnection: close\r\n\r\n");
        let chunk = vec![b'A'; 1 << 20];
        let mut left = sent;
        while left > 0 {
            let n = left.min(chunk.len());
            if s.write_all(&chunk[..n]).is_err() {
                break;
            }
            left -= n;
        }
    });
    (url, t)
}

fn dest(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("elo-depot-file-{tag}-{}", std::process::id()))
}

#[test]
fn a_file_longer_than_declared_is_refused_without_landing() {
    std::env::set_var("NO_PROXY", "127.0.0.1");
    let net = Net::new();
    const SENT: usize = 64 * 1024 * 1024;
    let (url, t) = host(SENT);
    let d = dest("long");
    let r = net.fetch_sized(&url, &d, 4);
    let on_disk = std::fs::metadata(&d).map(|m| m.len()).unwrap_or(0);
    let _ = std::fs::remove_file(&d);
    t.join().unwrap();
    assert!(r.is_err(), "a 4-byte file that kept coming must be refused");
    assert!(on_disk <= 5, "no more than the declared size (+1 to notice) lands: {on_disk}");
    assert!(format!("{}", r.unwrap_err()).contains("more than"), "the refusal says why");
}

#[test]
fn a_file_shorter_than_declared_is_refused() {
    let net = Net::new();
    let (url, t) = host(4);
    let d = dest("short");
    let r = net.fetch_sized(&url, &d, 10);
    let _ = std::fs::remove_file(&d);
    t.join().unwrap();
    assert!(r.is_err(), "a 10-byte file that stopped at 4 is not the file");
}

#[test]
fn a_file_of_exactly_its_declared_size_lands() {
    let net = Net::new();
    let (url, t) = host(1 << 20);
    let d = dest("exact");
    let r = net.fetch_sized(&url, &d, 1 << 20);
    let on_disk = std::fs::metadata(&d).map(|m| m.len()).unwrap_or(0);
    let _ = std::fs::remove_file(&d);
    t.join().unwrap();
    assert!(r.is_ok(), "{:?}", r.err().map(|e| e.to_string()));
    assert_eq!(on_disk, 1 << 20);
}
