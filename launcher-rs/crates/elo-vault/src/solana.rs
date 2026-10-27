//! The same account on Solana — the recovery words at Solana's path.
//!
//! # One phrase, two chains
//!
//! Phantom, Solflare, Backpack and MetaMask all do the same thing with twelve
//! words: the Ethereum key at `m/44'/60'/0'/0/0` and the Solana key at
//! `m/44'/501'/0'/0'`. This file is the second half, so the words a holder
//! wrote down bring back both addresses in any of those wallets, and both
//! addresses show here from the moment an account is made. `tests/solana.rs`
//! checks the derivation against `bip_utils` + `solders` and against the
//! address Phantom shows for the BIP-39 test phrase.
//!
//! # Nothing new is hand-rolled, and nothing new is downloaded
//!
//! | | |
//! |---|---|
//! | Ed25519 signing | `ring` — already linked for TLS, so zero new packages |
//! | SLIP-0010 derivation | HMAC-SHA512 from `hmac` + `sha2` |
//! | the on-curve test for a program address | field arithmetic from `crypto-bigint`, already here under `k256` |
//! | SHA-256 for a program address | `sha2` |
//!
//! What this file does is the assembly: base58, which bytes a transaction
//! message holds in which order, and where a signature goes. Each of those has
//! a reference answer and is tested against it byte for byte.
//!
//! # What a key here can sign
//!
//! A Solana transaction, and only through [`crate::LocalSigner`], which asks
//! for the passphrase every time — the same door an Ethereum send uses. There
//! is no "sign this message" for Solana: an ed25519 signature over bytes a page
//! chose is a signature over whatever transaction those bytes happen to be.
//!
//! ⚠ `ring` does not wipe its own copy of a key when it drops one. So a ring
//! key pair is made for one signature and dropped at once, and the 32-byte
//! seed that outlives it is [`Zeroizing`].

use crate::Zeroizing;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};

const HARD: u32 = 0x8000_0000;

/// `m/44'/501'/0'/0'` — Phantom's first account, and Solflare's and
/// Backpack's. Every step is hardened: SLIP-0010 for ed25519 has no other kind.
pub const PATH: [u32; 4] = [44 | HARD, 501 | HARD, HARD, HARD];
/// How the path is written for a person.
pub const PATH_TEXT: &str = "m/44'/501'/0'/0'";

/// What a holder sees the chain called.
pub const NAME: &str = "Solana";

/// The System Program, which owns every plain wallet and moves SOL.
pub const SYSTEM_PROGRAM: [u8; 32] = [0; 32];
/// The SPL Token program.
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
/// Token-2022, which newer coins (pump.fun's among them) are minted under.
pub const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
/// The Associated Token Account program: one token account per wallet per coin.
pub const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
/// The Compute Budget program — the only instruction a fee-setting wallet adds.
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";

/// One SOL in lamports.
pub const LAMPORTS: u64 = 1_000_000_000;

type HmacSha512 = Hmac<Sha512>;

/// SLIP-0010 private derivation for ed25519 from a BIP-39 seed.
///
/// Only hardened steps exist on this curve; an unhardened index is refused
/// rather than silently hardened, because that would derive a key no other
/// wallet derives.
pub fn derive(seed: &[u8], path: &[u32]) -> Result<Zeroizing<[u8; 32]>, String> {
    let step = |key: &[u8], parts: &[&[u8]]| -> Zeroizing<[u8; 64]> {
        let mut mac = HmacSha512::new_from_slice(key).expect("HMAC takes a key of any length");
        for p in parts {
            mac.update(p);
        }
        let mut out = Zeroizing::new([0u8; 64]);
        out.copy_from_slice(&mac.finalize().into_bytes());
        out
    };
    let mut i = step(b"ed25519 seed", &[seed]);
    for &index in path {
        if index & HARD == 0 {
            return Err("an ed25519 path is hardened at every step".into());
        }
        let (k, c) = (Zeroizing::new(<[u8; 32]>::try_from(&i[..32]).unwrap()),
                      Zeroizing::new(<[u8; 32]>::try_from(&i[32..]).unwrap()));
        i = step(c.as_ref(), &[&[0u8], k.as_ref(), &index.to_be_bytes()]);
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&i[..32]);
    Ok(out)
}

