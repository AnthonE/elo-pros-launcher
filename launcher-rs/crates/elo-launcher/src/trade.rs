//! `elo trade` — the terminal, in a terminal (`docs/store/LAUNCH-TERMINAL.md` §4).
//!
//! Every price and every transaction comes from the origin's trading API
//! (`/api/pairs`, `/api/quote`, `/api/prepare/swap/*`); this file reads them,
//! shows them, and signs the prepared transactions with the account on this
//! machine after the passphrase. The origin never sees a key, and nothing here
//! builds calldata of its own. Every step passes the compiled-in allowlist and
//! is written to the activity log (`elo_net::guard`,
//! `docs/client/SIGNING-ALLOWLIST.md`); the shared engine is `elo_net::trading`.
//!
//! **Resting orders, on this machine.** Every type the site has (limit,
//! take-profit, stop, trailing stop, trailing buy, bracket, DCA, TWAP), with
//! the same rules (`elo_net::orders`, held to the site's cases). `orders run`
//! asks for the passphrase once and keeps it masked in memory
//! (`elo_vault::masked`); it confirms each trigger at the order's own size, and
//! a trade that fails pauses its order. Orders fire only while it runs.

use crate::args::Args;
use crate::prompt;
use elo_broker::signer::Signer;
use elo_depot::DepotError;
use elo_net::chain::{self, HttpRpc};
use elo_net::guard::{self, Settings};
use elo_net::orders::{self, Order};
use elo_net::trading;
use serde_json::{json, Value};
use std::time::Duration;

pub const USAGE: &str = "\
usage: elo trade pairs                         every market: price and 24h change
       elo trade quote <amount> <from> <to>      what a swap returns right now
       elo trade buy <eth> <coin>                spend ETH on a coin
       elo trade sell <amount|N%> <coin>         sell a coin for ETH (50% sells half)
       elo trade swap <amount> <from> <to>       any route the pools offer
       elo trade watch [pair…] [--wallet 0x…]    new trades as they happen
       elo trade orders                          your resting orders, on this machine
       elo trade orders add limit <coin> <price> <eth>           buy at the price or under
       elo trade orders add take-profit <coin> <price> <amount>  sell at it or over
       elo trade orders add stop <coin> <price> <amount>         sell at it or under
       elo trade orders add trailing-stop <coin> <pct>% <amount> sell after a drop from the high
       elo trade orders add trailing-buy <coin> <pct>% <eth>     buy after a rise from the low
       elo trade orders add bracket <coin> <tp> <stop> <amount>  sell at either, whichever first
       elo trade orders add dca buy|sell <coin> <amount> every <30m|4h|1d> x<times> [cap <price>]
       elo trade orders add twap buy|sell <coin> <amount> over <2h> in <parts> [cap <price>]
       elo trade orders remove|resume <id>
       elo trade orders run [--every SECONDS]    watch and fire them (asks once)
       elo trade settings [<name> on|off]        approve_once · check · allow_outside
       elo trade log                             what this machine was asked to sign
  prices are ETH per coin. --slippage PCT (default 1). --json for scripts.";

fn api(rpc: &HttpRpc, host: &str, path: &str) -> Result<Value, DepotError> {
    trading::api(rpc, host, path).map_err(DepotError::new)
}

fn q(s: &str) -> String {
    trading::q(s)
}

fn slippage_bps(a: &Args) -> Result<u32, DepotError> {
    let pct: f64 = a.flag("--slippage").unwrap_or("1").parse()
        .map_err(|_| DepotError::new("--slippage is a percent, like 1 or 0.5"))?;
    if !(0.05..=20.0).contains(&pct) {
        return Err(DepotError::new("--slippage is between 0.05 and 20 percent"));
    }
    Ok((pct * 100.0).round() as u32)
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

pub fn fmt_price(p: f64) -> String {
    orders::fmt_price(p)
}

// ── pairs, quote, watch ─────────────────────────────────────────────────────
fn pairs(rpc: &HttpRpc, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let v = api(rpc, host, "/pairs")?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(0);
    }
    let rows = v.get("pairs").and_then(Value::as_array).cloned().unwrap_or_default();
    println!("  {:<16} {:>12} {:>9}  {}", "pair", "price", "24h", "venue");
    for p in rows {
        let key = p.get("key").and_then(Value::as_str).unwrap_or("?");
        let price = p.get("price").and_then(num).map(fmt_price).unwrap_or_else(|| "—".into());
        let ch = p.get("change_24h").and_then(num)
            .map(|c| format!("{:+.1}%", c * 100.0)).unwrap_or_else(|| "—".into());
        let venue = p.get("venue").and_then(Value::as_str).unwrap_or("");
        println!("  {key:<16} {price:>12} {ch:>9}  {venue}");
    }
    Ok(0)
}

