//! The wallet's rules against a chain that answers what each test says.

use elo_net::chain::Rpc;
use elo_net::wallet::{self, Coin};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;

const ME: &str = "0x2c7536E3605D9C16a7a3D7b1898e529396a65c23";
const FRIEND: &str = "0xF0109fC8DF283027b6285cc889F5aA624EaC1F55";
const JUNK: &str = "0xd6bd13a8a16e5249fa54961fc92eee0307e0f1b9";

fn addr(s: &str) -> [u8; 20] {
    elo_vault::parse_address(s).unwrap()
}

fn word(v: u128) -> Value {
    json!(format!("0x{:064x}", v))
}

/// Answers by method, and `eth_call` by its 4-byte selector.
struct Fake {
    answers: BTreeMap<String, Result<Value, String>>,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn new() -> Fake {
        let mut f = Fake { answers: BTreeMap::new(), calls: RefCell::new(Vec::new()) };
        f.set("eth_chainId", json!("0x1237"));
        f.set("eth_getBalance", json!("0xde0b6b3a7640000")); // 1 ETH
        f.set("eth_getTransactionCount", json!("0x7"));
        f.set("eth_getBlockByNumber", json!({"baseFeePerGas": "0x989680"})); // 0.01 gwei
        f.set("eth_maxPriorityFeePerGas", json!("0x0"));
        f.set("eth_estimateGas", json!("0x7530")); // 30000
        f.set("eth_getCode", json!("0x"));
        f.set("call:70a08231", word(100 * 10u128.pow(18))); // balanceOf: 100
        f.set("call:313ce567", word(18)); // decimals
        f.answers.insert("call:95d89b41".into(), Err("no symbol".into()));
        f
    }
    fn set(&mut self, k: &str, v: Value) {
        self.answers.insert(k.into(), Ok(v));
    }
    fn fail(&mut self, k: &str, why: &str) {
        self.answers.insert(k.into(), Err(why.into()));
    }
}

impl Rpc for Fake {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let key = if method == "eth_call" {
            let data = params[0]["data"].as_str().unwrap_or("");
            format!("call:{}", &data[2..10])
        } else {
            method.to_string()
        };
        self.calls.borrow_mut().push(key.clone());
        self.answers.get(&key).cloned().unwrap_or_else(|| Err(format!("no answer for {key}")))
    }
}

fn junk() -> Coin {
    Coin { sym: "JUNK".into(), token: Some(addr(JUNK)), decimals: 18, what: "Gates' coin".into() }
}

#[test]
fn amounts_are_exact_both_ways() {
    assert_eq!(wallet::parse_amount("12.5", 18), Ok(12_500_000_000_000_000_000));
    assert_eq!(wallet::parse_amount(" 0.000000000000000001 ", 18), Ok(1));
    assert_eq!(wallet::parse_amount(".5", 6), Ok(500_000));
    assert_eq!(wallet::parse_amount("7.", 0), Ok(7));
    assert!(wallet::parse_amount("0.0000000000000000001", 18).unwrap_err().contains("decimal places"));
    for bad in ["", ".", "-1", "1e18", "1,5", "1.2.3", "abc", "１"] {
        assert!(wallet::parse_amount(bad, 18).is_err(), "{bad:?} must be refused");
    }
    assert!(wallet::parse_amount("999999999999999999999999", 18).unwrap_err().contains("larger"));

    assert_eq!(wallet::format_amount(12_500_000_000_000_000_000, 18), "12.5");
    assert_eq!(wallet::format_amount(1, 18), "0.000000000000000001", "dust is shown, not rounded away");
    assert_eq!(wallet::format_amount(0, 18), "0");
    assert_eq!(wallet::format_amount(5_000_000, 6), "5");
    for raw in [1u128, 10, 123_456_789, u128::MAX] {
        assert_eq!(wallet::parse_amount(&wallet::format_amount(raw, 18), 18), Ok(raw));
    }
}

#[test]
fn an_address_is_grouped_for_comparing() {
    assert_eq!(wallet::grouped(FRIEND), "0x F010 9fC8 DF28 3027 b628 5cc8 89F5 aA62 4EaC 1F55");
}

