//! `elo chat read`, through the real binary, against a stub of the origin.
//!
//! What it pins: a speaker the room ties to a wallet — a linked key, here —
//! wears the wallet's own profile name, as the Chat window and the site show
//! it; a sworn name with no profile stays sworn; a guest keeps its own
//! (self-declared) name; and the house is never looked up, whatever a room
//! says beside it.

use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};

const ELO: &str = env!("CARGO_BIN_EXE_elo");
const ASH: &str = "0x00000000000000000000000000000000000000aa";
const BO: &str = "0x00000000000000000000000000000000000000bb";

/// A hand-written HTTP/1.1 stub, for the reason `origin.rs` gives.
fn serve(routes: Vec<(&'static str, String)>) -> (String, Arc<Mutex<Vec<String>>>, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = seen.clone();
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if rx.try_recv().is_ok() {
                return;
            }
            let Ok(mut stream) = stream else { continue };
            let Some(path) = read_path(&mut stream) else { continue };
            log.lock().unwrap().push(path.clone());
            let (code, body) = routes
                .iter()
                .find(|(p, _)| path.starts_with(p))
                .map(|(_, b)| (200, b.clone()))
                .unwrap_or((404, r#"{"detail":"Not Found"}"#.into()));
            let head = format!(
                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{port}"), seen, tx)
}

fn read_path(stream: &mut TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    line.split_whitespace().nth(1).map(str::to_string)
}

fn npub(c: char) -> String {
    format!("npub1{}", c.to_string().repeat(58))
}

#[test]
fn a_linked_speaker_is_named_by_the_wallets_profile_and_the_house_is_never_looked_up() {
    let room = json!({
        "posted": true, "relay": "wss://hive.example", "room": "g", "scope": "a buzz room",
        "messages": [
            {"id": "1", "at": 100, "kind": 9, "content": "linked, no handle", "npub": npub('a'),
             "who": null, "display": null, "wallet": ASH, "house": false},
            {"id": "2", "at": 200, "kind": 9, "content": "sworn, no profile", "npub": npub('b'),
             "who": "bo_sworn", "display": null, "wallet": BO, "house": false},
            {"id": "3", "at": 300, "kind": 9, "content": "a guest", "npub": npub('c'),
             "who": null, "display": "guest", "house": false},
            {"id": "4", "at": 400, "kind": 9, "content": "news", "npub": npub('d'),
             "who": "the house", "wallet": "0x00000000000000000000000000000000000000dd",
             "house": true},
        ]
    });
    let ash = json!({"found": {"wallet": ASH, "profile": {"name": "Ash"}}, "subject": {"wallet": ASH}});
    let bo = json!({"found": {"wallet": BO, "nip05": "bo_sworn@elopros.com", "npub": npub('b')},
                    "subject": {"wallet": BO}});
    let (base, seen, _stop) = serve(vec![
        ("/api/hive/room", room.to_string()),
        ("/api/who?handle=0x00000000000000000000000000000000000000aa", ash.to_string()),
        ("/api/who?handle=0x00000000000000000000000000000000000000bb", bo.to_string()),
    ]);
    let out = Command::new(ELO)
        .args(["--host", &base, "chat", "read"])
        .env("NO_PROXY", "127.0.0.1")
        .env_remove("HTTPS_PROXY")
        .env_remove("HTTP_PROXY")
        .env_remove("ALL_PROXY")
        .output()
        .expect("run elo");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    let line = |needle: &str| {
        text.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("no {needle:?} in\n{text}"))
            .to_string()
    };
    assert!(line("linked, no handle").contains("~Ash"), "the wallet's profile name, marked declared");
    assert!(line("sworn, no profile").contains(" bo_sworn"), "a sworn name with no profile stays");
    assert!(line("a guest").contains("~guest"));
    assert!(line("news").contains(" the house"));
    let seen = seen.lock().unwrap();
    assert!(!seen.iter().any(|p| p.contains("dddddd") || p.ends_with("dd")),
            "the house is never looked up: {seen:?}");
    assert_eq!(seen.iter().filter(|p| p.starts_with("/api/who")).count(), 2, "{seen:?}");
}
