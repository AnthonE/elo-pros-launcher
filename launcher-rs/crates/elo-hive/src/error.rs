//! One error type for the hive, carrying a sentence a player can read.
//!
//! Same rule as `DepotError`: the message is shown verbatim, so it says what
//! is wrong and never "an error occurred".

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveError(pub String);

impl HiveError {
    pub fn new(msg: impl Into<String>) -> Self {
        HiveError(msg.into())
    }
}

impl fmt::Display for HiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HiveError {}

impl From<std::io::Error> for HiveError {
    fn from(e: std::io::Error) -> Self {
        HiveError(e.to_string())
    }
}

impl From<HiveError> for elo_depot::DepotError {
    fn from(e: HiveError) -> Self {
        elo_depot::DepotError::new(e.0)
    }
}

pub type Result<T> = std::result::Result<T, HiveError>;
