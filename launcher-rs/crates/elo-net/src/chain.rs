//! Robinhood Chain, read directly: balances, and what a send needs.
//!
//! # Why the chain and not the origin
//!
//! The origin already serves balances (`/api/holdings/{wallet}`), and a page
//! can trust it for a picture. A wallet cannot: what it shows is what a holder
//! decides to send against, and the number a send is built from — the nonce,
//! the fee, a token's decimals — has to be the chain's own answer. So every
//! number here is a JSON-RPC call to the chain's public endpoint, and the
//! origin is asked only which coins exist (`wallet::book`).
//!
//! # A read that failed is not a zero
//!
//! Every call returns `Err` when it could not look, and the wallet shows "could
//! not read" for it. A dead RPC that rendered as an empty wallet would tell a
//! holder their money is gone (`CLAUDE.md` §traps).
//!
//! This file holds no key and signs nothing. It carries bytes the vault
//! already signed to `eth_sendRawTransaction`, and that is the whole of its
//! part in moving money.

use crate::Net;
use serde_json::{json, Value};

/// The chain's public RPC endpoint. `ELO_RPC` overrides it.
pub const RPC: &str = "https://rpc.mainnet.chain.robinhood.com";
/// Where a transaction can be looked at by anyone.
pub const EXPLORER: &str = "https://explorer.mainnet.chain.robinhood.com";
/// What a holder sees the chain called.
pub const NAME: &str = "Robinhood Chain";

pub fn rpc_url() -> String {
    std::env::var("ELO_RPC").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| RPC.to_string())
}

pub fn tx_url(hash: &str) -> String {
    format!("{EXPLORER}/tx/{hash}")
}

pub fn address_url(addr: &str) -> String {
    format!("{EXPLORER}/address/{addr}")
}

/// One JSON-RPC call. A trait so the wallet's rules can be tested against a
/// chain that answers what a test says.
pub trait Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value, String>;
}

/// The real one, over this client's own HTTP stack (proxies, timeouts, size
/// caps all as for every other read). Owns its `Net` so it can be made on
/// whichever thread does the reading.
pub struct HttpRpc {
    pub net: Net,
    pub url: String,
}

impl Default for HttpRpc {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpRpc {
    pub fn new() -> Self {
        HttpRpc { net: Net::new(), url: rpc_url() }
    }
}

impl Rpc for HttpRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let got = self.net.post_json(&self.url, &body);
        let Some(v) = got.value else {
            return Err(if got.reachable {
                format!("the chain's RPC answered oddly: {}", got.why)
            } else {
                format!("could not reach the chain: {}", got.why)
            });
        };
        if let Some(e) = v.get("error") {
            let msg = e.get("message").and_then(Value::as_str).unwrap_or("an error");
            return Err(format!("the chain said: {msg}"));
        }
        v.get("result").cloned().ok_or_else(|| "the chain's reply had no result".into())
    }
}

/// `0x…` with 40 hex characters, lowercase, for a JSON-RPC parameter.
pub fn addr_param(a: &[u8; 20]) -> String {
    format!("0x{}", hex(a))
}

/// A JSON-RPC quantity (`"0x1a"`) as a u128. Larger than u128 is an error, not
/// a wrap: no balance this wallet reads comes near it.
pub fn quantity(v: &Value) -> Result<u128, String> {
    let s = v.as_str().ok_or("expected a hex quantity")?;
    let h = s.strip_prefix("0x").ok_or("expected a 0x quantity")?;
    let h = h.trim_start_matches('0');
    if h.is_empty() {
        return Ok(0);
    }
    if h.len() > 32 {
        return Err("a number too large for this wallet to show".into());
    }
    u128::from_str_radix(h, 16).map_err(|_| "not a hex quantity".into())
}

/// A 32-byte ABI word (an `eth_call` result) as a u128, refusing anything
/// that does not fit.
pub fn word(v: &Value) -> Result<u128, String> {
    let s = v.as_str().ok_or("expected hex data")?;
    let b = unhex(s.strip_prefix("0x").unwrap_or(s))?;
    if b.len() != 32 {
        return Err(format!("expected one 32-byte word, got {} bytes", b.len()));
    }
    if b[..16].iter().any(|&x| x != 0) {
        return Err("a number too large for this wallet to show".into());
    }
    let mut lo = [0u8; 16];
    lo.copy_from_slice(&b[16..]);
    Ok(u128::from_be_bytes(lo))
}

pub fn chain_id(rpc: &dyn Rpc) -> Result<u64, String> {
    let id = quantity(&rpc.call("eth_chainId", json!([]))?)?;
    u64::try_from(id).map_err(|_| "a chain id out of range".into())
}

/// ETH, in wei.
pub fn balance(rpc: &dyn Rpc, who: &[u8; 20]) -> Result<u128, String> {
    quantity(&rpc.call("eth_getBalance", json!([addr_param(who), "latest"]))?)
}

fn view(rpc: &dyn Rpc, to: &[u8; 20], data: &[u8]) -> Result<Value, String> {
    rpc.call("eth_call", json!([{"to": addr_param(to), "data": format!("0x{}", hex(data))}, "latest"]))
}

