//! USDG transfer authorizations for buying Elo Pros credits: the second kind
//! of typed data this client signs, and as narrow as the first.
//!
//! # Why it exists
//!
//! `elo credits buy` pays the store's HTTP 402 door (`meter/paydoor.py`): the
//! account signs an EIP-3009 `TransferWithAuthorization` for USDG on Robinhood
//! Chain, and the store submits it and pays the gas. Seaport (`seaport.rs`)
//! states the danger in typed data: a generic signer signs whatever struct a
//! page names. Nothing here is generic. The domain is a constant (USDG's own:
//! *Global Dollar*, version 1, chain 4663, its address, matched against the
//! token's `DOMAIN_SEPARATOR` on 2026-10-30); the struct and its type string
//! are compiled in; no caller passes a type, a domain or a digest.
//!
//! [`check`] is the second half: the signer refuses an authorization that is
//! not from this account, to a credits address compiled in below, for at most
//! [`MAX_UNITS`], valid now and for at most [`MAX_WINDOW_S`]. A changed origin
//! can therefore name no other payee and no larger amount than a credit pack.
//!
//! The digest and signature match `eth_account` byte for byte (the test below
//! pins the vector it produced).
use sha3::{Digest, Keccak256};

/// USDG on Robinhood Chain.
pub const USDG: [u8; 20] = [
    0x5f, 0xc5, 0x36, 0x0d, 0x04, 0x00, 0xa0, 0xfd, 0x4f, 0x2a, 0xf5, 0x52, 0xad, 0xd0, 0x42, 0xd7, 0x16, 0xf1,
    0xd1, 0x68,
];
pub const CHAIN_ID: u64 = 4663;
/// The largest credit pack, $50, in USDG base units (6 decimals).
pub const MAX_UNITS: u128 = 50_000_000;
/// The pay door's challenges live ten minutes; an authorization signed here
/// lives no longer than fifteen.
pub const MAX_WINDOW_S: u64 = 900;
/// Where credits are paid: the store's credits address (`data/store/credits.json`
/// `crypto.rh_address`, HELD-KEYS row 3l). Changing it is a client release.
pub const CREDIT_ADDRESSES: [[u8; 20]; 1] = [[
    0x24, 0x44, 0x5e, 0xfd, 0xdb, 0x08, 0xd4, 0x93, 0x8e, 0x3e, 0x36, 0x27, 0x04, 0x2b, 0x2c, 0xf4, 0x06, 0x3d,
    0x90, 0x92,
]];

const DOMAIN: &str = "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const TWA: &str = "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,\
uint256 validBefore,bytes32 nonce)";

type Word = [u8; 32];

fn keccak(b: &[u8]) -> Word {
    Keccak256::digest(b).into()
}

