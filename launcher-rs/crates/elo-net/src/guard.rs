//! What this app lets its own key sign, the settings that shape it, and the log
//! of every request (`docs/client/SIGNING-ALLOWLIST.md`, the same list the
//! site's `js/signguard.js` carries).
//!
//! The list is compiled in. The origin prepares transactions; it cannot widen
//! what this binary will sign, so a breached origin still cannot point this
//! machine's key at a contract outside the ecosystem. A call is allowed when it
//! goes to a listed contract (a token contract only for an approval to a listed
//! spender), and refused otherwise — unless the holder turned on
//! `allow_outside`, in which case it is allowed, warned about, and logged.
//!
//! Settings live in `settings.json` and the log in `activity.jsonl`, both beside
//! the keystore. The log is append-only text on this machine and never sent.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One contract on Robinhood Chain this app signs for.
pub struct Listed {
    pub address: &'static str,
    pub what: &'static str,
    /// may be named as the spender of an approval
    pub spender: bool,
    /// a token: approvals only
    pub token: bool,
    /// may be named as the operator of `setApprovalForAll` on one of ours
    pub operator: bool,
    /// one of our collections: `setApprovalForAll` to a listed operator only
    pub nft: bool,
}

pub const ALLOW: &[Listed] = &[
    Listed { address: "0x8876789976decbfcbbbe364623c63652db8c0904", what: "Uniswap UniversalRouter", spender: true, token: false, operator: false, nft: false },
    Listed { address: "0x000000000022d473030f116ddee9f6b43ac78ba3", what: "Permit2", spender: true, token: false, operator: false, nft: false },
    Listed { address: "0xb477751b76cf82d00a686a1232f5fcd772414af3", what: "LI.FI (cross-chain)", spender: true, token: false, operator: false, nft: false },
    Listed { address: "0x0000000000000068f116a894984e2db1123eb395", what: "Seaport 1.6 (OpenSea)", spender: false, token: false, operator: true, nft: false },
    // OpenSea's conduit (one address on every chain it is on): an operator our
    // collections may approve, never called directly (`token` = no direct call)
    Listed { address: "0x1e0049783f008a0085193e00003d00cd54003c71", what: "OpenSea's conduit", spender: false, token: true, operator: true, nft: false },
    // our collections: a holder approves OpenSea's operator to list; nothing else
    Listed { address: "0x93f4965638f9c99dd53f13bf2b47358a6ff7630c", what: "Gates skins", spender: false, token: true, operator: false, nft: true },
    // a copy is bought from its own contract (mintFree, buyWith…) after an
    // exact-amount approval to it, so it is a spender and takes calls
    Listed { address: "0x54538c224ae8c5bacc6382b86eed301e10d3e5e6", what: "Gates copies", spender: true, token: false, operator: false, nft: true },
    Listed { address: "0x2f2169bcd4e674c76c417191d23ef3c1cfdabf7b", what: "Elo Pros", spender: false, token: true, operator: false, nft: true },
    // trading cards: approved to the market to list and to the kiln to craft
    Listed { address: "0x520259be81b84af336f8c2ab7c2b7ac2529a1c66", what: "Elo Pros cards", spender: false, token: true, operator: false, nft: true },
    Listed { address: "0xd71045e27d97814bb8f998876ab671e835d22d80", what: "Elo Pros badges", spender: false, token: true, operator: false, nft: true },
    Listed { address: "0xe77642f54a083e0703fd5485d8547d1fb84e5ee5", what: "the card market", spender: false, token: false, operator: true, nft: false },
    Listed { address: "0xbf513e02670f0c147ff47652359acf65fe76478e", what: "the card kiln", spender: false, token: false, operator: true, nft: false },
    // booster packs: bought for ETH, or for JUNK after an exact-amount approval
    Listed { address: "0xc29c3f56cdcd799c8cafba0e1a5830abab45f45c", what: "booster packs", spender: true, token: false, operator: false, nft: false },
    // the Gates skin shop (EloItemStore:GATES): buys a skin for JUNK after an
    // exact-amount approval to it
    Listed { address: "0x6ca8f8c4657c08ee291cead3caf827530220c260", what: "the Gates skin shop", spender: true, token: false, operator: false, nft: false },
    Listed { address: "0xf7997e4fd821e39d884d86d25e1655017f241a42", what: "the reserve token", spender: false, token: true, operator: false, nft: false },
    Listed { address: "0xd6bd13a8a16e5249fa54961fc92eee0307e0f1b9", what: "JUNK", spender: false, token: true, operator: false, nft: false },
    Listed { address: "0x0bd7d308f8e1639fab988df18a8011f41eacad73", what: "WETH", spender: false, token: true, operator: false, nft: false },
    Listed { address: "0x5fc5360d0400a0fd4f2af552add042d716f1d168", what: "USDG", spender: false, token: true, operator: false, nft: false },
];

