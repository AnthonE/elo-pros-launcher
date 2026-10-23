//! Trading for the desktop app, shared by `elo trade` and the window's Orders:
//! the origin's trading API read, its prepared steps checked and signed, and
//! one tick of the resting orders.
//!
//! **What signs.** Nothing here holds a key. A caller hands `execute` a `sign`
//! closure (the local keystore, unlocked by the caller however it chooses —
//! the order runner keeps its passphrase in `elo_vault::masked::Masked`). Every
//! step goes through [`crate::guard::decode`] before that closure is called: a
//! step outside the ecosystem is refused (or, with `allow_outside`, warned),
//! and every step — sent, skipped, refused — is written to the activity log.
//! `estimate_gas` is the check before signing: a step the chain expects to fail
//! is never signed.

use crate::chain::{self, HttpRpc, Rpc};
use crate::guard::{self, Settings};
use crate::orders::{self, Action, Order};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

const PERMIT2: &str = "0x000000000022d473030f116ddee9f6b43ac78ba3";

pub type Sign<'a> = &'a dyn Fn(&elo_vault::tx::Tx) -> Result<elo_vault::tx::Signed, String>;

pub fn q(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

/// One read of the origin's API, its own error sentence when it refuses.
pub fn api(rpc: &HttpRpc, host: &str, path: &str) -> Result<Value, String> {
    let got = rpc.net.get_json(&format!("{host}/api{path}"));
    match got.value {
        Some(v) => match v.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None => Ok(v),
        },
        None => Err(format!("the origin did not answer: {}", got.why)),
    }
}

pub fn unhex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim_start_matches("0x");
    if s.len() % 2 != 0 {
        return Err("odd-length hex from the origin".into());
    }
    (0..s.len()).step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "bad hex from the origin".to_string()))
        .collect()
}

fn word_at(data: &[u8], i: usize) -> Option<&[u8]> {
    data.get(4 + 32 * i..4 + 32 * (i + 1))
}
fn as_u128(w: &[u8]) -> u128 {
    // amounts this client trades fit in 128 bits; a larger word reads as "a lot"
    if w[..16].iter().any(|b| *b != 0) { u128::MAX } else { u128::from_be_bytes(w[16..32].try_into().unwrap()) }
}
fn addr_of(w: &[u8]) -> [u8; 20] {
    w[12..32].try_into().unwrap()
}

/// What an approval step asks for, read off its own calldata, and whether the
/// chain already has it — so a second trade does not pay for the same approval.
pub enum Need {
    Erc20 { token: [u8; 20], spender: [u8; 20], amount: u128 },
    Permit2 { token: [u8; 20], spender: [u8; 20], amount: u128 },
    Always,
}

pub fn need_of(to: &[u8; 20], data: &[u8]) -> Need {
    let sel = data.get(..4).map(|s| s.iter().map(|b| format!("{b:02x}")).collect::<String>()).unwrap_or_default();
    match sel.as_str() {
        "095ea7b3" => match (word_at(data, 0), word_at(data, 1)) {
            (Some(sp), Some(am)) => Need::Erc20 { token: *to, spender: addr_of(sp), amount: as_u128(am) },
            _ => Need::Always,
        },
        "87517c45" if chain::addr_param(to) == PERMIT2 => match (word_at(data, 0), word_at(data, 1), word_at(data, 2)) {
            (Some(tk), Some(sp), Some(am)) => Need::Permit2 { token: addr_of(tk), spender: addr_of(sp), amount: as_u128(am) },
            _ => Need::Always,
        },
        _ => Need::Always,
    }
}

fn already_has(rpc: &dyn Rpc, owner: &[u8; 20], need: &Need) -> bool {
    let pad = |a: &[u8; 20]| format!("{:0>64}", chain::addr_param(a).trim_start_matches("0x"));
    let call = |to: String, data: String| rpc.call("eth_call", json!([{"to": to, "data": data}, "latest"]));
    match need {
        Need::Erc20 { token, spender, amount } => call(chain::addr_param(token),
                format!("0xdd62ed3e{}{}", pad(owner), pad(spender)))
            .ok().and_then(|v| chain::word(&v).ok()).is_some_and(|have| have >= *amount),
        Need::Permit2 { token, spender, amount } => call(PERMIT2.into(),
                format!("0x927da105{}{}{}", pad(owner), pad(token), pad(spender)))
            .ok().and_then(|v| {
                let raw = unhex(v.as_str()?).ok()?;
                let have = u128::from_be_bytes(raw.get(16..32)?.try_into().ok()?);
                let exp = u64::from_be_bytes(raw.get(56..64)?.try_into().ok()?);
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
                Some(have >= *amount && exp > now + 60)
            }).unwrap_or(false),
        Need::Always => false,
    }
}