/// A Solana account this machine holds. **Never serialise this**; the
/// encrypted keystore beside the Ethereum one is the on-disk form.
pub struct SolAccount {
    seed: Zeroizing<[u8; 32]>,
    public: [u8; 32],
}

impl SolAccount {
    /// From the 32-byte seed (what SLIP-0010 yields, and the first half of the
    /// 64-byte secret key Phantom exports).
    pub fn from_secret(seed: &[u8]) -> Result<SolAccount, String> {
        let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed)
            .map_err(|_| "not an ed25519 secret: it must be 32 bytes".to_string())?;
        let mut public = [0u8; 32];
        public.copy_from_slice(ring::signature::KeyPair::public_key(&pair).as_ref());
        let mut s = Zeroizing::new([0u8; 32]);
        s.copy_from_slice(seed);
        Ok(SolAccount { seed: s, public })
    }

    /// A Solana secret key as a holder would paste it: base58 of the 64-byte
    /// key pair (Phantom's "export private key"), or of the 32-byte seed. A
    /// 64-byte pair whose second half is not the first half's public key is
    /// refused — it is damaged, or two keys glued together.
    pub fn from_typed(typed: &str) -> Result<SolAccount, String> {
        let raw = Zeroizing::new(base58_decode(typed.trim())
            .map_err(|_| "A Solana private key is base58 text, the way Phantom exports it.".to_string())?);
        match raw.len() {
            32 => SolAccount::from_secret(&raw),
            64 => {
                let acct = SolAccount::from_secret(&raw[..32])?;
                if raw[32..] != acct.public {
                    return Err("That key's two halves do not belong together. Copy it again.".into());
                }
                Ok(acct)
            }
            n => Err(format!("A Solana private key is 64 bytes; that was {n}.")),
        }
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    /// The address: base58 of the public key.
    pub fn address(&self) -> String {
        base58_encode(&self.public)
    }

    /// The 32 secret bytes, for the keystore and nothing else.
    pub fn secret(&self) -> Zeroizing<[u8; 32]> {
        self.seed.clone()
    }

    /// Ed25519 over `msg`. Crate-only: the public door is
    /// [`crate::LocalSigner::sign_solana_transaction`], which takes the
    /// passphrase and checks what it is signing.
    pub(crate) fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(self.seed.as_ref())
            .expect("the seed was checked when this account was made");
        let mut out = [0u8; 64];
        out.copy_from_slice(pair.sign(msg).as_ref());
        out
    }
}

// ── base58 ───────────────────────────────────────────────────────────────────

const B58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Bitcoin's alphabet, which is the one Solana uses. Leading zero bytes are
/// leading `1`s.
pub fn base58_encode(bytes: &[u8]) -> String {
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    let mut digits: Vec<u8> = Vec::with_capacity(bytes.len() * 138 / 100 + 1);
    for &b in &bytes[zeros..] {
        let mut carry = b as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat_n('1', zeros));
    out.extend(digits.iter().rev().map(|d| B58[*d as usize] as char));
    out
}

