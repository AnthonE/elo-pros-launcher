//! Who published this title, and whether that is a claim or a signature.
//!
//! # The gap this closes
//!
//! A depot is hash-verified byte for byte: `depot_digest` proves you got the
//! bytes the origin served. It proves **nothing about who published them**.
//! Until this module existed, `publisher: "Elo Pros"` was a string somebody
//! typed into a JSON file, and the honest answer to *"who is allowed to submit
//! a game"* was *"whoever can write to the origin's disk"*.
//!
//! Integrity and authenticity are different questions and only one of them was
//! answered.
//!
//! # What is signed, and why it is not the whole document
//!
//! **The origin edits the manifest it serves.** `meter/launcher.py` fills each
//! native build row in from what is actually published — sizes, digests,
//! `depot_state` — because those are measurements and must never be typed. So
//! a signature over the served document would break the moment the origin did
//! its job, and a publisher cannot sign facts they do not own anyway.
//!
//! The split follows ownership. The publisher signs **their own claims** — who
//! they are, what the title is, where its source lives. The origin measures the
//! builds, and those are covered by the depot digest, which is a better tool
//! for bytes than a signature over a description of them.
//!
//! The statement is a fixed line-per-field string rather than canonical JSON:
//! ```text
//! elo-manifest-v1
//! slug:gates
//! name:Gates
//! publisher:Elo Pros
//! repo:https://github.com/AnthonE/Gates
//! source_required:true
//! ```
//! Every field is present in a fixed order with no escaping questions, so two
//! implementations cannot disagree about what the bytes are. Canonical JSON is
//! the other correct answer and is harder to get right twice — key order,
//! unicode escaping, number formatting — for no gain on six known fields.
//!
//! # Secp256k1, and it was not the first choice
//!
//! An ed25519 verifier measured **+10 packages against 4 of headroom** in the
//! budget `supply_chain.rs` enforces, which the workspace manifest makes an
//! operator decision. `k256` is already here, so this costs nothing — and an
//! **Ethereum address** is the better publisher identity regardless: it is one
//! a contract, a merkle drop, an explorer and every wallet already understand,
//! where a second ed25519 identity would be legible only to us.
//!
//! # What this deliberately does NOT decide
//!
//! Whether a publisher is **allowed**. This reports who signed, and refuses to
//! confuse a valid signature with a trusted one — `Signed` carries the address
//! precisely so a caller can check it against a roster it trusts. A client that
//! treated "signed by someone" as "signed by the right someone" would be worse
//! than one that never checked, because it would look like it had.

use crate::Manifest;

/// A separate line-per-field statement, versioned so a later shape is a
/// different string rather than the same string meaning something new.
pub const STATEMENT_VERSION: &str = "elo-manifest-v1";

/// What a manifest's publisher signature amounts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublisherCheck {
    /// No signature offered. **The expected state today** and not a finding:
    /// no publisher has ever signed one, so a client must render this as
    /// "unsigned", never as "invalid".
    Unsigned,
    /// A signature that recovers to the address the manifest names.
    /// ⚠ Says *who*, never *whether they are allowed* — see the module note.
    Signed { address: String },
    /// A signature that recovers to somebody else. This is the one that
    /// matters: it is what a tampered manifest looks like.
    Mismatch { claimed: String, recovered: String },
    /// Malformed, or half-present. Never silently the same as unsigned: a
    /// manifest carrying a broken signature is trying to say something.
    Malformed { why: String },
}

impl PublisherCheck {
    /// One line for a person. Deliberately different words for every arm.
    pub fn line(&self) -> String {
        match self {
            PublisherCheck::Unsigned => "unsigned".into(),
            PublisherCheck::Signed { address } => format!("signed by {address}"),
            PublisherCheck::Mismatch { claimed, recovered } => {
                format!("SIGNATURE MISMATCH — claims {claimed}, signed by {recovered}")
            }
            PublisherCheck::Malformed { why } => format!("signature unreadable — {why}"),
        }
    }

    pub fn is_signed(&self) -> bool {
        matches!(self, PublisherCheck::Signed { .. })
    }

    /// True only for a signature that is present and wrong. Unsigned is not a
    /// failure, and conflating the two is how "nobody has signed yet" becomes
    /// a scary red row on every title in the store.
    pub fn is_broken(&self) -> bool {
        matches!(
            self,
            PublisherCheck::Mismatch { .. } | PublisherCheck::Malformed { .. }
        )
    }
}

/// The exact bytes a publisher signs. Recomputed by the verifier from the
/// manifest, never taken from it — a statement carried in the document could
/// be made to say anything while the fields said something else.
pub fn statement(m: &Manifest) -> String {
    format!(
        "{STATEMENT_VERSION}\nslug:{}\nname:{}\npublisher:{}\nrepo:{}\nsource_required:{}",
        m.slug, m.name, m.publisher, m.repo, m.source_required
    )
}

