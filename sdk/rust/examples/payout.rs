//! Write a closed wipe's payout file the way the meter reads it.
//!
//!     cargo run --example payout -- /tmp
//!
//! A game server writes this when a wipe closes and serves it at
//! `/payout/<wipe>.json` beside `status.json`.

use elo_sdk::{PayoutFile, PayoutRow};

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let file = PayoutFile::new(3)
        .pay(
            PayoutRow::new("0x2222222222222222222222222222222222222222", 120, "JUNK")
                .because("EXTRACTED 120"),
        )
        .pay(
            PayoutRow::new("0x3333333333333333333333333333333333333333", 40, "JUNK")
                .because("EXTRACTED 40"),
        );
    let problems = file.validate();
    if !problems.is_empty() {
        eprintln!("the meter would refuse this file: {}", problems.join("; "));
        std::process::exit(1);
    }
    let path = std::path::Path::new(&dir).join(file.file_name("island"));
    match file.write_atomic(&path) {
        Ok(()) => println!("wrote {}", path.display()),
        Err(e) => {
            eprintln!("could not write {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}