const APPROVE: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];
const PERMIT2_APPROVE: [u8; 4] = [0x87, 0x51, 0x7c, 0x45];
pub const SET_APPROVAL_FOR_ALL: [u8; 4] = [0xa2, 0x2c, 0xb4, 0x65];

fn hex20(a: &[u8; 20]) -> String {
    let mut s = String::from("0x");
    for b in a {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn find(a: &[u8; 20]) -> Option<&'static Listed> {
    let h = hex20(a);
    ALLOW.iter().find(|l| l.address == h)
}

fn word_addr(data: &[u8], i: usize) -> Option<[u8; 20]> {
    data.get(4 + 32 * i + 12..4 + 32 * (i + 1)).and_then(|w| w.try_into().ok())
}

/// What a call is, in plain words, and whether the list allows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub listed: bool,
    pub what: String,
}

pub fn decode(to: &[u8; 20], data: &[u8]) -> Verdict {
    let sel: Option<[u8; 4]> = data.get(..4).and_then(|s| s.try_into().ok());
    let name = |a: &[u8; 20]| find(a).map(|l| l.what.to_string()).unwrap_or_else(|| hex20(a));
    if sel == Some(SET_APPROVAL_FOR_ALL) {
        if let Some(op) = word_addr(data, 0) {
            let ok = find(to).is_some_and(|l| l.nft) && find(&op).is_some_and(|l| l.operator);
            return Verdict { listed: ok, what: format!("let {} move your {} when one sells", name(&op), name(to)) };
        }
    }
    if sel == Some(APPROVE) {
        if let Some(sp) = word_addr(data, 0) {
            let ok = find(&sp).is_some_and(|l| l.spender);
            let unlimited = data.get(36..52).is_some_and(|hi| hi.iter().all(|b| *b == 0xff));
            return Verdict { listed: ok, what: format!("approve {}{}", name(&sp),
                                                       if unlimited { " (unlimited)" } else { "" }) };
        }
    }
    if sel == Some(PERMIT2_APPROVE) && find(to).is_some_and(|l| l.what == "Permit2") {
        if let Some(sp) = word_addr(data, 1) {
            return Verdict { listed: find(&sp).is_some_and(|l| l.spender),
                             what: format!("Permit2: let {} spend it", name(&sp)) };
        }
    }
    match find(to) {
        Some(l) if !l.token => Verdict { listed: true, what: l.what.to_string() },
        Some(l) => Verdict { listed: false, what: format!("{}: a call that is not an approval", l.what) },
        None => Verdict { listed: false, what: format!("an unknown contract {}", hex20(to)) },
    }
}

/// [`decode`], with the ETH the call carries. An approval is allowed on its
/// SPENDER, whatever contract it is sent to, so that a coin the list does not
/// name can still be approved to the router — which is only safe while an
/// approval moves no money. One that carries ETH is not an approval anyone
/// asked for: it is a payment to whatever `to` is, and is refused like one.
pub fn verdict(to: &[u8; 20], data: &[u8], value: u128) -> Verdict {
    let v = decode(to, data);
    let sel: Option<[u8; 4]> = data.get(..4).and_then(|s| s.try_into().ok());
    let approval = matches!(sel, Some(APPROVE | PERMIT2_APPROVE | SET_APPROVAL_FOR_ALL));
    if v.listed && approval && value > 0 {
        return Verdict { listed: false, what: format!("{}, sending ETH with it", v.what) };
    }
    v
}

