//! Moving coins between this account's two chains — SOL to ETH on Robinhood
//! Chain and back, USDC to USDG and back — through the origin's `/api/xquote`
//! (LI.FI, `meter/xquote.py`).
//!
//! # Your own two addresses, nothing to paste
//!
//! A move always lands on this same account's address on the other chain: the
//! Solana address and the `0x` address the recovery words made together. That
//! is what makes it one step, and it is also a guard — no field here takes a
//! recipient, so a breached quote cannot send a move to somebody else without
//! the review showing a "to" that is not yours (the origin is asked with both
//! addresses and the review prints them back).
//!
//! # What is checked before the passphrase
//!
//! - **From Robinhood Chain**: the quote's transaction goes through
//!   [`crate::trading::execute`] like every trade — the compiled-in allowlist
//!   (LI.FI is on it), `eth_estimateGas`, the activity log. An approval is for
//!   the exact amount.
//! - **From Solana**: the quote's transaction must make this account its fee
//!   payer, and is simulated first ([`crate::solana::simulate`]): it may take
//!   the amount being moved plus a small allowance for fees and accounts, and
//!   may not take from any other coin this account holds that it marks
//!   writable — the only accounts a Solana transaction can take from, lookup
//!   tables included. Anything else is refused before signing.
//!
//! What a move never is: a bridge of ELO, a wrapped ELO, or a posted rate
//! between the two chains' coins (`TWO-CHAIN-STORE.md` §4). The routes below
//! hold no game coin and no reserve.

use crate::chain::HttpRpc;
use crate::solana::{self, After};
use crate::trading::{self, q};
use elo_vault::solana::{self as sol, Transaction};
use serde_json::{json, Value};

/// One way a move can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub from_chain: &'static str,
    pub from: &'static str,
    pub to_chain: &'static str,
    pub to: &'static str,
    pub decimals: u8,
}

impl Route {
    pub fn from_solana(&self) -> bool {
        self.from_chain == "sol"
    }

    /// `SOL on Solana → ETH on Robinhood Chain`.
    pub fn label(&self) -> String {
        format!("{} on {} → {} on {}", self.from, chain_name(self.from_chain), self.to, chain_name(self.to_chain))
    }
}

pub fn chain_name(c: &str) -> &'static str {
    if c == "sol" { sol::NAME } else { crate::chain::NAME }
}

/// The four moves, in the order a menu shows them.
pub const ROUTES: [Route; 4] = [
    Route { from_chain: "sol", from: "SOL", to_chain: "rh", to: "ETH", decimals: 9 },
    Route { from_chain: "rh", from: "ETH", to_chain: "sol", to: "SOL", decimals: 18 },
    Route { from_chain: "sol", from: "USDC", to_chain: "rh", to: "USDG", decimals: 6 },
    Route { from_chain: "rh", from: "USDG", to_chain: "sol", to: "USDC", decimals: 6 },
];

/// The route that moves `coin` off its chain: SOL and USDC leave Solana, ETH
/// and USDG leave Robinhood Chain.
pub fn route_for(coin: &str) -> Option<Route> {
    ROUTES.iter().copied().find(|r| r.from.eq_ignore_ascii_case(coin))
}

/// USDC's mint on Solana — the only Solana coin a move spends besides SOL.
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

/// Ask the origin for a move from this account to itself. With both addresses
/// the answer carries the source chain's transaction.
pub fn quote(origin: &HttpRpc, host: &str, r: &Route, amount: &str, sol_addr: &str, evm_addr: &str,
             slippage_bps: u32) -> Result<Value, String> {
    let (from_addr, to_addr) = if r.from_solana() { (sol_addr, evm_addr) } else { (evm_addr, sol_addr) };
    trading::api(origin, host, &format!(
        "/xquote?from_chain={}&from={}&amount={}&to={}&to_chain={}&from_address={}&to_address={}&slippage_bps={slippage_bps}",
        r.from_chain, r.from, q(amount), r.to, r.to_chain, q(from_addr), q(to_addr)))
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
}

