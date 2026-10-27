//! The wallet: which coins, how much of each, and a send planned well enough
//! to be shown to a person before anything is signed.
//!
//! # The order of a send
//!
//! [`plan`] does every refusal that needs no key: a bad or mistyped address, a
//! zero, more than you hold, a token sent to its own contract, a chain that is
//! not 4663, a transfer the chain says would fail. Only a plan that survives
//! all of that is shown on the review screen, and only after the holder
//! presses Send is the passphrase asked for. Nobody types a passphrase over a
//! send that was never going to land.
//!
//! # What the review shows is read back out of the transaction
//!
//! [`review`] reads the recipient and the amount from the `Tx` that will be
//! signed — the calldata for a token — not from the form they were typed
//! into. If the two ever disagreed, the screen would show the truth.

use crate::chain::{self, Rpc};
use elo_vault::tx::{self, Tx, CHAIN_ID};
use serde_json::Value;
use std::path::Path;

/// A coin this wallet can show and send. `token: None` is ETH, the chain's
/// own coin, which pays every fee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coin {
    pub sym: String,
    pub token: Option<[u8; 20]>,
    pub decimals: u8,
    /// Whose coin it is, in a few words (from the origin's book).
    pub what: String,
}

impl Coin {
    pub fn eth() -> Coin {
        Coin { sym: "ETH".into(), token: None, decimals: 18, what: "pays every fee on the chain".into() }
    }
}

/// The coins the origin lists, then each one's symbol and decimals as the
/// contract itself states them. ETH is always first and needs no origin.
///
/// The origin decides which coins appear; the chain decides what they are.
/// A coin whose contract will not say its decimals is left out, because a
/// balance or an amount read against guessed decimals is a wrong number.
pub fn book(listed: &Value, rpc: &dyn Rpc) -> Vec<Coin> {
    book_and_unread(listed, rpc).0
}

/// [`book`], and the symbols it left out because the chain did not answer for
/// them — so a wallet can say a coin is missing because nobody could look,
/// rather than show a list that is quietly one coin short.
pub fn book_and_unread(listed: &Value, rpc: &dyn Rpc) -> (Vec<Coin>, Vec<String>) {
    let mut out = vec![Coin::eth()];
    let mut unread = Vec::new();
    let rows = listed.get("coins").and_then(Value::as_array).cloned().unwrap_or_default();
    for row in rows {
        let Some(addr) = row.get("addr").and_then(Value::as_str) else { continue };
        let Ok(token) = elo_vault::parse_address(addr) else { continue };
        if out.iter().any(|c| c.token == Some(token)) {
            continue;
        }
        let listed_sym = row.get("sym").and_then(Value::as_str).unwrap_or("").to_string();
        let Ok(decimals) = chain::token_decimals(rpc, &token) else {
            unread.push(if listed_sym.is_empty() { addr.to_string() } else { listed_sym });
            continue;
        };
        let sym = chain::token_symbol(rpc, &token).ok().flatten().unwrap_or(listed_sym);
        if sym.is_empty() {
            continue;
        }
        let what = row
            .get("game")
            .and_then(|g| g.get("name"))
            .and_then(Value::as_str)
            .map(|n| format!("{n}'s coin"))
            .unwrap_or_else(|| "the platform's coin".to_string());
        out.push(Coin { sym, token: Some(token), decimals, what });
    }
    (out, unread)
}

/// Everything the wallet shows for one address, read in one go so it can be
/// read off the window's thread.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub coins: Vec<(Coin, Result<u128, String>)>,
    /// [`own_warning`], if the account needs one.
    pub warning: Option<String>,
    /// Why only ETH is listed, when the origin's coin list did not load.
    pub note: Option<String>,
}

impl Snapshot {
    /// No coin got a real answer: show "could not look", never an empty wallet.
    pub fn unreachable(&self) -> bool {
        self.coins.iter().all(|(_, h)| h.is_err())
    }
}

/// `listed` is the origin's `/api/holdings` answer, or `None` if it did not
/// load (ETH is still read).
pub fn snapshot(rpc: &dyn Rpc, listed: Result<&Value, &str>, who: &[u8; 20]) -> Snapshot {
    let (coins, note) = match listed {
        Ok(v) => match book_and_unread(v, rpc) {
            (coins, unread) if unread.is_empty() => (coins, None),
            (coins, unread) => {
                // The origin's text, so only the letters of a ticker survive.
                let names = unread.iter()
                    .map(|s| s.chars().filter(char::is_ascii_alphanumeric).take(16).collect::<String>())
                    .collect::<Vec<_>>().join(", ");
                (coins, Some(format!("the chain did not answer for {names}, so it is not shown — Refresh tries again")))
            }
        },
        Err(why) => (vec![Coin::eth()], Some(format!("the coin list did not load ({why}) — ETH only"))),
    };
    let coins = coins.into_iter().map(|c| { let h = held(rpc, &c, who); (c, h) }).collect();
    Snapshot { coins, warning: own_warning(rpc, who), note }
}

