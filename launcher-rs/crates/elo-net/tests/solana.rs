//! The Solana half's rules against a chain that answers what each test says.

use elo_net::chain::Rpc;
use elo_net::solana::{self, SolCoin};
use elo_net::xmove;
use elo_vault::solana as sol;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;

const ME: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const FRIEND: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const BLOCKHASH: &str = "EkSnNWid2cvwEVnVx9aBqawnmiCNiDgp3gUdkDPTKN1N";
const MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

fn key(s: &str) -> [u8; 32] {
    sol::parse_address(s).unwrap()
}

/// Answers by method; `getAccountInfo` by the address asked about.
struct Fake {
    answers: BTreeMap<String, Result<Value, String>>,
    calls: RefCell<Vec<(String, Value)>>,
}

impl Fake {
    fn new() -> Fake {
        let mut f = Fake { answers: BTreeMap::new(), calls: RefCell::new(Vec::new()) };
        f.set("getBalance", json!({"value": 2 * sol::LAMPORTS}));
        f.set("getLatestBlockhash", json!({"value": {"blockhash": BLOCKHASH}}));
        f.set("getMinimumBalanceForRentExemption", json!(890_880));
        f.set(&format!("getAccountInfo:{FRIEND}"),
              json!({"value": {"owner": "11111111111111111111111111111111", "executable": false}}));
        f.set("getTokenAccountsByOwner", json!({"value": []}));
        // spends 0.5 SOL + the fee
        f.set("simulateTransaction", json!({"value": {"err": null, "fee": 5000,
               "accounts": [{"lamports": 2 * sol::LAMPORTS - sol::LAMPORTS / 2 - 5000}]}}));
        f
    }
    fn set(&mut self, k: &str, v: Value) {
        self.answers.insert(k.into(), Ok(v));
    }
    fn called(&self, m: &str) -> bool {
        self.calls.borrow().iter().any(|(k, _)| k == m)
    }
}

impl Rpc for Fake {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.calls.borrow_mut().push((method.to_string(), params.clone()));
        if method == "getAccountInfo" {
            let k = format!("getAccountInfo:{}", params[0].as_str().unwrap_or(""));
            return self.answers.get(&k).cloned().unwrap_or(Ok(json!({"value": null})));
        }
        self.answers.get(method).cloned().unwrap_or_else(|| Err(format!("no answer for {method}")))
    }
}

fn sol_coin() -> SolCoin {
    solana::sol_coin(&key(ME))
}

#[test]
fn a_sol_send_is_planned_simulated_and_read_back() {
    let f = Fake::new();
    let p = solana::plan(&f, &key(ME), &sol_coin(), FRIEND, "0.5", &[]).unwrap();
    assert!(f.called("simulateTransaction"), "a send is simulated before it is shown");
    let lines = solana::review(&p).unwrap();
    assert_eq!(lines[0].1, "0.5 SOL");
    assert_eq!(lines[1].1, FRIEND);
    assert_eq!(lines[3].1, "Solana");
    assert!(p.notes.iter().any(|n| n.contains("not sent to this address")));
    assert_eq!(p.tx.message.keys[0], key(ME), "this account pays the fee");
}

#[test]
fn a_send_that_would_take_more_than_it_says_is_refused() {
    let mut f = Fake::new();
    f.set("simulateTransaction", json!({"value": {"err": null, "accounts": [{"lamports": 1000}]}}));
    let e = solana::plan(&f, &key(ME), &sol_coin(), FRIEND, "0.5", &[]).unwrap_err();
    assert!(e.contains("more than"), "{e}");
}

#[test]
fn a_failing_simulation_signs_nothing() {
    let mut f = Fake::new();
    f.set("simulateTransaction", json!({"value": {"err": {"InstructionError": [0, "Custom"]},
           "logs": ["Program log: Error: insufficient funds"]}}));
    let e = solana::plan(&f, &key(ME), &sol_coin(), FRIEND, "0.5", &[]).unwrap_err();
    assert!(e.contains("would fail") && e.contains("Nothing was signed"), "{e}");
}