fn amt(x: f64) -> String {
    let s = format!("{x:.8}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.to_string() }
}

/// The review's lines for a quoted move.
pub fn review(r: &Route, quote: &Value, sol_addr: &str, evm_addr: &str) -> Vec<(String, String)> {
    let to_addr = if r.from_solana() { evm_addr } else { sol_addr };
    let from_amt = num(quote.pointer("/from/amount")).map(amt).unwrap_or_else(|| "?".into());
    let out = num(quote.pointer("/to/amount")).map(amt).unwrap_or_else(|| "?".into());
    let min = num(quote.pointer("/to/min")).map(amt).unwrap_or_else(|| "?".into());
    let fees: f64 = ["fees", "gas"].iter()
        .flat_map(|k| quote.get(*k).and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|f| num(f.get("usd"))).sum();
    let mut lines = vec![
        ("Move".into(), format!("{from_amt} {} on {}", r.from, chain_name(r.from_chain))),
        ("Arrives".into(), format!("about {out} {} on {} (at least {min})", r.to, chain_name(r.to_chain))),
        ("To".into(), format!("your own address there: {}", elo_depot::streamer::shown(to_addr))),
    ];
    if let Some(via) = quote.get("via").and_then(Value::as_str) {
        let takes = num(quote.get("takes_s")).map(|s| format!(", about {}", crate::orders::dur(s))).unwrap_or_default();
        lines.push(("Via".into(), format!("{via} through LI.FI{takes}")));
    }
    lines.push(("Fees".into(), format!("about ${fees:.2} in all")));
    lines
}

/// How much SOL a move from Solana may take beyond the amount moved: the
/// network fee, a priority fee, and the small accounts a route opens.
pub const SOL_ALLOWANCE: u64 = 10_000_000; // 0.01 SOL

/// The Solana side of a move, checked and ready to sign.
#[derive(Debug, Clone)]
pub struct SolLeg {
    pub tx: Transaction,
    pub after: After,
    /// Lamports the simulation says leave this account, everything included.
    pub spent: u64,
}

/// Read the quote's Solana transaction and check it against what the review
/// said: this account pays the fee, it signs, and the simulation moves only
/// the amount (plus [`SOL_ALLOWANCE`] of SOL) and no other coin.
pub fn check_solana(rpc: &dyn crate::chain::Rpc, quote: &Value, r: &Route, me: &[u8; 32],
                    amount_raw: u64) -> Result<SolLeg, String> {
    let b64 = quote.pointer("/tx/serialized_b64").and_then(Value::as_str)
        .ok_or("the quote carried no Solana transaction")?;
    let tx = Transaction::parse(&sol::b64_decode(b64)?)?;
    if tx.message.keys.first() != Some(me) {
        return Err("this move's transaction is not paid for by your Solana account — refusing to sign".into());
    }
    let before_sol = solana::balance(rpc, me)?;
    // A Solana transaction can only take from an account it marks writable,
    // so those are the ones to watch: this account's token accounts among the
    // writable static keys and the writable addresses its lookup tables load.
    // (The RPC refuses to report more accounts than the transaction touches.)
    let writable = solana::writable_accounts(rpc, &tx)?;
    let held: Vec<solana::Holding> = solana::holdings(rpc, me)?.into_iter()
        .filter(|h| writable.contains(&h.account)).collect();
    let watch: Vec<[u8; 32]> = held.iter().map(|h| h.account).collect();
    let after = solana::simulate(rpc, &tx, me, &watch)?;
    let spent = after.spent.unwrap_or_else(|| before_sol.saturating_sub(after.lamports.unwrap_or(0)));
    let sol_cap = SOL_ALLOWANCE + if r.from == "SOL" { amount_raw } else { 0 };
    if spent > sol_cap {
        return Err(format!(
            "The chain says this move would take {} SOL from you, more than the {} it should. Nothing was signed.",
            crate::wallet::format_amount(spent as u128, 9), crate::wallet::format_amount(sol_cap as u128, 9)));
    }
    let usdc = sol::program(USDC_MINT);
    let allowed = |mint: &[u8; 32]| if r.from == "USDC" && *mint == usdc { amount_raw } else { 0 };
    // The simulation's own token balances when the RPC reports them (every
    // account of ours the transaction touches); the watched accounts if not.
    let drops: Vec<([u8; 32], u64)> = match &after.owned {
        Some(rows) => rows.iter().map(|(m, a, b)| (*m, a.saturating_sub(*b))).collect(),
        None => held.iter().zip(&after.tokens)
            .map(|(h, (_, left))| (h.mint, h.amount.saturating_sub(left.unwrap_or(0)))).collect(),
    };
    let mut by_mint: Vec<([u8; 32], u64)> = Vec::new();
    for (m, d) in drops {
        match by_mint.iter_mut().find(|(x, _)| *x == m) {
            Some(e) => e.1 += d,
            None => by_mint.push((m, d)),
        }
    }
    for (mint, dropped) in by_mint {
        if dropped > allowed(&mint) {
            return Err("The chain says this move would also take a coin you did not choose to move. \
                        Nothing was signed.".into());
        }
    }
    Ok(SolLeg { tx, after, spent })
}

/// How much ETH a move from Robinhood Chain may carry beyond the amount moved:
/// a bridge's own native fee, which some routes charge in the transaction's
/// value. The counterpart of [`SOL_ALLOWANCE`].
pub const ETH_ALLOWANCE: u128 = 2_000_000_000_000_000; // 0.002 ETH

/// USDG on Robinhood Chain: the one coin a move from there approves.
const USDG: &str = "0x5fc5360d0400a0fd4f2af552add042d716f1d168";

/// The quote's Robinhood Chain transaction as prepared steps for
/// [`trading::execute`]: an exact approval first when the coin is USDG, then
/// the move.
///
/// Checked against what the player typed, the way [`check_solana`] checks the
/// other side: the review is built from the quote, so a quote whose
/// transaction carries more than the amount (or approves another coin, or more
/// of it) would otherwise be signed under a review that never showed it.
pub fn evm_steps(quote: &Value, r: &Route, amount_raw: u128) -> Result<Value, String> {
    let tx = quote.get("tx").filter(|t| t.get("kind").and_then(Value::as_str) == Some("evm"))
        .ok_or("the quote carried no Robinhood Chain transaction")?;
    if tx.get("chain_id").and_then(Value::as_u64) != Some(elo_vault::tx::CHAIN_ID) {
        return Err("the quote's transaction is for another chain — refusing to sign".into());
    }
    let to = tx.get("to").and_then(Value::as_str).ok_or("the quote's transaction has no `to`")?;
    let data = tx.get("data").and_then(Value::as_str).unwrap_or("0x");
    let value_hex = tx.get("value").and_then(Value::as_str).unwrap_or("0x0");
    let h = value_hex.trim_start_matches("0x").trim_start_matches('0');
    let value = if h.is_empty() { 0 } else {
        u128::from_str_radix(h, 16).map_err(|_| "the quote's value is not a number".to_string())?
    };
    let pays_eth = r.from == "ETH";
    let cap = ETH_ALLOWANCE.saturating_add(if pays_eth { amount_raw } else { 0 });
    if value > cap {
        return Err(format!(
            "this move's transaction would send {} ETH, more than the {} it should. Nothing was signed.",
            crate::wallet::format_amount(value, 18), crate::wallet::format_amount(cap, 18)));
    }
    if pays_eth && quote.get("approve").is_some() {
        return Err("a move of ETH needs no approval, and this quote asks for one. Nothing was signed.".into());
    }
    if !pays_eth && quote.get("approve").is_none() {
        return Err(format!("a move of {} needs its approval, and this quote has none. Nothing was signed.", r.from));
    }
    let mut steps = Vec::new();
    if let Some(a) = quote.get("approve") {
        let token = a.get("token").and_then(Value::as_str).ok_or("an approval with no token")?;
        if !token.eq_ignore_ascii_case(USDG) {
            return Err("this move approves a coin you did not choose to move. Nothing was signed.".into());
        }
        let spender = elo_vault::parse_address(a.get("spender").and_then(Value::as_str).ok_or("an approval with no spender")?)?;
        let amount: u128 = a.get("amount").and_then(Value::as_str).and_then(|s| s.parse().ok())
            .ok_or("an approval with no amount")?;
        if amount > amount_raw {
            return Err(format!(
                "this move approves {} {}, more than the {} you typed. Nothing was signed.",
                crate::wallet::format_amount(amount, r.decimals), r.from,
                crate::wallet::format_amount(amount_raw, r.decimals)));
        }
        let pad = |b: &[u8]| format!("{:0>64}", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
        let data = format!("0x095ea7b3{}{}", pad(&spender), pad(&amount.to_be_bytes()));
        steps.push(json!({"to": token, "data": data, "value": "0", "why": "let LI.FI move this amount", "skip_if": true}));
    }
    steps.push(json!({"to": to, "data": data, "value": value.to_string(), "why": "the move"}));
    Ok(json!({"txs": steps}))
}

/// Where a move stands, from the origin (`/api/xquote/status`).
pub fn status(origin: &HttpRpc, host: &str, r: &Route, tx: &str) -> Result<Value, String> {
    trading::api(origin, host, &format!("/xquote/status?tx={}&from_chain={}&to_chain={}", q(tx), r.from_chain, r.to_chain))
}

/// Watch a sent move until it lands, fails, or five minutes pass. `say` hears
/// each change.
pub fn follow(origin: &HttpRpc, host: &str, r: &Route, tx: &str, say: &dyn Fn(&str)) -> Result<Value, String> {
    let start = std::time::Instant::now();
    let mut last = String::new();
    while start.elapsed() < std::time::Duration::from_secs(300) {
        if let Ok(s) = status(origin, host, r, tx) {
            let st = s.get("status").and_then(Value::as_str).unwrap_or("").to_string();
            if st != last {
                say(&format!("  {}", match st.as_str() {
                    "DONE" => "arrived",
                    "FAILED" => "failed",
                    "PENDING" => "on its way",
                    _ => "waiting to be seen",
                }));
                last = st.clone();
            }
            match st.as_str() {
                "DONE" => return Ok(s),
                "FAILED" => return Err(s.get("message").and_then(Value::as_str)
                    .unwrap_or("the route reported a failure").to_string()),
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(4));
    }
    Err("not arrived in five minutes — it may still land; LI.FI's explorer has it".into())
}
