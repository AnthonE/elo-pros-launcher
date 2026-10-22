//! Transactions: EIP-1559 (type 2), on chain 4663 only, signed here.
//!
//! # The one place money moves
//!
//! Everything else in this crate signs EIP-191 messages, whose prefix keeps
//! them from ever being a transaction. This file is the other kind, and it is
//! reachable from exactly one door: [`crate::LocalSigner::sign_transaction`],
//! which takes the passphrase every time. The game door (`elo-broker`) has no
//! verb that reaches it, and an unlocked session does not either.
//!
//! # Only type 2, ever
//!
//! No EIP-7702 authorization (type 4) and no typed-data permit is built
//! anywhere in this client. Those are the two signatures that hand an
//! account, or a token allowance, to someone else's contract, and they are
//! how most wallets are drained; a wallet that cannot make them cannot be
//! talked into one. `elo_net::wallet::own_warning` names an account that
//! carries a delegation anyway.
//!
//! # The chain id is a constant
//!
//! An RPC endpoint answers `eth_chainId` and could answer wrong. The id signed
//! into the transaction is [`CHAIN_ID`] and nothing a server says can change
//! it, so a signature made here cannot be replayed on another chain.
//!
//! # What is assembled here
//!
//! RLP and the EIP-2718 envelope: which bytes go where. The curve, the hash
//! and the nonce rule are `k256` and `sha3`. `tests/vectors.rs` checks the
//! output byte for byte against `eth_account`.

use crate::Account;
use k256::ecdsa::{RecoveryId, Signature};
use sha3::{Digest, Keccak256};

/// Robinhood Chain.
pub const CHAIN_ID: u64 = 4663;

/// `transfer(address,uint256)`.
pub const ERC20_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

/// An unsigned type-2 transaction. Amounts are wei (or a token's smallest
/// unit) as `u128`, which holds every real balance with room to spare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tx {
    pub nonce: u64,
    pub max_priority_fee: u128,
    pub max_fee: u128,
    pub gas: u64,
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
}

/// A signed transaction, ready for `eth_sendRawTransaction`.
#[derive(Debug, Clone)]
pub struct Signed {
    pub raw: Vec<u8>,
    pub hash: [u8; 32],
}

impl Signed {
    pub fn raw_hex(&self) -> String {
        format!("0x{}", crate::hex(&self.raw))
    }
    pub fn hash_hex(&self) -> String {
        format!("0x{}", crate::hex(&self.hash))
    }
}

impl Tx {
    /// The most this can cost in fees, in wei.
    pub fn max_fee_total(&self) -> u128 {
        self.max_fee.saturating_mul(self.gas as u128)
    }

    fn payload(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(64 + self.data.len());
        uint(&mut p, CHAIN_ID as u128);
        uint(&mut p, self.nonce as u128);
        uint(&mut p, self.max_priority_fee);
        uint(&mut p, self.max_fee);
        uint(&mut p, self.gas as u128);
        bytes(&mut p, &self.to);
        uint(&mut p, self.value);
        bytes(&mut p, &self.data);
        list(&mut p, &[]); // the access list, always empty
        p
    }

    /// `keccak256(0x02 || rlp([chainId, nonce, …, accessList]))`.
    pub fn signing_hash(&self) -> [u8; 32] {
        let mut h = Keccak256::new();
        h.update([0x02]);
        let mut body = Vec::new();
        list(&mut body, &self.payload());
        h.update(&body);
        h.finalize().into()
    }
}

impl Account {
    /// Sign a transaction. Crate-private: the only caller is
    /// `LocalSigner::sign_transaction`, which asked for the passphrase.
    pub(crate) fn sign_tx(&self, tx: &Tx) -> Result<Signed, String> {
        if tx.max_priority_fee > tx.max_fee {
            return Err("the priority fee is above the max fee; no node accepts that".into());
        }
        let digest = tx.signing_hash();
        let (sig, recid): (Signature, RecoveryId) = self
            .key
            .sign_prehash_recoverable(&digest)
            .map_err(|e| format!("signing failed: {e}"))?;
        let mut p = tx.payload();
        uint(&mut p, recid.is_y_odd() as u128);
        bytes(&mut p, strip(&sig.r().to_bytes()));
        bytes(&mut p, strip(&sig.s().to_bytes()));
        let mut raw = vec![0x02];
        list(&mut raw, &p);
        let hash = Keccak256::digest(&raw).into();
        Ok(Signed { raw, hash })
    }
}

/// The calldata of an ERC-20 `transfer(to, amount)`.
pub fn erc20_transfer(to: &[u8; 20], amount: u128) -> Vec<u8> {
    let mut d = Vec::with_capacity(68);
    d.extend_from_slice(&ERC20_TRANSFER);
    d.extend_from_slice(&[0u8; 12]);
    d.extend_from_slice(to);
    d.extend_from_slice(&[0u8; 16]);
    d.extend_from_slice(&amount.to_be_bytes());
    d
}

/// The recipient and amount back out of `transfer` calldata, or `None` if
/// these bytes are anything else. The review screen reads what will be signed
/// through this rather than trusting the form it was typed into.
pub fn read_erc20_transfer(data: &[u8]) -> Option<([u8; 20], u128)> {
    if data.len() != 68 || data[..4] != ERC20_TRANSFER {
        return None;
    }
    if data[4..16].iter().any(|&b| b != 0) || data[36..52].iter().any(|&b| b != 0) {
        return None;
    }
    let mut to = [0u8; 20];
    to.copy_from_slice(&data[16..36]);
    let mut amt = [0u8; 16];
    amt.copy_from_slice(&data[52..68]);
    Some((to, u128::from_be_bytes(amt)))
}

fn strip(b: &[u8]) -> &[u8] {
    let first = b.iter().position(|&x| x != 0).unwrap_or(b.len());
    &b[first..]
}

fn uint(out: &mut Vec<u8>, v: u128) {
    bytes(out, strip(&v.to_be_bytes()));
}

fn bytes(out: &mut Vec<u8>, b: &[u8]) {
    if b.len() == 1 && b[0] < 0x80 {
        out.push(b[0]);
    } else {
        head(out, 0x80, b.len());
        out.extend_from_slice(b);
    }
}

fn list(out: &mut Vec<u8>, payload: &[u8]) {
    head(out, 0xc0, payload.len());
    out.extend_from_slice(payload);
}

fn head(out: &mut Vec<u8>, base: u8, len: usize) {
    if len <= 55 {
        out.push(base + len as u8);
    } else {
        let l = strip(&(len as u64).to_be_bytes()).to_vec();
        out.push(base + 55 + l.len() as u8);
        out.extend_from_slice(&l);
    }
}
