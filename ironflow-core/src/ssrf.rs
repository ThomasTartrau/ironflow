//! SSRF guard shared by the outbound HTTP clients: the [`Http`](crate::operations::http::Http)
//! operation and the agent `web_fetch` tool.
//!
//! A request may reach a private, loopback, link-local or cloud metadata address only when
//! its host is explicitly allowed. The host is checked twice: by [`check_url`] before the
//! request, so a refused target fails with a clear message and without retries, and by
//! [`GuardedResolver`] when the connection is opened, so a DNS answer that changes between
//! the two lookups (DNS rebinding) or a redirect cannot reach an internal address either.
//!
//! Hosts are compared after [`Url`] normalization: `http://2130706433/`,
//! `http://0x7f000001/` and `http://0177.0.0.1/` all parse to `127.0.0.1`.

use std::error::Error;
use std::fmt;
use std::iter::successors;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::net::lookup_host;
use tracing::debug;
use url::{Host, Url};

/// Returns `true` if `ip` is a target that must not be reachable via SSRF: private,
/// loopback, link-local, cloud metadata, or an IPv6 form of one of them.
pub(crate) fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            let bits = v6.to_bits();
            v6.is_loopback()                  // ::1
                || v6.is_unspecified()         // ::
                || v6.is_unique_local()        // fc00::/7 (includes AWS metadata fd00:ec2::254)
                || v6.is_unicast_link_local()  // fe80::/10
                // ::ffff:a.b.c.d (mapped) and ::a.b.c.d (compatible) reach the IPv4 address
                || v6.to_ipv4().is_some_and(is_blocked_v4)
                // 64:ff9b::/96, NAT64: a.b.c.d through the NAT64 gateway
                || (bits >> 32 == 0x0064_ff9b_0000_0000_0000_0000
                    && is_blocked_v4(Ipv4Addr::from_bits(bits as u32)))
        }
    }
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    v4.is_loopback()                 // 127.0.0.0/8
        || v4.is_private()            // 10/8, 172.16/12, 192.168/16
        || v4.is_link_local()         // 169.254.0.0/16 (includes AWS/GCP/Azure metadata)
        || v4.is_broadcast()          // 255.255.255.255
        || a == 0                     // 0.0.0.0/8: 0.0.0.0 reaches the local host
        || (a == 100 && b & 0xc0 == 64) // 100.64.0.0/10 (includes Alibaba metadata 100.100.100.200)
}

/// A request target refused because it is, or resolves to, a blocked IP address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockedTarget {
    /// The domain name that resolved to `ip`, `None` when the URL host is `ip` itself.
    host: Option<String>,
    ip: IpAddr,
}

impl fmt::Display for BlockedTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.host {
            Some(host) => write!(
                f,
                "URL host {host} resolves to a blocked IP address ({})",
                self.ip
            )?,
            None => write!(f, "URL targets a blocked IP address ({})", self.ip)?,
        }
        f.write_str(
            ": private, loopback, link-local and cloud metadata addresses are not allowed \
             unless the host is explicitly allowed",
        )
    }
}

impl Error for BlockedTarget {}