#[test]
fn an_eth_send_is_built_from_the_chains_numbers() {
    let rpc = Fake::new();
    let p = wallet::plan(&rpc, &addr(ME), &Coin::eth(), FRIEND, "0.25", &[]).expect("plan");
    assert_eq!(p.tx.to, addr(FRIEND));
    assert_eq!(p.tx.value, 250_000_000_000_000_000);
    assert!(p.tx.data.is_empty());
    assert_eq!(p.tx.nonce, 7);
    assert_eq!(p.tx.gas, 37_500, "the estimate plus a quarter");
    assert_eq!(p.tx.max_fee, 20_000_000, "twice the base fee, plus no tip");
    assert!(p.notes.iter().any(|n| n.contains("not sent to this address")), "{:?}", p.notes);

    let lines = wallet::review(&p).expect("review");
    assert_eq!(lines[0], ("Send".into(), "0.25 ETH".into()));
    assert_eq!(lines[1].1, wallet::grouped(FRIEND));
    assert_eq!(lines[3].1, "Robinhood Chain (4663)");
    assert_eq!(lines[4].1, "at most 0.00000075 ETH");
}

#[test]
fn a_token_send_calls_transfer_on_the_token_and_the_review_reads_the_calldata() {
    let rpc = Fake::new();
    let p = wallet::plan(&rpc, &addr(ME), &junk(), FRIEND, "12.5", &[addr(FRIEND)]).expect("plan");
    assert_eq!(p.tx.to, addr(JUNK), "a token send goes to the token contract");
    assert_eq!(p.tx.value, 0);
    assert_eq!(elo_vault::tx::read_erc20_transfer(&p.tx.data),
               Some((addr(FRIEND), 12_500_000_000_000_000_000)));
    assert!(p.notes.is_empty(), "a known, plain recipient needs no note: {:?}", p.notes);
    assert_eq!(wallet::review(&p).unwrap()[0].1, "12.5 JUNK");

    // If the transaction ever said something other than what was typed, the
    // review refuses rather than showing the typed version.
    let mut forged = p.clone();
    forged.tx.data = elo_vault::tx::erc20_transfer(&addr(ME), 12_500_000_000_000_000_000);
    assert!(wallet::review(&forged).is_err());
}

#[test]
fn refusals_need_no_key_and_come_before_anything_is_signed() {
    let rpc = Fake::new();
    let me = addr(ME);
    let refuse = |coin: &Coin, to: &str, amt: &str, rpc: &Fake| {
        wallet::plan(rpc, &me, coin, to, amt, &[]).expect_err("must refuse")
    };
    assert!(refuse(&Coin::eth(), "0x0000000000000000000000000000000000000000", "1", &rpc).contains("zero address"));
    assert!(refuse(&junk(), JUNK, "1", &rpc).contains("own address"));
    assert!(refuse(&Coin::eth(), &FRIEND.replacen('F', "f", 1), "1", &rpc).contains("checksum"));
    assert!(refuse(&Coin::eth(), FRIEND, "0", &rpc).contains("above zero"));
    assert!(refuse(&junk(), FRIEND, "101", &rpc).contains("You hold 100 JUNK"));
    assert!(refuse(&Coin::eth(), FRIEND, "1", &rpc).contains("Not enough ETH"), "1 ETH plus a fee");

    let mut other_chain = Fake::new();
    other_chain.set("eth_chainId", json!("0xa4b1"));
    assert!(refuse(&Coin::eth(), FRIEND, "0.1", &other_chain).contains("chain 42161"));

    let mut reverts = Fake::new();
    reverts.fail("eth_estimateGas", "the chain said: execution reverted");
    assert!(refuse(&junk(), FRIEND, "1", &reverts).contains("would fail"));

    let mut no_fee_eth = Fake::new();
    no_fee_eth.set("eth_getBalance", json!("0x0"));
    assert!(refuse(&junk(), FRIEND, "1", &no_fee_eth).contains("Not enough ETH"), "tokens still need gas");

    let mut dead = Fake::new();
    dead.fail("call:70a08231", "could not reach the chain");
    assert!(refuse(&junk(), FRIEND, "1", &dead).contains("could not reach"), "a failed read is not a zero");
}

