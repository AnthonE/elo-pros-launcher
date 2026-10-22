//! A wallet's friends and buddies, and who is playing what.
//!
//!     cargo run --example friends -- 0xYOURWALLET
//!     cargo run --example friends -- 0xYOURWALLET ask 0xTHEIRWALLET
//!
//! The second form signs through the launcher: the SDK rebuilds the text
//! itself and refuses a server that prepares anything else.

use elo_sdk::{Client, Look, Overlay};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(wallet) = args.first() else {
        eprintln!("usage: friends 0xWALLET [op 0xOTHER]");
        std::process::exit(2);
    };
    let elo = Client::default();

    if let (Some(op), Some(other)) = (args.get(1), args.get(2)) {
        let mut ov = match Overlay::connect("friends-example", env!("CARGO_PKG_VERSION")) {
            Ok(ov) => ov,
            Err(why) => {
                eprintln!("a change is signed, and no launcher answered: {why}");
                std::process::exit(1);
            }
        };
        let mut sign = |text: &str| ov.sign(text, "a friends change").map(|s| s.signature);
        match elo.friend(wallet, op, other, &mut sign) {
            Ok(Look::Answer(done)) => println!("{} done", done.op),
            Ok(Look::Refused(r)) => println!("refused: {}", r.error),
            Ok(Look::CouldNotLook(why)) => println!("could not reach the origin: {why}"),
            Err(e) => println!("{e}"),
        }
        return;
    }

    match elo.friends(wallet) {
        Look::Answer(v) => {
            if !v.presence.reachable {
                println!(
                    "(presence could not be read: {})",
                    v.presence.why.unwrap_or_default()
                );
            }
            for f in v.friends.iter().chain(&v.buddies) {
                let tag = if f.is_buddy() { " [buddy]" } else { "" };
                match &f.playing {
                    Some(p) => println!(
                        "{}{tag} — playing {} on {}",
                        f.label(),
                        p.game,
                        p.server_name.as_deref().unwrap_or("?")
                    ),
                    None => println!(
                        "{}{tag} — {}",
                        f.label(),
                        if f.online { "online" } else { "offline" }
                    ),
                }
            }
            println!(
                "{} friends, {} buddies, {} asks waiting",
                v.count,
                v.buddies.len(),
                v.asks_in.len()
            );
        }
        Look::Refused(r) => println!("refused: {}", r.error),
        Look::CouldNotLook(why) => println!("could not look: {why}"),
    }
}
