//! `elo wallet` and `elo send` — what this account holds, and moving it.
//!
//! Balances are read from the chain, not the origin; the origin only names
//! which coins to look at (`elo_net::wallet::book`). A send is planned and
//! checked with no key, shown, and only then asks for the passphrase — the
//! passphrase IS the confirmation, typed after the review is on screen, and
//! it is asked for every send whether or not anything is unlocked.

use crate::args::Args;
use crate::prompt;
use elo_broker::signer::Signer;
use elo_depot::DepotError;
use elo_net::chain::{self, HttpRpc};
use elo_net::wallet::{self, Coin};
use serde_json::{json, Value};

/// The origin's coin list: which coins exist here. Their numbers come from
/// the chain.
fn listed(rpc: &HttpRpc, host: &str) -> Result<Value, String> {
    let got = rpc.net.get_json(&format!("{host}/api/holdings"));
    got.value.ok_or(got.why)
}

fn account() -> Result<(elo_vault::LocalSigner, [u8; 20]), DepotError> {
    let local = elo_vault::LocalSigner::at(elo_vault::keystore_path());
    let addr = local.address().ok_or_else(|| DepotError::new(
        "no account on this machine — `elo account new` makes one. A browser wallet \
         sends from the browser."))?;
    let raw = elo_vault::parse_address(&addr).map_err(DepotError::new)?;
    Ok((local, raw))
}

pub fn show(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let who = match a.flag("--wallet") {
        Some(w) => elo_vault::parse_address(w).map_err(DepotError::new)?,
        None => account()?.1,
    };
    let rpc = HttpRpc::new();
    let list = listed(&rpc, host);
    let snap = wallet::snapshot(&rpc, list.as_ref().map_err(String::as_str), &who);
    let rows = &snap.coins;
    let addr = elo_vault::to_checksum(&who);

    if json_out {
        let coins: Vec<Value> = rows.iter().map(|(c, h)| json!({
            "sym": c.sym,
            "token": c.token.map(|t| elo_vault::to_checksum(&t)),
            "decimals": c.decimals,
            "raw": h.as_ref().ok().map(u128::to_string),
            "amount": h.as_ref().ok().map(|r| wallet::format_amount(*r, c.decimals)),
            "error": h.as_ref().err(),
        })).collect();
        println!("{}", serde_json::to_string_pretty(&json!({
            "address": addr, "chain_id": elo_vault::tx::CHAIN_ID, "coins": coins,
            "note": snap.note, "warning": snap.warning,
        })).unwrap_or_default());
        return Ok(if snap.unreachable() { 1 } else { 0 });
    }

    println!("{}", elo_depot::streamer::shown(&addr));
    println!("  on {} · {}", chain::NAME, chain::address_url(&addr));
    println!();
    for (c, h) in rows {
        match h {
            Ok(r) => println!("  {:>28} {:<6}  {}", wallet::format_amount(*r, c.decimals), c.sym, c.what),
            Err(e) => println!("  {:>28} {:<6}  could not read: {e}", "?", c.sym),
        }
    }
    if let Some(n) = &snap.note {
        println!("\n  {n}");
    }
    if let Some(w) = &snap.warning {
        println!("\n  {w}");
    }
    Ok(if snap.unreachable() { 1 } else { 0 })
}

/// `elo send <amount> <coin> <to>`.
pub fn send(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let usage = "usage: elo send <amount> <coin> <to> — e.g. elo send 12.5 JUNK 0x…";
    let (Some(amount), Some(which), Some(to)) =
        (a.positional.first(), a.positional.get(1), a.positional.get(2)) else {
        return Err(DepotError::new(usage));
    };
    let (signer, from) = account()?;
    let rpc = HttpRpc::new();
    let coins = match listed(&rpc, host) {
        Ok(v) => wallet::book(&v, &rpc),
        Err(_) => vec![Coin::eth()],
    };
    let wanted = elo_vault::parse_address(which).ok();
    let matches: Vec<&Coin> = coins.iter()
        .filter(|c| match wanted {
            Some(t) => c.token == Some(t),
            None => c.sym.eq_ignore_ascii_case(which),
        })
        .collect();
    let coin = match matches.as_slice() {
        [c] => *c,
        [] => return Err(DepotError::new(format!(
            "{which} is not a coin this launcher lists — `elo wallet` shows the ones it does"))),
        _ => return Err(DepotError::new(format!(
            "more than one coin is called {which} — name it by its contract address"))),
    };

    let known = wallet::known_recipients(&elo_vault::recipients_path());
    let plan = wallet::plan(&rpc, &from, coin, to, amount, &known).map_err(DepotError::new)?;
    let lines = wallet::review(&plan).map_err(DepotError::new)?;

    eprintln!();
    for (k, v) in &lines {
        eprintln!("  {k:<8} {v}");
    }
    for n in &plan.notes {
        eprintln!("\n  {n}");
    }
    eprintln!();
    let pass = prompt::secret("passphrase to send (Ctrl-C to stop): ")
        .map_err(DepotError::new)?;
    let signed = signer.sign_transaction(&pass, &plan.tx).map_err(DepotError::new)?;
    drop(pass);
    let hash = chain::send_raw(&rpc, &signed.raw_hex()).map_err(|e| DepotError::new(format!(
        "the chain did not take it, so nothing was sent: {e}")))?;
    if !hash.eq_ignore_ascii_case(&signed.hash_hex()) {
        eprintln!("  ⚠ the chain named hash {hash}; this launcher computed {}", signed.hash_hex());
    }
    let _ = wallet::remember_recipient(&elo_vault::recipients_path(), &plan.to);

    if json_out {
        println!("{}", serde_json::to_string_pretty(&json!({
            "hash": signed.hash_hex(), "url": chain::tx_url(&signed.hash_hex()),
        })).unwrap_or_default());
    } else {
        println!("{}", signed.hash_hex());
        println!("  sent — it lands in a few seconds: {}", chain::tx_url(&signed.hash_hex()));
    }
    Ok(0)
}
