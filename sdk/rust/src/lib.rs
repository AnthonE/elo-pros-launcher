//! # elo-sdk — the Elo Pros platform SDK for Rust
//!
//! Two halves, one crate:
//!
//! - **The overlay** ([`overlay`], re-exported at the top level): a player's
//!   identity and EIP-191 signatures from the Elo launcher running on the same
//!   machine, over its local socket. The game never holds a key, and there is
//!   no code path here that could. It is `sdk/rust/elo_overlay.rs`, the same
//!   bytes a game may vendor on its own (`sdk/SHA256SUMS`), and its wire is
//!   `sdk/PROTOCOL.md`.
//! - **The platform API** ([`Client`], feature `http`, on by default): the
//!   friends list with buddies, tickets, items, faces, profiles and
//!   leaderboards at `https://elopros.com/api/…`, plus [`payout`] for the
//!   per-wipe payout file a game server writes.
//!
//! ```no_run
//! # #[cfg(feature = "http")]
//! # fn main() {
//! use elo_sdk::{Client, Look};
//!
//! let elo = Client::new("https://elopros.com");
//! match elo.owns("gates", "0x1111111111111111111111111111111111111111") {
//!     Look::Answer(true) => println!("owns a copy"),
//!     Look::Answer(false) => println!("no copy"),
//!     Look::Refused(r) => println!("refused: {}", r.error),
//!     // Not "no". Never kick a player on this.
//!     Look::CouldNotLook(why) => println!("could not look: {why}"),
//! }
//! # }
//! # #[cfg(not(feature = "http"))]
//! # fn main() {}
//! ```
//!
//! **"Could not look" is never "no".** Every call answers a [`Look`]: an
//! answer, a refusal in the server's words, or could-not-look (unreachable,
//! timed out, not JSON, or the server saying it could not read). Read the
//! third as "ask again later", never as "nothing".
//!
//! **What you sign is rebuilt here.** [`Client::friend`] and
//! [`Client::set_face`] ask the server to prepare a change, rebuild the text
//! from what you asked for ([`texts`]), and refuse a server that offers any
//! other text before the signer is asked.

#[path = "../elo_overlay.rs"]
pub mod overlay;

pub use overlay::{
    default_socket, play_message, utc_day, Overlay, Profile, Proof, SignError, Signature,
    DEFAULT_TIMEOUT, FAMILIES, PROTOCOL, SOCKET_ENV,
};

pub mod look;
pub mod models;
pub mod payout;
pub mod texts;

#[cfg(feature = "http")]
pub mod http;

pub use look::{classify, Look, Refusal};
pub use payout::{PayoutFile, PayoutRow};

#[cfg(feature = "http")]
pub use http::{ActError, Client, FaceChange, DEFAULT_ORIGIN};
