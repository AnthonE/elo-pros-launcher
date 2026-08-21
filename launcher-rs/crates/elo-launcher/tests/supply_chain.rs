//! The dependency policy, enforced.
//!
//! Operator, 2026-08-05: *"if anything like veersion lock because lost of npm
//! type chain attacks recently."*
//!
//! The workspace manifest states the policy; this is what makes it a rule
//! rather than a paragraph. Four properties, each of which has to be *checked*
//! because each fails silently:
//!
//! 1. every third-party package in the lock carries a checksum,
//! 2. no crate appears at two versions (two chances to be compromised, and the
//!    reason a "harmless" duplicate is not),
//! 3. the tree stays under a budget — a crate whose compromise is our
//!    compromise is a cost, so the count is a budget and never a metric,
//! 4. no direct dependency is declared with a floating range.
//!
//! ⚠ Raising `MAX_PACKAGES` is a decision. Do it in the same commit as the
//! crate that needed it, so the diff shows what was bought.
//!
//! What none of this buys, said plainly: a pinned, checksummed dependency is
//! still someone else's code running as you. The lock stops the bytes changing
//! under you. It does not make the bytes good.

use std::collections::BTreeMap;

const LOCK: &str = include_str!("../../../Cargo.lock");
const MANIFEST: &str = include_str!("../../../Cargo.toml");

/// Raised 85 → 110 on 2026-08-05, in the commit that spent it, per the policy.
///
/// What it bought, and both are the kind of thing the budget exists to make
/// deliberate rather than to forbid:
///
/// * **`k256` + `bech32` (+13)** — BIP-340 schnorr and NIP-19, so the client
///   can speak in the hive. `SDK.md` §2a already made this call in Python:
///   *signing belongs in an audited library rather than in code written here*,
///   and a hand-rolled nonce leaks the key silently.
/// * **`tungstenite` + `rustls` + `webpki-roots` + `rustls-pemfile` (+13)** —
///   buzz binds an author to the authenticated connection, so posting needs
///   our own socket. Deliberately configured WITHOUT tungstenite's own TLS
///   feature so the workspace links one TLS stack and one root store; letting
///   it bring its own added a second copy of the CA bundle for nothing.
///
/// Declined in the same session: `jiff`, at 14 packages to render "3m ago".
///
/// **Unchanged on 2026-08-06 when the windows arrived, which is the whole
/// point of having had the number.** Operator: *"fltk it is"* — and the
/// toolkit was chosen by measuring against this cap rather than by taste:
///
/// | toolkit | packages it would add |
/// |---|---|
/// | **fltk** | **15 standalone, +9 here** (bitflags, once_cell and the
/// |          | crossbeam pair were already in the tree) |
/// | eframe/egui | 402 |
/// | iced | 414 |
/// | slint | 531 |
///
/// A whole GUI toolkit landed inside the existing budget with headroom to
/// spare, while every pure-Rust alternative is **four to five times the entire
/// tree** — that is not a cap to raise, it is a posture to abandon, and the
/// posture is an operator sentence about npm-style attacks. What fltk costs
/// instead is a C++ toolchain at build time (cmake, a C++ compiler, the
/// X11/Xft/pango/cairo dev headers), which falls on whoever cuts a release and
/// never on a player. `docs/client/LAUNCHER.md` §10b2 carries the trade in full.
///
/// ⚠ **Its licence is invisible to `cargo deny`.** `fltk` and `fltk-sys` both
/// declare MIT — the binding — while `fltk-sys` vendors FLTK itself under the
/// FLTK License (LGPL with exceptions). Static linking is explicitly permitted
/// and the licence text need not ship, but the program *must identify its use
/// of FLTK*. `elo-ui/tests/credit.rs` is what asserts that, because a licence
/// check reading crate metadata never will.
///
/// ---
///
/// **Raised 110 → 122 on 2026-08-06, for the one thing the budget was always
/// going to be spent on.** Operator: *"don't hand roll anything."* The local
/// account needs real secp256k1, real keccak, a real KDF and a real cipher,
/// and the alternative to those crates is writing them — which `vault.py` and
/// `SDK.md` §2a both already refuse, because a subtly wrong nonce or a weak
/// KDF leaks a key with **no symptom** until somebody else has the money.
///
/// Eleven third-party packages, and none of them is optional:
///
/// | | why it cannot be dropped |
/// |---|---|
/// | `sha3` + `keccak` | keccak-256 — the EIP-191 digest AND the V3 keystore MAC |
/// | `scrypt` + `salsa20` + `pbkdf2` | the V3 KDF at geth's standard work factor. scrypt IS pbkdf2 over a salsa20 core; they arrive together |
/// | `aes` + `ctr` + `cipher` + `inout` | AES-128-CTR, the V3 cipher, and RustCrypto's shared traits |
/// | `hmac` + `rfc6979` | deterministic ECDSA nonces. **This is the one that matters most** — RFC 6979 is what stops a repeated nonce, and a repeated nonce publishes the private key |
///
/// **What it did NOT cost, because it was measured first:** `scrypt`, `sha3`,
/// `aes` and `ctr` are all taken `default-features = false`. Their defaults
/// pulled `password-hash` and `base64ct` for a string API this never calls —
/// two packages saved by reading the manifest rather than the README.
///
/// The tree is RustCrypto throughout and that is why eleven is cheap: the
/// digest, block-buffer, crypto-common and generic-array halves were already
/// here via `k256` and `sha2`.
///
/// ⚠ **`k256` gained the `ecdsa` feature** in the same change. It was here for
/// the hive's schnorr; signing an EIP-191 message needs the other curve API on
/// the same crate, not a second implementation of the same curve.
const MAX_PACKAGES: usize = 122;

