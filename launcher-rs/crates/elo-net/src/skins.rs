//! Our collections on OpenSea, for `elo skins` and the window's Skins: what
//! this account holds, its open listings, and the steps to sell, cancel, buy
//! or take the best offer (`docs/store/LAUNCH-TERMINAL.md` §13).
//!
//! The origin prepares everything (`/api/collections/*`); this file checks a
//! listing against what was asked before the vault signs it. The vault adds
//! its own refusal (`elo_vault::seaport::check_listing`): one item, offered by
//! this account, paid to this account first in ETH. Here, on top: the item and
//! collection are the ones asked for, the price is the price typed, every
//! other payment goes to a fee recipient the origin named, and those fees
//! together stay under [`MAX_FEE_BPS`] — so a breached origin can still not
//! slip a large cut, or a different item, past the review.

use crate::chain::HttpRpc;
use crate::trading::{api, q};
use elo_vault::seaport::{self, Order};
use serde_json::{json, Value};

/// The three keys `/api/collections` answers for.
pub const KEYS: [&str; 3] = ["skins", "pros", "copies"];

/// The most a listing may pay out in fees, in basis points. OpenSea's own fee
/// is a small fraction of this; a prepared order asking for more is refused.
pub const MAX_FEE_BPS: u128 = 1000;

/// One thing this account holds in a collection.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub id: String,
    pub name: String,
}

/// One open listing of this account's.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub id: String,
    pub order: String,
    pub price_eth: Option<f64>,
}

pub fn collection(rpc: &HttpRpc, host: &str, key: &str) -> Result<Value, String> {
    let all = api(rpc, host, "/collections")?;
    all.get("collections").and_then(Value::as_array)
        .and_then(|r| r.iter().find(|c| c.get("key").and_then(Value::as_str) == Some(key)).cloned())
        .ok_or_else(|| format!("a collection is one of {}", KEYS.join(", ")))
}

/// What `wallet` holds here: skins from `/items/of`, characters from
/// `/seat/of`, copies from `/tickets/of?ids=1`. A read that failed is an
/// error, never an empty list.
pub fn held(rpc: &HttpRpc, host: &str, key: &str, wallet: &str) -> Result<Vec<Held>, String> {
    if key == "copies" {
        let v = api(rpc, host, &format!("/tickets/of/{}?slugs=gates&ids=1", q(wallet)))?;
        let g = v.pointer("/titles/gates").cloned().unwrap_or(Value::Null);
        if g.get("reachable").and_then(Value::as_bool) == Some(false) {
            return Err("could not look: the chain did not answer".into());
        }
        if g.get("copies").and_then(Value::as_u64).unwrap_or(0) > 0 && !g.get("ids").is_some_and(Value::is_array) {
            return Err(g.get("ids_why").and_then(Value::as_str).unwrap_or("could not list which copies").to_string());
        }
        return Ok(g.get("ids").and_then(Value::as_array).cloned().unwrap_or_default().iter()
            .filter_map(|i| i.as_str().map(|id| Held { id: id.to_string(), name: "Gates".into() })).collect());
    }
    let (path, list, name) = match key {
        "skins" => (format!("/items/of/{}?limit=200", q(wallet)), "items", Some("catalog_name")),
        _ => (format!("/seat/of/{}", q(wallet)), "seats", None),
    };
    let v = api(rpc, host, &path)?;
    if v.get("reachable").and_then(Value::as_bool) == Some(false) {
        return Err(format!("could not look: {}", v.get("error").and_then(Value::as_str).unwrap_or("the index did not answer")));
    }
    Ok(v.get(list).and_then(Value::as_array).cloned().unwrap_or_default().iter().filter_map(|it| {
        let id = match it.get("token_id")? {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => return None,
        };
        let n = name.and_then(|k| it.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
        Some(Held { id, name: n })
    }).collect())
}

pub fn listings(rpc: &HttpRpc, host: &str, key: &str, wallet: &str) -> Result<Vec<Listing>, String> {
    let v = api(rpc, host, &format!("/collections/{key}/mine?wallet={}", q(wallet)))?;
    Ok(v.get("listings").and_then(Value::as_array).cloned().unwrap_or_default().iter().map(|l| Listing {
        id: l.get("token_id").and_then(Value::as_str).unwrap_or("?").to_string(),
        order: l.get("order").and_then(Value::as_str).unwrap_or("").to_string(),
        price_eth: l.get("price_eth").and_then(Value::as_f64),
    }).collect())
}

/// ETH as typed ("0.01") in wei, exactly; anything that is not a plain
/// positive decimal with at most 18 places is refused.
pub fn wei(price: &str) -> Result<u128, String> {
    let p = price.trim();
    let (whole, frac) = p.split_once('.').unwrap_or((p, ""));
    let ok = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (whole.is_empty() && frac.is_empty()) || !ok(whole) || !ok(frac) || frac.len() > 18 {
        return Err(format!("a price is a number of ETH, like 0.01 — not {p:?}"));
    }
    let w: u128 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| "that price is too large".to_string())? };
    let f: u128 = if frac.is_empty() { 0 } else { format!("{frac:0<18}").parse().map_err(|_| "bad price".to_string())? };
    let total = w.checked_mul(10u128.pow(18)).and_then(|x| x.checked_add(f)).ok_or("that price is too large")?;
    if total == 0 {
        return Err("a price is above zero".into());
    }
    Ok(total)
}

