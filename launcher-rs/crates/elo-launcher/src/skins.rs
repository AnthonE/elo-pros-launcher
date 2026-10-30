//! `elo skins` — sell, list, cancel and buy our items on OpenSea from this
//! machine (`docs/store/LAUNCH-TERMINAL.md` §13, `docs/client/LAUNCHER.md` §10u).
//!
//! The origin prepares every step (`/api/collections/*`); `elo_net::skins`
//! checks a listing against what was typed, and the vault signs it as a
//! Seaport 1.6 order — one of the two kinds of typed data this client makes. Every
//! transaction (the one-time approval, a cancel, a buy, accepting an offer)
//! goes through `elo_net::trading::execute`: the compiled-in allowlist, the
//! check against the chain, the activity log. A listing never moves the item:
//! it stays in the wallet, and in game, until it sells.

use crate::args::Args;
use crate::prompt;
use elo_broker::signer::Signer;
use elo_depot::DepotError;
use elo_net::chain::{self, HttpRpc};
use elo_net::guard::{self, Settings};
use elo_net::{skins, trading};
use serde_json::{json, Value};

pub const USAGE: &str = "\
usage: elo skins                              what this account holds, and its listings
       elo skins market                       the floor and the best listings
       elo skins sell <id> <price> [--days N] list an item for <price> ETH (7 days)
       elo skins cancel <id|order>            cancel a listing (a transaction)
       elo skins buy <order>                  buy a listing from `market`
       elo skins accept <id>                  sell an item to its best offer
       elo skins open [id]                    the item, or the collection, on OpenSea
  --collection skins|pros|copies (default skins). --json for scripts.
  A listing stays in your wallet, and in game, until it sells. OpenSea takes
  its fee; we take none.";

fn err(e: String) -> DepotError {
    DepotError::new(e)
}

fn account() -> Result<(elo_vault::LocalSigner, [u8; 20], String), DepotError> {
    let local = elo_vault::LocalSigner::at(elo_vault::keystore_path());
    let addr = local.address().ok_or_else(|| DepotError::new(
        "no account on this machine — `elo account new` makes one, or sell in the browser"))?;
    let raw = elo_vault::parse_address(&addr).map_err(err)?;
    Ok((local, raw, addr.to_lowercase()))
}

fn key_of(a: &Args) -> Result<&str, DepotError> {
    let k = a.flag("--collection").unwrap_or("skins");
    if skins::KEYS.contains(&k) { Ok(k) } else { Err(DepotError::new(format!("--collection is one of {}", skins::KEYS.join(", ")))) }
}

fn mine(a: &Args, rpc: &HttpRpc, host: &str, key: &str, json_out: bool) -> Result<i32, DepotError> {
    let who = match a.flag("--wallet") {
        Some(w) => w.to_lowercase(),
        None => account()?.2,
    };
    let held = skins::held(rpc, host, key, &who);
    let listed = skins::listings(rpc, host, key, &who);
    if json_out {
        let h = held.as_ref().map(|r| r.iter().map(|i| json!({"id": i.id, "name": i.name})).collect::<Vec<_>>());
        let l = listed.as_ref().map(|r| r.iter().map(|i| json!({"id": i.id, "order": i.order, "price_eth": i.price_eth}))
            .collect::<Vec<_>>());
        println!("{}", json!({"wallet": who, "collection": key,
            "held": h.as_ref().ok(), "held_error": held.as_ref().err(),
            "listings": l.as_ref().ok(), "listings_error": listed.as_ref().err()}));
        return Ok(0);
    }
    let held = held.map_err(err)?;
    let listed = listed.unwrap_or_else(|e| { eprintln!("  (couldn't read your listings: {e})"); Vec::new() });
    if held.is_empty() {
        println!("  {who} holds none of these");
        return Ok(0);
    }
    for h in &held {
        let l = listed.iter().find(|l| l.id == h.id);
        let tag = match l {
            Some(l) => format!("listed at {} ETH", l.price_eth.map(|p| format!("{p}")).unwrap_or_else(|| "?".into())),
            None => String::new(),
        };
        println!("  #{:<6} {:<28} {tag}", h.id, h.name);
    }
    println!("\n  `elo skins sell <id> <price>` lists one; it stays yours, and in game, until it sells");
    Ok(0)
}