fn quote_of(rpc: &HttpRpc, host: &str, amount: &str, from: &str, to: &str, bps: u32) -> Result<Value, DepotError> {
    api(rpc, host, &format!("/quote?from={}&to={}&amount={}&slippage_bps={bps}", q(from), q(to), q(amount)))
}

fn route_line(qv: &Value) -> String {
    qv.get("route").and_then(Value::as_array).map(|r| {
        let mut s = qv.pointer("/from/sym").and_then(Value::as_str).unwrap_or("?").to_string();
        for leg in r {
            s.push_str(&format!(" → {} ({})", leg.pointer("/out/sym").and_then(Value::as_str).unwrap_or("?"),
                                leg.get("venue").and_then(Value::as_str).unwrap_or("?")));
        }
        s
    }).unwrap_or_default()
}

fn show_quote(qv: &Value) {
    let get = |p: &str| qv.pointer(p).and_then(Value::as_str).unwrap_or("?").to_string();
    eprintln!("  you pay    {} {}", get("/amount_in/human"), get("/amount_in/sym"));
    eprintln!("  you get    {} {}", get("/amount_out/human"), get("/amount_out/sym"));
    eprintln!("  at least   {} {}", get("/min_out/human"), get("/min_out/sym"));
    if let Some(i) = qv.get("price_impact_incl_fees").and_then(num) {
        eprintln!("  impact     {:.2}% with fees", i * 100.0);
    }
    eprintln!("  route      {}", route_line(qv));
}

fn quote(a: &Args, rpc: &HttpRpc, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let (Some(amount), Some(from), Some(to)) = (a.positional.get(1), a.positional.get(2), a.positional.get(3)) else {
        return Err(DepotError::new(USAGE));
    };
    let v = quote_of(rpc, host, amount, from, to, slippage_bps(a)?)?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    } else {
        show_quote(&v);
    }
    Ok(0)
}

