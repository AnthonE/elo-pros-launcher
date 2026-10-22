//! The MCP face, driven the way a client drives it.
//!
//! The properties worth pinning are protocol-shaped, not feature-shaped: a
//! notification must get **no** reply, a tool failure must come back as a
//! *result* the model can read rather than a transport error it cannot, and
//! nothing here may be able to write.

// Shared with the binary, so the test target legitimately does not
// exercise every item in it.
#[allow(dead_code)]
#[path = "../src/args.rs"]
mod args;
// Shared with the binary, so the test target legitimately does not
// exercise every item in it.
#[allow(dead_code)]
#[path = "../src/mcp.rs"]
mod mcp;

use serde_json::{json, Value};

fn server(games: &std::path::Path, state: &std::path::Path) -> mcp::Server {
    mcp::Server::new(games, state, "https://example.invalid", "linux-x86_64")
}

/// Drive one request through the same entry point `serve` uses.
fn ask(s: &mcp::Server, msg: Value) -> Option<Value> {
    s.handle_line_for_test(&msg.to_string())
        .map(|r| serde_json::from_str(&r).expect("replies are JSON"))
}

#[test]
fn initialize_answers_with_a_protocol_and_a_name() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), tmp.path());
    let r = ask(
        &s,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize",
               "params":{"protocolVersion":"2024-11-05"}}),
    )
    .expect("initialize gets a reply");
    assert_eq!(r["id"], 1);
    // A version the client asked for and we know is echoed rather than
    // overridden — a client pinned to an older revision still works.
    assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(r["result"]["serverInfo"]["name"], "elo-launcher");

    // …and one we do not know falls back to ours rather than being repeated.
    let r = ask(
        &s,
        json!({"jsonrpc":"2.0","id":2,"method":"initialize",
               "params":{"protocolVersion":"1999-01-01"}}),
    )
    .unwrap();
    assert_eq!(r["result"]["protocolVersion"], mcp::PROTOCOL);
}

#[test]
fn a_notification_gets_no_reply_at_all() {
    // Answering a notification is a protocol error some clients treat as
    // fatal, and it is the easiest thing in a hand-written server to get wrong.
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), tmp.path());
    assert!(ask(&s, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
    assert!(ask(&s, json!({"jsonrpc":"2.0","method":"anything/at/all"})).is_none());
}

#[test]
fn there_is_exactly_one_tool_and_it_is_read_only() {
    // A tool costs context in every session whether it is called or not.
    // One dispatcher, not five flat tools.
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), tmp.path());
    let r = ask(&s, json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1, "one tool: {tools:?}");
    assert_eq!(tools[0]["name"], "launcher");
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);

    // The advertised enum is the whole surface, and none of it writes.
    let whats: Vec<&str> = tools[0]["inputSchema"]["properties"]["what"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for forbidden in ["install", "play", "uninstall", "say", "prune", "update",
                      "ask", "accept", "remove", "block"] {
        assert!(!whats.contains(&forbidden), "{forbidden} must not be reachable over MCP");
    }
    // The friends READ is, with the one argument it takes.
    assert!(whats.contains(&"friends"), "{whats:?}");
    assert_eq!(tools[0]["inputSchema"]["properties"]["wallet"]["type"], "string");
}

#[test]
fn an_empty_library_reads_as_a_real_count_not_a_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), tmp.path());
    let r = ask(
        &s,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"launcher","arguments":{"what":"library"}}}),
    )
    .unwrap();
    assert_eq!(r["result"]["isError"], false);
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Nothing is installed"), "{text}");
    // The distinction the whole client turns on, said to the model too.
    assert!(text.contains("not a failed lookup"), "{text}");
}

#[test]
fn a_tool_failure_is_a_result_the_model_can_read() {
    // Not a JSON-RPC error: a transport error is invisible to the model, and
    // a model that cannot see the failure cannot act on it.
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), tmp.path());
    for bad in [
        json!({"name":"launcher","arguments":{"what":"launch-a-missile"}}),
        json!({"name":"launcher","arguments":{"what":"status"}}),
        json!({"name":"some-other-tool","arguments":{}}),
    ] {
        let r = ask(&s, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":bad}))
            .unwrap();
        assert!(r["error"].is_null(), "should not be a transport error: {r}");
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert!(!r["result"]["content"][0]["text"].as_str().unwrap().is_empty());
    }
}

#[test]
fn an_unknown_method_is_a_transport_error() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), tmp.path());
    let r = ask(&s, json!({"jsonrpc":"2.0","id":9,"method":"resources/list"})).unwrap();
    assert_eq!(r["error"]["code"], -32601);
    assert_eq!(r["id"], 9);
}

