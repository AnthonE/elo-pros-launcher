//! The roots every TLS connection here is built with.

use elo_net::tls;

/// eMudhra's three roots in Mozilla's list (October 2026): emSign Root CA
/// C1, emSign Root CA G1, emSign ECC Root CA C3.
const EMUDHRA_ROOTS: usize = 3;

#[test]
fn the_relay_store_drops_exactly_the_distrusted_roots() {
    let all = webpki_roots::TLS_SERVER_ROOTS.len();
    let kept = tls::root_store();
    assert_eq!(kept.roots.len(), all - EMUDHRA_ROOTS);
    assert!(kept.roots.iter().all(|a| !tls::distrusted(&a.subject)));
}

#[test]
fn the_ureq_roots_drop_them_too() {
    let all = webpki_root_certs::TLS_SERVER_ROOT_CERTS.len();
    let ureq::tls::RootCerts::Specific(certs) = tls::ureq_roots() else {
        panic!("the agents must use our list, not ureq's built-in one");
    };
    assert_eq!(certs.len(), all - EMUDHRA_ROOTS);
    for c in certs.iter() {
        let der = rustls::pki_types::CertificateDer::from(c.der());
        let anchor = webpki::anchor_from_trusted_cert(&der).expect("a root");
        assert!(!tls::distrusted(&anchor.subject));
    }
}

#[test]
fn the_match_is_on_the_subject_and_names_emsign() {
    let dropped: Vec<_> = webpki_roots::TLS_SERVER_ROOTS
        .iter()
        .filter(|a| tls::distrusted(&a.subject))
        .map(|a| String::from_utf8_lossy(&a.subject).into_owned())
        .collect();
    assert_eq!(dropped.len(), EMUDHRA_ROOTS);
    assert!(dropped.iter().all(|s| s.contains("emSign")), "{dropped:?}");
}