pub fn base58_decode(s: &str) -> Result<Vec<u8>, String> {
    let ones = s.bytes().take_while(|b| *b == b'1').count();
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    for c in s.bytes().skip(ones) {
        let v = B58.iter().position(|x| *x == c).ok_or("not base58")? as u32;
        let mut carry = v;
        for b in bytes.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let mut out = vec![0u8; ones];
    out.extend(bytes.iter().rev());
    Ok(out)
}

/// An address someone typed or pasted, as 32 bytes. Base58 carries no
/// checksum, so a mistyped character is usually still an address — which is
/// why a send also says whether you have sent there before.
pub fn parse_address(typed: &str) -> Result<[u8; 32], String> {
    let t = typed.trim();
    if t.starts_with("0x") || t.starts_with("0X") {
        return Err("That is a Robinhood Chain (0x…) address, not a Solana one.".into());
    }
    let raw = base58_decode(t).map_err(|_| "A Solana address is base58: letters and digits, \
                                             without 0, O, I or l.".to_string())?;
    <[u8; 32]>::try_from(raw.as_slice()).map_err(|_| "That is not a Solana address: it decodes \
                                                       to the wrong length.".to_string())
}

/// A program id from its well-known text. Panics only on a typo in this file,
/// which the tests would catch first.
pub fn program(text: &str) -> [u8; 32] {
    parse_address(text).expect("a program id in this file is valid base58")
}

// ── program addresses ────────────────────────────────────────────────────────

crypto_bigint::impl_modulus!(P25519, crypto_bigint::U256,
    "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffed");
type Fe = crypto_bigint::modular::constant_mod::Residue<P25519, { crypto_bigint::U256::LIMBS }>;

/// Whether 32 bytes decompress to a point on ed25519 — what Solana's
/// `is_on_curve` asks. A wallet's address is on the curve; a program-derived
/// address must not be, which is how nobody can hold its key.
///
/// The test is the decompression's own: with `y` from the bytes (sign bit
/// dropped, reduced mod p), `x² = (y² − 1) / (d·y² + 1)` has a root exactly
/// when that ratio is zero or a square — Euler's criterion. Nothing here is
/// secret, so nothing here needs to be constant-time.
pub fn is_on_curve(bytes: &[u8; 32]) -> bool {
    use crypto_bigint::U256;
    let mut le = *bytes;
    le[31] &= 0x7f;
    let mut y = U256::from_le_slice(&le);
    let p = U256::from_be_hex("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffed");
    if y >= p {
        y = y.wrapping_sub(&p);
    }
    // d = −121665/121666 mod p
    let d = Fe::new(&U256::from_be_hex(
        "52036cee2b6ffe738cc740797779e89800700a4d4141d8ab75eb4dca135978a3"));
    let y = Fe::new(&y);
    let yy = y.square();
    let u = yy.sub(&Fe::ONE);
    let v = d.mul(&yy).add(&Fe::ONE);
    if u.retrieve() == U256::ZERO {
        return true;
    }
    let (v_inv, ok) = v.invert();
    if !bool::from(ok) {
        return false;
    }
    let ratio = u.mul(&v_inv);
    // (p − 1) / 2
    let half = U256::from_be_hex("3ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff6");
    ratio.pow(&half).retrieve() == U256::ONE
}

/// `find_program_address`: the first bump from 255 down whose hash is off the
/// curve.
pub fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> Option<([u8; 32], u8)> {
    for bump in (0..=255u8).rev() {
        let mut h = Sha256::new();
        for s in seeds {
            h.update(s);
        }
        h.update([bump]);
        h.update(program_id);
        h.update(b"ProgramDerivedAddress");
        let out: [u8; 32] = h.finalize().into();
        if !is_on_curve(&out) {
            return Some((out, bump));
        }
    }
    None
}

/// The token account a wallet holds a coin in, under the coin's own token
/// program (SPL Token or Token-2022 — they derive different accounts).
pub fn associated_token_address(owner: &[u8; 32], mint: &[u8; 32], token_program: &[u8; 32]) -> [u8; 32] {
    find_program_address(&[owner, token_program, mint], &program(ATA_PROGRAM))
        .expect("an associated token address exists for every owner and mint")
        .0
}

// ── transactions ─────────────────────────────────────────────────────────────

/// One account an instruction touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub key: [u8; 32],
    pub signer: bool,
    pub writable: bool,
}

/// One instruction: a program, the accounts it reads or writes, its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ix {
    pub program: [u8; 32],
    pub accounts: Vec<Meta>,
    pub data: Vec<u8>,
}