#[test]
fn malformed_json_does_not_take_the_server_down() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), tmp.path());
    let r: Value =
        serde_json::from_str(&s.handle_line_for_test("{not json at all").unwrap()).unwrap();
    assert_eq!(r["error"]["code"], -32700);
    // …and the next line still works.
    assert!(ask(&s, json!({"jsonrpc":"2.0","id":1,"method":"ping"})).is_some());
}

#[test]
fn sessions_never_folds_an_unwatched_launch_into_playtime() {
    use elo_depot::session::{self, Ending, Session};
    let games = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    session::append(
        state.path(),
        Session {
            slug: "gates".into(),
            build: "1".into(),
            started_at: 0,
            ending: Ending::Measured { ended_at: 3600, seconds: 3600, code: Some(0) },
        },
    )
    .unwrap();
    session::append(
        state.path(),
        Session {
            slug: "gates".into(),
            build: "1".into(),
            started_at: 0,
            ending: Ending::Detached { pid: 4242 },
        },
    )
    .unwrap();

    let s = server(games.path(), state.path());
    let r = ask(
        &s,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"launcher","arguments":{"what":"sessions"}}}),
    )
    .unwrap();
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("1h 0m measured"), "{text}");
    assert!(text.contains("1 launches not measured"), "{text}");
    assert!(text.contains("excluded from playtime rather than guessed"), "{text}");
}

const ME: &str = "0x1111111111111111111111111111111111111111";

/// `GET /api/friends/{wallet}` answered from the fixture the meter's suite
/// writes against, once per connection, on loopback (which no proxy carries).
/// Hands back the base url and the paths it was asked for.
fn friends_origin() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, BufReader, Write};
    let body = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../sdk/testdata/friends_view.json"),
    )
    .expect("the fixture");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
            }
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            log.lock().unwrap().push(path);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
    (base, seen)
}

fn call(s: &mcp::Server, arguments: Value) -> (bool, String) {
    let r = ask(
        s,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"launcher","arguments":arguments}}),
    )
    .unwrap();
    assert!(r["error"].is_null(), "a tool failure is a result: {r}");
    (
        r["result"]["isError"].as_bool().unwrap(),
        r["result"]["content"][0]["text"].as_str().unwrap().to_string(),
    )
}

#[test]
fn friends_reads_the_list_the_cli_prints_buddies_included() {
    let tmp = tempfile::tempdir().unwrap();
    let (base, seen) = friends_origin();
    let s = mcp::Server::new(tmp.path(), tmp.path(), &base, "linux-x86_64")
        .with_whose(Box::new(|| Ok(ME.to_string())));

    // No `wallet`: the machine's own, as `elo friends` reads it.
    let (err, text) = call(&s, json!({"what": "friends"}));
    assert!(!err, "{text}");
    assert_eq!(seen.lock().unwrap().as_slice(), [format!("/api/friends/{ME}")]);
    let doc: Value = serde_json::from_str(text.split("\n\n").next().unwrap()).unwrap();
    assert_eq!(doc["wallet"], ME);
    assert_eq!(doc["count"], 3);
    assert_eq!(doc["friends"][0]["presence"], "playing Gates · Gates US East 1");
    assert_eq!(doc["buddies"][0]["name"], "Pip");
    assert_eq!(doc["buddies"][0]["buddy"]["id"], "pip");
    assert_eq!(doc["asks_in"][0]["name"], "rook");
    assert!(text.contains("UNTRUSTED"), "names are strangers' text: {text}");

    // A named wallet wins over the default, and is read as typed.
    let other = "0x2222222222222222222222222222222222222222";
    let (err, _) = call(&s, json!({"what": "friends", "wallet": other}));
    assert!(!err);
    assert_eq!(seen.lock().unwrap().last().unwrap(), &format!("/api/friends/{other}"));
}

#[test]
fn friends_refuses_what_is_not_a_wallet_and_says_whose_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // Nothing is listening here; neither call may get as far as asking.
    let s = mcp::Server::new(tmp.path(), tmp.path(), "http://127.0.0.1:9", "linux-x86_64")
        .with_whose(Box::new(|| Err("whose friends? — no wallet on this machine".into())));
    let (err, text) = call(&s, json!({"what": "friends", "wallet": "ash"}));
    assert!(err && text.contains("not a wallet"), "{text}");
    let (err, text) = call(&s, json!({"what": "friends"}));
    assert!(err && text.contains("whose friends?"), "{text}");
    // An origin nobody reached is not an empty list.
    let (err, text) = call(&s, json!({"what": "friends", "wallet": ME}));
    assert!(err && text.contains("NOT 'no friends'"), "{text}");
}
