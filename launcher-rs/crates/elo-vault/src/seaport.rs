//! Seaport 1.6 listings: one of the two kinds of typed data this client signs
//! (the other is a USDG credit pack, `usdg.rs`).
//!
//! # Why this exists when `tx.rs` says no typed data
//!
//! Selling an item on OpenSea is a Seaport order signed with EIP-712, and the
//! operator decided the desktop app sells (2026-10-23). The danger in typed
//! data is a *generic* signer: a page hands over a struct it names, and the
//! wallet signs a permit that gives a token allowance away. Nothing here is
//! generic. The domain is a constant (Seaport, version 1.6, chain 4663,
//! Seaport's address); the struct is Seaport's `OrderComponents`, hashed by
//! code in this file with its type string compiled in; no caller can pass a
//! type, a domain or a digest. A signature made here is only ever a Seaport
//! order on this chain, and Seaport only moves what the order's own items say.
//!
//! [`check_listing`] is the second half: the signer refuses any order that is
//! not one item offered by this account and paid to this account first, in
//! ETH, at a fixed price. Collection and fee checks live with the caller, which
//! knows what it asked the origin to build (`elo skins sell` rebuilds it).
//!
//! The hash matches the meter's `meter/seaport.py` and `eth_account`
//! (`tests/seaport.rs` pins the fixture digest both produce).

use serde_json::Value;
use sha3::{Digest, Keccak256};

/// Seaport 1.6, at the same address on every chain.
pub const SEAPORT: [u8; 20] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x68, 0xf1, 0x16, 0xa8, 0x94, 0x98, 0x4e, 0x2d, 0xb1, 0x12, 0x3e,
    0xb3, 0x95,
];

const OFFER_ITEM: &str = "OfferItem(uint8 itemType,address token,uint256 identifierOrCriteria,\
uint256 startAmount,uint256 endAmount)";
const CONSIDERATION_ITEM: &str = "ConsiderationItem(uint8 itemType,address token,uint256 identifierOrCriteria,\
uint256 startAmount,uint256 endAmount,address recipient)";
const ORDER_COMPONENTS: &str = "OrderComponents(address offerer,address zone,OfferItem[] offer,\
ConsiderationItem[] consideration,uint8 orderType,uint256 startTime,uint256 endTime,bytes32 zoneHash,\
uint256 salt,bytes32 conduitKey,uint256 counter)";
const DOMAIN: &str = "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

pub const NATIVE: u8 = 0;
pub const ERC721: u8 = 2;

/// A 256-bit number, big-endian.
pub type Word = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub item_type: u8,
    pub token: [u8; 20],
    pub identifier: Word,
    pub start: Word,
    pub end: Word,
    /// consideration items only
    pub recipient: Option<[u8; 20]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    pub offerer: [u8; 20],
    pub zone: [u8; 20],
    pub offer: Vec<Item>,
    pub consideration: Vec<Item>,
    pub order_type: u8,
    pub start_time: Word,
    pub end_time: Word,
    pub zone_hash: Word,
    pub salt: Word,
    pub conduit_key: Word,
    pub counter: Word,
}

fn keccak(b: &[u8]) -> Word {
    Keccak256::digest(b).into()
}

fn addr_word(a: &[u8; 20]) -> Word {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

fn small(v: u8) -> Word {
    let mut w = [0u8; 32];
    w[31] = v;
    w
}

/// A decimal string (or a JSON number) as a 256-bit word.
pub fn dec_word(s: &str) -> Result<Word, String> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("not a whole number: {s:?}"));
    }
    let mut w = [0u8; 32];
    for d in s.bytes() {
        let mut carry = (d - b'0') as u32;
        for byte in w.iter_mut().rev() {
            let x = (*byte as u32) * 10 + carry;
            *byte = (x & 0xff) as u8;
            carry = x >> 8;
        }
        if carry != 0 {
            return Err(format!("{s} does not fit 256 bits"));
        }
    }
    Ok(w)
}

