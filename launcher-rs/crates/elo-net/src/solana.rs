//! Solana, read directly: balances, and what a send needs — the same account's
//! other chain, the way Phantom shows it.
//!
//! # The same rules as `chain` and `wallet`, for the other chain
//!
//! Every number a send is built from is the chain's own answer over its public
//! JSON-RPC (`ELO_SOLANA_RPC` overrides the endpoint), never the origin's. The
//! origin is asked only which coins are ours, so they can be named. A read that
//! failed is `Err`, never a zero.
//!
//! # A Solana transaction is simulated before it is shown
//!
//! A Solana transaction is opaque in a way an ERC-20 transfer is not, so the
//! wallet does what Phantom does: it asks the chain to run the transaction
//! without landing it (`simulateTransaction`) and reads back what this
//! account's SOL and coins would be afterwards. A send's review and a move's
//! review are built from that answer, and a transaction that would take more
//! than the review says is refused before the passphrase is asked.
//!
//! This file holds no key and signs nothing.

use crate::chain::{HttpRpc, Rpc};
use crate::wallet::format_amount;
use elo_vault::solana::{self as sol, Transaction};
use serde_json::{json, Value};

/// Solana's public mainnet RPC. `ELO_SOLANA_RPC` overrides it.
pub const RPC: &str = "https://api.mainnet-beta.solana.com";
/// Where a transaction or an address can be looked at by anyone.
pub const EXPLORER: &str = "https://solscan.io";
pub const NAME: &str = sol::NAME;

pub fn rpc_url() -> String {
    std::env::var("ELO_SOLANA_RPC").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| RPC.to_string())
}

/// A Solana RPC over this client's own HTTP stack.
pub fn http() -> HttpRpc {
    HttpRpc { net: crate::Net::new(), url: rpc_url() }
}

pub fn tx_url(sig: &str) -> String {
    format!("{EXPLORER}/tx/{sig}")
}

pub fn address_url(addr: &str) -> String {
    format!("{EXPLORER}/account/{addr}")
}

fn b58(k: &[u8; 32]) -> String {
    sol::base58_encode(k)
}

fn key_of(v: &Value) -> Option<[u8; 32]> {
    sol::parse_address(v.as_str()?).ok()
}

/// Lamports held by an address. A missing account is zero lamports — that is
/// the chain's own answer, not a failed read.
pub fn balance(rpc: &dyn Rpc, who: &[u8; 32]) -> Result<u64, String> {
    let r = rpc.call("getBalance", json!([b58(who), {"commitment": "confirmed"}]))?;
    r.get("value").and_then(Value::as_u64).ok_or_else(|| "the Solana RPC answered oddly".into())
}

/// One coin a wallet holds, in one token account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holding {
    pub mint: [u8; 32],
    /// SPL Token or Token-2022 — whichever owns the mint.
    pub program: [u8; 32],
    pub account: [u8; 32],
    pub amount: u64,
    pub decimals: u8,
}

/// Every token account a wallet owns, under both token programs.
pub fn holdings(rpc: &dyn Rpc, who: &[u8; 32]) -> Result<Vec<Holding>, String> {
    let mut out = Vec::new();
    for prog in [sol::TOKEN_PROGRAM, sol::TOKEN_2022_PROGRAM] {
        let r = rpc.call("getTokenAccountsByOwner", json!([b58(who), {"programId": prog},
                                                          {"encoding": "jsonParsed", "commitment": "confirmed"}]))?;
        let rows = r.get("value").and_then(Value::as_array).ok_or("the Solana RPC answered oddly")?;
        for row in rows {
            let Some(account) = row.get("pubkey").and_then(key_of) else { continue };
            if out.iter().any(|h: &Holding| h.account == account) {
                continue;
            }
            let info = row.pointer("/account/data/parsed/info");
            let Some(mint) = info.and_then(|i| i.get("mint")).and_then(key_of) else { continue };
            let ta = info.and_then(|i| i.get("tokenAmount"));
            let Some(amount) = ta.and_then(|t| t.get("amount")).and_then(Value::as_str)
                .and_then(|a| a.parse::<u64>().ok()) else { continue };
            let Some(decimals) = ta.and_then(|t| t.get("decimals")).and_then(Value::as_u64) else { continue };
            out.push(Holding { mint, program: sol::program(prog), account, amount, decimals: decimals as u8 });
        }
    }
    Ok(out)
}

