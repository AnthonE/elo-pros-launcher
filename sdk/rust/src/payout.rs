//! A wipe's payout file, as a game server writes it.
//!
//! When a wipe closes, the game writes `<world>.payout-<wipe>.json` and its
//! status responder serves it at `/payout/<wipe>.json` beside `status.json`.
//! The meter reads it off the shards the operator names and pays each row
//! once through the cashier (`meter/gamepay.py`):
//!
//! ```json
//! {"wipe": 3, "payouts": [{"address": "0x…", "amount": 120, "ticker": "JUNK",
//!                          "for": ["EXTRACTED 120"]}]}
//! ```
//!
//! Amounts are whole coins in game units; the meter applies the posted
//! ratio. [`check_row`] and [`check_doc`] are `gamepay`'s own format checks,
//! pinned by `sdk/testdata/payout.json` and `payout_bad.json`. Which coins a
//! cashier pays at all is its policy, not the format, and is not checked here.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One wallet's pay for the wipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutRow {
    /// `0x` and 40 hex characters.
    pub address: String,
    /// Whole coins, at least 1, in game units.
    pub amount: u64,
    /// 1 to 12 letters and digits; the meter reads it upper case.
    pub ticker: String,
    /// The lines a player reads: what the amount was for.
    #[serde(rename = "for", default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

impl PayoutRow {
    pub fn new(address: &str, amount: u64, ticker: &str) -> PayoutRow {
        PayoutRow {
            address: address.to_string(),
            amount,
            ticker: ticker.to_string(),
            reasons: Vec::new(),
        }
    }

    /// Add a line saying what this was for (`EXTRACTED 120`).
    pub fn because(mut self, line: &str) -> PayoutRow {
        self.reasons.push(line.to_string());
        self
    }
}

/// A closed wipe's payout file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutFile {
    /// The wipe that closed, from 1.
    pub wipe: u64,
    pub payouts: Vec<PayoutRow>,
}

fn is_ticker(t: &str) -> bool {
    (1..=12).contains(&t.len())
        && t.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

fn whole(v: &Value) -> Option<u64> {
    // A JSON bool or a float is not a whole number, whatever it is worth.
    v.as_u64()
}

/// Why `gamepay` would not owe this row, or Ok. The words are `gamepay._check`'s.
pub fn check_row(row: &Value) -> Result<(), String> {
    let addr = row.get("address").and_then(Value::as_str).unwrap_or("");
    if !crate::texts::is_wallet(addr) {
        return Err("not a wallet".into());
    }
    match row.get("amount") {
        Some(v) if whole(v).is_some_and(|n| n >= 1) => {}
        other => {
            let shown = other.map_or("None".to_string(), |v| v.to_string());
            return Err(format!("amount {shown}"));
        }
    }
    let ticker = row
        .get("ticker")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_uppercase();
    if !is_ticker(&ticker) {
        return Err(format!("ticker {ticker:?}"));
    }
    Ok(())
}

/// Everything `gamepay` would refuse in a whole file: the file's own shape
/// first (`wipe` a whole number from 1, `payouts` an array), then each row as
/// `row N: why`, counted from 1. Empty when every row would be owed.
pub fn check_doc(doc: &Value) -> Vec<String> {
    let wipe = doc.get("wipe");
    if !wipe.and_then(whole).is_some_and(|w| w >= 1) {
        let shown = wipe.map_or("None".to_string(), |v| v.to_string());
        return vec![format!("the file's wipe is {shown}, a whole number from 1")];
    }
    let Some(rows) = doc.get("payouts").and_then(Value::as_array) else {
        return vec!["no payouts array".into()];
    };
    rows.iter()
        .enumerate()
        .filter_map(|(i, r)| {
            check_row(r)
                .err()
                .map(|why| format!("row {}: {why}", i + 1))
        })
        .collect()
}

impl PayoutFile {
    pub fn new(wipe: u64) -> PayoutFile {
        PayoutFile {
            wipe,
            payouts: Vec::new(),
        }
    }

    /// Add a row.
    pub fn pay(mut self, row: PayoutRow) -> PayoutFile {
        self.payouts.push(row);
        self
    }

    /// What `gamepay` would refuse; empty when the whole file would be owed.
    pub fn validate(&self) -> Vec<String> {
        match serde_json::to_value(self) {
            Ok(v) => check_doc(&v),
            Err(e) => vec![format!("the file does not serialise: {e}")],
        }
    }

    /// The file as the game writes it.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// `<world>.payout-<wipe>.json`, the name the game writes it under.
    pub fn file_name(&self, world: &str) -> String {
        format!("{world}.payout-{}.json", self.wipe)
    }

    /// Validate, then write to `path` atomically: a temp file beside it,
    /// flushed, then renamed over it, so the status responder never serves
    /// half a file. A file `gamepay` would refuse is not written.
    pub fn write_atomic(&self, path: &Path) -> io::Result<()> {
        let bad = self.validate();
        if !bad.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, bad.join("; ")));
        }
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path names no file"))?;
        let tmp = path.with_file_name(format!(
            ".{}.tmp-{}",
            name.to_string_lossy(),
            std::process::id()
        ));
        let result = (|| {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(self.to_json().as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_file_round_trips_and_validates() {
        let f = PayoutFile::new(3)
            .pay(
                PayoutRow::new("0x2222222222222222222222222222222222222222", 120, "JUNK")
                    .because("EXTRACTED 120"),
            )
            .pay(PayoutRow::new(
                "0x3333333333333333333333333333333333333333",
                1,
                "JUNK",
            ));
        assert!(f.validate().is_empty());
        let back: PayoutFile = serde_json::from_str(&f.to_json()).unwrap();
        assert_eq!(back, f);
        assert!(
            !f.to_json().contains(r#""for": []"#),
            "an empty `for` is left out"
        );
        assert_eq!(f.file_name("island"), "island.payout-3.json");
    }

    #[test]
    fn a_bad_row_is_named_by_its_place() {
        let f = PayoutFile::new(1).pay(PayoutRow::new("0x123", 5, "JUNK"));
        assert_eq!(f.validate(), vec!["row 1: not a wallet".to_string()]);
        assert_eq!(PayoutFile::new(0).validate().len(), 1);
    }
}
