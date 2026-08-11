//! Which hosts bypass the proxy.
//!
//! ⚠ **`ureq` 3.1.2 reads `ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` and does not
//! implement `NO_PROXY` at all** (`Proxy::try_from_env`). Left alone, that
//! makes a launcher on a proxied network tunnel *every* request — including
//! the ones it must not:
//!
//! * `--host http://127.0.0.1:3600`, a local uvicorn, which a client
//!   supports and any developer running the origin uses;
//! * a depot served from a LAN mirror on `192.168.x.x`;
//! * a local test server, which is how this repo tests the transport at all.
//!
//! Found because the transport suite's "nothing is listening" case came back
//! `reachable: true` — the proxy had answered for a port with nothing behind
//! it. That is the repo's own trap wearing a new coat: a read that cannot tell
//! *unreachable* from *answered* is exactly what `Fetched.reachable` exists to
//! prevent, and it was being defeated one layer below us.
//!
//! So the agent is chosen per request: a direct one for anything matching
//! `NO_PROXY` or living on loopback/private address space, the proxied one for
//! the rest.

use std::net::{IpAddr, Ipv4Addr};

/// Hosts that never go through a proxy, whatever the environment says.
/// Loopback is not a policy choice — a proxy cannot reach our own machine's
/// idea of `localhost` on our behalf.
pub fn is_always_direct(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return true;
    }
    match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => false,
    }
}

/// Parse `NO_PROXY` into entries. Empty when unset.
pub fn no_proxy_entries() -> Vec<String> {
    std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Does `host` match a `NO_PROXY` entry?
///
/// The de-facto rules, which no RFC defines: `*` bypasses everything, an entry
/// matches an exact host, a domain suffix with or without a leading dot, a
/// `*.example.com` wildcard, or an IPv4 CIDR.
pub fn matches_no_proxy(host: &str, entries: &[String]) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    for entry in entries {
        if entry == "*" {
            return true;
        }
        if let Some((net, bits)) = entry.split_once('/') {
            if let (Ok(ip), Ok(net), Ok(bits)) = (
                host.parse::<Ipv4Addr>(),
                net.parse::<Ipv4Addr>(),
                bits.parse::<u32>(),
            ) {
                if bits <= 32 {
                    // A /0 shifts by 32, which is UB-adjacent in Rust; mask it
                    // to "match everything" explicitly instead.
                    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
                    if u32::from(ip) & mask == u32::from(net) & mask {
                        return true;
                    }
                }
            }
            continue;
        }
        let bare = entry.trim_start_matches('*').trim_start_matches('.');
        if host == bare || host.ends_with(&format!(".{bare}")) {
            return true;
        }
    }
    false
}

/// The whole decision, for one URL.
pub fn should_bypass(url: &str, entries: &[String]) -> bool {
    let host = host_of(url);
    is_always_direct(&host) || matches_no_proxy(&host, entries)
}

/// The host of a URL, without a scheme, userinfo, port or path. Hand-rolled
/// because pulling a URL crate in to read one field would spend the workspace's
/// dependency budget on a `split`.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    // IPv6 literals are bracketed and full of colons, so the port cannot be
    // split off by looking for the first one.
    if let Some(end) = authority.find(']') {
        return authority[..=end].to_string();
    }
    authority
        .split_once(':')
        .map(|(h, _)| h)
        .unwrap_or(authority)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_private_are_always_direct() {
        for host in ["127.0.0.1", "localhost", "::1", "10.1.2.3", "192.168.1.9", "172.16.0.1"] {
            assert!(is_always_direct(host), "{host} should bypass");
        }
        for host in ["scry.moreright.xyz", "8.8.8.8", "example.com"] {
            assert!(!is_always_direct(host), "{host} should not bypass");
        }
    }

    #[test]
    fn no_proxy_matches_suffixes_wildcards_and_cidr() {
        let e: Vec<String> = ["anthropic.com", ".internal", "*.corp.example", "10.0.0.0/8"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(matches_no_proxy("anthropic.com", &e));
        assert!(matches_no_proxy("api.anthropic.com", &e));
        assert!(matches_no_proxy("box.internal", &e));
        assert!(matches_no_proxy("a.corp.example", &e));
        assert!(matches_no_proxy("10.255.1.1", &e));
        assert!(!matches_no_proxy("11.0.0.1", &e));
        assert!(!matches_no_proxy("notanthropic.com", &e));
        assert!(!matches_no_proxy("scry.moreright.xyz", &e));
    }

    #[test]
    fn a_star_bypasses_everything_and_a_slash_zero_does_not_panic() {
        assert!(matches_no_proxy("anything.at.all", &["*".to_string()]));
        assert!(matches_no_proxy("8.8.8.8", &["0.0.0.0/0".to_string()]));
    }

    #[test]
    fn the_host_is_read_off_a_url_without_a_url_crate() {
        assert_eq!(host_of("https://scry.moreright.xyz/api/tape"), "scry.moreright.xyz");
        assert_eq!(host_of("http://127.0.0.1:3600/health"), "127.0.0.1");
        assert_eq!(host_of("https://user:pw@example.com:8443/x"), "example.com");
        assert_eq!(host_of("http://[::1]:8080/x"), "[::1]");
        assert_eq!(host_of("https://example.com"), "example.com");
    }
}