#[test]
fn the_wrong_chains_address_is_named() {
    let f = Fake::new();
    let e = solana::plan(&f, &key(ME), &sol_coin(), "0x2c7536E3605D9C16a7a3D7b1898e529396a65c23", "1", &[])
        .unwrap_err();
    assert!(e.contains("Robinhood Chain"), "{e}");
}

#[test]
fn a_token_account_or_a_program_is_not_a_wallet() {
    let mut f = Fake::new();
    f.set(&format!("getAccountInfo:{FRIEND}"),
          json!({"value": {"owner": sol::TOKEN_PROGRAM, "executable": false}}));
    assert!(solana::plan(&f, &key(ME), &sol_coin(), FRIEND, "0.5", &[]).unwrap_err().contains("token account"));
    f.set(&format!("getAccountInfo:{FRIEND}"),
          json!({"value": {"owner": "BPFLoaderUpgradeab1e11111111111111111111111", "executable": true}}));
    assert!(solana::plan(&f, &key(ME), &sol_coin(), FRIEND, "0.5", &[]).unwrap_err().contains("program"));
}

#[test]
fn a_fresh_address_needs_the_rent_minimum() {
    let mut f = Fake::new();
    f.answers.remove(&format!("getAccountInfo:{FRIEND}"));
    let e = solana::plan(&f, &key(ME), &sol_coin(), FRIEND, "0.0001", &[]).unwrap_err();
    assert!(e.contains("at least 0.00089088 SOL"), "{e}");
}

#[test]
fn more_than_you_hold_is_refused() {
    let f = Fake::new();
    let e = solana::plan(&f, &key(ME), &sol_coin(), FRIEND, "3", &[]).unwrap_err();
    assert!(e.contains("Not enough SOL"), "{e}");
}

fn usdc_coin(account: [u8; 32]) -> SolCoin {
    SolCoin { sym: "USDC".into(), mint: Some(key(MINT)), program: key(sol::TOKEN_PROGRAM), decimals: 6,
              account, what: String::new() }
}

#[test]
fn a_coin_send_opens_their_token_account_and_pays_the_right_one() {
    let mut f = Fake::new();
    let mine = sol::associated_token_address(&key(ME), &key(MINT), &key(sol::TOKEN_PROGRAM));
    f.set("getTokenAccountsByOwner", json!({"value": [{"pubkey": sol::base58_encode(&mine), "account": {"data":
           {"parsed": {"info": {"mint": MINT, "tokenAmount": {"amount": "5000000", "decimals": 6}}}}}}]}));
    f.set("getMinimumBalanceForRentExemption", json!(2_039_280));
    // 2.5 USDC leave the token account; SOL pays rent + fee
    let left = 2_500_000u64.to_le_bytes();
    let mut data = vec![0u8; 165];
    data[64..72].copy_from_slice(&left);
    f.set("simulateTransaction", json!({"value": {"err": null, "accounts": [
        {"lamports": 2 * sol::LAMPORTS - 2_039_280 - 5000},
        {"lamports": 2_039_280, "data": [sol::b64_encode(&data), "base64"]}]}}));
    let p = solana::plan(&f, &key(ME), &usdc_coin(mine), FRIEND, "2.5", &[]).unwrap();
    assert_eq!(p.tx.message.instructions.len(), 2, "create-if-missing, then the transfer");
    assert!(p.notes.iter().any(|n| n.contains("first USDC")));
    let lines = solana::review(&p).unwrap();
    assert_eq!(lines[0].1, "2.5 USDC");
    assert_eq!(lines[1].1, FRIEND);

    // and a simulation that moves a different amount is refused
    data[64..72].copy_from_slice(&0u64.to_le_bytes());
    f.set("simulateTransaction", json!({"value": {"err": null, "accounts": [
        {"lamports": 2 * sol::LAMPORTS - 2_039_280 - 5000},
        {"lamports": 2_039_280, "data": [sol::b64_encode(&data), "base64"]}]}}));
    assert!(solana::plan(&f, &key(ME), &usdc_coin(mine), FRIEND, "2.5", &[]).unwrap_err()
        .contains("different amount"));
}