/// Check a manifest's publisher signature.
pub fn verify(m: &Manifest) -> PublisherCheck {
    let obj = m.raw.as_object();
    let get = |k: &str| {
        obj.and_then(|o| o.get(k))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let claimed = get("publisher_address");
    let sig = get("publisher_sig");

    match (claimed, sig) {
        (None, None) => PublisherCheck::Unsigned,
        // Half a signature is not half a claim — it is a broken one. An address
        // with no signature is exactly what an attacker stripping the signature
        // would leave behind, so it must never read as "unsigned".
        (Some(_), None) => PublisherCheck::Malformed {
            why: "names publisher_address but carries no publisher_sig".into(),
        },
        (None, Some(_)) => PublisherCheck::Malformed {
            why: "carries publisher_sig but names no publisher_address".into(),
        },
        (Some(claimed), Some(sig)) => match elo_vault::recover_signer(&statement(m), sig) {
            Ok(recovered) => {
                // Compared case-insensitively: EIP-55 casing is a checksum, and
                // a manifest hand-written with a lowercase address is careless
                // rather than forged. The address REPORTED is always the
                // recovered checksummed one, so nothing downstream sees the
                // sloppy form.
                if recovered.eq_ignore_ascii_case(claimed) {
                    PublisherCheck::Signed { address: recovered }
                } else {
                    PublisherCheck::Mismatch {
                        claimed: claimed.to_string(),
                        recovered,
                    }
                }
            }
            Err(why) => PublisherCheck::Malformed { why },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest;
    use serde_json::json;

    fn base() -> serde_json::Value {
        json!({
            "manifest_version": 1,
            "slug": "gates",
            "name": "Gates",
            "publisher": "Elo Pros",
            "repo": "https://github.com/AnthonE/Gates",
            "source_required": true,
            "builds": [],
        })
    }

    fn signed_by(secret_byte: u8, raw: serde_json::Value) -> (serde_json::Value, String) {
        let secret = [secret_byte; 32];
        let account = elo_vault::Account::from_secret(&secret).expect("account");
        let m = parse_manifest(&raw).expect("parse");
        let sig = account.sign_message(&statement(&m)).expect("sign");
        let mut out = raw;
        out["publisher_address"] = json!(account.address());
        out["publisher_sig"] = json!(sig);
        (out, account.address())
    }

    #[test]
    fn an_unsigned_manifest_is_unsigned_and_not_invalid() {
        let m = parse_manifest(&base()).expect("parse");
        assert_eq!(verify(&m), PublisherCheck::Unsigned);
        assert!(!verify(&m).is_broken(), "unsigned is the expected state, not a fault");
    }

    #[test]
    fn a_real_signature_verifies_and_names_its_signer() {
        let (raw, address) = signed_by(3, base());
        let m = parse_manifest(&raw).expect("parse");
        assert_eq!(verify(&m), PublisherCheck::Signed { address });
    }

    /// The test that gives the whole thing its point: change a signed field and
    /// the signature must stop matching.
    #[test]
    fn tampering_with_a_signed_field_breaks_the_signature() {
        for field in ["slug", "name", "publisher", "repo"] {
            let (mut raw, _) = signed_by(4, base());
            raw[field] = json!("tampered");
            let m = parse_manifest(&raw).expect("parse");
            let got = verify(&m);
            assert!(got.is_broken(), "changing {field} must break the signature, got {got:?}");
            assert!(matches!(got, PublisherCheck::Mismatch { .. }),
                    "and it should read as a mismatch: {got:?}");
        }
        // source_required is a bool and is signed too.
        let (mut raw, _) = signed_by(4, base());
        raw["source_required"] = json!(false);
        let m = parse_manifest(&raw).expect("parse");
        assert!(verify(&m).is_broken(), "source_required is inside the statement");
    }

    /// Someone else's valid signature is a MISMATCH, not a pass. This is the
    /// attack the address comparison exists for: a real signature, over the
    /// real statement, by the wrong key.
    #[test]
    fn a_signature_from_the_wrong_key_is_refused() {
        let (mut raw, _) = signed_by(5, base());
        // Keep the signature; claim to be somebody else.
        raw["publisher_address"] = json!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
        let m = parse_manifest(&raw).expect("parse");
        assert!(matches!(verify(&m), PublisherCheck::Mismatch { .. }));
    }

    /// Stripping the signature must not look like a title that never had one.
    #[test]
    fn half_a_signature_is_malformed_rather_than_unsigned() {
        let (mut raw, _) = signed_by(6, base());
        raw.as_object_mut().unwrap().remove("publisher_sig");
        let m = parse_manifest(&raw).expect("parse");
        let got = verify(&m);
        assert!(got.is_broken(), "a stripped signature must not read as unsigned: {got:?}");

        let (mut raw2, _) = signed_by(6, base());
        raw2.as_object_mut().unwrap().remove("publisher_address");
        let m2 = parse_manifest(&raw2).expect("parse");
        assert!(verify(&m2).is_broken());
    }

    #[test]
    fn a_lowercase_address_still_matches_and_is_reported_checksummed() {
        let (mut raw, address) = signed_by(7, base());
        raw["publisher_address"] = json!(address.to_lowercase());
        let m = parse_manifest(&raw).expect("parse");
        assert_eq!(verify(&m), PublisherCheck::Signed { address },
                   "casing is a checksum, not an identity — and we report the checksummed form");
    }

    #[test]
    fn garbage_is_malformed_and_says_why() {
        let (mut raw, _) = signed_by(8, base());
        raw["publisher_sig"] = json!("0xnothex");
        let m = parse_manifest(&raw).expect("parse");
        assert!(matches!(verify(&m), PublisherCheck::Malformed { .. }));
    }

    /// The statement is recomputed from the fields, so a statement smuggled
    /// into the document cannot be what gets checked.
    #[test]
    fn the_statement_covers_every_field_it_claims_to() {
        let m = parse_manifest(&base()).expect("parse");
        let s = statement(&m);
        assert!(s.starts_with(STATEMENT_VERSION), "versioned first: {s}");
        for expect in ["slug:gates", "name:Gates", "publisher:Elo Pros",
                       "repo:https://github.com/AnthonE/Gates", "source_required:true"] {
            assert!(s.contains(expect), "statement is missing {expect}:\n{s}");
        }
    }
}