/// Who owns an account, and whether it is a program. `None`: it does not exist
/// yet (a fresh wallet nobody has paid).
pub fn owner_of(rpc: &dyn Rpc, key: &[u8; 32]) -> Result<Option<([u8; 32], bool)>, String> {
    let r = rpc.call("getAccountInfo", json!([b58(key), {"encoding": "base64",
                                                        "dataSlice": {"offset": 0, "length": 0},
                                                        "commitment": "confirmed"}]))?;
    let v = r.get("value").ok_or("the Solana RPC answered oddly")?;
    if v.is_null() {
        return Ok(None);
    }
    let owner = v.get("owner").and_then(key_of).ok_or("the Solana RPC answered oddly")?;
    let exec = v.get("executable").and_then(Value::as_bool).unwrap_or(false);
    Ok(Some((owner, exec)))
}

/// The lamports an account of `size` bytes needs to exist.
pub fn rent(rpc: &dyn Rpc, size: usize) -> Result<u64, String> {
    rpc.call("getMinimumBalanceForRentExemption", json!([size]))?
        .as_u64().ok_or_else(|| "the Solana RPC answered oddly".into())
}

pub fn latest_blockhash(rpc: &dyn Rpc) -> Result<[u8; 32], String> {
    let r = rpc.call("getLatestBlockhash", json!([{"commitment": "confirmed"}]))?;
    r.pointer("/value/blockhash").and_then(key_of).ok_or_else(|| "the Solana RPC answered oddly".into())
}

/// Send a signed transaction. Returns the signature the chain names.
pub fn send(rpc: &dyn Rpc, tx: &Transaction) -> Result<String, String> {
    let r = rpc.call("sendTransaction", json!([sol::b64_encode(&tx.to_bytes()),
                                               {"encoding": "base64", "preflightCommitment": "confirmed"}]))?;
    r.as_str().map(str::to_string).ok_or_else(|| "the Solana RPC answered oddly".into())
}

/// Where a sent transaction stands: `Ok(None)` not seen yet, `Ok(Some(level))`
/// processed/confirmed/finalized, `Err` when it failed on chain.
pub fn status(rpc: &dyn Rpc, sig: &str) -> Result<Option<String>, String> {
    let r = rpc.call("getSignatureStatuses", json!([[sig], {"searchTransactionHistory": true}]))?;
    let s = r.pointer("/value/0").cloned().unwrap_or(Value::Null);
    if s.is_null() {
        return Ok(None);
    }
    if let Some(e) = s.get("err").filter(|e| !e.is_null()) {
        return Err(format!("it failed on chain: {e}"));
    }
    Ok(s.get("confirmationStatus").and_then(Value::as_str).map(str::to_string))
}