/// What one address holds of one coin. `Err` is "could not read", never zero.
pub fn held(rpc: &dyn Rpc, coin: &Coin, who: &[u8; 20]) -> Result<u128, String> {
    match coin.token {
        None => chain::balance(rpc, who),
        Some(t) => chain::token_balance(rpc, &t, who),
    }
}

/// `12.5` → `12500000000000000000` at 18 decimals.
///
/// Strict on purpose: no sign, no exponent, no thousands separators (a comma
/// is a decimal point in half the world), and no more decimal places than the
/// coin has — refused rather than rounded, so the amount sent is the amount
/// typed.
pub fn parse_amount(typed: &str, decimals: u8) -> Result<u128, String> {
    let t = typed.trim();
    let (whole, frac) = t.split_once('.').unwrap_or((t, ""));
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if (whole.is_empty() && frac.is_empty()) || !digits(whole) || !digits(frac) {
        return Err("Type an amount as digits, with a dot for decimals: 12.5".into());
    }
    if frac.len() > decimals as usize {
        return Err(format!("This coin has {decimals} decimal places; that amount has more."));
    }
    let too_big = || "That amount is larger than any balance.".to_string();
    let scale = 10u128.checked_pow(decimals as u32).ok_or_else(too_big)?;
    let w: u128 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| too_big())? };
    let f: u128 = if frac.is_empty() {
        0
    } else {
        let pad = 10u128.pow((decimals as usize - frac.len()) as u32);
        frac.parse::<u128>().map_err(|_| too_big())?.checked_mul(pad).ok_or_else(too_big)?
    };
    w.checked_mul(scale).and_then(|x| x.checked_add(f)).ok_or_else(too_big)
}

/// The exact amount, with trailing zeros dropped. Never rounded: a wallet
/// that shows 0.00 for a balance of dust has hidden money.
pub fn format_amount(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let s = format!("{raw:0>width$}", width = decimals as usize + 1);
    let (whole, frac) = s.split_at(s.len() - decimals as usize);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{frac}")
    }
}

/// An address in groups of four after the `0x`, so two can be compared a
/// block at a time instead of by skimming the ends.
pub fn grouped(addr: &str) -> String {
    let body = addr.strip_prefix("0x").unwrap_or(addr);
    let groups: Vec<&str> = (0..body.len()).step_by(4).map(|i| &body[i..(i + 4).min(body.len())]).collect();
    format!("0x {}", groups.join(" "))
}

/// A send that passed every check that needs no key, ready to review.
#[derive(Debug, Clone)]
pub struct Plan {
    pub coin: Coin,
    pub from: [u8; 20],
    pub to: [u8; 20],
    pub amount: u128,
    pub tx: Tx,
    /// Things the holder should read before pressing Send. Not refusals: a
    /// send to a contract may be exactly what they meant.
    pub notes: Vec<String>,
}

/// How much headroom over the chain's gas estimate. An estimate is made
/// against the state now; the margin is what keeps a send from failing on a
/// small change before it lands, and only gas actually used is paid.
const GAS_MARGIN_PCT: u64 = 25;

/// Check everything that can be checked without a key, and build the
/// transaction. `known` is the addresses this launcher has sent to before.
pub fn plan(
    rpc: &dyn Rpc,
    from: &[u8; 20],
    coin: &Coin,
    to_typed: &str,
    amount_typed: &str,
    known: &[[u8; 20]],
) -> Result<Plan, String> {
    let to = elo_vault::parse_address(to_typed)?;
    if to == [0u8; 20] {
        return Err("That is the zero address. Anything sent there is gone for good.".into());
    }
    if coin.token == Some(to) {
        return Err(format!(
            "That is the {} contract's own address. Tokens sent to it are lost; \
             send to a person's wallet instead.", coin.sym));
    }
    let amount = parse_amount(amount_typed, coin.decimals)?;
    if amount == 0 {
        return Err("Type an amount above zero.".into());
    }
    let id = chain::chain_id(rpc)?;
    if id != CHAIN_ID {
        return Err(format!(
            "The RPC this launcher is pointed at is chain {id}, not {} ({CHAIN_ID}). \
             Nothing was signed.", chain::NAME));
    }

    let (tx_to, value, data) = match coin.token {
        None => (to, amount, Vec::new()),
        Some(t) => (t, 0, tx::erc20_transfer(&to, amount)),
    };
    let have = held(rpc, coin, from)?;
    if have < amount {
        return Err(format!(
            "You hold {} {}, less than that.", format_amount(have, coin.decimals), coin.sym));
    }
    let estimate = chain::estimate_gas(rpc, from, &tx_to, value, &data)
        .map_err(|e| format!("The chain says this send would fail, so nothing was signed. {e}"))?;
    let gas = estimate.saturating_add(estimate * GAS_MARGIN_PCT / 100);
    let (max_fee, max_priority_fee) = chain::fees(rpc)?;
    let nonce = chain::nonce(rpc, from)?;
    let tx = Tx { nonce, max_priority_fee, max_fee, gas, to: tx_to, value, data };

    let eth = if coin.token.is_none() { have } else { chain::balance(rpc, from)? };
    let need = tx.max_fee_total().saturating_add(value);
    if eth < need {
        return Err(format!(
            "Not enough ETH for this and its fee: it can cost up to {} ETH in all, and \
             you hold {} ETH.", format_amount(need, 18), format_amount(eth, 18)));
    }

    let mut notes = Vec::new();
    let lookalike = known.iter().find(|k| **k != to && looks_like(k, &to));
    if let Some(k) = lookalike {
        notes.push(format!(
            "⚠ This address looks like {}, which you have sent to before, but it is a \
             different address. Scammers plant lookalikes in your history. Do not send \
             unless you checked every character.",
            elo_vault::to_checksum(k)));
    } else if !known.contains(&to) {
        notes.push("You have not sent to this address from this launcher before. Check \
                    every character against where you got it."
            .into());
    }
    if to == *from {
        notes.push("This is your own address.".into());
    }
    match chain::holder(rpc, &to) {
        Ok(chain::Holder::Contract) => notes.push(
            "The recipient is a contract, not a person's wallet. Only send if you meant to.".into()),
        Ok(chain::Holder::Delegated(c)) => notes.push(format!(
            "The recipient is a wallet that has handed control to a contract ({}, EIP-7702). \
             That contract can move anything sent there.", elo_vault::to_checksum(&c))),
        _ => {}
    }
    if let Some(w) = own_warning(rpc, from) {
        notes.push(w);
    }
    Ok(Plan { coin: coin.clone(), from: *from, to, amount, tx, notes })
}