/// Finds a [`BlockedTarget`] in the source chain of `err`, where the HTTP client puts the
/// error returned by [`GuardedResolver`] or by a redirect policy.
pub(crate) fn find_blocked<'a>(err: &'a (dyn Error + 'static)) -> Option<&'a BlockedTarget> {
    successors(Some(err), |&e| e.source()).find_map(|e| e.downcast_ref::<BlockedTarget>())
}

/// Hosts exempted from the SSRF guard, compared case-insensitively and without the
/// brackets of an IPv6 literal.
#[derive(Debug, Clone, Default)]
pub(crate) struct AllowedHosts(Vec<String>);

impl AllowedHosts {
    /// Parses a comma-separated list, ignoring blank entries.
    pub(crate) fn parse_list(list: &str) -> Self {
        let mut hosts = Self::default();
        list.split(',').for_each(|host| hosts.add(host));
        hosts
    }

    /// Adds `host`. A blank host is ignored.
    pub(crate) fn add(&mut self, host: &str) {
        let host = trim_host(host).to_ascii_lowercase();
        if !host.is_empty() && !self.contains(&host) {
            self.0.push(host);
        }
    }

    /// Returns `true` if `host` is allowed.
    pub(crate) fn contains(&self, host: &str) -> bool {
        let host = trim_host(host);
        self.0
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    }

    /// Returns `true` if the host of `url` is allowed.
    pub(crate) fn contains_url_host(&self, url: &Url) -> bool {
        url.host_str().is_some_and(|host| self.contains(host))
    }
}

fn trim_host(host: &str) -> &str {
    host.trim().trim_start_matches('[').trim_end_matches(']')
}

/// Returns the [`BlockedTarget`] if the host of `url` is an IP literal in a blocked range.
/// A domain name is not resolved here: see [`check_url`] and [`GuardedResolver`].
pub(crate) fn blocked_literal(url: &Url) -> Option<BlockedTarget> {
    let ip = match url.host()? {
        Host::Ipv4(v4) => IpAddr::V4(v4),
        Host::Ipv6(v6) => IpAddr::V6(v6),
        Host::Domain(_) => return None,
    };
    is_blocked_ip(ip).then_some(BlockedTarget { host: None, ip })
}

/// Checks that `url` neither is nor resolves to a blocked IP address. Allowed hosts are
/// the caller's business: call this only for a host that is not allowed.
///
/// A host that does not resolve passes: the HTTP client resolves it again through
/// [`GuardedResolver`] and reports the DNS error itself, with its usual retry semantics.
///
/// # Errors
///
/// Returns the [`BlockedTarget`] when the URL host is a blocked IP literal, or when any
/// of the addresses it resolves to is blocked.
pub(crate) async fn check_url(url: &Url) -> Result<(), BlockedTarget> {
    let Some(Host::Domain(name)) = url.host() else {
        return blocked_literal(url).map_or(Ok(()), Err);
    };
    let port = url.port_or_known_default().unwrap_or(0);
    match lookup_host((name, port)).await {
        Ok(addrs) => reject_blocked(name, addrs),
        Err(err) => {
            debug!(host = name, error = %err, "ssrf pre-check could not resolve host");
            Ok(())
        }
    }
}

fn reject_blocked(
    host: &str,
    addrs: impl IntoIterator<Item = SocketAddr>,
) -> Result<(), BlockedTarget> {
    match addrs.into_iter().find(|addr| is_blocked_ip(addr.ip())) {
        Some(addr) => Err(BlockedTarget {
            host: Some(host.to_string()),
            ip: addr.ip(),
        }),
        None => Ok(()),
    }
}

/// DNS resolver for [`reqwest`] that refuses a name resolving to a blocked IP address,
/// unless the name is allowed.
///
/// The client connects only to the addresses this resolver returns, so the check holds at
/// connection time: for redirects, and when a DNS answer changes after [`check_url`].
/// An IP literal never reaches a resolver: redirects to one are checked with
/// [`blocked_literal`].
#[derive(Debug, Clone, Default)]
pub(crate) struct GuardedResolver {
    allowed: AllowedHosts,
}

impl GuardedResolver {
    /// Creates a resolver that lets `allowed` hosts resolve to any address.
    #[cfg(any(test, feature = "tool-web-fetch"))]
    pub(crate) fn new(allowed: AllowedHosts) -> Self {
        Self { allowed }
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allowed = self.allowed.clone();
        Box::pin(async move {
            let host = name.as_str();
            let addrs: Vec<SocketAddr> = lookup_host((host, 0)).await?.collect();
            if !allowed.contains(host) {
                reject_blocked(host, addrs.iter().copied())?;
            }
            let addrs: Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::net::Ipv6Addr;
    use std::str::FromStr;

    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn blocks_every_internal_range() {
        for addr in [
            "127.0.0.1",
            "127.255.255.254",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "169.254.0.1",
            "100.100.100.200",
            "100.64.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "febf::1",
            "fc00::1",
            "fd00:ec2::254",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "64:ff9b::a00:1",
        ] {
            assert!(is_blocked_ip(ip(addr)), "{addr} should be blocked");
        }
    }

    #[test]
    fn lets_public_addresses_through() {
        for addr in [
            "8.8.8.8",
            "1.1.1.1",
            "172.32.0.1",
            "100.128.0.1",
            "169.255.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(!is_blocked_ip(ip(addr)), "{addr} should be allowed");
        }
    }

    #[test]
    fn blocked_literal_normalizes_decimal_hex_and_octal_ipv4() {
        for raw in [
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0177.0.0.1/",
            "http://0x7f.1/",
            "http://[::ffff:7f00:1]/",
        ] {
            let blocked = blocked_literal(&url(raw));
            assert!(blocked.is_some(), "{raw} should be blocked");
        }
    }

    #[test]
    fn blocked_literal_ignores_domains_and_public_ips() {
        assert_eq!(blocked_literal(&url("http://localhost/")), None);
        assert_eq!(blocked_literal(&url("http://8.8.8.8/")), None);
    }

    #[tokio::test]
    async fn check_url_rejects_name_resolving_to_loopback() {
        let err = check_url(&url("http://localhost:8080/")).await.unwrap_err();
        assert_eq!(err.host.as_deref(), Some("localhost"));
        assert!(err.ip.is_loopback());
        assert!(
            err.to_string()
                .contains("localhost resolves to a blocked IP address")
        );
    }

    #[tokio::test]
    async fn check_url_rejects_literal() {
        let err = check_url(&url("http://169.254.169.254/latest/meta-data/"))
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "URL targets a blocked IP address (169.254.169.254): private, loopback, link-local \
             and cloud metadata addresses are not allowed unless the host is explicitly allowed"
        );
    }

    #[tokio::test]
    async fn check_url_lets_unresolvable_host_through() {
        let result = check_url(&url("http://ironflow-ssrf-test.invalid/")).await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn resolver_refuses_name_resolving_to_loopback() {
        let name = Name::from_str("localhost").unwrap();
        let err = match GuardedResolver::default().resolve(name).await {
            Ok(_) => panic!("localhost should be refused"),
            Err(err) => err,
        };
        let blocked = err
            .downcast_ref::<BlockedTarget>()
            .expect("a BlockedTarget");
        assert_eq!(blocked.host.as_deref(), Some("localhost"));
    }

    #[tokio::test]
    async fn resolver_resolves_allowed_name() {
        let resolver = GuardedResolver::new(AllowedHosts::parse_list("LocalHost"));
        let name = Name::from_str("localhost").unwrap();
        let addrs: Vec<SocketAddr> = match resolver.resolve(name).await {
            Ok(addrs) => addrs.collect(),
            Err(err) => panic!("localhost is allowed: {err}"),
        };
        assert!(addrs.iter().all(|addr| addr.ip().is_loopback()));
        assert!(!addrs.is_empty());
    }

    #[test]
    fn allowed_hosts_parse_list_trims_lowercases_and_skips_blanks() {
        let hosts = AllowedHosts::parse_list(" Docs.Internal , ,[::1],10.0.0.5,");
        assert!(hosts.contains("docs.internal"));
        assert!(hosts.contains("DOCS.INTERNAL"));
        assert!(hosts.contains("::1"));
        assert!(hosts.contains("[::1]"));
        assert!(hosts.contains("10.0.0.5"));
        assert!(!hosts.contains(""));
        assert!(!hosts.contains("internal"));
        assert_eq!(hosts.0.len(), 3);
    }

    #[test]
    fn allowed_hosts_empty_list_allows_nothing() {
        let hosts = AllowedHosts::parse_list("");
        assert!(!hosts.contains("localhost"));
        assert!(!hosts.contains_url_host(&url("http://localhost/")));
    }

    #[test]
    fn allowed_hosts_match_normalized_url_host() {
        let hosts = AllowedHosts::parse_list("127.0.0.1,::1");
        assert!(hosts.contains_url_host(&url("http://2130706433:8080/")));
        assert!(hosts.contains_url_host(&url("http://[0:0:0:0:0:0:0:1]/")));
        assert!(!hosts.contains_url_host(&url("http://127.0.0.2/")));
    }

    #[test]
    fn find_blocked_walks_the_source_chain() {
        #[derive(Debug)]
        struct Wrapper(BlockedTarget);
        impl fmt::Display for Wrapper {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("wrapper")
            }
        }
        impl Error for Wrapper {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&self.0)
            }
        }
        let target = BlockedTarget {
            host: Some("localhost".into()),
            ip: IpAddr::V6(Ipv6Addr::LOCALHOST),
        };
        let wrapper = Wrapper(target.clone());
        assert_eq!(find_blocked(&wrapper), Some(&target));
        assert_eq!(find_blocked(&io::Error::other("connection refused")), None);
    }
}
