//! The recovery phrase — twelve words that ARE the account.
//!
//! # Why words at all
//!
//! Until this existed the keystore file was the only copy of a key made here,
//! and "back it up" meant copying a JSON file somewhere. A file goes with a
//! disk and a passphrase goes with a memory; twelve words on paper survive
//! both. They are BIP-39 words at BIP-44's Ethereum path, `m/44'/60'/0'/0/0`,
//! so typing them into MetaMask, Rabby or a hardware wallet brings back the
//! same address — and Solana's path, `m/44'/501'/0'/0'`, brings back the
//! same Solana address in Phantom (`solana.rs`). That is the exit, and `tests/vectors.rs` checks it against
//! `eth_account`'s own derivation and two vectors every wallet already knows.
//!
//! # Nothing new is hand-rolled
//!
//! | | |
//! |---|---|
//! | PBKDF2-HMAC-SHA512, 2048 rounds | `pbkdf2` + `hmac` + `sha2`, already in the tree under `scrypt` |
//! | HMAC-SHA512 for BIP-32 | `hmac` + `sha2` |
//! | adding scalars mod n | `k256` |
//! | the wordlist | the official English list, pinned by its SHA-256 |
//!
//! What this file does is the assembly: entropy to words, words to seed, seed
//! down the path.
//!
//! # The words are never written by us
//!
//! A `Phrase` lives in memory long enough to be shown and checked, and is
//! zeroed when dropped. The keystore holds the derived key, encrypted, and
//! nothing on disk can give the words back.

use crate::Account;
use hmac::{Hmac, Mac};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::elliptic_curve::PrimeField;
use k256::{FieldBytes, ProjectivePoint, Scalar};
use sha2::{Digest, Sha256, Sha512};
use zeroize::Zeroizing;

/// The official BIP-39 English list. Its hash is pinned in
/// `tests/vectors.rs`; a list with one word changed derives other people's
/// accounts from your words.
const ENGLISH: &str = include_str!("english.txt");

/// `m/44'/60'/0'/0/0` — what MetaMask's first account is. Hardened steps carry
/// the top bit.
pub const PATH: [u32; 5] = [44 | HARD, 60 | HARD, HARD, 0, 0];
const HARD: u32 = 0x8000_0000;

/// The list as words, in order. `lines()` rather than splitting on `\n` so a
/// checkout that turned the file into CRLF still reads 2048 clean words.
pub fn wordlist() -> Vec<&'static str> {
    ENGLISH.lines().map(str::trim).filter(|w| !w.is_empty()).collect()
}

/// A recovery phrase, held only in memory and zeroed on drop.
///
/// No `Debug`, `Display` or `Clone` derive: each one is a way for the words to
/// land in a log or a second buffer nobody zeroes.
pub struct Phrase {
    /// The words, lowercase, joined by single spaces: the exact bytes BIP-39
    /// feeds to PBKDF2.
    text: Zeroizing<String>,
}

impl std::fmt::Debug for Phrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Phrase({} words)", self.words().len())
    }
}

impl Phrase {
    /// Twelve new words from the OS entropy pool.
    pub fn generate() -> Result<Phrase, String> {
        let mut entropy = Zeroizing::new([0u8; 16]);
        getrandom::fill(entropy.as_mut()).map_err(|e| format!("no OS entropy: {e}"))?;
        Phrase::from_entropy(entropy.as_ref())
    }

    /// Words for given entropy: 16 bytes make 12 words, 32 make 24.
    pub fn from_entropy(entropy: &[u8]) -> Result<Phrase, String> {
        if !(16..=32).contains(&entropy.len()) || entropy.len() % 4 != 0 {
            return Err(format!(
                "BIP-39 entropy is 16 to 32 bytes in steps of 4; that was {}",
                entropy.len()
            ));
        }
        let list = wordlist();
        let check_bits = entropy.len() / 4;
        let hash = Sha256::digest(entropy);
        // The entropy, then the first `check_bits` of its hash, read 11 bits
        // at a time.
        let bit = |i: usize| -> u32 {
            let byte = if i < entropy.len() * 8 { entropy[i / 8] } else { hash[(i / 8) - entropy.len()] };
            ((byte >> (7 - (i % 8))) & 1) as u32
        };
        let total = entropy.len() * 8 + check_bits;
        let mut text = Zeroizing::new(String::with_capacity(total / 11 * 9));
        for w in 0..total / 11 {
            let mut idx = 0u32;
            for b in 0..11 {
                idx = (idx << 1) | bit(w * 11 + b);
            }
            if w > 0 {
                text.push(' ');
            }
            text.push_str(list[idx as usize]);
        }
        Ok(Phrase { text })
    }