/// Wait until a transaction is confirmed, up to about a minute.
pub fn wait(rpc: &dyn Rpc, sig: &str) -> Result<(), String> {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(75) {
        if let Some(level) = status(rpc, sig)? {
            if level == "confirmed" || level == "finalized" {
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    Err(format!("not confirmed within a minute; look it up: {}", tx_url(sig)))
}

/// What a simulated transaction would leave this wallet with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct After {
    /// Lamports after, or `None` if the account would be closed.
    pub lamports: Option<u64>,
    /// Each watched token account's amount after (`None`: closed).
    pub tokens: Vec<([u8; 32], Option<u64>)>,
    /// The fee the chain would charge, in lamports, when it says.
    pub fee: Option<u64>,
    /// Lamports the fee payer would lose, from the simulation's own pre- and
    /// post-balances — one atomic answer, so a deposit landing between two
    /// reads cannot hide a cost. `None` from an RPC that does not report them.
    pub spent: Option<u64>,
    /// Every token account the transaction touches that `me` owns, as
    /// (mint, amount before, amount after), from the simulation's token
    /// balances. `None` from an RPC that does not report them.
    pub owned: Option<Vec<([u8; 32], u64, u64)>>,
}

/// Run a transaction without landing it and read back `me` and `watch`.
/// `Err` carries the chain's own reason when it would fail.
pub fn simulate(rpc: &dyn Rpc, tx: &Transaction, me: &[u8; 32], watch: &[[u8; 32]]) -> Result<After, String> {
    let mut addrs = vec![b58(me)];
    addrs.extend(watch.iter().map(b58));
    let r = rpc.call("simulateTransaction", json!([sol::b64_encode(&tx.to_bytes()), {
        "encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": true,
        "commitment": "confirmed", "accounts": {"encoding": "base64", "addresses": addrs}}]))?;
    let v = r.get("value").ok_or("the Solana RPC answered oddly")?;
    if let Some(e) = v.get("err").filter(|e| !e.is_null()) {
        let logs: Vec<&str> = v.get("logs").and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let last = logs.iter().rev().find(|l| l.contains("rror") || l.contains("insufficient"))
            .or(logs.last()).copied().unwrap_or("");
        return Err(format!("Solana says it would fail: {e} {last}").trim().to_string());
    }
    let accts = v.get("accounts").and_then(Value::as_array).ok_or("the simulation named no accounts")?;
    let lamports = accts.first().and_then(|a| a.get("lamports")).and_then(Value::as_u64);
    let tokens = watch.iter().enumerate().map(|(i, k)| {
        let amt = accts.get(i + 1).and_then(|a| a.pointer("/data/0")).and_then(Value::as_str)
            .and_then(|d| sol::b64_decode(d).ok())
            .and_then(|d| d.get(64..72).map(|b| u64::from_le_bytes(b.try_into().unwrap())));
        (*k, amt)
    }).collect();
    let fee = v.get("fee").and_then(Value::as_u64);
    let bal = |k: &str| v.get(k).and_then(Value::as_array)
        .and_then(|a| a.first()).and_then(Value::as_u64);
    // the fee payer is always key 0
    let spent = match (bal("preBalances"), bal("postBalances")) {
        (Some(pre), Some(post)) => Some(pre.saturating_sub(post)),
        _ => None,
    };
    let owned = match (v.get("preTokenBalances").and_then(Value::as_array),
                       v.get("postTokenBalances").and_then(Value::as_array)) {
        (Some(pre), Some(post)) => {
            let mine = |rows: &Vec<Value>| -> Vec<(u64, [u8; 32], u64)> {
                rows.iter().filter(|r| r.get("owner").and_then(key_of) == Some(*me))
                    .filter_map(|r| Some((r.get("accountIndex")?.as_u64()?, key_of(r.get("mint")?)?,
                                          r.pointer("/uiTokenAmount/amount")?.as_str()?.parse().ok()?)))
                    .collect()
            };
            let (pre, post) = (mine(pre), mine(post));
            let mut out: Vec<([u8; 32], u64, u64)> = Vec::new();
            for (i, mint, amt) in &pre {
                let after = post.iter().find(|(j, _, _)| j == i).map(|(_, _, a)| *a).unwrap_or(0);
                out.push((*mint, *amt, after));
            }
            for (i, mint, amt) in &post {
                if !pre.iter().any(|(j, _, _)| j == i) {
                    out.push((*mint, 0, *amt));
                }
            }
            Some(out)
        }
        _ => None,
    };
    Ok(After { lamports, tokens, fee, spent, owned })
}

/// Every account a transaction may write: its writable static keys, and the
/// writable addresses its lookup tables load (each table read from chain).
pub fn writable_accounts(rpc: &dyn Rpc, tx: &Transaction) -> Result<Vec<[u8; 32]>, String> {
    let m = &tx.message;
    let mut out: Vec<[u8; 32]> = (0..m.keys.len()).filter(|i| m.is_writable(*i)).map(|i| m.keys[i]).collect();
    for (table, writable, _) in &m.lookups {
        if writable.is_empty() {
            continue;
        }
        let r = rpc.call("getAccountInfo", json!([b58(table), {"encoding": "base64", "commitment": "confirmed"}]))?;
        let data = r.pointer("/value/data/0").and_then(Value::as_str)
            .ok_or("a lookup table this transaction uses could not be read")?;
        let addrs = sol::lookup_table_addresses(&sol::b64_decode(data)?);
        for i in writable {
            out.push(*addrs.get(*i as usize).ok_or("a lookup table index points past its table")?);
        }
    }
    Ok(out)
}

// ── the wallet's Solana half ─────────────────────────────────────────────────

/// A coin on Solana. `mint: None` is SOL, which pays every fee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolCoin {
    pub sym: String,
    pub mint: Option<[u8; 32]>,
    pub program: [u8; 32],
    pub decimals: u8,
    /// Where it is held (the wallet itself for SOL).
    pub account: [u8; 32],
    pub what: String,
}

/// Our Solana coins from the origin's pairs (`chain: "solana"` rows), as
/// mint → symbol. Empty until one has a record, which is the held rule.
pub fn ours(pairs: &Value) -> Vec<([u8; 32], String)> {
    let rows = pairs.get("pairs").and_then(Value::as_array).or_else(|| pairs.as_array());
    rows.into_iter().flatten()
        .filter(|r| r.get("chain").and_then(Value::as_str) == Some("solana"))
        .filter_map(|r| Some((key_of(r.get("base_addr")?)?, r.get("base")?.as_str()?.to_string())))
        .collect()
}

/// Everything the Solana half shows, read in one go.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub sol: Result<u64, String>,
    pub coins: Result<Vec<(SolCoin, u64)>, String>,
}