pub fn wait_receipt(rpc: &dyn Rpc, hash: &str) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(120) {
        if let Ok(r) = rpc.call("eth_getTransactionReceipt", json!([hash])) {
            if let Some(st) = r.get("status").and_then(Value::as_str) {
                return if st == "0x1" { Ok(()) } else { Err(format!("it reverted on chain: {}", chain::tx_url(hash))) };
            }
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    Err(format!("no receipt in two minutes; look it up: {}", chain::tx_url(hash)))
}

/// `/prepare/swap/exact_in` for this wallet, asking for one-time approvals when
/// the holder's settings say so.
pub fn prepare(rpc: &HttpRpc, host: &str, amount: &str, from: &str, to: &str, bps: u32,
               wallet: &[u8; 20], settings: &Settings) -> Result<Value, String> {
    api(rpc, host, &format!("/prepare/swap/exact_in?from={}&to={}&amount={}&slippage_bps={bps}&wallet={}&approve={}",
        q(from), q(to), q(amount), chain::addr_param(wallet), if settings.approve_once { "once" } else { "exact" }))
}

/// Does any step wait on a contract that is not live yet?
pub fn waits_on_router(prep: &Value) -> bool {
    prep.get("txs").and_then(Value::as_array)
        .is_some_and(|t| t.iter().any(|s| s.get("to").is_none_or(Value::is_null)))
}

/// Check, log and send the prepared steps in order. Returns the last hash.
/// `say` hears each step as it happens.
pub fn execute(prep: &Value, rpc: &HttpRpc, from: &[u8; 20], settings: &Settings, log: &Path,
               source: &str, sign: Sign, say: &dyn Fn(&str)) -> Result<String, String> {
    let steps = prep.get("txs").and_then(Value::as_array).cloned().unwrap_or_default();
    if steps.is_empty() {
        return Err("the origin prepared nothing to sign".into());
    }
    let mut last = String::new();
    for s in steps {
        let Some(to_s) = s.get("to").and_then(Value::as_str) else {
            return Err("this route opens when our router is live — nothing was sent".into());
        };
        let to = elo_vault::parse_address(to_s)?;
        let data_hex = s.get("data").and_then(Value::as_str).unwrap_or("0x");
        let data = unhex(data_hex)?;
        let value: u128 = s.get("value").and_then(Value::as_str).unwrap_or("0").parse()
            .map_err(|_| "a step's value is not a number".to_string())?;
        let why = s.get("why").and_then(Value::as_str).unwrap_or("a step").to_string();
        let v = guard::decode(&to, &data);
        let entry = json!({"source": source, "to": to_s, "value": value.to_string(),
                           "data": &data_hex[..data_hex.len().min(1200)], "what": v.what, "why": why});
        let with = |verdict: &str, extra: Value| {
            let mut e = entry.clone();
            e["verdict"] = json!(verdict);
            if let (Some(o), Some(x)) = (e.as_object_mut(), extra.as_object()) {
                o.extend(x.clone());
            }
            e
        };
        if !v.listed {
            if !settings.allow_outside {
                guard::log(log, with("refused", json!({"reason": "outside our ecosystem"})));
                return Err(format!("refused: this asks the key to sign for {}, which is outside our ecosystem. \
                                    Nothing was sent. (`elo trade settings allow_outside on` allows it.)", v.what));
            }
            say(&format!("  warning: {} is outside our ecosystem; sending because allow_outside is on", v.what));
        }
        if s.get("skip_if").is_some() && already_has(rpc, from, &need_of(&to, &data)) {
            say(&format!("  skip     {why} (already in place)"));
            guard::log(log, with("skipped", json!({})));
            continue;
        }
        let gas = match chain::estimate_gas(rpc, from, &to, value, &data) {
            Ok(g) => g,
            Err(e) => {
                guard::log(log, with("stopped", json!({"reason": e})));
                return Err(format!("the chain expects this to fail, so nothing was sent: {e}"));
            }
        };
        let (max_fee, tip) = chain::fees(rpc)?;
        let nonce = chain::nonce(rpc, from)?;
        let tx = elo_vault::tx::Tx { nonce, max_priority_fee: tip, max_fee, gas: gas.saturating_mul(12) / 10,
                                     to, value, data };
        let signed = sign(&tx)?;
        let hash = match chain::send_raw(rpc, &signed.raw_hex()) {
            Ok(h) => h,
            Err(e) => {
                guard::log(log, with("declined", json!({"reason": e})));
                return Err(format!("the chain did not take it: {e}"));
            }
        };
        say(&format!("  sent     {why}: {hash}"));
        let landed = wait_receipt(rpc, &hash);
        guard::log(log, with(if !v.listed { "warned" } else { "ok" },
                             json!({"hash": hash, "result": landed.as_ref().err().cloned().unwrap_or_else(|| "landed".into())})));
        landed?;
        last = hash;
    }
    Ok(last)
}

/// ETH per coin from a quote of the trade itself.
pub fn fill_price(qv: &Value, buying: bool) -> Option<f64> {
    let inn: f64 = qv.pointer("/amount_in/human").and_then(Value::as_str)?.parse().ok()?;
    let out: f64 = qv.pointer("/amount_out/human").and_then(Value::as_str)?.parse().ok()?;
    if inn <= 0.0 || out <= 0.0 { return None; }
    Some(if buying { inn / out } else { out / inn })
}

// ── the order book on this machine ──────────────────────────────────────────
pub fn orders_path() -> std::path::PathBuf {
    elo_vault::keystore_path().with_file_name("orders.json")
}

/// The book, v2. A v1 row (`limit`/`take-profit`/`stop` with `coin`, `price`,
/// `amount`) reads as itself: the v2 fields are a superset.
pub fn load_orders(p: &Path) -> Vec<Order> {
    std::fs::read_to_string(p).ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("orders").and_then(Value::as_array).cloned())
        .map(|r| r.iter().filter_map(Order::from_json).collect())
        .unwrap_or_default()
}

