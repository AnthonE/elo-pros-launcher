//! The depot seam — how a game's desktop build arrives, and why you can check
//! it.
//!
//! A **manifest** says what a title is and which builds exist; a **depot** is
//! one build: a flat list of files, each with a sha256, plus the one command
//! that starts it. Installing is fetch, hash, move. Verifying is re-hash.
//! Both are Steam's, and neither is interesting.
//!
//! What is interesting is the third thing, and it is the row Steam cannot copy
//! (`STEAM.md` §1 rows 3–4): **the depot's own digest is a `bytes32` that can
//! be notarized on chain.** So a player can ask a question no other storefront
//! can answer — *is the build I just installed the build the house listed?* —
//! and answer it from a block explorer, without trusting the launcher, the
//! CDN, or us.
//!
//! ## Why this crate is Rust and the rest of the client still is not
//!
//! This is the port's beachhead: 500 lines of pure logic with no UI, the
//! highest-risk code in the client (it writes files a stranger named), and the
//! piece a game's own updater may want to link. Porting it first proves the
//! rules survive the move before any pixels do — and the rules, not the code,
//! are the asset.
//!
//! ## The trust boundary, stated plainly
//!
//! Installing a native build means running someone else's binary. No hash
//! check changes that — a correctly-hashed backdoor installs perfectly. What
//! the hash buys is that **what you ran is what was notarized**, so a bad
//! build is attributable and cannot be swapped silently for one player. The
//! listing rule is what makes it safe to run at all: a listed game is open
//! source (`GATES.md` §5), so the binary has a source tree anyone can diff.

/// Join links — `scry://join/<slug>/<host:port>`. The scheme the platform
/// owns, so one link works for every title and a link for an uninstalled one
/// is still actionable.
pub mod deeplink;
pub mod depot;
pub mod digest;
pub mod error;
pub mod install;
pub mod launch;
pub mod manifest;
pub mod path;
/// `scry-shardlist-v1` — the list a title serves so its shards can be found.
pub mod shardlist;
/// Who published a title — a signature, where `publisher` was a typed string.
pub mod publisher;
pub mod session;
pub mod update;

pub const CHUNK: usize = 262_144;

pub use deeplink::{Join, SCHEME as LINK_SCHEME};
pub use depot::{parse_depot, Depot, DepotFile, DEPOT_VERSION};
pub use digest::{canonical, digest, sha256_file};
pub use error::{DepotError, Result};
pub use install::{
    build_dir, install, installed, plan, uninstall, verify, Fetcher, Install, Plan, Verdict,
    RECEIPT_NAME,
};
pub use launch::{platform_tag, browser_argv};
pub use manifest::{parse_manifest, Build, Manifest, BUILD_KINDS, MANIFEST_VERSION, PLATFORMS};
pub use session::{Ending, Session};
pub use update::{check, superseded, Checked, JsonSource, UpdateState};