impl Snapshot {
    pub fn unreachable(&self) -> bool {
        self.sol.is_err() && self.coins.is_err()
    }
}

pub fn sol_coin(me: &[u8; 32]) -> SolCoin {
    SolCoin { sym: "SOL".into(), mint: None, program: sol::SYSTEM_PROGRAM, decimals: 9, account: *me,
              what: "pays every fee on Solana".into() }
}

/// SOL, then every coin with a balance: ours named, any other shown by its
/// mint (Phantom shows those too; a coin nobody asked for is often spam).
pub fn snapshot(rpc: &dyn Rpc, me: &[u8; 32], ours: &[([u8; 32], String)]) -> Snapshot {
    let sol_bal = balance(rpc, me);
    let coins = holdings(rpc, me).map(|hs| {
        let mut out: Vec<(SolCoin, u64)> = Vec::new();
        for h in hs.into_iter().filter(|h| h.amount > 0) {
            let named = ours.iter().find(|(m, _)| *m == h.mint).map(|(_, s)| s.clone());
            let short = b58(&h.mint);
            let (sym, what) = match named {
                Some(s) => (s, "ours, on Solana".to_string()),
                None => (format!("{}…", &short[..4]),
                         format!("a coin we do not list ({}…{})", &short[..4], &short[short.len() - 4..])),
            };
            out.push((SolCoin { sym, mint: Some(h.mint), program: h.program, decimals: h.decimals,
                                account: h.account, what }, h.amount));
        }
        out
    }).map_err(|e| e.to_string());
    Snapshot { sol: sol_bal, coins }
}

/// A Solana send that passed every check that needs no key.
#[derive(Debug, Clone)]
pub struct Plan {
    pub coin: SolCoin,
    pub from: [u8; 32],
    pub to: [u8; 32],
    pub amount: u64,
    pub tx: Transaction,
    /// The fee plus any account the send has to open, in lamports.
    pub cost: u64,
    pub notes: Vec<String>,
}

/// The fee floor: one signature.
const FEE: u64 = 5_000;

