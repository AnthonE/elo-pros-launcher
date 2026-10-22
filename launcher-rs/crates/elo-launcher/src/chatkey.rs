//! `elo chat link` and `elo chat unlink` — the wallet signs for its own chat
//! key, so rooms show the wallet's profile name and picture beside it.
//!
//! The flow is `elo_hive::link`, the one the Chat window runs, so a link made
//! here and one made in the window or on elopros.com are the same link to the
//! same key. What lives here is the terminal's half of showing: **every text
//! is printed before the signer is asked.** The local keystore's passphrase
//! prompt is the consent for the first signature; one that would go unasked
//! (the second, on a keystore already unlocked) waits for a `y`, or `--yes`.
//! A paired browser shows each text on its own screen.

use crate::args::Args;
use crate::prompt;
use crate::signer;
use elo_broker::signer::Signer;
use elo_depot::DepotError;
use elo_hive::link::{self, RealLinkHttp, Request};
use elo_hive::who;
use serde_json::Value;
use std::path::Path;

pub fn run(a: &Args, host: &str, voice_dir: &Path, unlink: bool) -> Result<i32, DepotError> {
    let mut signer = signer::choose(a, host).map_err(DepotError::new)?;
    let wallet = signer
        .address()
        .ok_or_else(|| DepotError::new("the signer names no address"))?
        .to_lowercase();
    let yes = a.has("--yes");
    let mut sign = |r: &Request| -> Result<String, String> {
        // stderr: what a human reads before deciding. stdout is the result.
        eprintln!("\nabout to sign, as {}:\n", who::short(&wallet));
        for line in r.text.lines() {
            eprintln!("    {line}");
        }
        eprintln!("\n{}", r.about);
        if !signer.will_ask() && !yes && !prompt::confirm(&format!("{}? [y/N] ", r.button))? {
            return Err("Cancelled — nothing was signed.".into());
        }
        let out = signer.sign(r.text, r.why).map_err(|refused| refused.reason)?;
        out.get("signature")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "the signer returned no signature".to_string())
    };

    if unlink {
        let was = link::unlink(&RealLinkHttp, host, &wallet, &mut sign).map_err(DepotError::new)?;
        println!("unlinked {was}");
        println!("wallet   {wallet}");
        println!("\nRooms stop showing your profile name and picture beside that key. It");
        println!("stays on this machine and still speaks; `elo chat link` links it again.");
        return Ok(0);
    }
    let done = link::link(&RealLinkHttp, host, &wallet, voice_dir, &mut sign)
        .map_err(DepotError::new)?;
    println!("linked  {}", done.npub);
    println!("wallet  {wallet}");
    println!("key     {}", voice_dir.join(elo_hive::voice::KEY_NAME).display());
    if let Some(kept) = &done.kept {
        println!("kept    {}  (the key that was here)", kept.display());
    }
    println!("\nThis machine now speaks with your wallet's own chat key, the one");
    println!("elopros.com makes, and rooms show your profile name and picture.");
    Ok(0)
}