/// The holder's settings, and the defaults: low friction, with the list on.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub approve_once: bool,
    pub check: bool,
    pub allow_outside: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { approve_once: true, check: true, allow_outside: false }
    }
}

pub const SETTING_NAMES: [&str; 3] = ["approve_once", "check", "allow_outside"];

pub fn settings_path() -> PathBuf {
    elo_vault::keystore_path().with_file_name("settings.json")
}

pub fn activity_path() -> PathBuf {
    elo_vault::keystore_path().with_file_name("activity.jsonl")
}

impl Settings {
    pub fn load(p: &Path) -> Settings {
        let v: Value = std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let d = Settings::default();
        let b = |k: &str, def: bool| v.get(k).and_then(Value::as_bool).unwrap_or(def);
        Settings { approve_once: b("approve_once", d.approve_once), check: b("check", d.check),
                   allow_outside: b("allow_outside", d.allow_outside) }
    }

    pub fn save(&self, p: &Path) -> Result<(), String> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let body = json!({"approve_once": self.approve_once, "check": self.check, "allow_outside": self.allow_outside});
        std::fs::write(p, serde_json::to_string_pretty(&body).unwrap_or_default()).map_err(|e| e.to_string())
    }

    pub fn set(&mut self, name: &str, on: bool) -> Result<(), String> {
        match name {
            "approve_once" => self.approve_once = on,
            "check" => self.check = on,
            "allow_outside" => self.allow_outside = on,
            _ => return Err(format!("a setting is one of {}", SETTING_NAMES.join(", "))),
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<bool> {
        match name {
            "approve_once" => Some(self.approve_once),
            "check" => Some(self.check),
            "allow_outside" => Some(self.allow_outside),
            _ => None,
        }
    }
}

/// Append one entry to the activity log. Never fails the caller: a log that
/// cannot be written says so on stderr and the trade's own result stands.
pub fn log(p: &Path, entry: Value) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut e = entry;
    if let Some(o) = e.as_object_mut() {
        o.insert("t".into(), json!(now));
    }
    let write = || -> std::io::Result<()> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
        writeln!(f, "{e}")
    };
    if let Err(err) = write() {
        eprintln!("  (the activity log could not be written: {err})");
    }
}