/// Check everything that can be checked without a key, build the transaction,
/// and simulate it. `known` is the Solana addresses this launcher sent to.
pub fn plan(rpc: &dyn Rpc, from: &[u8; 32], coin: &SolCoin, to_typed: &str, amount_typed: &str,
            known: &[[u8; 32]]) -> Result<Plan, String> {
    let to = sol::parse_address(to_typed)?;
    if to == sol::SYSTEM_PROGRAM {
        return Err("That is the System Program's address. Anything sent there is gone for good.".into());
    }
    if Some(to) == coin.mint {
        return Err(format!("That is the {} mint itself. Coins sent there are lost; send to a \
                            person's wallet instead.", coin.sym));
    }
    let amount: u64 = crate::wallet::parse_amount(amount_typed, coin.decimals)?
        .try_into().map_err(|_| "That amount is larger than any balance.".to_string())?;
    if amount == 0 {
        return Err("Type an amount above zero.".into());
    }
    let mut notes = Vec::new();
    let recipient = owner_of(rpc, &to)?;
    match recipient {
        Some((_, true)) => return Err("That address is a program, not a wallet. Nothing was signed.".into()),
        Some((owner, _)) if owner == sol::program(sol::TOKEN_PROGRAM)
            || owner == sol::program(sol::TOKEN_2022_PROGRAM) => return Err(
            "That address is a token account, not a wallet. Ask for their wallet address — the \
             one their wallet shows to receive.".into()),
        Some((owner, _)) if owner != sol::SYSTEM_PROGRAM => notes.push(
            "The recipient is an account a program owns, not a plain wallet. Only send if you meant to.".into()),
        _ => {}
    }
    if !sol::is_on_curve(&to) {
        notes.push("⚠ This address has no private key (a program address). Only send if the app you are \
                    paying told you to use it.".into());
    }

    let sol_have = balance(rpc, from)?;
    let mut cost = FEE;
    let mut token_have = None;
    let ixs = match coin.mint {
        None => {
            if recipient.is_none() {
                let min = rent(rpc, 0)?;
                if amount < min {
                    return Err(format!("A Solana address nobody has paid yet needs at least {} SOL to exist, \
                                        so send at least that much.", format_amount(min as u128, 9)));
                }
            }
            vec![sol::system_transfer(from, &to, amount)]
        }
        Some(mint) => {
            let have = holdings(rpc, from)?.into_iter().filter(|h| h.mint == mint && h.account == coin.account)
                .map(|h| h.amount).sum::<u64>();
            if have < amount {
                return Err(format!("You hold {} {}, less than that.",
                                   format_amount(have as u128, coin.decimals), coin.sym));
            }
            token_have = Some(have);
            let dest = sol::associated_token_address(&to, &mint, &coin.program);
            let mut ixs = Vec::new();
            if owner_of(rpc, &dest)?.is_none() {
                // 165 bytes is a plain token account; Token-2022 coins can carry
                // more, and the simulation below prices the real one.
                let r = rent(rpc, 165)?;
                cost += r;
                notes.push(format!("This is their first {} — about {} SOL opens a token account for it, \
                                    paid by you, as every Solana wallet does.", coin.sym,
                                   format_amount(r as u128, 9)));
                ixs.push(sol::create_ata_idempotent(from, &to, &mint, &coin.program));
            }
            ixs.push(sol::transfer_checked(&coin.account, &mint, &dest, from, amount, coin.decimals,
                                           &coin.program));
            ixs
        }
    };
    let need = cost + if coin.mint.is_none() { amount } else { 0 };
    if sol_have < need {
        return Err(format!("Not enough SOL for this and its fee: it costs about {} SOL in all, and you \
                            hold {} SOL.", format_amount(need as u128, 9), format_amount(sol_have as u128, 9)));
    }
    let blockhash = latest_blockhash(rpc)?;
    let tx = Transaction::unsigned(sol::compile(from, &ixs, &blockhash))?;
    let watch: Vec<[u8; 32]> = coin.mint.map(|_| vec![coin.account]).unwrap_or_default();
    let after = simulate(rpc, &tx, from, &watch)
        .map_err(|e| format!("{e}\n\nNothing was signed."))?;
    let spent = after.spent.unwrap_or_else(|| sol_have.saturating_sub(after.lamports.unwrap_or(0)));
    // The simulation is the check: it may move this wallet's SOL by the amount
    // and the fee and rent, and no more.
    if spent > need + FEE {
        return Err(format!("The chain says this would take {} SOL from you, more than the {} the send \
                            should. Nothing was signed.", format_amount(spent as u128, 9),
                           format_amount(need as u128, 9)));
    }
    if let (Some(have), Some(mint)) = (token_have, coin.mint) {
        let moved = match &after.owned {
            Some(rows) => rows.iter().filter(|(m, _, _)| *m == mint)
                .map(|(_, a, b)| a.saturating_sub(*b)).sum::<u64>(),
            None => have.saturating_sub(after.tokens.first().and_then(|(_, a)| *a).unwrap_or(0)),
        };
        if moved != amount {
            return Err(format!("The chain says this would move a different amount of {} than you typed. \
                                Nothing was signed.", coin.sym));
        }
    }

    if let Some(k) = known.iter().find(|k| **k != to && looks_like(k, &to)) {
        notes.insert(0, format!("⚠ This address looks like {}, which you have sent to before, but it is a \
                                 different address. Scammers plant lookalikes in your history. Do not send \
                                 unless you checked every character.", b58(k)));
    } else if !known.contains(&to) {
        notes.push("You have not sent to this address from this launcher before. Check every character \
                    against where you got it.".into());
    }
    if to == *from {
        notes.push("This is your own address.".into());
    }
    Ok(Plan { coin: coin.clone(), from: *from, to, amount, tx, cost, notes })
}