pub fn eth(wei: u128) -> String {
    let s = format!("{}.{:018}", wei / 10u128.pow(18), wei % 10u128.pow(18));
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The origin's prepared listing: its approval steps, its typed data, its review.
pub fn prepare_sell(rpc: &HttpRpc, host: &str, key: &str, id: &str, price: &str, days: u32, wallet: &str)
    -> Result<Value, String> {
    wei(price)?;
    if !(1..=180).contains(&days) {
        return Err("a listing runs 1 to 180 days".into());
    }
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("an item is its number, like 7".into());
    }
    api(rpc, host, &format!("/collections/{key}/list/prepare?id={}&price={}&days={days}&wallet={}",
                            q(id), q(price), q(wallet)))
}

/// Check the prepared order against what was asked, and return it ready for
/// the vault: the collection's own address, this item, this account, the price
/// typed, and fees only to the recipients the origin's facts name, under the cap.
pub fn check_sell(prep: &Value, collection: &[u8; 20], id: &str, price_wei: u128, me: &[u8; 20])
    -> Result<Order, String> {
    let td = prep.get("typed_data").ok_or("the origin prepared no order")?;
    let d = td.get("domain").cloned().unwrap_or(Value::Null);
    let sea = format!("0x{}", hex(&seaport::SEAPORT));
    if d.get("name").and_then(Value::as_str) != Some("Seaport") || d.get("version").and_then(Value::as_str) != Some("1.6")
        || d.get("chainId").and_then(Value::as_u64) != Some(elo_vault::tx::CHAIN_ID)
        || d.get("verifyingContract").and_then(Value::as_str).map(str::to_ascii_lowercase) != Some(sea) {
        return Err("the origin's order is not for Seaport 1.6 on Robinhood Chain".into());
    }
    let o = Order::from_json(td.get("message").ok_or("the order has no message")?)?;
    seaport::check_listing(&o, me)?;
    if o.offer[0].token != *collection {
        return Err("the order offers an item from another collection".into());
    }
    if seaport::word_dec(&o.offer[0].identifier) != id {
        return Err(format!("the order offers a different item than #{id}"));
    }
    let amounts: Vec<u128> = o.consideration.iter()
        .map(|c| seaport::word_u128(&c.start).ok_or("an amount is out of range".to_string()))
        .collect::<Result<_, _>>()?;
    let total: u128 = amounts.iter().sum();
    if total != price_wei {
        return Err(format!("the order's price is {} ETH, not the {} ETH asked", eth(total), eth(price_wei)));
    }
    let named: Vec<[u8; 20]> = prep.pointer("/facts/fees").and_then(Value::as_array).cloned().unwrap_or_default()
        .iter().filter_map(|f| f.get("recipient").and_then(Value::as_str))
        .filter_map(|r| elo_vault::parse_address(r).ok()).collect();
    let fees: u128 = amounts[1..].iter().sum();
    for c in &o.consideration[1..] {
        if !c.recipient.is_some_and(|r| named.contains(&r)) {
            return Err("the order pays someone who is not a fee the origin named".into());
        }
    }
    if fees.saturating_mul(10_000) > price_wei.saturating_mul(MAX_FEE_BPS) {
        return Err(format!("the order's fees are {} ETH of {} — more than this app allows", eth(fees), eth(price_wei)));
    }
    Ok(o)
}

/// Post the signed listing. Returns OpenSea's page for the item.
pub fn submit(rpc: &HttpRpc, host: &str, key: &str, prep: &Value, signature: &str) -> Result<String, String> {
    let body = json!({"order": prep.pointer("/typed_data/message").cloned().unwrap_or(Value::Null),
                      "signature": signature});
    let got = rpc.net.post_json_told(&format!("{host}/api/collections/{key}/list"), &body);
    match got.value {
        Some(v) if got.ok => Ok(v.get("opensea").and_then(Value::as_str).unwrap_or("").to_string()),
        _ => Err(if got.why.is_empty() { "the origin did not take it".into() } else { got.why }),
    }
}