/// The address-poisoning shape: same first four and last four characters as
/// someone you pay, different in the middle.
#[test]
fn a_lookalike_of_a_known_recipient_is_called_out() {
    let rpc = Fake::new();
    let real = addr(FRIEND);
    let mut fake = real;
    fake[9] ^= 0xff;
    let p = wallet::plan(&rpc, &addr(ME), &Coin::eth(), &elo_vault::to_checksum(&fake), "0.1", &[real])
        .expect("plan");
    assert!(p.notes.iter().any(|n| n.starts_with('⚠') && n.contains(FRIEND)), "{:?}", p.notes);
}

#[test]
fn a_contract_recipient_is_noted() {
    let mut rpc = Fake::new();
    rpc.set("eth_getCode", json!("0x6080"));
    let p = wallet::plan(&rpc, &addr(ME), &Coin::eth(), FRIEND, "0.1", &[addr(FRIEND)]).unwrap();
    assert!(p.notes.iter().any(|n| n.contains("is a contract")), "{:?}", p.notes);
}

/// EIP-7702: an account whose code is `0xef0100` + a contract has handed that
/// contract control. Measured on Robinhood Chain 2026-10-22 at
/// `eth_account`'s public docs key, which carries exactly this.
#[test]
fn a_delegated_account_is_named_on_both_sides() {
    let mut rpc = Fake::new();
    rpc.set("eth_getCode", json!("0xef01008a67b5020ee254ef48e3b6a04927f39baf7e408a"));
    assert_eq!(elo_net::chain::holder(&rpc, &addr(ME)),
               Ok(elo_net::chain::Holder::Delegated(addr("0x8a67b5020ee254ef48e3b6a04927f39baf7e408a"))));
    let own = wallet::own_warning(&rpc, &addr(ME)).expect("our own delegation is a warning");
    assert!(own.starts_with('⚠') && own.contains("never signs one"), "{own}");
    let p = wallet::plan(&rpc, &addr(ME), &Coin::eth(), FRIEND, "0.1", &[addr(FRIEND)]).unwrap();
    assert!(p.notes.iter().any(|n| n.contains("handed control")), "{:?}", p.notes);

    rpc.set("eth_getCode", json!("0x"));
    assert_eq!(wallet::own_warning(&rpc, &addr(ME)), None);
}

/// The origin names the coins; the contract says what they are.
#[test]
fn the_book_takes_decimals_from_the_chain_and_drops_what_will_not_say() {
    let mut rpc = Fake::new();
    rpc.set("call:313ce567", word(6));
    let listed = json!({"coins": [
        {"sym": "JUNK", "addr": JUNK, "dec": 18, "game": {"name": "Gates"}},
        {"sym": "NOPE", "addr": null},
        {"sym": "BAD", "addr": "0x1234"},
    ]});
    let book = wallet::book(&listed, &rpc);
    assert_eq!(book[0], Coin::eth());
    assert_eq!(book.len(), 2);
    assert_eq!(book[1].decimals, 6, "the contract's decimals, not the list's");
    assert_eq!(book[1].what, "Gates's coin");

    let mut silent = Fake::new();
    silent.fail("call:313ce567", "reverted");
    assert_eq!(wallet::book(&listed, &silent).len(), 1, "no decimals, no row");
    // But the row is said to be missing, not quietly dropped.
    let (_, unread) = wallet::book_and_unread(&listed, &silent);
    assert_eq!(unread, ["JUNK"]);
}

#[test]
fn recipients_round_trip_and_a_missing_file_is_an_empty_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recipients.json");
    assert!(wallet::known_recipients(&path).is_empty());
    wallet::remember_recipient(&path, &addr(FRIEND)).unwrap();
    wallet::remember_recipient(&path, &addr(FRIEND)).unwrap();
    wallet::remember_recipient(&path, &addr(ME)).unwrap();
    assert_eq!(wallet::known_recipients(&path), vec![addr(FRIEND), addr(ME)]);
}