struct Package {
    name: String,
    version: String,
    checksum: Option<String>,
    source: Option<String>,
}

/// Cargo.lock is TOML, and parsing it here with a TOML crate would grow the
/// tree this file exists to bound. The format is regular enough to scan.
fn packages() -> Vec<Package> {
    let mut out = Vec::new();
    let mut cur: Option<Package> = None;
    for line in LOCK.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            if let Some(p) = cur.take() {
                out.push(p);
            }
            cur = Some(Package {
                name: String::new(),
                version: String::new(),
                checksum: None,
                source: None,
            });
            continue;
        }
        let Some(p) = cur.as_mut() else { continue };
        let Some((key, value)) = line.split_once(" = ") else { continue };
        let value = value.trim().trim_matches('"').to_string();
        match key {
            "name" => p.name = value,
            "version" => p.version = value,
            "checksum" => p.checksum = Some(value),
            "source" => p.source = Some(value),
            _ => {}
        }
    }
    if let Some(p) = cur {
        out.push(p);
    }
    out
}

#[test]
fn the_lockfile_exists_and_is_populated() {
    // The control. Deleting Cargo.lock is the supply-chain regression, not
    // editing a version number — without it, `cargo build` is free to resolve
    // a compromised maintainer's next patch release.
    let all = packages();
    assert!(
        all.len() > 20,
        "Cargo.lock looks empty or unparsed — {} packages",
        all.len()
    );
}

#[test]
fn every_third_party_package_is_checksummed() {
    for p in packages() {
        if p.source.is_none() {
            // A path dependency: one of ours, in this repo, reviewed as source.
            assert!(
                p.name.starts_with("elo-"),
                "{} has no source and is not ours",
                p.name
            );
            continue;
        }
        assert!(
            p.checksum.is_some(),
            "{} {} came from {:?} with no checksum",
            p.name,
            p.version,
            p.source
        );
    }
}

/// Duplicates we cannot resolve from here, each with the reason it exists.
///
/// These are **not** waived because duplicates are fine — a crate at two
/// versions is two maintainers' worth of exposure for one capability. They are
/// waived because the fork is upstream of us and the alternative is pinning a
/// crate we do not control to a version its own dependents will not accept.
/// Anything not on this list still fails, which is the point.
const KNOWN_DUPLICATES: [(&str, &str); 4] = [
    // `ring` (via rustls) pins getrandom 0.2; `tempfile` is on 0.3.
    ("getrandom", "ring pins 0.2, tempfile uses 0.3"),
    // Windows-only, pulled transitively at two generations. Never compiled on
    // the platforms this client currently ships to.
    ("windows-sys", "transitive, and not built on linux or mac"),
    // `k256` 0.13 is on the RustCrypto 0.6 generation; tungstenite's `rand`
    // is on 0.9. Upstream of us, and converging only when k256 majors.
    ("rand_core", "k256 0.13 uses 0.6, tungstenite's rand uses 0.9"),
    // A proc-macro crate: build-time only, never linked into the binary, so
    // the duplication costs compile time and no attack surface at runtime.
    ("syn", "proc-macro only — build-time, never in the shipped binary"),
];

#[test]
fn no_crate_appears_at_two_versions() {
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in packages() {
        seen.entry(p.name).or_default().push(p.version);
    }
    let dupes: Vec<String> = seen
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .filter(|(k, _)| !KNOWN_DUPLICATES.iter().any(|(name, _)| name == k))
        .map(|(k, v)| format!("{k} at {}", v.join(", ")))
        .collect();
    assert!(
        dupes.is_empty(),
        "a crate at two versions is two chances to be compromised: {dupes:?}. \
         If it is genuinely forced upstream, add it to KNOWN_DUPLICATES with the reason."
    );
}

#[test]
fn the_duplicate_waivers_are_still_needed() {
    // A waiver that stopped being true is a rule quietly getting weaker. When
    // upstream converges, this fails and the entry comes out.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for p in packages() {
        *seen.entry(p.name).or_default() += 1;
    }
    for (name, why) in KNOWN_DUPLICATES {
        assert!(
            seen.get(name).copied().unwrap_or(0) > 1,
            "{name} is no longer duplicated ({why}) — drop the waiver"
        );
    }
}

#[test]
fn the_tree_stays_inside_its_budget() {
    let all = packages();
    assert!(
        all.len() <= MAX_PACKAGES,
        "{} packages, budget {MAX_PACKAGES}. Raising the cap is a decision — \
         make it in the same commit as the crate that needed it.",
        all.len()
    );
}

#[test]
fn every_direct_dependency_is_pinned_exactly() {
    // A caret range lets a compromised maintainer's next patch release in on
    // the next `cargo update`. An exact pin makes taking it a visible diff.
    let mut in_deps = false;
    let mut checked = 0;
    for line in MANIFEST.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line == "[workspace.dependencies]";
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, rest)) = line.split_once(" = ") else { continue };
        // Either `name = "=1.2.3"` or `name = { version = "=1.2.3", … }`.
        let version = rest
            .split("version = ")
            .nth(if rest.trim_start().starts_with('{') { 1 } else { 0 })
            .unwrap_or(rest)
            .trim()
            .trim_start_matches('"')
            .to_string();
        assert!(
            version.starts_with('='),
            "{name} is not pinned exactly: {rest}"
        );
        checked += 1;
    }
    assert!(checked >= 5, "only found {checked} direct dependencies to check");
}