#[test]
fn a_lookalike_of_a_past_recipient_is_flagged() {
    let f = Fake::new();
    // same first and last four characters, different address
    let mut chars: Vec<char> = FRIEND.chars().collect();
    chars[20] = if chars[20] == 'a' { 'b' } else { 'a' };
    let past = key(&chars.into_iter().collect::<String>());
    let p = solana::plan(&f, &key(ME), &sol_coin(), FRIEND, "0.5", &[past]).unwrap();
    assert!(p.notes[0].starts_with('⚠') && p.notes[0].contains("looks like"), "{:?}", p.notes);
}

#[test]
fn recipients_on_both_chains_share_a_file_without_losing_either() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recipients.json");
    solana::remember_recipient(&path, &key(FRIEND)).unwrap();
    elo_net::wallet::remember_recipient(&path, &elo_vault::parse_address(
        "0xF0109fC8DF283027b6285cc889F5aA624EaC1F55").unwrap()).unwrap();
    assert_eq!(solana::known_recipients(&path), vec![key(FRIEND)]);
    assert_eq!(elo_net::wallet::known_recipients(&path).len(), 1);
}

#[test]
fn a_snapshot_names_our_coins_and_shows_others_by_mint() {
    let mut f = Fake::new();
    let acct = [3u8; 32];
    f.set("getTokenAccountsByOwner", json!({"value": [{"pubkey": sol::base58_encode(&acct), "account": {"data":
           {"parsed": {"info": {"mint": MINT, "tokenAmount": {"amount": "7", "decimals": 0}}}}}}]}));
    let s = solana::snapshot(&f, &key(ME), &[]);
    let coins = s.coins.unwrap();
    // asked under both token programs; one account is one row
    assert_eq!(coins.len(), 1);
    assert!(coins[0].0.what.contains("do not list"));
    let s = solana::snapshot(&f, &key(ME), &[(key(MINT), "COIN".into())]);
    assert_eq!(s.coins.unwrap()[0].0.sym, "COIN");
    assert_eq!(s.sol.unwrap(), 2 * sol::LAMPORTS);

    let ours = solana::ours(&json!({"pairs": [
        {"chain": "rh", "base": "JUNK", "base_addr": "0xd6bd13a8a16e5249fa54961fc92eee0307e0f1b9"},
        {"chain": "solana", "base": "COIN", "base_addr": MINT}]}));
    assert_eq!(ours, vec![(key(MINT), "COIN".to_string())]);
}

#[test]
fn a_dead_rpc_is_not_an_empty_wallet() {
    let mut f = Fake::new();
    f.answers.insert("getBalance".into(), Err("could not reach".into()));
    f.answers.insert("getTokenAccountsByOwner".into(), Err("could not reach".into()));
    assert!(solana::snapshot(&f, &key(ME), &[]).unreachable());
}

// ── moves between chains ─────────────────────────────────────────────────────

fn move_quote(tx: &sol::Transaction) -> Value {
    json!({"from": {"amount": 0.1}, "to": {"amount": 0.0043, "min": 0.0042}, "via": "Relay", "takes_s": 2,
           "fees": [{"usd": "0.05"}], "gas": [{"usd": "0.03"}],
           "tx": {"kind": "svm", "serialized_b64": sol::b64_encode(&tx.to_bytes())}})
}

fn someone_elses_tx(payer: [u8; 32]) -> sol::Transaction {
    sol::Transaction::unsigned(sol::compile(&payer, &[sol::system_transfer(&payer, &key(FRIEND), 100_000_000)],
                                            &key(BLOCKHASH))).unwrap()
}

#[test]
fn a_move_from_solana_is_checked_against_its_simulation() {
    let r = xmove::route_for("SOL").unwrap();
    assert_eq!(r.label(), "SOL on Solana → ETH on Robinhood Chain");
    let mut f = Fake::new();
    let tx = someone_elses_tx(key(ME));
    f.set("simulateTransaction", json!({"value": {"err": null, "accounts": [
        {"lamports": 2 * sol::LAMPORTS - 100_000_000 - 5000}]}}));
    let leg = xmove::check_solana(&f, &move_quote(&tx), &r, &key(ME), 100_000_000).unwrap();
    assert_eq!(leg.spent, 100_005_000);

    // taking a full SOL for a 0.1 SOL move is refused
    f.set("simulateTransaction", json!({"value": {"err": null, "accounts": [{"lamports": sol::LAMPORTS}]}}));
    assert!(xmove::check_solana(&f, &move_quote(&tx), &r, &key(ME), 100_000_000).unwrap_err()
        .contains("more than"));

    // a transaction somebody else pays for is not ours to sign
    let theirs = someone_elses_tx(key(FRIEND));
    assert!(xmove::check_solana(&f, &move_quote(&theirs), &r, &key(ME), 100_000_000).unwrap_err()
        .contains("not paid for by your Solana account"));
}