fn watch(a: &Args, rpc: &HttpRpc, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let mut keys: Vec<String> = a.positional.iter().skip(1).cloned().collect();
    if keys.is_empty() {
        let v = api(rpc, host, "/pairs")?;
        keys = v.get("pairs").and_then(Value::as_array).map(|r| r.iter()
            .filter_map(|p| p.get("key").and_then(Value::as_str).map(str::to_string)).collect())
            .unwrap_or_default();
    }
    let wallet = a.flag("--wallet").map(str::to_ascii_lowercase);
    let mut seen = std::collections::HashSet::new();
    let mut first = true;
    eprintln!("  watching {} — Ctrl-C to stop", keys.join(", "));
    loop {
        for k in &keys {
            let Ok(v) = api(rpc, host, &format!("/pairs/{}", q(k))) else { continue };
            let mut trades = v.get("trades").and_then(Value::as_array).cloned().unwrap_or_default();
            trades.reverse();
            for t in trades {
                let tx = t.get("tx").and_then(Value::as_str).unwrap_or("").to_string();
                if !seen.insert(format!("{k}:{tx}")) || first {
                    continue;
                }
                let from = t.get("from").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
                if wallet.as_ref().is_some_and(|w| *w != from) {
                    continue;
                }
                if json_out {
                    println!("{}", json!({"pair": k, "trade": t}));
                } else {
                    println!("  {} {:<12} {:<4} {:>14} @ {}  {}",
                        t.get("at").and_then(Value::as_str).unwrap_or(""), k,
                        t.get("side").and_then(Value::as_str).unwrap_or(""),
                        t.get("base").and_then(num).map(|b| format!("{b:.2}")).unwrap_or_default(),
                        t.get("price").and_then(num).map(fmt_price).unwrap_or_default(),
                        elo_depot::streamer::shown(&from));
                }
            }
        }
        if first {
            eprintln!("  (only trades from now on are shown)");
        }
        first = false;
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// `50%` of what this account holds of `coin`, read off the chain.
fn resolve_amount(rpc: &HttpRpc, host: &str, amount: &str, coin: &str, who: &[u8; 20]) -> Result<String, DepotError> {
    let Some(pct) = amount.strip_suffix('%') else { return Ok(amount.to_string()) };
    let pct: f64 = pct.parse().map_err(|_| DepotError::new("a percent is like 50%"))?;
    if !(0.0..=100.0).contains(&pct) || pct == 0.0 {
        return Err(DepotError::new("a percent is above 0 and at most 100"));
    }
    let toks = api(rpc, host, "/quote/tokens")?;
    let t = toks.get("tokens").and_then(Value::as_array).and_then(|r| r.iter()
        .find(|t| t.get("sym").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case(coin))).cloned())
        .ok_or_else(|| DepotError::new(format!("{coin} is not a coin the pools route")))?;
    let dec = t.get("dec").and_then(Value::as_u64).unwrap_or(18) as u32;
    let addr = elo_vault::parse_address(t.get("addr").and_then(Value::as_str).unwrap_or(""))
        .map_err(DepotError::new)?;
    let held = if addr == [0u8; 20] { chain::balance(rpc, who) } else { chain::token_balance(rpc, &addr, who) }
        .map_err(|e| DepotError::new(format!("could not read your balance, so nothing was sent: {e}")))?;
    // Multiply before dividing: dividing first rounds a small balance to
    // nothing, and leaves dust behind on 100%.
    let bps = (pct * 100.0).round() as u128;
    let raw = if bps >= 10_000 {
        held
    } else {
        held.checked_mul(bps).map_or(held / 10_000 * bps, |x| x / 10_000)
    };
    if held == 0 {
        return Err(DepotError::new(format!("this account holds no {coin}")));
    }
    if raw == 0 {
        return Err(DepotError::new(format!("{pct}% of your {coin} rounds to nothing")));
    }
    Ok(elo_net::wallet::format_amount(raw, dec as u8).replace(',', ""))
}

fn account() -> Result<(elo_vault::LocalSigner, [u8; 20]), DepotError> {
    let local = elo_vault::LocalSigner::at(elo_vault::keystore_path());
    let addr = local.address().ok_or_else(|| DepotError::new(
        "no account on this machine — `elo account new` makes one, or trade in the browser"))?;
    let raw = elo_vault::parse_address(&addr).map_err(DepotError::new)?;
    Ok((local, raw))
}

fn swap(a: &Args, rpc: &HttpRpc, host: &str, json_out: bool, amount: &str, from: &str, to: &str)
    -> Result<i32, DepotError> {
    let bps = slippage_bps(a)?;
    let (signer, me) = account()?;
    let settings = Settings::load(&guard::settings_path());
    let amount = resolve_amount(rpc, host, amount, from, &me)?;
    let prep = trading::prepare(rpc, host, &amount, from, to, bps, &me, &settings).map_err(DepotError::new)?;
    let qv = prep.get("quote").cloned().unwrap_or(Value::Null);
    eprintln!();
    show_quote(&qv);
    eprintln!("  steps    {}", prep.get("txs").and_then(Value::as_array).map(Vec::len).unwrap_or(0));
    eprintln!("\n  Prices move, and these pools are thin. Not financial advice.\n");
    if trading::waits_on_router(&prep) {
        return Err(DepotError::new("this route opens when our router is live; until then trade it on the \
            pool's own exchange (`elo trade pairs --json` names it) — nothing was asked or sent"));
    }
    if a.has("--dry-run") {
        if json_out {
            println!("{}", serde_json::to_string_pretty(&prep).unwrap_or_default());
        }
        return Ok(0);
    }
    let pass = prompt::secret("passphrase to trade (Ctrl-C to stop): ").map_err(DepotError::new)?;
    let sign = |tx: &elo_vault::tx::Tx| signer.sign_transaction(&pass, tx);
    let hash = trading::execute(&prep, rpc, &me, &settings, &guard::activity_path(), "trade", &sign,
                                &|line: &str| eprintln!("{line}")).map_err(DepotError::new)?;
    if json_out {
        println!("{}", json!({"hash": hash, "url": chain::tx_url(&hash)}));
    } else {
        println!("{hash}");
        println!("  done: {}", chain::tx_url(&hash));
    }
    Ok(0)
}

// ── resting orders ──────────────────────────────────────────────────────────
pub use orders::parse_dur;

fn number(s: &str, what: &str) -> Result<f64, String> {
    s.trim_end_matches('%').parse().map_err(|_| format!("{what} is a number, not {s}"))
}

/// The words after `orders add`, as an order.
pub fn parse_order(args: &[String], id: u64) -> Result<Order, String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut o = Order { id, kind: a.first().copied().unwrap_or("").to_string(), coin: String::new(), side: None,
                        amount: String::new(), price: None, pct: None, tp: None, stop: None, every_s: None,
                        times: None, over_s: None, parts: None, cap: None, state: Default::default(), paused: None };
    let need = |n: usize| if a.len() >= n { Ok(()) } else { Err(format!("orders add {} needs more words — `elo trade help`", o.kind)) };
    match o.kind.as_str() {
        "limit" | "take-profit" | "stop" => {
            need(4)?;
            o.coin = a[1].to_ascii_uppercase(); o.price = Some(number(a[2], "the price")?); o.amount = a[3].into();
        }
        "trailing-stop" | "trailing-buy" => {
            need(4)?;
            o.coin = a[1].to_ascii_uppercase(); o.pct = Some(number(a[2], "the trail")?); o.amount = a[3].into();
        }
        "bracket" => {
            need(5)?;
            o.coin = a[1].to_ascii_uppercase(); o.tp = Some(number(a[2], "the take-profit")?);
            o.stop = Some(number(a[3], "the stop")?); o.amount = a[4].into();
        }
        "dca" | "twap" => {
            need(6)?;
            o.side = Some(a[1].to_string()); o.coin = a[2].to_ascii_uppercase(); o.amount = a[3].into();
            let mut i = 4;
            while i < a.len() {
                match (a[i], a.get(i + 1)) {
                    ("every", Some(d)) => { o.every_s = Some(parse_dur(d)?); i += 2; }
                    ("over", Some(d)) => { o.over_s = Some(parse_dur(d)?); i += 2; }
                    ("in", Some(n)) => { o.parts = Some(number(n, "the parts")? as u32); i += 2; }
                    ("cap", Some(p)) => { o.cap = Some(number(p, "the cap")?); i += 2; }
                    (t, _) if t.starts_with('x') => { o.times = Some(number(&t[1..], "the count")? as u32); i += 1; }
                    (t, _) => return Err(format!("{t}? — `elo trade help` shows the words")),
                }
            }
        }
        _ => {}
    }
    o.validate()?;
    Ok(o)
}