fn market(rpc: &HttpRpc, host: &str, key: &str, json_out: bool) -> Result<i32, DepotError> {
    let v = trading::api(rpc, host, &format!("/collections/{key}")).map_err(err)?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(0);
    }
    let name = v.get("name").and_then(Value::as_str).unwrap_or(key);
    if let Some(why) = v.get("why_not").and_then(Value::as_str) {
        println!("  {name} trade on OpenSea: {}", v.get("opensea").and_then(Value::as_str).unwrap_or(""));
        println!("  (the market reads here once the origin can: {why})");
        return Ok(0);
    }
    let floor = v.pointer("/market/floor").and_then(Value::as_f64).map(|f| format!("{f}")).unwrap_or_else(|| "—".into());
    println!("  {name}: floor {floor} ETH");
    for li in v.get("listings").and_then(Value::as_array).cloned().unwrap_or_default() {
        println!("  #{:<6} {:>12} ETH  {}", li.get("token_id").and_then(Value::as_str).unwrap_or("?"),
                 li.get("price_eth").and_then(Value::as_f64).map(|p| format!("{p}")).unwrap_or_default(),
                 li.get("order").and_then(Value::as_str).unwrap_or(""));
    }
    if let Ok(sv) = trading::api(rpc, host, &format!("/collections/{key}/sales")) {
        let last: Vec<String> = sv.get("sales").and_then(Value::as_array).cloned().unwrap_or_default().iter().take(5)
            .map(|x| format!("#{} {} {}", x.get("token_id").and_then(Value::as_str).unwrap_or("?"),
                             x.get("price").and_then(Value::as_f64).map(|p| format!("{p}")).unwrap_or_else(|| "?".into()),
                             x.get("symbol").and_then(Value::as_str).unwrap_or("ETH")))
            .collect();
        if !last.is_empty() {
            println!("  last sold: {}", last.join(" · "));
        }
    }
    println!("\n  `elo skins buy <order>` buys one");
    Ok(0)
}

/// Run the prepared transactions through the guard, after the passphrase.
fn run_steps(prep: &Value, rpc: &HttpRpc, me: &[u8; 20], signer: &elo_vault::LocalSigner, pass: &str,
             source: &str) -> Result<String, DepotError> {
    let settings = Settings::load(&guard::settings_path());
    let sign = |tx: &elo_vault::tx::Tx| signer.sign_transaction(pass, tx);
    trading::execute(prep, rpc, me, &settings, &guard::activity_path(), source, &sign,
                     &|line: &str| eprintln!("{line}")).map_err(err)
}

fn sell(a: &Args, rpc: &HttpRpc, host: &str, key: &str, json_out: bool) -> Result<i32, DepotError> {
    let (Some(id), Some(price)) = (a.positional.get(1), a.positional.get(2)) else {
        return Err(DepotError::new(USAGE));
    };
    let days: u32 = a.flag("--days").unwrap_or("7").parse().map_err(|_| DepotError::new("--days is a whole number"))?;
    let (signer, me, who) = account()?;
    let price_wei = skins::wei(price).map_err(err)?;
    let col = skins::collection(rpc, host, key).map_err(err)?;
    let col_addr = elo_vault::parse_address(col.get("address").and_then(Value::as_str).unwrap_or(""))
        .map_err(|_| DepotError::new("the origin names no address for that collection"))?;
    let prep = skins::prepare_sell(rpc, host, key, id, price, days, &who).map_err(err)?;
    let order = skins::check_sell(&prep, &col_addr, id, price_wei, &me).map_err(err)?;
    let r = |k: &str| prep.pointer(&format!("/review/{k}")).and_then(Value::as_str).unwrap_or("?").to_string();
    eprintln!();
    eprintln!("  sell       {}", r("give"));
    eprintln!("  price      {}", r("price"));
    for f in prep.pointer("/review/fees").and_then(Value::as_array).cloned().unwrap_or_default() {
        eprintln!("  fee        {} to {}", f.get("amount").and_then(Value::as_str).unwrap_or("?"),
                  f.get("to").and_then(Value::as_str).unwrap_or("?"));
    }
    eprintln!("  you get    {}", r("you_get"));
    eprintln!("  ends       {}", r("ends"));
    if let Some(n) = prep.pointer("/review/note").and_then(Value::as_str) {
        eprintln!("  note       {n}");
    }
    eprintln!("  steps      {} approval(s), then one signature", prep.get("txs").and_then(Value::as_array).map(Vec::len).unwrap_or(0));
    eprintln!("\n  It stays in your wallet, and in game, until it sells. OpenSea takes its fee; we take none.\n");
    if a.has("--dry-run") {
        if json_out {
            println!("{}", serde_json::to_string_pretty(&prep).unwrap_or_default());
        }
        return Ok(0);
    }
    let pass = prompt::secret("passphrase to list it (Ctrl-C to stop): ").map_err(err)?;
    if prep.get("txs").and_then(Value::as_array).is_some_and(|t| !t.is_empty()) {
        run_steps(&prep, rpc, &me, &signer, &pass, "skins")?;
    }
    let sig = signer.sign_seaport_listing(&pass, &order).map_err(err)?;
    guard::log(&guard::activity_path(), json!({"source": "skins", "to": "Seaport 1.6 (OpenSea)", "verdict": "ok",
        "what": format!("a listing: {} for {}", r("give"), r("price")),
        "order": prep.get("order_hash").cloned().unwrap_or(Value::Null)}));
    let page = skins::submit(rpc, host, key, &prep, &sig).map_err(err)?;
    if json_out {
        println!("{}", json!({"listed": true, "order_hash": prep.get("order_hash"), "opensea": page}));
    } else {
        println!("  listed: {page}");
    }
    Ok(0)
}

