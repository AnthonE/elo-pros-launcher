//! Which certificate authorities this client trusts: Mozilla's list, minus
//! the ones we have stopped trusting.
//!
//! The list ships inside the binary (`webpki-roots`), so it changes only when
//! we rebuild, and `rustls` checks no revocation. When a CA goes bad the
//! browsers block it within days and this client keeps trusting it until a
//! release says otherwise. [`DISTRUSTED`] is that release, and every TLS
//! connection the launcher makes (the origin, the chain's RPC, depots, the
//! relay) builds its roots through here.

use std::sync::Arc;

/// Organizations whose roots are dropped, matched in the certificate's
/// subject, and why. Adding a row is the whole response to the next one.
pub const DISTRUSTED: &[(&[u8], &str)] = &[(
    b"eMudhra",
    "eMudhra's intermediate CA keys were weak enough to brute-force (October \
     2026, CVE-2026-111943). Browsers blocked them; a client with a bundled \
     root list has to drop the roots itself.",
)];

/// Whether a root with this DER-encoded subject is one we no longer trust.
pub fn distrusted(subject: &[u8]) -> bool {
    DISTRUSTED.iter().any(|(org, _)| subject.windows(org.len()).any(|w| w == *org))
}

/// The roots as a `rustls` store, for a connection built by hand (the relay).
pub fn root_store() -> rustls::RootCertStore {
    let mut store = rustls::RootCertStore::empty();
    store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().filter(|a| !distrusted(&a.subject)).cloned());
    store
}

/// The same roots as whole certificates, which is the only form `ureq` takes
/// a custom list in. Built once per process: `Net::new` makes four agents.
pub fn ureq_roots() -> ureq::tls::RootCerts {
    static ROOTS: std::sync::OnceLock<ureq::tls::RootCerts> = std::sync::OnceLock::new();
    ROOTS.get_or_init(build_ureq_roots).clone()
}

fn build_ureq_roots() -> ureq::tls::RootCerts {
    let certs: Vec<ureq::tls::Certificate<'static>> = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter(|der| match webpki::anchor_from_trusted_cert(der) {
            Ok(anchor) => !distrusted(&anchor.subject),
            Err(_) => false,
        })
        .map(|der| ureq::tls::Certificate::from_der(der.as_ref()).to_owned())
        .collect();
    ureq::tls::RootCerts::Specific(Arc::new(certs))
}

/// What every `ureq` agent here is built with.
pub fn ureq_config() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder().root_certs(ureq_roots()).build()
}