fn show_book(book: &[Order], json_out: bool) {
    if json_out {
        println!("{}", json!({"orders": book.iter().map(Order::to_json).collect::<Vec<_>>()}));
        return;
    }
    if book.is_empty() {
        println!("  no resting orders — `elo trade orders add limit ELO 0.00000001 0.01`");
        return;
    }
    for o in book {
        let st = &o.state;
        let state = if st.finished { "done".to_string() }
            else if let Some(p) = &o.paused { format!("paused: {p}") }
            else if st.done > 0 { format!("{} done", st.done) }
            else if let Some(p) = st.peak { format!("high {}", fmt_price(p)) }
            else if let Some(l) = st.low { format!("low {}", fmt_price(l)) }
            else { "waiting".into() };
        println!("  #{:<3} {:<60} {state}", o.id, o.describe());
    }
    println!("\n  they fire only while `elo trade orders run` (or the window's Orders) is running");
}

fn orders_cmd(a: &Args, rpc: &HttpRpc, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let path = trading::orders_path();
    let mut book = trading::load_orders(&path);
    let save = |b: &[Order]| trading::save_orders(&path, b).map_err(DepotError::new);
    let id_at = |i: usize| a.positional.get(i).and_then(|s| s.trim_start_matches('#').parse::<u64>().ok())
        .ok_or_else(|| DepotError::new("which order? give its number"));
    match a.positional.get(1).map(String::as_str) {
        None => { show_book(&book, json_out); Ok(0) }
        Some("add") => {
            let id = book.iter().map(|o| o.id).max().unwrap_or(0) + 1;
            let o = parse_order(&a.positional[2..], id).map_err(DepotError::new)?;
            println!("  #{} kept on this machine: {}", o.id, o.describe());
            book.push(o);
            save(&book)?;
            Ok(0)
        }
        Some("remove") => {
            let id = id_at(2)?;
            let before = book.len();
            book.retain(|o| o.id != id);
            if book.len() == before {
                return Err(DepotError::new(format!("no order #{id}")));
            }
            save(&book)?;
            println!("  removed #{id}");
            Ok(0)
        }
        Some("resume") => {
            let id = id_at(2)?;
            let o = book.iter_mut().find(|o| o.id == id).ok_or_else(|| DepotError::new(format!("no order #{id}")))?;
            o.paused = None;
            save(&book)?;
            println!("  #{id} is live again");
            Ok(0)
        }
        Some("run") => run_orders(a, rpc, host),
        Some(other) => Err(DepotError::new(format!("orders {other}? {USAGE}"))),
    }
}

