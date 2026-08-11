//! A real broker on a real socket, with no window — so a test in another
//! language can drive it.
//!
//! `scry-gui` is the launcher a player runs, and it links FLTK, so it cannot be
//! started on a headless box or in CI. That was fine while a second broker
//! existed in Python and `sdk/test_sdk.py` stood *that* one up. The Python
//! client was deleted on 2026-08-07, and with it went the only server the
//! Python SDK client had to talk to.
//!
//! This is that server. It is deliberately an **example and not a binary**:
//! nothing ships it, it is built by the test that needs it, and a player can
//! never accidentally run a broker whose consent answer is a command-line flag.
//!
//! ```text
//! cargo build --example headless -p scry-broker
//! ./target/debug/examples/headless <socket-path> [--refuse-consent] [--conns N]
//! ```
//!
//! ⚠ **The key here is a fixed test secret and the address is derived from it.**
//! That matters more than it looks: `prove` emits a SIWE message that names the
//! address, so a host whose `address()` and signing key disagree issues proofs
//! that cannot verify. Deriving both from one secret makes that impossible to
//! get wrong here, which is exactly the bug the real hosts have a guard for.

use scry_broker::{protocol::Refusal, serve_conn, transport::Listener, Host, What};
use serde_json::{json, Value};

/// The same secret `sdk_parity.rs` uses, so a signature produced here recovers
/// to the address that suite already asserts against.
const SECRET: [u8; 32] = [9u8; 32];

struct Headless {
    /// Whether the "player" approves a `sign` prompt. `prove` ignores this by
    /// construction — that is the property the verb exists for.
    allow: bool,
}

impl Headless {
    fn account() -> scry_vault::Account {
        scry_vault::Account::from_secret(&SECRET).expect("the fixed test secret is valid")
    }
}

impl Host for Headless {
    /// EIP-55 checksummed, from the same key that signs below.
    fn address(&self) -> Option<String> {
        Some(Self::account().address())
    }

    fn signer_name(&self) -> String {
        "local".into()
    }

    fn host_url(&self) -> String {
        "https://scry.test".into()
    }

    fn ask_consent(&mut self, _game: &str, _family: &str, _text: &str, _why: &str) -> Option<u32> {
        // One signature per grant, so a second ask re-prompts — the real
        // launcher's shape, not an "always allow" that would make the counted
        // consent tests meaningless.
        if self.allow {
            Some(1)
        } else {
            None
        }
    }

    fn sign(&mut self, text: &str, _why: &str) -> Result<Value, Refusal> {
        let acct = Self::account();
        Ok(json!({
            "signature": acct.sign_message(text).map_err(Refusal::new)?,
            "address": acct.address(),
            "backend": "local",
        }))
    }

    fn prove(&mut self, message: &str) -> Result<Value, Refusal> {
        let acct = Self::account();
        Ok(json!({
            "signature": acct.sign_message(message).map_err(Refusal::new)?,
            "address": acct.address(),
            "backend": "local",
        }))
    }

    /// A fixed `/api/who` body, so the SDK client's profile path is exercised
    /// without this example reaching the network. **A test server that made a
    /// live HTTP call would fail for reasons that have nothing to do with the
    /// protocol**, which is how a suite starts getting muted.
    fn profile(&mut self, address: &str) -> Result<Value, Refusal> {
        if address.eq_ignore_ascii_case("0x000000000000000000000000000000000000dEaD") {
            // The 404 shape: we looked, nobody has sworn here. A real answer,
            // and NOT the same as being unable to look.
            return Ok(json!({"reachable": true, "profile": Value::Null,
                             "note": "no sworn identity for that address. Playing \
                                      unsworn is a normal state, and this is a real \
                                      answer rather than a failed read."}));
        }
        let who = json!({
            "authoritative": true,
            "matches": 1,
            "found": {
                "agent": "Dummmy", "kind": "unknown", "vow_id": "1a434221c5333fa7",
                "wallet": address, "npub": Value::Null, "nip05": Value::Null,
                "sandbox": false, "avatar": "/vow/1a434221c5333fa7/mark.svg",
                "face": Value::Null,
                "conduct": {"vow": "/vow/1a434221c5333fa7"},
                "reach": {"in_the_hive": false, "tip": "/hive/tip?to=1a434221c5333fa7"},
                "reputation": {"at": format!("/reputation/{address}")},
            },
            "subject": {"wallet": address, "identities": 2},
        });
        Ok(json!({
            "reachable": true,
            "profile": scry_broker::protocol::flatten_who(&who, address),
            "source": "https://scry.test/api/who",
        }))
    }

    fn title_url(&self, slug: &str, what: What) -> Option<String> {
        match what {
            What::Manifest => Some(format!("https://scry.test/api/launcher/manifest/{slug}")),
            // No shard list, which is a real state the SDK must render.
            What::Servers => None,
        }
    }

    fn open(&mut self, _url: &str) -> Result<(), Refusal> {
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(endpoint) = args.get(1) else {
        eprintln!("usage: headless <socket-path> [--refuse-consent] [--conns N]");
        std::process::exit(2);
    };
    let allow = !args.iter().any(|a| a == "--refuse-consent");
    let conns: usize = args
        .iter()
        .position(|a| a == "--conns")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(64);

    let listener = match Listener::bind(endpoint) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("headless: cannot bind {endpoint}: {e}");
            std::process::exit(1);
        }
    };
    // Printed AFTER the bind succeeds, so a caller that waits for this line
    // knows the socket is there. Racing on "the file exists" is how a flaky
    // integration test is born.
    println!("ready {endpoint}");

    for _ in 0..conns {
        match listener.accept() {
            Ok(conn) => {
                let mut host = Headless { allow };
                let _ = serve_conn(conn, &mut host);
            }
            Err(e) => {
                eprintln!("headless: accept failed: {e}");
                break;
            }
        }
    }
}