fn addr_word(a: &[u8; 20]) -> Word {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

fn u128_word(v: u128) -> Word {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

/// One EIP-3009 `transferWithAuthorization` of USDG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    pub from: [u8; 20],
    pub to: [u8; 20],
    pub value: u128,
    pub valid_after: u64,
    pub valid_before: u64,
    pub nonce: Word,
}

pub fn domain_separator() -> Word {
    let mut chain = [0u8; 32];
    chain[24..].copy_from_slice(&CHAIN_ID.to_be_bytes());
    let mut b = Vec::with_capacity(32 * 5);
    for w in [keccak(DOMAIN.as_bytes()), keccak(b"Global Dollar"), keccak(b"1"), chain, addr_word(&USDG)] {
        b.extend_from_slice(&w);
    }
    keccak(&b)
}

impl Authorization {
    pub fn hash(&self) -> Word {
        let mut b = Vec::with_capacity(32 * 7);
        for w in [keccak(TWA.as_bytes()), addr_word(&self.from), addr_word(&self.to), u128_word(self.value),
                  u128_word(self.valid_after as u128), u128_word(self.valid_before as u128), self.nonce] {
            b.extend_from_slice(&w);
        }
        keccak(&b)
    }

    /// What the key signs: `keccak256(0x1901 ‖ domain ‖ hash)`.
    pub fn digest(&self) -> Word {
        let mut b = Vec::with_capacity(66);
        b.extend_from_slice(&[0x19, 0x01]);
        b.extend_from_slice(&domain_separator());
        b.extend_from_slice(&self.hash());
        keccak(&b)
    }
}

/// Every refusal before a key is touched.
pub fn check(a: &Authorization, me: &[u8; 20], now: u64) -> Result<(), String> {
    if &a.from != me {
        return Err("an authorization pays from this account".into());
    }
    if !CREDIT_ADDRESSES.contains(&a.to) {
        return Err("an authorization pays the Elo Pros credits address, and no other".into());
    }
    if a.value == 0 || a.value > MAX_UNITS {
        return Err(format!("an authorization is for 0.000001 to {} USDG", MAX_UNITS / 1_000_000));
    }
    if a.valid_after > now {
        return Err("an authorization is valid from now".into());
    }
    if a.valid_before <= now || a.valid_before > now + MAX_WINDOW_S {
        return Err(format!("an authorization lives at most {MAX_WINDOW_S} seconds"));
    }
    Ok(())
}

impl crate::Account {
    /// Sign a USDG authorization. Crate-private: the only caller is
    /// `LocalSigner::sign_usdg_authorization`, which asked for the passphrase.
    pub(crate) fn sign_usdg(&self, a: &Authorization, now: u64) -> Result<String, String> {
        use k256::ecdsa::{RecoveryId, Signature};
        let me = crate::parse_address(&self.address())?;
        check(a, &me, now)?;
        let (sig, recid): (Signature, RecoveryId) = self
            .key
            .sign_prehash_recoverable(&a.digest())
            .map_err(|e| format!("signing failed: {e}"))?;
        let mut out = Vec::with_capacity(65);
        out.extend_from_slice(&sig.r().to_bytes());
        out.extend_from_slice(&sig.s().to_bytes());
        out.push(recid.to_byte() + 27);
        Ok(format!("0x{}", crate::hex(&out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (crate::Account, Authorization) {
        // The well-known test key; the vector below is eth_account's for it.
        let key = crate::unhex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318").unwrap();
        let acct = crate::Account::from_secret(&key).unwrap();
        let from = crate::parse_address(&acct.address()).unwrap();
        (acct, Authorization { from, to: CREDIT_ADDRESSES[0], value: 10_000_000, valid_after: 0,
                               valid_before: 1_800_000_000, nonce: [0x11; 32] })
    }

    #[test]
    fn digest_and_signature_match_eth_account() {
        let (acct, a) = fixture();
        assert_eq!(acct.address(), "0x2c7536E3605D9C16a7a3D7b1898e529396a65c23");
        assert_eq!(crate::hex(&a.digest()), "f6cbbe91c19f4410209f152be6be77d42e8b558048eb5319f2eb7d1edb34cf27");
        let sig = acct.sign_usdg(&a, 1_800_000_000 - 300).unwrap();
        assert_eq!(sig, "0xa30543a213557cc846070e8c9dd4de73dc9111ec2aa2626315894a392090e6080e9742480c4a96d11cb7a368de55ad58a33b5f67761cbca8d5ca6e00aacedb8b1b");
    }

    #[test]
    fn refuses_what_is_not_a_credit_pack() {
        let (acct, a) = fixture();
        let now = 1_800_000_000 - 300;
        let other = Authorization { to: [0x22; 20], ..a.clone() };
        assert!(acct.sign_usdg(&other, now).unwrap_err().contains("credits address"));
        let big = Authorization { value: MAX_UNITS + 1, ..a.clone() };
        assert!(acct.sign_usdg(&big, now).is_err());
        let long = Authorization { valid_before: now + MAX_WINDOW_S + 1, ..a.clone() };
        assert!(acct.sign_usdg(&long, now).is_err());
        let stale = Authorization { valid_before: now, ..a.clone() };
        assert!(acct.sign_usdg(&stale, now).is_err());
        let theirs = Authorization { from: [0x33; 20], ..a.clone() };
        assert!(acct.sign_usdg(&theirs, now).unwrap_err().contains("this account"));
    }
}