/// `balanceOf(who)` on a token.
pub fn token_balance(rpc: &dyn Rpc, token: &[u8; 20], who: &[u8; 20]) -> Result<u128, String> {
    let mut data = vec![0x70, 0xa0, 0x82, 0x31];
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(who);
    word(&view(rpc, token, &data)?)
}

/// `decimals()` on a token — read from the contract, never taken from a list,
/// because an amount typed against the wrong decimals sends a different
/// amount.
pub fn token_decimals(rpc: &dyn Rpc, token: &[u8; 20]) -> Result<u8, String> {
    let d = word(&view(rpc, token, &[0x31, 0x3c, 0xe5, 0x67])?)?;
    u8::try_from(d).ok().filter(|&d| d <= 36).ok_or_else(|| format!("a token claiming {d} decimals"))
}

/// `symbol()` on a token, from the ABI string it returns. `None` when the
/// contract answers with something that is not a short printable string.
pub fn token_symbol(rpc: &dyn Rpc, token: &[u8; 20]) -> Result<Option<String>, String> {
    let v = view(rpc, token, &[0x95, 0xd8, 0x9b, 0x41])?;
    let s = v.as_str().ok_or("expected hex data")?;
    let b = unhex(s.strip_prefix("0x").unwrap_or(s))?;
    if b.len() < 64 {
        return Ok(None);
    }
    let len = u128::from_be_bytes(b[48..64].try_into().unwrap_or([0; 16])) as usize;
    let Some(raw) = b.get(64..64 + len) else { return Ok(None) };
    let sym = String::from_utf8(raw.to_vec()).ok();
    Ok(sym.filter(|s| !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_graphic())))
}

/// What sits at an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    /// A plain wallet: a key and nothing else.
    Wallet,
    /// A wallet that has handed control to a contract (EIP-7702): its code is
    /// `0xef0100` and the contract's address. That contract can move whatever
    /// the wallet holds.
    Delegated([u8; 20]),
    /// A contract.
    Contract,
}

pub fn holder(rpc: &dyn Rpc, who: &[u8; 20]) -> Result<Holder, String> {
    let v = rpc.call("eth_getCode", json!([addr_param(who), "latest"]))?;
    let code = v.as_str().ok_or("expected hex code")?;
    let body = code.strip_prefix("0x").unwrap_or(code);
    if body.is_empty() {
        return Ok(Holder::Wallet);
    }
    if body.len() == 46 && body.to_ascii_lowercase().starts_with("ef0100") {
        let mut to = [0u8; 20];
        to.copy_from_slice(&unhex(&body[6..])?);
        return Ok(Holder::Delegated(to));
    }
    Ok(Holder::Contract)
}

/// The next nonce, counting transactions still waiting in the pool.
pub fn nonce(rpc: &dyn Rpc, who: &[u8; 20]) -> Result<u64, String> {
    let n = quantity(&rpc.call("eth_getTransactionCount", json!([addr_param(who), "pending"]))?)?;
    u64::try_from(n).map_err(|_| "a nonce out of range".into())
}

/// `(max fee, priority fee)` per gas: twice the latest base fee plus the tip,
/// so a send survives the base fee doubling before it lands. The holder is
/// shown the most it can cost and pays the base fee actually charged.
pub fn fees(rpc: &dyn Rpc) -> Result<(u128, u128), String> {
    let block = rpc.call("eth_getBlockByNumber", json!(["latest", false]))?;
    let base = quantity(block.get("baseFeePerGas").ok_or("the latest block names no base fee")?)?;
    // Nitro chains take no tip and answer 0; a node without the method is
    // read the same way.
    let tip = rpc.call("eth_maxPriorityFeePerGas", json!([]))
        .and_then(|v| quantity(&v))
        .unwrap_or(0);
    Ok((base.saturating_mul(2).saturating_add(tip), tip))
}

/// What the chain expects a call to cost in gas. An error here is often the
/// chain saying the transfer would fail, and the caller stops on it.
pub fn estimate_gas(
    rpc: &dyn Rpc,
    from: &[u8; 20],
    to: &[u8; 20],
    value: u128,
    data: &[u8],
) -> Result<u64, String> {
    let g = quantity(&rpc.call("eth_estimateGas", json!([{
        "from": addr_param(from),
        "to": addr_param(to),
        "value": format!("0x{value:x}"),
        "data": format!("0x{}", hex(data)),
    }]))?)?;
    u64::try_from(g).map_err(|_| "a gas estimate out of range".into())
}

/// Hand signed bytes to the chain. Returns the hash the chain names.
pub fn send_raw(rpc: &dyn Rpc, raw_hex: &str) -> Result<String, String> {
    rpc.call("eth_sendRawTransaction", json!([raw_hex]))?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "the chain took it but named no hash".into())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    // ASCII first: the slices below are by byte, and an RPC's text is not ours.
    if !s.is_ascii() {
        return Err("not hex".into());
    }
    if s.len() % 2 != 0 {
        return Err("odd-length hex".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "not hex".to_string()))
        .collect()
}