fn run_orders(a: &Args, rpc: &HttpRpc, host: &str) -> Result<i32, DepotError> {
    let every: u64 = a.flag("--every").unwrap_or("15").parse()
        .map_err(|_| DepotError::new("--every is whole seconds"))?;
    let every = every.max(5);
    let bps = slippage_bps(a)?;
    let path = trading::orders_path();
    if !trading::load_orders(&path).iter().any(|o| !o.state.finished && o.paused.is_none()) {
        println!("  no live orders to watch");
        return Ok(0);
    }
    let (signer, me) = account()?;
    let pass = prompt::secret("passphrase, held masked while this runs (Ctrl-C to stop): ").map_err(DepotError::new)?;
    let masked = elo_vault::masked::Masked::new(pass).map_err(DepotError::new)?;
    let sign = |tx: &elo_vault::tx::Tx| masked.with(|p| signer.sign_transaction(p, tx));
    eprintln!("  watching every {every}s. Orders fire only while this runs; Ctrl-C stops it.");
    loop {
        let settings = Settings::load(&guard::settings_path());
        for line in trading::tick(rpc, host, &path, &me, &settings, &guard::activity_path(), bps, &sign,
                                  &|l: &str| eprintln!("{l}")) {
            eprintln!("  {line}");
        }
        if !trading::load_orders(&path).iter().any(|o| !o.state.finished && o.paused.is_none()) {
            eprintln!("  nothing left to watch: every order is done, paused or removed");
            return Ok(0);
        }
        std::thread::sleep(Duration::from_secs(every));
    }
}

fn settings_cmd(a: &Args, json_out: bool) -> Result<i32, DepotError> {
    let p = guard::settings_path();
    let mut s = Settings::load(&p);
    if let Some(name) = a.positional.get(1) {
        let on = match a.positional.get(2).map(String::as_str) {
            Some("on") | Some("true") => true,
            Some("off") | Some("false") => false,
            _ => return Err(DepotError::new("settings <name> on|off")),
        };
        s.set(name, on).map_err(DepotError::new)?;
        s.save(&p).map_err(DepotError::new)?;
    }
    if json_out {
        println!("{}", json!({"approve_once": s.approve_once, "check": s.check, "allow_outside": s.allow_outside}));
        return Ok(0);
    }
    let what = [("approve_once", "a coin's first sale approves it for good; later sales are one transaction"),
                ("check", "each transaction is tried against the chain before it is signed"),
                ("allow_outside", "sign for contracts outside our ecosystem, with a warning")];
    for (k, w) in what {
        println!("  {:<14} {:<4} {w}", k, if s.get(k) == Some(true) { "on" } else { "off" });
    }
    println!("\n  kept at {}", p.display());
    Ok(0)
}