#[test]
fn a_move_that_touches_another_coin_is_refused() {
    let r = xmove::route_for("SOL").unwrap();
    let mut f = Fake::new();
    // the simulation's own token balances: a coin of ours drained to zero
    let row = |amt: &str| json!([{"accountIndex": 3, "mint": MINT, "owner": ME,
                                  "uiTokenAmount": {"amount": amt, "decimals": 6}}]);
    f.set("simulateTransaction", json!({"value": {"err": null,
        "accounts": [{"lamports": 2 * sol::LAMPORTS - 100_005_000}],
        "preBalances": [2 * sol::LAMPORTS], "postBalances": [2 * sol::LAMPORTS - 100_005_000],
        "preTokenBalances": row("9000000"), "postTokenBalances": row("0")}}));
    let e = xmove::check_solana(&f, &move_quote(&someone_elses_tx(key(ME))), &r, &key(ME), 100_000_000)
        .unwrap_err();
    assert!(e.contains("did not choose"), "{e}");

    // the same balances, untouched, pass — and somebody else's coin is not ours to count
    f.set("simulateTransaction", json!({"value": {"err": null,
        "accounts": [{"lamports": 2 * sol::LAMPORTS - 100_005_000}],
        "preBalances": [2 * sol::LAMPORTS], "postBalances": [2 * sol::LAMPORTS - 100_005_000],
        "preTokenBalances": [{"accountIndex": 3, "mint": MINT, "owner": FRIEND,
                              "uiTokenAmount": {"amount": "9", "decimals": 6}}],
        "postTokenBalances": row("9000000")}}));
    assert!(xmove::check_solana(&f, &move_quote(&someone_elses_tx(key(ME))), &r, &key(ME), 100_000_000).is_ok());
}

#[test]
fn the_cost_is_read_from_one_simulation_not_two_reads() {
    // a deposit lands between the balance read and the simulation: the
    // simulation's own pre/post balances still show the real cost
    let r = xmove::route_for("SOL").unwrap();
    let mut f = Fake::new();
    f.set("simulateTransaction", json!({"value": {"err": null,
        "accounts": [{"lamports": 5 * sol::LAMPORTS}],
        "preBalances": [6 * sol::LAMPORTS], "postBalances": [5 * sol::LAMPORTS]}}));
    let e = xmove::check_solana(&f, &move_quote(&someone_elses_tx(key(ME))), &r, &key(ME), 100_000_000)
        .unwrap_err();
    assert!(e.contains("would take 1 SOL"), "{e}");
}

#[test]
fn a_move_from_robinhood_chain_becomes_guarded_steps() {
    let usdg = xmove::route_for("USDG").unwrap();
    let q = json!({"tx": {"kind": "evm", "chain_id": 4663, "to": "0xB477751B76CF82d00a686A1232f5fCD772414Af3",
                          "data": "0xdeadbeef", "value": "0x0"},
                   "approve": {"token": "0x5fc5360d0400a0fd4f2af552add042d716f1d168",
                               "spender": "0xB477751B76CF82d00a686A1232f5fCD772414Af3", "amount": "1000000"}});
    let steps = xmove::evm_steps(&q, &usdg, 1_000_000).unwrap();
    let txs = steps["txs"].as_array().unwrap();
    assert_eq!(txs.len(), 2);
    assert_eq!(txs[1]["value"], "0");
    let data = elo_net::trading::unhex(txs[0]["data"].as_str().unwrap()).unwrap();
    let to = elo_vault::parse_address(txs[0]["to"].as_str().unwrap()).unwrap();
    let v = elo_net::guard::decode(&to, &data);
    assert!(v.listed && v.what.contains("LI.FI") && !v.what.contains("unlimited"), "{v:?}");
    let to = elo_vault::parse_address(txs[1]["to"].as_str().unwrap()).unwrap();
    assert!(elo_net::guard::decode(&to, &[0xde, 0xad, 0xbe, 0xef]).listed);

    let wrong = json!({"tx": {"kind": "evm", "chain_id": 8453, "to": "0x1231DEB6f5749EF6cE6943a275A1D3E7486F4EaE",
                              "data": "0x", "value": "0x0"}});
    assert!(xmove::evm_steps(&wrong, &usdg, 1_000_000).unwrap_err().contains("another chain"));
}

