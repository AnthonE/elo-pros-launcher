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
    assert_eq!(r["result"]["serverInfo"]["name"], "scry-launcher");

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
    // `COMPANION.md` §4a: a tool costs context in every session whether it is
    // called or not. One dispatcher, not five flat tools.
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
    for forbidden in ["install", "play", "uninstall", "say", "prune", "update"] {
        assert!(!whats.contains(&forbidden), "{forbidden} must not be reachable over MCP");
    }
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
    use scry_depot::session::{self, Ending, Session};
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