pub fn save_orders(p: &Path, book: &[Order]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot make {}: {e}", d.display()))?;
    }
    let body = json!({"version": 2, "orders": book.iter().map(Order::to_json).collect::<Vec<_>>(),
                      "note": "resting orders; they fire only while the desktop app's order runner is running"});
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&body).unwrap_or_default())
        .and_then(|_| std::fs::rename(&tmp, p))
        .map_err(|e| format!("cannot write {}: {e}", p.display()))
}

/// One pass over the book: price every live order from one `/pairs` read,
/// confirm a trigger at the order's own size, trade it, and save. A trade that
/// fails pauses its order with the reason; nothing is retried behind the
/// holder's back. Returns what happened, one line each.
#[allow(clippy::too_many_arguments)]
pub fn tick(rpc: &HttpRpc, host: &str, path: &Path, me: &[u8; 20], settings: &Settings, log: &Path,
            bps: u32, sign: Sign, say: &dyn Fn(&str)) -> Vec<String> {
    let mut said = Vec::new();
    let mut book = load_orders(path);
    if !book.iter().any(|o| !o.state.finished && o.paused.is_none()) {
        return said;
    }
    let Ok(pairs) = api(rpc, host, "/pairs") else { return said };
    let pairs = pairs.get("pairs").and_then(Value::as_array).cloned().unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let mut changed = false;
    for i in 0..book.len() {
        if book[i].state.finished || book[i].paused.is_some() {
            continue;
        }
        let o = book[i].clone();
        let price = orders::price_eth(&pairs, &o.coin).unwrap_or(f64::NAN);
        let (act, st) = orders::step(&o, price, now);
        let Action::Fire { side, amount } = act else {
            if st != o.state { book[i].state = st; changed = true; }
            continue;
        };
        let amt = orders::amount_text(amount);
        let (from, to) = if side == "buy" { ("ETH", o.coin.as_str()) } else { (o.coin.as_str(), "ETH") };
        // confirm at the order's own size before anything is prepared
        let confirmed = api(rpc, host, &format!("/quote?from={}&to={}&amount={}", q(from), q(to), q(&amt)))
            .ok().and_then(|qv| fill_price(&qv, side == "buy"))
            .map(|fp| orders::step(&o, fp, now));
        let Some((Action::Fire { .. }, st2)) = confirmed else {
            continue;
        };
        say(&format!("  #{} fires: {}", o.id, o.describe()));
        // on disk before the first signature: a crash mid-trade leaves the order
        // paused with this note, never ready to fire the same slot again
        book[i].paused = Some("interrupted while trading — check your wallet before resuming".into());
        let _ = save_orders(path, &book);
        let res = prepare(rpc, host, &amt, from, to, bps, me, settings)
            .and_then(|prep| execute(&prep, rpc, me, settings, log, &format!("order #{}", o.id), sign, say));
        match res {
            Ok(h) => {
                book[i].state = st2;
                book[i].paused = None;
                said.push(format!("#{} traded: {}", o.id, chain::tx_url(&h)));
            }
            Err(e) => { book[i].paused = Some(e.clone()); said.push(format!("#{} paused: {e}", o.id)); }
        }
        changed = true;
        let _ = save_orders(path, &book);
    }
    if changed {
        let _ = save_orders(path, &book);
    }
    said
}
