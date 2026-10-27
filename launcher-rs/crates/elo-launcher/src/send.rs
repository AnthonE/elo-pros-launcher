//! `elo wallet`, `elo send` and `elo move` — what this account holds on both
//! of its chains, sending it, and moving it between them.
//!
//! One account, two chains, the way Phantom shows one: `elo wallet` reads
//! Robinhood Chain and Solana together, `elo send` picks the chain from the
//! address it is given (`0x…` or base58), and `elo move` takes SOL, ETH, USDC
//! or USDG across to this same account's address on the other chain.
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
use elo_net::solana as sol_net;
use elo_net::wallet::{self, Coin};
use elo_net::xmove;
use elo_vault::solana as sol;
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

/// Our Solana coins, named by the origin's pairs; empty when it does not say.
fn ours_on_solana(rpc: &HttpRpc, host: &str) -> Vec<([u8; 32], String)> {
    rpc.net.get_json(&format!("{host}/api/pairs")).value.map(|v| sol_net::ours(&v)).unwrap_or_default()
}

fn sol_rows(snap: &sol_net::Snapshot) -> Vec<(String, Result<String, String>, String)> {
    let mut rows = vec![("SOL".to_string(), snap.sol.as_ref().map(|l| wallet::format_amount(*l as u128, 9))
                         .map_err(String::clone), "pays every fee on Solana".to_string())];
    match &snap.coins {
        Ok(cs) => rows.extend(cs.iter().map(|(c, amt)| (c.sym.clone(),
            Ok(wallet::format_amount(*amt as u128, c.decimals)), c.what.clone()))),
        Err(e) => rows.push(("coins".into(), Err(e.clone()), String::new())),
    }
    rows
}

fn print_solana(addr: &str, snap: &sol_net::Snapshot) {
    println!();
    println!("{}", elo_depot::streamer::shown(addr));
    println!("  on {} · {}", sol::NAME, sol_net::address_url(addr));
    println!();
    for (sym, amt, what) in sol_rows(snap) {
        match amt {
            Ok(x) => println!("  {:>28} {:<6}  {what}", x, sym),
            Err(e) => println!("  {:>28} {:<6}  could not read: {e}", "?", sym),
        }
    }
}

