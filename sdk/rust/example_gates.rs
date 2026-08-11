//! What Gates' desktop build does at startup, end to end.
//!
//! Also the SDK's integration test: `sdk/test_sdk.py` compiles this file
//! against a REAL launcher broker and checks every line of output, so the Rust
//! client and the Python server cannot drift apart while both suites stay
//! green. A protocol whose two halves are only tested separately is a
//! protocol with no test.
//!
//!     rustc --edition 2021 -o gates_demo example_gates.rs && ./gates_demo
//!
//! The shape to copy is the `match` on `SignError`. Three of its four arms are
//! not failures a game should retry, and collapsing them into "signing broke"
//! is the mistake this example exists to prevent.

#[path = "scry_overlay.rs"]
mod scry_overlay;

use scry_overlay::{play_message, Overlay, SignError};

fn main() {
    // 1. Find the launcher. Not finding one is NORMAL — a game plays without
    //    it. What a game must never do is fall back to asking for a key.
    let mut scry = match Overlay::connect("gates", "0.1.0") {
        Ok(ov) => ov,
        Err(why) => {
            println!("no-launcher: {why}");
            println!("playing anyway — offline is a supported state");
            return;
        }
    };
    println!("connected: protocol={} signer={}", scry.protocol(), scry.signer());

    // 2. Who is playing. An address is a CLAIM; verify it with a signature if
    //    it matters to you.
    match scry.address() {
        Some(addr) => println!("address: {addr}"),
        None => println!("address: none — the player has not set one, which is fine"),
    }

    // 3. Where the shards are. The launcher hands back a URL and never the
    //    list: this is our list to serve.
    match scry.servers_url("gates") {
        Some(url) => println!("servers: {url}"),
        None => println!("servers: none published"),
    }

    // 4. Sign a game action. Four outcomes, and three of them are not bugs.
    let msg = play_message("duel", "vow_demo", "ETH up 5", Some("2026-08-04"));
    match scry.sign(&msg, "settling round 41") {
        Ok(sig) => println!("signed: {} via {}", &sig.signature, sig.backend),
        Err(SignError::Handoff(url)) => {
            // NOT an error. The player's signer is their browser; show the
            // link and carry on. Retrying here is an infinite loop.
            println!("handoff: {url}");
        }
        Err(SignError::RefusedByPlayer(why)) => {
            println!("refused-by-player: {why}");
        }
        Err(SignError::Refused(why)) => println!("refused: {why}"),
        Err(SignError::NoLauncher(why)) => println!("lost-launcher: {why}"),
    }

    // 5. What the launcher will not do for us, and should not.
    // ── the overlay: what a HUD polls, drawn by US, approved by nobody here ──
    //
    // A real game would call this each frame and render the result in its own
    // renderer. It may DISPLAY anything it gets; it may never draw its own
    // approval dialog, because a game controls its own pixels and that dialog
    // would be a forgery. Signing goes through `scry.sign` above, where the
    // launcher asks the player in the launcher's own window.
    match scry.overlay("gates") {
        Some(hud) => {
            println!(
                "overlay: host={} signer={} consent-drawn-by={}",
                hud.str_or_empty("host"),
                hud.str_or_empty("signer"),
                hud.get("consent")
                    .and_then(|c| c.get("drawn_by"))
                    .and_then(|d| d.as_str())
                    .unwrap_or("?")
            );
        }
        None => println!("overlay: none — the launcher did not answer"),
    }

    // 6. Prove who is playing, in a form OUR OWN SERVER can check.
    //
    // This is the verb to reach for, not `address()` above — an address is a
    // claim any modified client can make. Our server issues the nonce and
    // gives its own DOMAIN; the launcher writes every word of the message, so
    // no consent prompt fires and a game cannot smuggle a sentence into it.
    //
    // The reply is a SIWE (EIP-4361) message, so our backend verifies it with
    // an off-the-shelf `siwe` library and needs no scry-specific code.
    //
    // ⚠ The nonce must be one WE issued, not seen before, and at least 8
    // alphanumeric characters — EIP-4361's rule. A dashed uuid does not fit;
    // strip the dashes.
    match scry.prove("shard-3.gates.example", "8f14e45fceea167a") {
        Ok(proof) => {
            println!("proof: {} for {:?}", &proof.signature, proof.address);
            // On the server: parse this with `siwe` and verify it against the
            // nonce WE issued and the domain WE serve. ⚠ Do NOT rebuild the
            // string and compare — it carries the launcher's `Issued At`, so
            // checking the fields is the check.
            println!("proof-message: {:?}", proof.message);
        }
        // Every one of these is a normal way to play. A game that refuses to
        // start without a proof has made the launcher required, which it never
        // is.
        Err(SignError::Refused(why)) => println!("no-proof: {why}"),
        Err(e) => println!("no-proof: {e}"),
    }

    // 7. A name and a face for the scoreboard — Steam's GetPlayerSummaries.
    //
    // ⚠ Three states, and collapsing them tells a lie about a person. `None`
    // means we could NOT LOOK; `found == false` means we looked and nobody has
    // sworn on this address. Drawing "anonymous" for the first invents a fact.
    match scry.profile("") {
        Some(p) if p.found => {
            // `label()` is the safe string: a sworn name bare, anything
            // self-declared prefixed with `~`. Never show a typed name in the
            // clothes of a checked one.
            println!("profile: {} (sworn={})", p.label(), p.sworn());
            // Achievements and holdings are separate reads, exactly as Steam
            // splits summaries from achievements. Fetch them with our own HTTP
            // stack — the launcher hands back a path and does not proxy it.
            if let Some(url) = p.link("achievements") {
                println!("achievements-at: {url}");
            }
        }
        Some(_) => println!("profile: nobody has sworn on this address — a normal state"),
        None => println!("profile: could not look — NOT the same as having none"),
    }

    println!("done");
}