fn order_for(rpc: &HttpRpc, host: &str, key: &str, who: &str, which: &str) -> Result<String, DepotError> {
    if which.starts_with("0x") {
        return Ok(which.to_string());
    }
    let id = which.trim_start_matches('#');
    skins::listings(rpc, host, key, who).map_err(err)?.into_iter().find(|l| l.id == id).map(|l| l.order)
        .ok_or_else(|| DepotError::new(format!("#{id} has no open listing from this account")))
}

/// Every step and what it pays, said before the passphrase. A step's value is
/// the origin's number, so it is shown to the holder rather than taken on
/// trust: `sell` has its own review, and a buy used to ask for the passphrase
/// with nothing on screen but the order hash.
fn review_steps(prep: &Value) {
    let steps = prep.get("txs").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut total: u128 = 0;
    eprintln!();
    for s in &steps {
        let why = elo_hive::render::safe_line(s.get("why").and_then(Value::as_str).unwrap_or("a step"));
        match s.get("value").and_then(Value::as_str).and_then(|v| v.parse::<u128>().ok()) {
            Some(v) if v > 0 => {
                total = total.saturating_add(v);
                eprintln!("  step       {why} — pays {} ETH", skins::eth(v));
            }
            _ => eprintln!("  step       {why}"),
        }
    }
    eprintln!("  pays       {} ETH in all, plus gas\n", skins::eth(total));
}

fn step_cmd(a: &Args, rpc: &HttpRpc, host: &str, key: &str, json_out: bool, verb: &str) -> Result<i32, DepotError> {
    let Some(which) = a.positional.get(1) else { return Err(DepotError::new(USAGE)) };
    let (signer, me, who) = account()?;
    let prep = match verb {
        "cancel" => skins::prepare_cancel(rpc, host, key, &order_for(rpc, host, key, &who, which)?, &who),
        "buy" => skins::prepare_buy(rpc, host, key, which, &who),
        _ => skins::prepare_accept(rpc, host, key, which.trim_start_matches('#'), &who),
    }.map_err(err)?;
    if verb == "accept" {
        let o = prep.get("offer").cloned().unwrap_or(Value::Null);
        eprintln!("  best offer on #{}: {} {}", which.trim_start_matches('#'),
                  o.get("amount").and_then(Value::as_f64).map(|x| format!("{x}")).unwrap_or_else(|| "?".into()),
                  o.get("currency").and_then(Value::as_str).unwrap_or(""));
    }
    review_steps(&prep);
    if a.has("--dry-run") {
        println!("{}", serde_json::to_string_pretty(&prep).unwrap_or_default());
        return Ok(0);
    }
    let pass = prompt::secret(&format!("passphrase to {verb} (Ctrl-C to stop): ")).map_err(err)?;
    let hash = run_steps(&prep, rpc, &me, &signer, &pass, "skins")?;
    if json_out {
        println!("{}", json!({"hash": hash, "url": chain::tx_url(&hash)}));
    } else {
        println!("  done: {}", chain::tx_url(&hash));
    }
    Ok(0)
}

fn open(a: &Args, rpc: &HttpRpc, host: &str, key: &str) -> Result<i32, DepotError> {
    let col = skins::collection(rpc, host, key).map_err(err)?;
    let base = col.get("opensea").and_then(Value::as_str).unwrap_or("").to_string();
    let url = match (a.positional.get(1), col.get("address").and_then(Value::as_str)) {
        (Some(id), Some(addr)) if id.trim_start_matches('#').bytes().all(|b| b.is_ascii_digit()) =>
            format!("https://opensea.io/item/robinhood/{addr}/{}", id.trim_start_matches('#')),
        _ => base,
    };
    if url.is_empty() {
        return Err(DepotError::new("the origin names no OpenSea page for that collection"));
    }
    println!("  {url}");
    elo_depot::launch::open_page(&url).map_err(|e| DepotError::new(format!("couldn't open a browser: {e}")))?;
    Ok(0)
}

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let rpc = HttpRpc::new();
    let key = key_of(a)?;
    match a.positional.first().map(String::as_str) {
        None | Some("mine") => mine(a, &rpc, host, key, json_out),
        Some("market") => market(&rpc, host, key, json_out),
        Some("sell") => sell(a, &rpc, host, key, json_out),
        Some(v @ ("cancel" | "buy" | "accept")) => step_cmd(a, &rpc, host, key, json_out, v),
        Some("open") => open(a, &rpc, host, key),
        Some("help") => { println!("{USAGE}"); Ok(0) }
        Some(other) => Err(DepotError::new(format!("{other}? — {USAGE}"))),
    }
}