/// Same first four and last four characters: what a glance checks, and what
/// an address-poisoning lookalike is made to match.
fn looks_like(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let (x, y) = (b58(a), b58(b));
    x.get(..4) == y.get(..4) && x.get(x.len() - 4..) == y.get(y.len() - 4..)
}

/// The review's lines, read back out of the transaction: the recipient and the
/// amount come from its instructions, not from the form.
pub fn review(plan: &Plan) -> Result<Vec<(String, String)>, String> {
    let m = &plan.tx.message;
    let last = m.instructions.len().checked_sub(1).ok_or("an empty transaction")?;
    let (prog, accts, data) = &m.instructions[last];
    let key = |i: u8| m.keys.get(i as usize).copied();
    let (to, amount) = match plan.coin.mint {
        None => {
            if m.keys.get(*prog as usize) != Some(&sol::SYSTEM_PROGRAM) || data.len() != 12
                || data[..4] != 2u32.to_le_bytes() {
                return Err("this transaction is not a plain SOL transfer".into());
            }
            (key(*accts.get(1).ok_or("malformed")?).ok_or("malformed")?,
             u64::from_le_bytes(data[4..12].try_into().unwrap()))
        }
        Some(mint) => {
            if m.keys.get(*prog as usize) != Some(&plan.coin.program) || data.len() != 10 || data[0] != 12 {
                return Err("this transaction is not a plain coin transfer".into());
            }
            let dest = key(*accts.get(2).ok_or("malformed")?).ok_or("malformed")?;
            if dest != sol::associated_token_address(&plan.to, &mint, &plan.coin.program) {
                return Err("the transaction does not pay the address that was typed — nothing was signed".into());
            }
            (plan.to, u64::from_le_bytes(data[1..9].try_into().unwrap()))
        }
    };
    if to != plan.to || amount != plan.amount {
        return Err("the transaction does not match what was typed — nothing was signed".into());
    }
    Ok(vec![
        ("Send".into(), format!("{} {}", format_amount(amount as u128, plan.coin.decimals), plan.coin.sym)),
        ("To".into(), b58(&to)),
        ("From".into(), elo_depot::streamer::shown(&b58(&plan.from))),
        ("Network".into(), NAME.into()),
        ("Fee".into(), format!("about {} SOL", format_amount(plan.cost as u128, 9))),
    ])
}

/// Solana addresses this launcher has sent to (`recipients.json`'s
/// `sol_sent_to`, beside the Ethereum list).
pub fn known_recipients(path: &std::path::Path) -> Vec<[u8; 32]> {
    std::fs::read_to_string(path).ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("sol_sent_to").and_then(Value::as_array).cloned())
        .unwrap_or_default().iter().filter_map(key_of).collect()
}

pub fn remember_recipient(path: &std::path::Path, to: &[u8; 32]) -> Result<(), String> {
    let mut doc: Value = std::fs::read_to_string(path).ok()
        .and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}));
    if !doc.is_object() {
        doc = json!({});
    }
    let mut all = known_recipients(path);
    if all.contains(to) {
        return Ok(());
    }
    all.push(*to);
    doc["sol_sent_to"] = json!(all.iter().map(b58).collect::<Vec<_>>());
    elo_vault::write_private(path, doc.to_string().as_bytes(), false).map_err(|e| e.to_string())
}