pub fn prepare_cancel(rpc: &HttpRpc, host: &str, key: &str, order: &str, wallet: &str) -> Result<Value, String> {
    api(rpc, host, &format!("/collections/{key}/cancel/prepare?order={}&wallet={}", q(order), q(wallet)))
}

pub fn prepare_accept(rpc: &HttpRpc, host: &str, key: &str, id: &str, wallet: &str) -> Result<Value, String> {
    api(rpc, host, &format!("/collections/{key}/accept/prepare?id={}&wallet={}", q(id), q(wallet)))
}

pub fn prepare_buy(rpc: &HttpRpc, host: &str, key: &str, order: &str, wallet: &str) -> Result<Value, String> {
    api(rpc, host, &format!("/collections/{key}/buy?order={}&protocol=0x{}&wallet={}", q(order),
                            hex(&seaport::SEAPORT), q(wallet)))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "0x17c5185167401ed00cf5f5b2fc97d9bbfdb7d025";
    const SKINS: &str = "0x93f4965638f9c99dd53f13bf2b47358a6ff7630c";
    const FEE: &str = "0x2222222222222222222222222222222222222222";

    fn prep(pay_me: &str, fee: &str, fee_to: &str, id: &str, token: &str) -> Value {
        json!({
            "typed_data": {"domain": {"name": "Seaport", "version": "1.6", "chainId": 4663,
                                      "verifyingContract": "0x0000000000000068f116a894984e2db1123eb395"},
                           "message": {"offerer": ME, "zone": "0x0000000000000000000000000000000000000000",
                                       "offer": [{"itemType": 2, "token": token, "identifierOrCriteria": id,
                                                  "startAmount": "1", "endAmount": "1"}],
                                       "consideration": [
                                           {"itemType": 0, "token": "0x0000000000000000000000000000000000000000",
                                            "identifierOrCriteria": "0", "startAmount": pay_me, "endAmount": pay_me,
                                            "recipient": ME},
                                           {"itemType": 0, "token": "0x0000000000000000000000000000000000000000",
                                            "identifierOrCriteria": "0", "startAmount": fee, "endAmount": fee,
                                            "recipient": fee_to}],
                                       "orderType": 0, "startTime": "1", "endTime": "2",
                                       "zoneHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                                       "salt": "1", "conduitKey": "0x0000000000000000000000000000000000000000000000000000000000000000",
                                       "counter": "0"}},
            "facts": {"fees": [{"recipient": FEE, "bps": 100}]},
        })
    }

    fn a(s: &str) -> [u8; 20] {
        elo_vault::parse_address(s).unwrap()
    }

    #[test]
    fn prices_are_exact_wei() {
        assert_eq!(wei("0.01").unwrap(), 10_000_000_000_000_000);
        assert_eq!(wei("1").unwrap(), 10u128.pow(18));
        assert_eq!(wei(".5").unwrap(), 5 * 10u128.pow(17));
        for bad in ["0", "", "-1", "1e18", "0.0000000000000000001", "abc", "1.2.3"] {
            assert!(wei(bad).is_err(), "{bad}");
        }
        assert_eq!(eth(9_900_000_000_000_000), "0.0099");
        assert_eq!(eth(10u128.pow(18)), "1");
    }

    #[test]
    fn a_good_listing_passes_and_every_swap_is_caught() {
        let p = wei("0.01").unwrap();
        let good = prep("9900000000000000", "100000000000000", FEE, "7", SKINS);
        assert!(check_sell(&good, &a(SKINS), "7", p, &a(ME)).is_ok());
        let cases = [
            (prep("9900000000000000", "100000000000000", FEE, "8", SKINS), "another item"),
            (prep("9900000000000000", "100000000000000", FEE, "7", "0x6666666666666666666666666666666666666666"), "another collection"),
            (prep("9900000000000000", "100000000000000", "0x6666666666666666666666666666666666666666", "7", SKINS), "a stranger's cut"),
            (prep("8900000000000000", "100000000000000", FEE, "7", SKINS), "a different price"),
            (prep("5000000000000000", "5000000000000000", FEE, "7", SKINS), "fees over the cap"),
        ];
        for (v, why) in cases {
            assert!(check_sell(&v, &a(SKINS), "7", p, &a(ME)).is_err(), "{why}");
        }
        let mut other_chain = good.clone();
        other_chain["typed_data"]["domain"]["chainId"] = json!(1);
        assert!(check_sell(&other_chain, &a(SKINS), "7", p, &a(ME)).is_err(), "another chain");
    }
}
