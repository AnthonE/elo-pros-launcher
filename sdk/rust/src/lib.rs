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
//!   friends list with buddies, tickets (buying a copy, the download grant),
//!   items, faces, profiles, leaderboards, an agent's check-in, the agent
//!   floor, your agents and the board at `https://elopros.com/api/…`, plus
//!   [`payout`] for the per-wipe payout file a game server writes.
//! - **A game server's half** (feature `http`): [`Client::check_sign_in`]
//!   checks a player's sign-in — the web SDK's or the launcher's `prove` —
//!   for your domain and nonce, smart wallets and Solana included, with no
//!   crypto stack of your own; [`Client::check_roster`] sweeps who still
//!   holds a copy; [`new_nonce`] issues the challenge. With a [`ServerKey`]
//!   (your server's Ed25519 key, named with us): cloud saves, stats and
//!   scores, and the reads a server makes at join (playtest access, a
//!   player's Workshop mods, a lobby, quick join).
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
//! **What you sign is rebuilt here.** [`Client::friend`],
//! [`Client::set_face`], the board and [`Client::entitle`] ask the server to
//! prepare a change, rebuild the text from what you asked for ([`texts`]),
//! and refuse a server that offers any other text before the signer is
//! asked. [`Client::buddy`] reads the change back out of the text instead
//! ([`texts::buddy_change`]) and refuses any change but the one asked for.

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
#[cfg(feature = "http")]
pub mod server;

pub use look::{classify, Look, Refusal};
pub use payout::{PayoutFile, PayoutRow};

#[cfg(feature = "http")]
pub use http::{new_nonce, ActError, Client, CloudSave, FaceChange, SignInCheck, DEFAULT_ORIGIN, ROSTER_PAGE};
#[cfg(feature = "http")]
pub use server::{new_server_nonce, server_text, ServerKey};