/// A warning about this launcher's own account, if it needs one.
///
/// This launcher only ever signs EIP-191 messages and type-2 transactions; it
/// never signs an EIP-7702 authorization. So a delegation on one of our
/// accounts was signed somewhere else: a wallet's own "smart account" switch
/// (MetaMask has one), or a phishing page, and the contract it names can move
/// everything. Only the holder knows which, so this says both.
pub fn own_warning(rpc: &dyn Rpc, who: &[u8; 20]) -> Option<String> {
    match chain::holder(rpc, who) {
        Ok(chain::Holder::Delegated(c)) => Some(format!(
            "⚠ This account has handed control to a contract ({}, EIP-7702), which can \
             move everything in it. Some wallets do this when you turn on a smart account; \
             drainer sites do it too. This launcher never signs one. If you did not turn it \
             on yourself, make a new account and send everything to it now.",
            elo_vault::to_checksum(&c))),
        _ => None,
    }
}

/// Same first four and last four hex characters: what a glance at an address
/// checks, and what an address-poisoning lookalike is made to match.
fn looks_like(a: &[u8; 20], b: &[u8; 20]) -> bool {
    a[..2] == b[..2] && a[18..] == b[18..]
}

/// The review screen's lines, read out of the transaction itself.
pub fn review(plan: &Plan) -> Result<Vec<(String, String)>, String> {
    let (to, amount) = match plan.coin.token {
        None => (plan.tx.to, plan.tx.value),
        Some(t) => {
            if plan.tx.to != t || plan.tx.value != 0 {
                return Err("this transaction is not a plain token transfer".into());
            }
            tx::read_erc20_transfer(&plan.tx.data).ok_or("this transaction is not a plain token transfer")?
        }
    };
    if to != plan.to || amount != plan.amount {
        return Err("the transaction does not match what was typed — nothing was signed".into());
    }
    let to_s = elo_vault::to_checksum(&to);
    Ok(vec![
        ("Send".into(), format!("{} {}", format_amount(amount, plan.coin.decimals), plan.coin.sym)),
        ("To".into(), grouped(&to_s)),
        // Your own address is hidden on stream like everywhere else; the
        // recipient never is, because checking it is what this screen is for.
        ("From".into(), elo_depot::streamer::shown(&elo_vault::to_checksum(&plan.from))),
        ("Network".into(), format!("{} ({CHAIN_ID})", chain::NAME)),
        ("Fee".into(), format!("at most {} ETH", format_amount(plan.tx.max_fee_total(), 18))),
    ])
}

/// Addresses this launcher has sent to, oldest first. A missing or unreadable
/// file is an empty history; the cost is a "first time" note, never a send.
pub fn known_recipients(path: &Path) -> Vec<[u8; 20]> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("sent_to").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|a| elo_vault::parse_address(a).ok())
        .collect()
}

/// Remember an address after a send to it went out. Public data only, kept
/// beside the keystore at 0600 like the pairing.
pub fn remember_recipient(path: &Path, to: &[u8; 20]) -> Result<(), String> {
    let mut all = known_recipients(path);
    if all.contains(to) {
        return Ok(());
    }
    all.push(*to);
    let list: Vec<String> = all.iter().map(|a| elo_vault::to_checksum(a)).collect();
    // Merged, not replaced: the same file keeps the Solana list
    // (`solana::remember_recipient`), and a send on one chain must not
    // forget the other chain's history.
    let mut doc: Value = std::fs::read_to_string(path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    doc["sent_to"] = serde_json::json!(list);
    elo_vault::write_private(path, doc.to_string().as_bytes(), false).map_err(|e| e.to_string())
}
