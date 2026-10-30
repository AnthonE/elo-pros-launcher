//! `elo credits` — what this wallet's account holds in dev credits, and a
//! top-up paid from this machine (`docs/money/CREDITS.md`).
//!
//! Credits pay for what costs the store money: hosting, Claude builds in a
//! repo, buddies' thinking time, the stream relay. They never buy a coin, a
//! skin or an asset, and they cannot be sent or cashed out.
//!
//! `buy` pays the store's HTTP 402 door (`/api/credits/topup`, x402 v2): it
//! reads the challenge, checks it names USDG on Robinhood Chain and one of the
//! posted packs, and has the vault sign the transfer authorization
//! (`elo_vault::usdg`, which refuses any payee but the credits address and any
//! amount over a pack). The store submits the transfer and pays the gas; the
//! credits land on this wallet's account. The balance read signs in with the
//! same wallet (the OAuth wallet grant), so no token is kept on disk.
use crate::args::Args;
use crate::friends::told;
use crate::prompt;
use crate::signer;
use elo_broker::signer::Signer;
use elo_depot::DepotError;
use elo_hive::render::safe_line;
use elo_net::Net;
use elo_vault::solana::{b64_decode, b64_encode};
use elo_vault::usdg;
use serde_json::{json, Value};

pub const USAGE: &str = "\
usage: elo credits                   this wallet's credits balance and its last lines
       elo credits buy [ten|fifty]   pay for a pack in USDG on Robinhood Chain (HTTP 402);
                                     the store pays the gas, the credits land on this wallet
       elo credits card              what credits pay for, the packs, and the ways in
  --json for scripts. Credits pay for hosting, Claude builds, buddies and the
  relay. They can't be sent to anyone or cashed out.
";

const WALLET_GRANT: &str = "urn:elopros:params:oauth:grant-type:wallet";

fn err(e: impl Into<String>) -> DepotError {
    DepotError::new(e.into())
}

pub fn run(a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let net = Net::new();
    match a.positional.first().map(String::as_str) {
        None | Some("balance") => balance(&net, a, host, json_out),
        Some("buy") => buy(&net, a, host, json_out),
        Some("card") => card(&net, host, json_out),
        Some("help") => {
            print!("{USAGE}");
            Ok(0)
        }
        Some(other) => Err(err(format!("unknown credits command {} — `elo credits help`", safe_line(other)))),
    }
}

fn dollars(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}${}.{:02}", cents.abs() / 100, cents.abs() % 100)
}

fn card(net: &Net, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let c = told(net.get_json_told(&format!("{host}/api/credits")))?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&c).unwrap_or_default());
        return Ok(0);
    }
    println!("credits pay for:");
    for b in c.get("buys").and_then(Value::as_array).into_iter().flatten() {
        let live = b.get("live").and_then(Value::as_bool).unwrap_or(false);
        println!("  {} {} — {}", if live { "·" } else { "○" },
                 safe_line(b.get("what").and_then(Value::as_str).unwrap_or("")),
                 safe_line(b.get("about").and_then(Value::as_str).unwrap_or("")));
    }
    let packs: Vec<String> = c.get("packs").and_then(Value::as_array).into_iter().flatten()
        .map(|p| format!("{} ({})", safe_line(p.get("id").and_then(Value::as_str).unwrap_or("?")),
                         dollars(p.get("usd_cents").and_then(Value::as_i64).unwrap_or(0))))
        .collect();
    println!("packs: {}", packs.join(", "));
    let agents = c.get("agents").cloned().unwrap_or(Value::Null);
    if agents.get("on").and_then(Value::as_bool) == Some(true) {
        println!("pay from here: `elo credits buy ten` (USDG on Robinhood Chain, no gas)");
    }
    println!("by card or by sending a coin: {host}/dev-console.html#credits");
    Ok(0)
}

/// The account's wallet, from the local keystore: the one signer that can
/// make the narrow USDG authorization (a paired browser cannot).
fn local_account() -> Result<(elo_vault::LocalSigner, [u8; 20], String), DepotError> {
    let local = elo_vault::LocalSigner::at(elo_vault::keystore_path());
    let addr = local.address().ok_or_else(|| err(
        "no account on this machine — `elo account new` makes one, or buy on the site"))?;
    let raw = elo_vault::parse_address(&addr).map_err(err)?;
    Ok((local, raw, addr.to_lowercase()))
}