/// Move SOL: System Program `Transfer`.
pub fn system_transfer(from: &[u8; 32], to: &[u8; 32], lamports: u64) -> Ix {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    Ix {
        program: SYSTEM_PROGRAM,
        accounts: vec![Meta { key: *from, signer: true, writable: true },
                       Meta { key: *to, signer: false, writable: true }],
        data,
    }
}

/// Make the recipient's token account if it is not there yet, and do nothing
/// if it is: `CreateIdempotent`. The payer pays its rent (about 0.002 SOL),
/// which is what every Solana wallet does on a first send of a coin.
pub fn create_ata_idempotent(payer: &[u8; 32], owner: &[u8; 32], mint: &[u8; 32],
                             token_program: &[u8; 32]) -> Ix {
    let ata = associated_token_address(owner, mint, token_program);
    Ix {
        program: program(ATA_PROGRAM),
        accounts: vec![
            Meta { key: *payer, signer: true, writable: true },
            Meta { key: ata, signer: false, writable: true },
            Meta { key: *owner, signer: false, writable: false },
            Meta { key: *mint, signer: false, writable: false },
            Meta { key: SYSTEM_PROGRAM, signer: false, writable: false },
            Meta { key: *token_program, signer: false, writable: false },
        ],
        data: vec![1],
    }
}

/// `TransferChecked`: the amount AND the decimals, so a coin whose decimals
/// were misread fails on chain instead of sending a thousand times too much.
pub fn transfer_checked(source: &[u8; 32], mint: &[u8; 32], dest: &[u8; 32], owner: &[u8; 32],
                        amount: u64, decimals: u8, token_program: &[u8; 32]) -> Ix {
    let mut data = vec![12u8];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    Ix {
        program: *token_program,
        accounts: vec![
            Meta { key: *source, signer: false, writable: true },
            Meta { key: *mint, signer: false, writable: false },
            Meta { key: *dest, signer: false, writable: true },
            Meta { key: *owner, signer: true, writable: false },
        ],
        data,
    }
}

fn compact_u16(n: usize, out: &mut Vec<u8>) {
    let mut n = n as u16;
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        b |= 0x80;
        out.push(b);
    }
}

fn read_compact_u16(b: &[u8], at: &mut usize) -> Result<usize, String> {
    let mut n = 0usize;
    for shift in [0, 7, 14] {
        let byte = *b.get(*at).ok_or("the transaction ends early")?;
        *at += 1;
        n |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Ok(n);
        }
    }
    Err("a length in the transaction is malformed".into())
}

/// A legacy message: header, account keys, blockhash, instructions — the
/// bytes a signature covers. Keys are ordered the way Solana's own compiler
/// orders them (payer first; then writable signers, read-only signers,
/// writable others, read-only others, each group sorted by key), so the bytes
/// match what any Solana library makes for the same instructions.
pub fn compile(payer: &[u8; 32], ixs: &[Ix], blockhash: &[u8; 32]) -> Vec<u8> {
    let mut metas: Vec<Meta> = vec![Meta { key: *payer, signer: true, writable: true }];
    let mut add = |m: &Meta| {
        if let Some(e) = metas.iter_mut().find(|e| e.key == m.key) {
            e.signer |= m.signer;
            e.writable |= m.writable;
        } else {
            metas.push(m.clone());
        }
    };
    for ix in ixs {
        for a in &ix.accounts {
            add(a);
        }
        add(&Meta { key: ix.program, signer: false, writable: false });
    }
    let payer_meta = metas.remove(0);
    let group = |s: bool, w: bool| {
        let mut g: Vec<[u8; 32]> = metas.iter().filter(|m| m.signer == s && m.writable == w)
            .map(|m| m.key).collect();
        g.sort();
        g
    };
    let (sw, sr, uw, ur) = (group(true, true), group(true, false), group(false, true), group(false, false));
    let mut keys = vec![payer_meta.key];
    keys.extend(sw);
    let ro_signed = sr.len();
    keys.extend(sr);
    keys.extend(uw);
    let ro_unsigned = ur.len();
    keys.extend(ur);
    let signers = 1 + metas.iter().filter(|m| m.signer).count();

    let mut out = vec![signers as u8, ro_signed as u8, ro_unsigned as u8];
    compact_u16(keys.len(), &mut out);
    for k in &keys {
        out.extend_from_slice(k);
    }
    out.extend_from_slice(blockhash);
    compact_u16(ixs.len(), &mut out);
    let idx = |k: &[u8; 32]| keys.iter().position(|x| x == k).expect("every key was added") as u8;
    for ix in ixs {
        out.push(idx(&ix.program));
        compact_u16(ix.accounts.len(), &mut out);
        for a in &ix.accounts {
            out.push(idx(&a.key));
        }
        compact_u16(ix.data.len(), &mut out);
        out.extend_from_slice(&ix.data);
    }
    out
}