    /// Read words someone typed. Case and spacing are forgiven; a word not on
    /// the list, a wrong count or a failed checksum is refused.
    ///
    /// ⚠ No error repeats a word. The input is a key, and an error that echoed
    /// part of it would carry it into a screenshot or a bug report.
    pub fn parse(typed: &str) -> Result<Phrase, String> {
        let list = wordlist();
        let lower = Zeroizing::new(typed.to_lowercase());
        let words: Vec<&str> = lower.split_whitespace().collect();
        if ![12, 15, 18, 21, 24].contains(&words.len()) {
            return Err(format!(
                "A recovery phrase is 12 or 24 words; that was {}.",
                words.len()
            ));
        }
        let mut bits = Zeroizing::new(Vec::with_capacity(words.len() * 11));
        for (n, w) in words.iter().enumerate() {
            let idx = list
                .binary_search(w)
                .map_err(|_| format!("Word {} is not on the BIP-39 list. Check its spelling.", n + 1))?;
            for b in (0..11).rev() {
                bits.push(((idx >> b) & 1) as u8);
            }
        }
        let check_bits = bits.len() / 33;
        let ent_bits = bits.len() - check_bits;
        let mut entropy = Zeroizing::new(vec![0u8; ent_bits / 8]);
        for (i, b) in bits[..ent_bits].iter().enumerate() {
            entropy[i / 8] |= b << (7 - (i % 8));
        }
        let hash = Sha256::digest(entropy.as_slice());
        for i in 0..check_bits {
            if bits[ent_bits + i] != (hash[i / 8] >> (7 - (i % 8))) & 1 {
                return Err("Those words are all on the list but their checksum fails: \
                            one is wrong or two are swapped."
                    .into());
            }
        }
        Ok(Phrase { text: Zeroizing::new(words.join(" ")) })
    }

    pub fn words(&self) -> Vec<&str> {
        self.text.split(' ').collect()
    }

    /// The BIP-39 seed, with no extra passphrase. That is what MetaMask and
    /// every wallet's "import recovery phrase" assume.
    pub fn seed(&self) -> Zeroizing<[u8; 64]> {
        let mut seed = Zeroizing::new([0u8; 64]);
        pbkdf2::pbkdf2_hmac::<Sha512>(self.text.as_bytes(), b"mnemonic", 2048, seed.as_mut());
        seed
    }

    /// The account at [`PATH`].
    pub fn account(&self) -> Result<Account, String> {
        let key = derive(self.seed().as_ref(), &PATH)?;
        Account::from_secret(key.as_ref())
    }

    /// The Solana account the same words derive, at
    /// [`crate::solana::PATH`] — what Phantom shows for them.
    pub fn solana(&self) -> Result<crate::solana::SolAccount, String> {
        let key = crate::solana::derive(self.seed().as_ref(), &crate::solana::PATH)?;
        crate::solana::SolAccount::from_secret(key.as_ref())
    }

    /// Three different word positions (1-based, ascending) to ask back.
    pub fn quiz(&self) -> Result<[usize; 3], String> {
        let n = self.words().len();
        let mut picked: Vec<usize> = Vec::with_capacity(3);
        while picked.len() < 3 {
            let mut b = [0u8; 2];
            getrandom::fill(&mut b).map_err(|e| format!("no OS entropy: {e}"))?;
            let i = (u16::from_le_bytes(b) as usize % n) + 1;
            if !picked.contains(&i) {
                picked.push(i);
            }
        }
        picked.sort_unstable();
        Ok([picked[0], picked[1], picked[2]])
    }

    /// Whether `answer` is the words at `positions`, in order. Case and
    /// spacing are forgiven, as in [`Phrase::parse`].
    pub fn check(&self, positions: &[usize], answer: &str) -> bool {
        let words = self.words();
        let lower = Zeroizing::new(answer.to_lowercase());
        let given: Vec<&str> = lower.split_whitespace().collect();
        given.len() == positions.len()
            && positions.iter().zip(&given).all(|(&p, g)| p >= 1 && words.get(p - 1) == Some(g))
    }
}

type HmacSha512 = Hmac<Sha512>;

fn hmac512(key: &[u8], parts: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC takes a key of any length");
    for p in parts {
        mac.update(p);
    }
    let mut out = Zeroizing::new([0u8; 64]);
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

fn scalar(bytes: &[u8]) -> Option<Scalar> {
    Option::from(Scalar::from_repr(*FieldBytes::from_slice(bytes)))
}

/// BIP-32 private derivation from a seed down `path`.
///
/// A step whose intermediate is out of range is an error, not a skip to the
/// next index as BIP-32 allows: the odds are about 1 in 2^127, and a silent
/// skip would make an account other wallets do not derive.
pub fn derive(seed: &[u8], path: &[u32]) -> Result<Zeroizing<[u8; 32]>, String> {
    let i = hmac512(b"Bitcoin seed", &[seed]);
    let mut k = scalar(&i[..32]).filter(|s| !bool::from(s.is_zero())).ok_or("seed gives no valid key")?;
    let mut chain = Zeroizing::new([0u8; 32]);
    chain.copy_from_slice(&i[32..]);

    for &index in path {
        let key_bytes = Zeroizing::new(<[u8; 32]>::from(k.to_bytes()));
        let i = if index & HARD != 0 {
            hmac512(chain.as_ref(), &[&[0u8], key_bytes.as_ref(), &index.to_be_bytes()])
        } else {
            let point = (ProjectivePoint::GENERATOR * k).to_affine().to_encoded_point(true);
            hmac512(chain.as_ref(), &[point.as_bytes(), &index.to_be_bytes()])
        };
        let tweak = scalar(&i[..32]).ok_or("a derivation step fell outside the curve order")?;
        k += tweak;
        if bool::from(k.is_zero()) {
            return Err("a derivation step produced the zero key".into());
        }
        chain.copy_from_slice(&i[32..]);
    }
    Ok(Zeroizing::new(k.to_bytes().into()))
}