fn header<'a>(hs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn hex_to<const N: usize>(s: &str) -> Option<[u8; N]> {
    let h = s.strip_prefix("0x")?;
    if h.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn buy(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let pack = a.positional.get(1).map(String::as_str).unwrap_or("ten");
    if !pack.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(err("a pack is a word like ten or fifty"));
    }
    let (local, me_raw, me) = local_account()?;
    let url = format!("{host}/api/credits/topup?pack={pack}");

    // 1 · the challenge
    let (code, hs, body) = net.request_full("GET", &url, &[], None, 64 * 1024).map_err(err)?;
    if code != 402 {
        let why = serde_json::from_slice::<Value>(&body).ok()
            .and_then(|v| v.get("why_not").or(v.get("error")).and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| format!("the store answered {code}"));
        return Err(err(format!("the pay door is not taking payments: {}", safe_line(&why))));
    }
    let pr: Value = header(&hs, "payment-required")
        .and_then(|h| b64_decode(h).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| err("the store's 402 carried no readable PAYMENT-REQUIRED"))?;
    let req = pr.get("accepts").and_then(Value::as_array).and_then(|v| v.first()).cloned()
        .ok_or_else(|| err("the challenge offers no way to pay"))?;
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    if s("scheme") != "exact" || s("network") != "eip155:4663" {
        return Err(err("the challenge is not an exact payment on Robinhood Chain"));
    }
    if hex_to::<20>(&s("asset").to_lowercase()) != Some(usdg::USDG) {
        return Err(err("the challenge asks for a coin other than USDG"));
    }
    let to = hex_to::<20>(&s("payTo").to_lowercase()).ok_or_else(|| err("the challenge names no payee"))?;
    let value: u128 = s("amount").parse().map_err(|_| err("the challenge's amount is not a number"))?;

    // 2 · the authorization, checked by the vault before the key is touched
    let now = elo_net::pairing::now();
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|e| err(format!("no OS entropy: {e}")))?;
    let auth = usdg::Authorization { from: me_raw, to, value, valid_after: 0, valid_before: now + 300, nonce };
    usdg::check(&auth, &me_raw, now).map_err(err)?;
    let usd = format!("{}.{:02}", value / 1_000_000, (value % 1_000_000) / 10_000);
    eprintln!("  pay {usd} USDG from {me} to the credits address 0x{} for ${usd} of Elo Pros credits",
              hexs(&to));
    eprintln!("  on Robinhood Chain; the store submits it and pays the gas");
    if a.has("--dry-run") {
        println!("nothing was signed — drop --dry-run to pay");
        return Ok(0);
    }
    if !a.has("--yes") && !prompt::confirm("  pay? [y/N] ").map_err(err)? {
        println!("nothing was signed");
        return Ok(1);
    }
    let pass = prompt::secret("passphrase: ").map_err(err)?;
    let sig = local.sign_usdg_authorization(&pass, &auth, now).map_err(err)?;
    drop(pass);

    // 3 · the payment, sent back to the same door
    let payload = json!({
        "x402Version": 2, "resource": pr.get("resource").cloned().unwrap_or(Value::Null), "accepted": req,
        "payload": {"signature": sig, "authorization": {
            "from": me, "to": format!("0x{}", hexs(&to)), "value": value.to_string(),
            "validAfter": "0", "validBefore": auth.valid_before.to_string(), "nonce": format!("0x{}", hexs(&nonce))}}});
    let hdr = b64_encode(serde_json::to_string(&payload).unwrap_or_default().as_bytes());
    let mut tries = 0;
    let out: Value = loop {
        let (code, _hs, body) = net
            .request_full("POST", &url, &[("payment-signature", hdr.as_str())], None, 64 * 1024)
            .map_err(err)?;
        let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        match code {
            200 => break v,
            202 if tries < 10 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            _ => {
                let why = v.get("error").and_then(Value::as_str).unwrap_or("the payment was refused");
                return Err(err(format!("{} ({code})", safe_line(why))));
            }
        }
    };
    if json_out {
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(0);
    }
    println!("${usd} of credits landed on {}", safe_line(out.get("account").and_then(Value::as_str).unwrap_or("")));
    println!("  balance {}", dollars(out.get("balance_usd_cents").and_then(Value::as_i64).unwrap_or(0)));
    println!("  tx {}", safe_line(out.get("tx").and_then(Value::as_str).unwrap_or("")));
    Ok(0)
}

fn balance(net: &Net, a: &Args, host: &str, json_out: bool) -> Result<i32, DepotError> {
    let mut s = signer::choose(a, host).map_err(err)?;
    let me = s.address().ok_or_else(|| err("the signer names no address"))?.to_lowercase();
    // sign in with this wallet: the text is the store's sign-in, for this wallet
    let prep = told(net.post_json_told(&format!("{host}/api/account/siwe/prepare"), &json!({"wallet": me})))?;
    let msg = prep.get("message").and_then(Value::as_str).unwrap_or("");
    let nonce = prep.get("nonce").and_then(Value::as_str).unwrap_or("");
    if !msg.to_lowercase().contains(&me) || nonce.is_empty() || !msg.contains(nonce) {
        return Err(err("the store's sign-in text does not name this wallet and its nonce — nothing was signed"));
    }
    let signed = s.sign(msg, "credits: sign in to read your balance").map_err(|r| err(r.reason))?;
    let sig = signed.get("signature").and_then(Value::as_str).unwrap_or("");
    let tok = told(net.post_json_told(&format!("{host}/api/oauth/token"),
                                      &json!({"grant_type": WALLET_GRANT, "nonce": nonce, "signature": sig})))?;
    let bearer = format!("Bearer {}", tok.get("access_token").and_then(Value::as_str).unwrap_or(""));
    let (code, _hs, body) = net
        .request_full("GET", &format!("{host}/api/credits/me"), &[("authorization", bearer.as_str())], None, 1 << 20)
        .map_err(err)?;
    let v: Value = serde_json::from_slice(&body).map_err(|_| err(format!("the store answered {code}")))?;
    if code != 200 {
        return Err(err(safe_line(v.get("error").and_then(Value::as_str).unwrap_or("could not read the balance"))));
    }
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(0);
    }
    println!("{} · {}", safe_line(v.get("account").and_then(Value::as_str).unwrap_or("")),
             dollars(v.get("balance_usd_cents").and_then(Value::as_i64).unwrap_or(0)));
    let lines = v.get("lines").and_then(Value::as_array).cloned().unwrap_or_default();
    for l in lines.iter().rev().take(8) {
        println!("  {:>9}  {}", dollars(l.get("cents").and_then(Value::as_i64).unwrap_or(0)),
                 safe_line(l.get("what").and_then(Value::as_str).unwrap_or("")));
    }
    if lines.is_empty() {
        println!("  no lines yet — `elo credits buy ten` tops up from this wallet");
    }
    Ok(0)
}