/// A message read back: enough to say who must sign, what it calls, and to
/// check that a transaction someone else built is the one a review showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// `None` for a legacy message, `Some(0)` for v0.
    pub version: Option<u8>,
    pub signers: usize,
    pub readonly_signed: usize,
    pub readonly_unsigned: usize,
    pub keys: Vec<[u8; 32]>,
    pub blockhash: [u8; 32],
    /// (program index, account indexes, data). An index past `keys` points
    /// into an address lookup table.
    pub instructions: Vec<(u8, Vec<u8>, Vec<u8>)>,
    /// v0 only: (table address, writable indexes, read-only indexes). The
    /// table's own account on chain says which address each index is.
    pub lookups: Vec<([u8; 32], Vec<u8>, Vec<u8>)>,
}

impl Message {
    /// The program an instruction calls, if it is a static key (a program id
    /// is never loaded from a lookup table).
    pub fn program_of(&self, i: usize) -> Option<[u8; 32]> {
        self.instructions.get(i).and_then(|(p, _, _)| self.keys.get(*p as usize).copied())
    }

    pub fn is_writable(&self, i: usize) -> bool {
        if i < self.signers {
            i < self.signers - self.readonly_signed
        } else {
            i < self.keys.len() - self.readonly_unsigned
        }
    }
}

pub fn parse_message(b: &[u8]) -> Result<Message, String> {
    let mut at = 0usize;
    let first = *b.first().ok_or("an empty message")?;
    let version = if first & 0x80 != 0 {
        at += 1;
        let v = first & 0x7f;
        if v != 0 {
            return Err(format!("a version {v} transaction — this app reads legacy and v0"));
        }
        Some(v)
    } else {
        None
    };
    let hdr = b.get(at..at + 3).ok_or("the transaction ends early")?;
    let (signers, readonly_signed, readonly_unsigned) = (hdr[0] as usize, hdr[1] as usize, hdr[2] as usize);
    at += 3;
    let n = read_compact_u16(b, &mut at)?;
    let mut keys = Vec::with_capacity(n);
    for _ in 0..n {
        let k = b.get(at..at + 32).ok_or("the transaction ends early")?;
        keys.push(<[u8; 32]>::try_from(k).unwrap());
        at += 32;
    }
    if signers == 0 || signers > keys.len() || readonly_signed >= signers
        || readonly_unsigned > keys.len() - signers {
        return Err("the transaction's header does not fit its keys".into());
    }
    let blockhash = <[u8; 32]>::try_from(b.get(at..at + 32).ok_or("the transaction ends early")?).unwrap();
    at += 32;
    let ni = read_compact_u16(b, &mut at)?;
    let mut instructions = Vec::with_capacity(ni);
    for _ in 0..ni {
        let p = *b.get(at).ok_or("the transaction ends early")?;
        at += 1;
        let na = read_compact_u16(b, &mut at)?;
        let accts = b.get(at..at + na).ok_or("the transaction ends early")?.to_vec();
        at += na;
        let nd = read_compact_u16(b, &mut at)?;
        let data = b.get(at..at + nd).ok_or("the transaction ends early")?.to_vec();
        at += nd;
        instructions.push((p, accts, data));
    }
    let mut lookups = Vec::new();
    if version.is_some() {
        let n = read_compact_u16(b, &mut at)?;
        for _ in 0..n {
            let table = <[u8; 32]>::try_from(b.get(at..at + 32).ok_or("the transaction ends early")?).unwrap();
            at += 32;
            let w = read_compact_u16(b, &mut at)?;
            let writable = b.get(at..at + w).ok_or("the transaction ends early")?.to_vec();
            at += w;
            let r = read_compact_u16(b, &mut at)?;
            let readonly = b.get(at..at + r).ok_or("the transaction ends early")?.to_vec();
            at += r;
            lookups.push((table, writable, readonly));
        }
    }
    if at != b.len() {
        return Err("the transaction carries bytes after its message".into());
    }
    Ok(Message { version, signers, readonly_signed, readonly_unsigned, keys, blockhash,
                 instructions, lookups })
}