/// The last `n` entries, newest last.
pub fn read_log(p: &Path, n: usize) -> Vec<Value> {
    let text = std::fs::read_to_string(p).unwrap_or_default();
    let all: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let skip = all.len().saturating_sub(n);
    all.into_iter().skip(skip).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> [u8; 20] {
        let s = s.trim_start_matches("0x");
        let mut out = [0u8; 20];
        for i in 0..20 {
            out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }
    fn call(sel: [u8; 4], words: &[Vec<u8>]) -> Vec<u8> {
        let mut d = sel.to_vec();
        for w in words {
            d.extend(w);
        }
        d
    }
    fn addr_word(s: &str) -> Vec<u8> {
        let mut w = vec![0u8; 12];
        w.extend(a(s));
        w
    }
    const P2: &str = "0x000000000022d473030f116ddee9f6b43ac78ba3";
    const UR: &str = "0x8876789976decbfcbbbe364623c63652db8c0904";
    const RES: &str = "0xf7997e4fd821e39d884d86d25e1655017f241a42";
    const EVIL: &str = "0x6666666666666666666666666666666666666666";

    #[test]
    fn approvals_only_to_listed_spenders() {
        let max = vec![0xffu8; 32];
        assert!(decode(&a(RES), &call(APPROVE, &[addr_word(P2), max.clone()])).listed);
        assert!(decode(&a(RES), &call(APPROVE, &[addr_word(P2), max.clone()])).what.contains("unlimited"));
        assert!(!decode(&a(RES), &call(APPROVE, &[addr_word(EVIL), max])).listed);
    }

    #[test]
    fn an_approval_that_carries_eth_is_refused_wherever_it_is_sent() {
        // An approval is judged on its spender, not its `to` — so the ETH it
        // carries is the only thing standing between a breached origin and a
        // payment to any contract that answers to `approve`.
        let ok = call(APPROVE, &[addr_word(UR), vec![0u8; 32]]);
        assert!(verdict(&a(RES), &ok, 0).listed);
        assert!(!verdict(&a(EVIL), &ok, 1).listed);
        assert!(!verdict(&a(RES), &ok, 10u128.pow(18)).listed);
        let p2 = call(PERMIT2_APPROVE, &[addr_word(RES), addr_word(UR), vec![0xff; 32], vec![0xff; 32]]);
        assert!(!verdict(&a(P2), &p2, 1).listed);
        // A call to a listed router may still pay: that is what a swap of ETH is.
        assert!(verdict(&a(UR), &[0x35, 0x93, 0x56, 0x4c], 5).listed);
    }

    #[test]
    fn calls_only_to_listed_contracts_and_never_a_token_transfer() {
        assert!(decode(&a(UR), &[0x35, 0x93, 0x56, 0x4c]).listed);
        assert!(!decode(&a(EVIL), &[0x35, 0x93, 0x56, 0x4c]).listed);
        let transfer = call([0xa9, 0x05, 0x9c, 0xbb], &[addr_word(EVIL), vec![1u8; 32]]);
        assert!(!decode(&a(RES), &transfer).listed);
    }

    #[test]
    fn permit2_names_its_spender() {
        let w = call(PERMIT2_APPROVE, &[addr_word(RES), addr_word(UR), vec![0xff; 32], vec![0xff; 32]]);
        assert!(decode(&a(P2), &w).listed);
        let bad = call(PERMIT2_APPROVE, &[addr_word(RES), addr_word(EVIL), vec![0xff; 32], vec![0xff; 32]]);
        assert!(!decode(&a(P2), &bad).listed);
    }

    #[test]
    fn our_collections_approve_opensea_and_nobody_else() {
        const SKINS: &str = "0x93f4965638f9c99dd53f13bf2b47358a6ff7630c";
        const SEA: &str = "0x0000000000000068f116a894984e2db1123eb395";
        const CONDUIT: &str = "0x1e0049783f008a0085193e00003d00cd54003c71";
        let mut yes = vec![0u8; 32];
        yes[31] = 1;
        let safa = |to: &str, op: &str| decode(&a(to), &call(SET_APPROVAL_FOR_ALL, &[addr_word(op), yes.clone()]));
        assert!(safa(SKINS, SEA).listed, "our skins to Seaport");
        assert!(safa(SKINS, CONDUIT).listed, "our skins to OpenSea's conduit");
        assert!(!safa(SKINS, EVIL).listed, "our skins to a stranger");
        assert!(!safa(EVIL, SEA).listed, "someone else's collection");
        assert!(!safa(RES, SEA).listed, "a coin is not a collection");
        let transfer = call([0x23, 0xb8, 0x72, 0xdd], &[addr_word(EVIL), addr_word(EVIL), vec![7u8; 32]]);
        assert!(!decode(&a(SKINS), &transfer).listed, "no direct call to a collection");
        assert!(!decode(&a(CONDUIT), &[0x4c, 0xe3, 0x4a, 0xa2]).listed, "no direct call to the conduit");
        assert!(decode(&a(SEA), &[0xfd, 0x9f, 0x1e, 0x10]).listed, "Seaport itself: buy, cancel, accept");
    }

    #[test]
    fn the_defaults_are_low_friction_with_the_list_on() {
        assert_eq!(Settings::default(), Settings { approve_once: true, check: true, allow_outside: false });
    }

    #[test]
    fn settings_and_the_log_persist() {
        let dir = tempfile::tempdir().unwrap();
        let sp = dir.path().join("settings.json");
        let mut s = Settings::load(&sp);
        s.set("approve_once", false).unwrap();
        assert!(s.set("everything", true).is_err());
        s.save(&sp).unwrap();
        assert!(!Settings::load(&sp).approve_once);
        let lp = dir.path().join("activity.jsonl");
        log(&lp, json!({"what": "one"}));
        log(&lp, json!({"what": "two"}));
        let got = read_log(&lp, 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["what"], "two");
        assert!(got[0]["t"].as_u64().is_some());
    }
}