/// A word as a decimal string.
pub fn word_dec(w: &Word) -> String {
    let mut n = w.to_vec();
    let mut digits = Vec::new();
    while n.iter().any(|b| *b != 0) {
        let mut rem = 0u32;
        for byte in n.iter_mut() {
            let x = (rem << 8) | *byte as u32;
            *byte = (x / 10) as u8;
            rem = x % 10;
        }
        digits.push(b'0' + rem as u8);
    }
    if digits.is_empty() {
        return "0".into();
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}

/// The low 128 bits of a word, or None when it is larger.
pub fn word_u128(w: &Word) -> Option<u128> {
    if w[..16].iter().any(|b| *b != 0) {
        return None;
    }
    Some(u128::from_be_bytes(w[16..].try_into().ok()?))
}

fn hex32(s: &str) -> Result<Word, String> {
    let raw = crate::unhex(s.trim_start_matches("0x"))?;
    raw.try_into().map_err(|_| format!("{s} is not 32 bytes"))
}

fn num_of(v: &Value, k: &str) -> Result<Word, String> {
    match v.get(k) {
        Some(Value::String(s)) => dec_word(s),
        Some(Value::Number(n)) => dec_word(&n.to_string()),
        _ => Err(format!("the order has no {k}")),
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v.get(k).and_then(Value::as_str).ok_or_else(|| format!("the order has no {k}"))
}

fn small_of(v: &Value, k: &str) -> Result<u8, String> {
    let w = num_of(v, k)?;
    if w[..31].iter().any(|b| *b != 0) {
        return Err(format!("{k} is out of range"));
    }
    Ok(w[31])
}

fn item_of(v: &Value, consideration: bool) -> Result<Item, String> {
    Ok(Item {
        item_type: small_of(v, "itemType")?,
        token: crate::parse_address(str_of(v, "token")?)?,
        identifier: num_of(v, "identifierOrCriteria")?,
        start: num_of(v, "startAmount")?,
        end: num_of(v, "endAmount")?,
        recipient: if consideration { Some(crate::parse_address(str_of(v, "recipient")?)?) } else { None },
    })
}

impl Order {
    /// The order from the `message` of the typed data the origin prepared.
    pub fn from_json(m: &Value) -> Result<Order, String> {
        let list = |k: &str, cons: bool| -> Result<Vec<Item>, String> {
            m.get(k).and_then(Value::as_array).ok_or_else(|| format!("the order has no {k}"))?
                .iter().map(|i| item_of(i, cons)).collect()
        };
        Ok(Order {
            offerer: crate::parse_address(str_of(m, "offerer")?)?,
            zone: crate::parse_address(str_of(m, "zone")?)?,
            offer: list("offer", false)?,
            consideration: list("consideration", true)?,
            order_type: small_of(m, "orderType")?,
            start_time: num_of(m, "startTime")?,
            end_time: num_of(m, "endTime")?,
            zone_hash: hex32(str_of(m, "zoneHash")?)?,
            salt: num_of(m, "salt")?,
            conduit_key: hex32(str_of(m, "conduitKey")?)?,
            counter: num_of(m, "counter")?,
        })
    }

    /// Seaport's `getOrderHash`: the OrderComponents struct hash.
    pub fn hash(&self) -> Word {
        let offer_th = keccak(OFFER_ITEM.as_bytes());
        let cons_th = keccak(CONSIDERATION_ITEM.as_bytes());
        let mut offers = Vec::new();
        for i in &self.offer {
            let mut b = Vec::with_capacity(6 * 32);
            for w in [offer_th, small(i.item_type), addr_word(&i.token), i.identifier, i.start, i.end] {
                b.extend_from_slice(&w);
            }
            offers.extend_from_slice(&keccak(&b));
        }
        let mut cons = Vec::new();
        for i in &self.consideration {
            let mut b = Vec::with_capacity(7 * 32);
            for w in [cons_th, small(i.item_type), addr_word(&i.token), i.identifier, i.start, i.end,
                      addr_word(&i.recipient.unwrap_or([0u8; 20]))] {
                b.extend_from_slice(&w);
            }
            cons.extend_from_slice(&keccak(&b));
        }
        let order_th = keccak(format!("{ORDER_COMPONENTS}{CONSIDERATION_ITEM}{OFFER_ITEM}").as_bytes());
        let mut b = Vec::with_capacity(12 * 32);
        for w in [order_th, addr_word(&self.offerer), addr_word(&self.zone), keccak(&offers), keccak(&cons),
                  small(self.order_type), self.start_time, self.end_time, self.zone_hash, self.salt,
                  self.conduit_key, self.counter] {
            b.extend_from_slice(&w);
        }
        keccak(&b)
    }

    /// What the key signs: `keccak256(0x1901 ‖ domain ‖ hash)`, the domain
    /// fixed to Seaport 1.6 on chain 4663.
    pub fn digest(&self) -> Word {
        let mut b = vec![0x19, 0x01];
        b.extend_from_slice(&domain_separator());
        b.extend_from_slice(&self.hash());
        keccak(&b)
    }
}

pub fn domain_separator() -> Word {
    let mut chain = [0u8; 32];
    chain[24..].copy_from_slice(&crate::tx::CHAIN_ID.to_be_bytes());
    let mut b = Vec::with_capacity(5 * 32);
    for w in [keccak(DOMAIN.as_bytes()), keccak(b"Seaport"), keccak(b"1.6"), chain, addr_word(&SEAPORT)] {
        b.extend_from_slice(&w);
    }
    keccak(&b)
}

/// The shape of a listing this account may sign: one item it offers, paid in
/// ETH at a fixed price, to itself first. Anything else is refused here, in
/// the vault, whatever the caller asked for.
pub fn check_listing(o: &Order, me: &[u8; 20]) -> Result<(), String> {
    if o.offerer != *me {
        return Err("this order lists for another account".into());
    }
    if o.offer.len() != 1 || o.offer[0].item_type != ERC721 || word_u128(&o.offer[0].start) != Some(1)
        || o.offer[0].start != o.offer[0].end {
        return Err("a listing offers exactly one item".into());
    }
    if o.consideration.is_empty() || o.consideration[0].recipient != Some(*me) {
        return Err("a listing pays this account first".into());
    }
    if o.consideration.iter().any(|c| c.item_type != NATIVE || c.token != [0u8; 20] || c.start != c.end) {
        return Err("a listing is paid in ETH at one fixed price".into());
    }
    Ok(())
}

impl crate::Account {
    /// Sign a Seaport listing. Crate-private: the only caller is
    /// `LocalSigner::sign_seaport_listing`, which asked for the passphrase.
    pub(crate) fn sign_listing(&self, o: &Order) -> Result<String, String> {
        use k256::ecdsa::{RecoveryId, Signature};
        let me = crate::parse_address(&self.address())?;
        check_listing(o, &me)?;
        let (sig, recid): (Signature, RecoveryId) = self
            .key
            .sign_prehash_recoverable(&o.digest())
            .map_err(|e| format!("signing failed: {e}"))?;
        let mut out = Vec::with_capacity(65);
        out.extend_from_slice(&sig.r().to_bytes());
        out.extend_from_slice(&sig.s().to_bytes());
        out.push(recid.to_byte() + 27);
        Ok(format!("0x{}", crate::hex(&out)))
    }
}