/// The addresses an address lookup table holds, from its account data: a
/// 56-byte header, then 32 bytes per address.
pub fn lookup_table_addresses(data: &[u8]) -> Vec<[u8; 32]> {
    data.get(56..).unwrap_or(&[]).chunks_exact(32).map(|c| <[u8; 32]>::try_from(c).unwrap()).collect()
}

/// A whole transaction: its signature slots and its message.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub signatures: Vec<[u8; 64]>,
    pub message_bytes: Vec<u8>,
    pub message: Message,
}

impl Transaction {
    /// An unsigned transaction around a message this app compiled.
    pub fn unsigned(message_bytes: Vec<u8>) -> Result<Transaction, String> {
        let message = parse_message(&message_bytes)?;
        Ok(Transaction { signatures: vec![[0u8; 64]; message.signers], message_bytes, message })
    }

    /// Read the wire form (what a quote's `serialized_b64` decodes to).
    pub fn parse(b: &[u8]) -> Result<Transaction, String> {
        let mut at = 0usize;
        let n = read_compact_u16(b, &mut at)?;
        let mut signatures = Vec::with_capacity(n);
        for _ in 0..n {
            let s = b.get(at..at + 64).ok_or("the transaction ends early")?;
            signatures.push(<[u8; 64]>::try_from(s).unwrap());
            at += 64;
        }
        let message_bytes = b[at..].to_vec();
        let message = parse_message(&message_bytes)?;
        if message.signers != n {
            return Err("the transaction's signature count does not match its message".into());
        }
        Ok(Transaction { signatures, message_bytes, message })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + 64 * self.signatures.len() + self.message_bytes.len());
        compact_u16(self.signatures.len(), &mut out);
        for s in &self.signatures {
            out.extend_from_slice(s);
        }
        out.extend_from_slice(&self.message_bytes);
        out
    }

    /// The transaction's id once it is signed: its first signature.
    pub fn id(&self) -> String {
        base58_encode(&self.signatures[0])
    }

    /// Put `account`'s signature in its slot. Refused when `account` is not one
    /// of the message's signers, and when, after signing, some other signer's
    /// slot is still empty — such a transaction could never land, and signing
    /// half of one for someone else to finish is not a thing a wallet does on
    /// a holder's behalf.
    pub(crate) fn sign_with(&mut self, account: &SolAccount) -> Result<(), String> {
        let me = account.public();
        let slot = self.message.keys[..self.message.signers].iter().position(|k| *k == me)
            .ok_or("this transaction is not asking your Solana account to sign it")?;
        self.signatures[slot] = account.sign(&self.message_bytes);
        if self.signatures.contains(&[0u8; 64]) {
            self.signatures[slot] = [0u8; 64];
            return Err("this transaction also needs another signer's signature, \
                        so it was not signed".into());
        }
        Ok(())
    }
}

/// Standard base64, for the RPC's `sendTransaction` and a quote's transaction.
pub fn b64_encode(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let val = |c: u8| -> Result<u32, String> {
        Ok(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err("not base64".into()),
        } as u32)
    };
    let s = s.trim().trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}