#[test]
fn a_move_from_robinhood_chain_takes_no_more_than_was_typed() {
    // The review is built from the quote's own numbers, so the transaction
    // under it is held to the amount the player typed — as `check_solana`
    // holds the other side.
    let lifi = "0xB477751B76CF82d00a686A1232f5fCD772414Af3";
    let eth = xmove::route_for("ETH").unwrap();
    let pay = |value: &str| json!({"tx": {"kind": "evm", "chain_id": 4663, "to": lifi, "data": "0xdeadbeef",
                                          "value": value}});
    // 0.01 ETH typed, 0.01 ETH in the transaction: fine, and one step.
    let ok = xmove::evm_steps(&pay("0x2386f26fc10000"), &eth, 10_000_000_000_000_000).unwrap();
    assert_eq!(ok["txs"].as_array().unwrap().len(), 1);
    // 0.01 typed, 10 ETH in the transaction: refused.
    let e = xmove::evm_steps(&pay("0x8ac7230489e80000"), &eth, 10_000_000_000_000_000).unwrap_err();
    assert!(e.contains("Nothing was signed"), "{e}");
    // ETH needs no approval.
    let mut with_approval = pay("0x2386f26fc10000");
    with_approval["approve"] = json!({"token": "0x5fc5360d0400a0fd4f2af552add042d716f1d168",
                                      "spender": lifi, "amount": "1"});
    assert!(xmove::evm_steps(&with_approval, &eth, 10_000_000_000_000_000).is_err());

    let usdg = xmove::route_for("USDG").unwrap();
    let approve = |token: &str, amount: &str| json!({
        "tx": {"kind": "evm", "chain_id": 4663, "to": lifi, "data": "0xdeadbeef", "value": "0x0"},
        "approve": {"token": token, "spender": lifi, "amount": amount}});
    let usdg_addr = "0x5fc5360d0400a0fd4f2af552add042d716f1d168";
    assert!(xmove::evm_steps(&approve(usdg_addr, "1000000"), &usdg, 1_000_000).is_ok());
    // More approved than typed, or another coin approved: refused.
    assert!(xmove::evm_steps(&approve(usdg_addr, "5000000"), &usdg, 1_000_000).is_err());
    assert!(xmove::evm_steps(&approve("0xd6bd13a8a16e5249fa54961fc92eee0307e0f1b9", "1000000"), &usdg, 1_000_000)
        .is_err());
    // A USDG move with no approval cannot be what was quoted.
    assert!(xmove::evm_steps(&pay("0x0"), &usdg, 1_000_000).is_err());
}

#[test]
fn a_moves_review_says_where_it_lands() {
    let r = xmove::route_for("ETH").unwrap();
    let q = json!({"from": {"amount": 0.01}, "to": {"amount": 0.23, "min": 0.22}, "via": "Relay", "takes_s": 3,
                   "fees": [{"usd": 0.1}], "gas": [{"usd": "0.02"}]});
    let lines = xmove::review(&r, &q, ME, "0x2c7536E3605D9C16a7a3D7b1898e529396a65c23");
    assert_eq!(lines[0].1, "0.01 ETH on Robinhood Chain");
    assert_eq!(lines[1].1, "about 0.23 SOL on Solana (at least 0.22)");
    assert!(lines[2].1.ends_with(ME), "{}", lines[2].1);
    assert_eq!(lines.last().unwrap().1, "about $0.12 in all");
}