pub fn show(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    // `--wallet` takes either chain's address; a Solana one shows Solana only.
    if let Some(w) = a.flag("--wallet").filter(|w| !w.trim().starts_with("0x")) {
        let me = sol::parse_address(w).map_err(DepotError::new)?;
        let rpc = HttpRpc::new();
        let snap = sol_net::snapshot(&sol_net::http(), &me, &ours_on_solana(&rpc, host));
        let addr = sol::base58_encode(&me);
        if json_out {
            println!("{}", serde_json::to_string_pretty(&json!({"solana": sol_json(&addr, &snap)})).unwrap_or_default());
        } else {
            print_solana(&addr, &snap);
        }
        return Ok(if snap.unreachable() { 1 } else { 0 });
    }
    let sol_addr = a.flag("--wallet").is_none().then(|| account().ok()).flatten()
        .and_then(|(s, _)| s.sol_address());
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
        let solana = sol_addr.as_ref().map(|addr| {
            let me = sol::parse_address(addr).expect("the keystore names a valid address");
            sol_json(addr, &sol_net::snapshot(&sol_net::http(), &me, &ours_on_solana(&rpc, host)))
        });
        println!("{}", serde_json::to_string_pretty(&json!({
            "address": addr, "chain_id": elo_vault::tx::CHAIN_ID, "coins": coins,
            "note": snap.note, "warning": snap.warning, "solana": solana,
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
    match &sol_addr {
        Some(addr) => {
            let me = sol::parse_address(addr).map_err(DepotError::new)?;
            print_solana(addr, &sol_net::snapshot(&sol_net::http(), &me, &ours_on_solana(&rpc, host)));
        }
        None if a.flag("--wallet").is_none() => {
            println!("\n  no Solana address yet — `elo account solana` adds the one your recovery words make");
        }
        None => {}
    }
    Ok(if snap.unreachable() { 1 } else { 0 })
}

fn sol_json(addr: &str, snap: &sol_net::Snapshot) -> Value {
    let coins: Vec<Value> = std::iter::once(json!({
        "sym": "SOL", "mint": null, "decimals": 9,
        "raw": snap.sol.as_ref().ok().map(u64::to_string),
        "amount": snap.sol.as_ref().ok().map(|l| wallet::format_amount(*l as u128, 9)),
        "error": snap.sol.as_ref().err(),
    })).chain(snap.coins.as_ref().map(|cs| cs.iter().map(|(c, amt)| json!({
        "sym": c.sym, "mint": c.mint.map(|m| sol::base58_encode(&m)), "decimals": c.decimals,
        "raw": amt.to_string(), "amount": wallet::format_amount(*amt as u128, c.decimals), "what": c.what,
    })).collect::<Vec<_>>()).unwrap_or_default()).collect();
    json!({"address": addr, "chain": "solana", "coins": coins,
           "error": snap.coins.as_ref().err()})
}

/// `elo send <amount> <coin> <to>`.
pub fn send(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let usage = "usage: elo send <amount> <coin> <to> — e.g. elo send 12.5 JUNK 0x…, \
                 or elo send 0.5 SOL <a Solana address>";
    let (Some(amount), Some(which), Some(to)) =
        (a.positional.first(), a.positional.get(1), a.positional.get(2)) else {
        return Err(DepotError::new(usage));
    };
    // The address decides the chain, as in any two-chain wallet.
    if !to.trim().starts_with("0x") && !to.trim().starts_with("0X") {
        return send_solana(host, amount, which, to, json_out);
    }
    if ["SOL", "USDC"].iter().any(|c| which.eq_ignore_ascii_case(c)) {
        return Err(DepotError::new(format!(
            "{which} lives on Solana, and that is a Robinhood Chain address. To bring it across to \
             your own account there, `elo move {amount} {which}`.")));
    }
    let (signer, from) = account()?;
    let rpc = HttpRpc::new();
    // ETH needs no list. Anything else does, and a list we could not read is
    // "could not look", never "not a coin we list".
    let (coins, unread) = match listed(&rpc, host) {
        Ok(v) => (wallet::book(&v, &rpc), None),
        Err(why) => (vec![Coin::eth()], Some(why)),
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
        [] => return Err(DepotError::new(match unread {
            Some(why) => format!("could not read the coin list from {host}, so {which} could not \
                                  be looked up — {why}"),
            None => format!("{which} is not a coin this launcher lists — `elo wallet` shows the \
                             ones it does"),
        })),
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

/// `elo send` to a Solana address: SOL or a coin this account holds there.
fn send_solana(host: &str, amount: &str, which: &str, to: &str, json_out: bool) -> Result<i32, DepotError> {
    let (signer, _) = account()?;
    let me_s = signer.sol_address().ok_or_else(|| DepotError::new(
        "that is a Solana address, and this account has no Solana address yet — \
         `elo account solana` adds the one your recovery words make"))?;
    let me = sol::parse_address(&me_s).map_err(DepotError::new)?;
    let rpc = sol_net::http();
    let coin = if which.eq_ignore_ascii_case("SOL") {
        sol_net::sol_coin(&me)
    } else {
        let origin = HttpRpc::new();
        let snap = sol_net::snapshot(&rpc, &me, &ours_on_solana(&origin, host));
        let coins = snap.coins.map_err(|e| DepotError::new(format!("could not read your Solana coins: {e}")))?;
        let mint = sol::parse_address(which).ok();
        let hits: Vec<_> = coins.into_iter().filter(|(c, _)| match mint {
            Some(m) => c.mint == Some(m),
            None => c.sym.eq_ignore_ascii_case(which),
        }).collect();
        match hits.as_slice() {
            [(c, _)] => c.clone(),
            [] => return Err(DepotError::new(format!(
                "this account holds no {which} on Solana — `elo wallet` shows what it does"))),
            _ => return Err(DepotError::new(format!(
                "more than one coin here is called {which} — name it by its mint address"))),
        }
    };
    let known_path = elo_vault::recipients_path();
    let plan = sol_net::plan(&rpc, &me, &coin, to, amount, &sol_net::known_recipients(&known_path))
        .map_err(DepotError::new)?;
    let lines = sol_net::review(&plan).map_err(DepotError::new)?;
    eprintln!();
    for (k, v) in &lines {
        eprintln!("  {k:<8} {v}");
    }
    for n in &plan.notes {
        eprintln!("\n  {n}");
    }
    eprintln!();
    let pass = prompt::secret("passphrase to send (Ctrl-C to stop): ").map_err(DepotError::new)?;
    let mut tx = plan.tx.clone();
    signer.sign_solana_transaction(&pass, &mut tx).map_err(DepotError::new)?;
    drop(pass);
    let sig = sol_net::send(&rpc, &tx).map_err(|e| DepotError::new(format!(
        "Solana did not take it, so nothing was sent: {e}")))?;
    let _ = sol_net::remember_recipient(&known_path, &plan.to);
    if json_out {
        println!("{}", serde_json::to_string_pretty(&json!({"signature": sig, "url": sol_net::tx_url(&sig)}))
            .unwrap_or_default());
    } else {
        println!("{sig}");
        println!("  sent — it lands in a few seconds: {}", sol_net::tx_url(&sig));
    }
    Ok(0)
}

/// `elo move <amount> <SOL|ETH|USDC|USDG>` — to this account's own address on
/// the other chain, quoted, shown, then the passphrase.
pub fn move_coins(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let usage = "usage: elo move <amount> <coin> — SOL or USDC go to Robinhood Chain (as ETH or USDG), \
                 ETH or USDG go to Solana (as SOL or USDC). Always to your own address there.";
    let (Some(amount), Some(which)) = (a.positional.first(), a.positional.get(1)) else {
        return Err(DepotError::new(usage));
    };
    let r = xmove::route_for(which).ok_or_else(|| DepotError::new(usage))?;
    let (signer, evm) = account()?;
    let sol_s = signer.sol_address().ok_or_else(|| DepotError::new(
        "a move needs this account's Solana address — `elo account solana` adds it"))?;
    let me_sol = sol::parse_address(&sol_s).map_err(DepotError::new)?;
    let evm_s = elo_vault::to_checksum(&evm);
    let pct: f64 = a.flag("--slippage").unwrap_or("1").parse()
        .map_err(|_| DepotError::new("--slippage is a percent, like 1 or 0.5"))?;
    if !(0.1..=5.0).contains(&pct) {
        return Err(DepotError::new("--slippage for a move is between 0.1 and 5 percent"));
    }
    let raw = wallet::parse_amount(amount, r.decimals).map_err(DepotError::new)?;
    if raw == 0 {
        return Err(DepotError::new("type an amount above zero"));
    }
    let origin = HttpRpc::new();
    eprintln!("  {} — asking for a route…", r.label());
    let q = xmove::quote(&origin, host, &r, amount, &sol_s, &evm_s, (pct * 100.0).round() as u32)
        .map_err(|e| DepotError::new(format!("no route right now: {e}")))?;
    let lines = xmove::review(&r, &q, &sol_s, &evm_s);

    // The source chain's transaction is checked before the review is shown.
    let sol_leg = if r.from_solana() {
        let raw = u64::try_from(raw).map_err(|_| DepotError::new("that amount is larger than any balance"))?;
        Some(xmove::check_solana(&sol_net::http(), &q, &r, &me_sol, raw).map_err(DepotError::new)?)
    } else {
        None
    };
    let steps = if r.from_solana() { None } else { Some(xmove::evm_steps(&q, &r, raw).map_err(DepotError::new)?) };

    eprintln!();
    for (k, v) in &lines {
        eprintln!("  {k:<8} {v}");
    }
    if let Some(leg) = &sol_leg {
        eprintln!("\n  the chain's own dry run: {} SOL leaves your Solana account, fees included",
                  wallet::format_amount(leg.spent as u128, 9));
    }
    eprintln!("\n  A move cannot be undone. ELO itself never moves between chains, and no rate between them is implied.");
    eprintln!();
    let pass = prompt::secret("passphrase to move (Ctrl-C to stop): ").map_err(DepotError::new)?;

    let sent = if let Some(leg) = sol_leg {
        let mut tx = leg.tx;
        signer.sign_solana_transaction(&pass, &mut tx).map_err(DepotError::new)?;
        drop(pass);
        let rpc = sol_net::http();
        let sig = sol_net::send(&rpc, &tx).map_err(|e| DepotError::new(format!(
            "Solana did not take it, so nothing was moved: {e}")))?;
        eprintln!("  sent on Solana: {}", sol_net::tx_url(&sig));
        sig
    } else {
        let steps = steps.expect("an EVM move has steps");
        let rpc = HttpRpc::new();
        let settings = elo_net::guard::Settings::load(&elo_net::guard::settings_path());
        let sign = |tx: &elo_vault::tx::Tx| signer.sign_transaction(&pass, tx);
        let hash = elo_net::trading::execute(&steps, &rpc, &evm, &settings, &elo_net::guard::activity_path(),
                                             "elo move", &sign, &|l| eprintln!("{l}"))
            .map_err(DepotError::new)?;
        drop(pass);
        hash
    };
    let done = xmove::follow(&origin, host, &r, &sent, &|l| eprintln!("{l}"));
    if json_out {
        println!("{}", serde_json::to_string_pretty(&json!({
            "sent": sent, "from_chain": r.from_chain, "to_chain": r.to_chain,
            "status": done.as_ref().ok(), "error": done.as_ref().err(),
        })).unwrap_or_default());
    } else {
        match &done {
            Ok(s) => println!("arrived{}", s.get("receiving_link").and_then(Value::as_str)
                .map(|l| format!(": {l}")).unwrap_or_default()),
            Err(e) => println!("{e}"),
        }
    }
    Ok(if done.is_ok() { 0 } else { 1 })
}