fn log_cmd(json_out: bool) -> Result<i32, DepotError> {
    let p = guard::activity_path();
    let rows = guard::read_log(&p, 40);
    if json_out {
        println!("{}", json!({"log": rows}));
        return Ok(0);
    }
    if rows.is_empty() {
        println!("  nothing signed from this machine yet");
        return Ok(0);
    }
    for e in rows {
        let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        println!("  {} {:<8} {:<10} {}{}", e.get("t").and_then(Value::as_u64).unwrap_or(0), s("verdict"), s("source"),
                 s("what"), if s("hash").is_empty() { String::new() } else { format!("  {}", s("hash")) });
    }
    println!("\n  the whole log: {}", p.display());
    Ok(0)
}

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let rpc = HttpRpc::new();
    let p = |i: usize| a.positional.get(i).map(String::as_str);
    match p(0) {
        None | Some("pairs") => pairs(&rpc, host, json_out),
        Some("quote") => quote(a, &rpc, host, json_out),
        Some("buy") => match (p(1), p(2)) {
            (Some(amt), Some(coin)) => swap(a, &rpc, host, json_out, amt, "ETH", coin),
            _ => Err(DepotError::new(USAGE)),
        },
        Some("sell") => match (p(1), p(2)) {
            (Some(amt), Some(coin)) => swap(a, &rpc, host, json_out, amt, coin, "ETH"),
            _ => Err(DepotError::new(USAGE)),
        },
        Some("swap") => match (p(1), p(2), p(3)) {
            (Some(amt), Some(f), Some(t)) => swap(a, &rpc, host, json_out, amt, f, t),
            _ => Err(DepotError::new(USAGE)),
        },
        Some("watch") => watch(a, &rpc, host, json_out),
        Some("orders") => orders_cmd(a, &rpc, host, json_out),
        Some("settings") => settings_cmd(a, json_out),
        Some("log") => log_cmd(json_out),
        Some("help") => { println!("{USAGE}"); Ok(0) }
        Some(other) => Err(DepotError::new(format!("trade {other}? {USAGE}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn every_kind_reads_from_its_words() {
        assert_eq!(parse_order(&w("limit elo 0.00000001 0.01"), 1).unwrap().price, Some(1e-8));
        assert_eq!(parse_order(&w("trailing-stop ELO 10% 500"), 1).unwrap().pct, Some(10.0));
        let b = parse_order(&w("bracket ELO 0.00000003 0.00000001 500"), 1).unwrap();
        assert_eq!((b.tp, b.stop), (Some(3e-8), Some(1e-8)));
        let d = parse_order(&w("dca buy ELO 0.01 every 4h x6 cap 0.00000002"), 1).unwrap();
        assert_eq!((d.every_s, d.times, d.cap, d.side.as_deref()), (Some(14400.0), Some(6), Some(2e-8), Some("buy")));
        let t = parse_order(&w("twap sell ELO 1000000 over 2h in 8"), 1).unwrap();
        assert_eq!((t.over_s, t.parts), (Some(7200.0), Some(8)));
        assert_eq!(t.describe(), "sell 1000000 ELO over 2 h in 8 parts");
    }

    #[test]
    fn an_order_is_refused_unless_it_is_whole() {
        assert!(parse_order(&w("market ELO 1 1"), 1).is_err());
        assert!(parse_order(&w("limit ETH 1 1"), 1).is_err());
        assert!(parse_order(&w("stop ELO -1 1"), 1).is_err());
        assert!(parse_order(&w("stop ELO 1"), 1).is_err());
        assert!(parse_order(&w("bracket ELO 1 2 5"), 1).is_err());
        assert!(parse_order(&w("dca hold ELO 1 every 1h x2"), 1).is_err());
        assert!(parse_order(&w("dca buy ELO 1 every 1y x2"), 1).is_err());
    }

    #[test]
    fn durations_read_as_people_write_them() {
        assert_eq!(parse_dur("30m").unwrap(), 1800.0);
        assert_eq!(parse_dur("1d").unwrap(), 86400.0);
        assert_eq!(parse_dur("90").unwrap(), 90.0);
        assert!(parse_dur("soon").is_err());
    }

    #[test]
    fn a_v1_book_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("orders.json");
        std::fs::write(&p, r#"{"orders":[{"id":3,"kind":"take-profit","coin":"JUNK","price":0.001,"amount":"500"}]}"#).unwrap();
        let book = trading::load_orders(&p);
        assert_eq!(book.len(), 1);
        assert_eq!(book[0].describe(), "sell 500 JUNK at 1.000e-3 or over");
        trading::save_orders(&p, &book).unwrap();
        assert_eq!(trading::load_orders(&p), book);
    }

    #[test]
    fn query_values_are_escaped() {
        assert_eq!(q("a b&c"), "a%20b%26c");
    }
}
