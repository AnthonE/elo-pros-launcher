use std::fmt;

/// A depot that must not be installed.
///
/// The message is shown to the player verbatim, so it says what is wrong and
/// never "an error occurred". Ported from `depot.py`'s `DepotError`, including
/// that rule — it is the reason this is one type with a string and not an
/// enum of codes nobody renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotError(pub String);

impl DepotError {
    pub fn new(msg: impl Into<String>) -> Self {
        DepotError(msg.into())
    }
}

impl fmt::Display for DepotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DepotError {}

impl From<std::io::Error> for DepotError {
    fn from(e: std::io::Error) -> Self {
        DepotError(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DepotError>;
