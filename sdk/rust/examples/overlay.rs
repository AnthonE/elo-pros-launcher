//! Who is playing, and a signature, from the launcher on this machine.
//!
//!     cargo run --example overlay
//!
//! No launcher is a normal state: the game plays anonymously, and it never
//! asks the player for a key instead.

use elo_sdk::{play_message, Overlay, SignError};

fn main() {
    let mut elo = match Overlay::connect("gates", env!("CARGO_PKG_VERSION")) {
        Ok(ov) => ov,
        Err(why) => {
            println!("no elo launcher ({why}) — playing anonymously");
            return;
        }
    };
    let Some(wallet) = elo.address() else {
        println!("the launcher has no account — playing anonymously");
        return;
    };
    println!("playing as {wallet} (signer: {})", elo.signer());
    let text = play_message("duel", &wallet, "round 41: I won", None);
    match elo.sign(&text, "settling round 41") {
        Ok(sig) => println!("signed: {}", sig.signature),
        Err(SignError::Handoff(url)) => println!("finish in your browser: {url}"),
        Err(e) => println!("not signed: {e}"),
    }
}
