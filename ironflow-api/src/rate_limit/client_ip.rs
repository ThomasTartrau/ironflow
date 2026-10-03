//! Client IP resolution for rate limiting.
//!
//! The client is the TCP peer, unless that peer is a trusted reverse proxy:
//! then `X-Forwarded-For` is read from the right, each trusted hop skipped,
//! and the first untrusted hop is the client. Hops further left were written
//! by the client itself and are never read.

use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::Once;

use axum::http::HeaderMap;
use ipnet::IpNet;
use thiserror::Error;
use tracing::warn;

/// Reverse proxies whose `X-Forwarded-For` and `X-Real-IP` headers are
/// believed.
///
/// Empty by default: the client IP is then always the TCP peer. Behind a
/// reverse proxy, list its address (or range), otherwise every client shares
/// the proxy's bucket.
///
/// Parsed from a comma-separated list of IP addresses and CIDR ranges.
///
/// # Examples
///
/// ```
/// use ironflow_api::rate_limit::TrustedProxies;
///
/// let proxies: TrustedProxies = "10.0.0.0/8, 192.168.1.10".parse()?;
/// assert!(proxies.contains("10.1.2.3".parse()?));
/// assert!(proxies.contains("192.168.1.10".parse()?));
/// assert!(!proxies.contains("203.0.113.7".parse()?));
/// assert!(TrustedProxies::default().is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedProxies(Vec<IpNet>);

impl TrustedProxies {
    /// Whether `ip` belongs to one of the trusted proxies.
    ///
    /// An IPv4-mapped IPv6 address (`::ffff:10.0.0.1`) matches its IPv4 form.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::rate_limit::TrustedProxies;
    ///
    /// let proxies: TrustedProxies = "10.0.0.1".parse()?;
    /// assert!(proxies.contains("::ffff:10.0.0.1".parse()?));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.0.iter().any(|net| net.contains(&ip))
    }

    /// Whether no proxy is trusted.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::rate_limit::TrustedProxies;
    ///
    /// assert!(TrustedProxies::default().is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A [`TrustedProxies`] entry that is neither an IP address nor a CIDR range.
///
/// # Examples
///
/// ```
/// use ironflow_api::rate_limit::TrustedProxies;
///
/// let err = "10.0.0.0/8, proxy.local".parse::<TrustedProxies>().unwrap_err();
/// assert_eq!(err.0, "proxy.local");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid trusted proxy {0:?}: expected an IP address or a CIDR range")]
pub struct InvalidTrustedProxy(pub String);

impl FromStr for TrustedProxies {
    type Err = InvalidTrustedProxy;

    /// Parse a comma-separated list. Blank entries are skipped, so an empty
    /// string trusts no proxy.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                entry
                    .parse::<IpNet>()
                    .or_else(|_| entry.parse::<IpAddr>().map(IpNet::from))
                    .map_err(|_| InvalidTrustedProxy(entry.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(TrustedProxies)
    }
}

static MISSING_PEER_WARNING: Once = Once::new();

/// The IP address a request is rate limited under.
///
/// `peer` is the TCP peer, `None` when the server was not built with
/// `into_make_service_with_connect_info`. Every such request then shares one
/// bucket: limiting everyone together beats letting a header pick the
/// bucket.
pub(super) fn client_ip(
    peer: Option<IpAddr>,
    headers: &HeaderMap,
    trusted: &TrustedProxies,
) -> IpAddr {
    let Some(peer) = peer.map(|ip| ip.to_canonical()) else {
        MISSING_PEER_WARNING.call_once(|| {
            warn!(
                "rate limiter cannot see the client address: serve the router with \
                 into_make_service_with_connect_info::<SocketAddr>(); all clients \
                 share one bucket until then"
            );
        });
        return IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    };
    if !trusted.contains(peer) {
        return peer;
    }

    let hops: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();

    if hops.is_empty() {
        return headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
            .map_or(peer, |ip| ip.to_canonical());
    }

    let mut client = peer;
    for hop in hops.iter().rev() {
        // A trusted proxy always writes a valid address: an unparsable hop
        // comes from beyond the trusted chain, so the last trusted hop wins.
        let Ok(ip) = hop.parse::<IpAddr>() else {
            return client;
        };
        let ip = ip.to_canonical();
        if !trusted.contains(ip) {
            return ip;
        }
        client = ip;
    }
    client
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn proxies(s: &str) -> TrustedProxies {
        s.parse().unwrap()
    }

    #[test]
    fn untrusted_peer_ignores_forwarding_headers() {
        let h = headers(&[("x-forwarded-for", "9.9.9.9"), ("x-real-ip", "8.8.8.8")]);
        assert_eq!(
            client_ip(Some(ip("203.0.113.7")), &h, &TrustedProxies::default()),
            ip("203.0.113.7")
        );
        assert_eq!(
            client_ip(Some(ip("203.0.113.7")), &h, &proxies("10.0.0.0/8")),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn trusted_peer_uses_the_last_forwarded_hop() {
        let h = headers(&[("x-forwarded-for", "6.6.6.6, 198.51.100.4")]);
        assert_eq!(
            client_ip(Some(ip("10.0.0.2")), &h, &proxies("10.0.0.0/8")),
            ip("198.51.100.4")
        );
    }

    #[test]
    fn trusted_hops_are_skipped_from_the_right() {
        let h = headers(&[("x-forwarded-for", "6.6.6.6, 198.51.100.4, 10.0.0.9")]);
        assert_eq!(
            client_ip(Some(ip("10.0.0.2")), &h, &proxies("10.0.0.0/8")),
            ip("198.51.100.4")
        );
    }

    #[test]
    fn all_hops_trusted_gives_the_leftmost() {
        let h = headers(&[("x-forwarded-for", "10.0.0.8, 10.0.0.9")]);
        assert_eq!(
            client_ip(Some(ip("10.0.0.2")), &h, &proxies("10.0.0.0/8")),
            ip("10.0.0.8")
        );
    }

    #[test]
    fn unparsable_hop_stops_at_the_last_trusted_hop() {
        let h = headers(&[("x-forwarded-for", "198.51.100.4, garbage, 10.0.0.9")]);
        assert_eq!(
            client_ip(Some(ip("10.0.0.2")), &h, &proxies("10.0.0.0/8")),
            ip("10.0.0.9")
        );
    }

    #[test]
    fn several_forwarded_for_headers_are_read_in_order() {
        let h = headers(&[
            ("x-forwarded-for", "6.6.6.6"),
            ("x-forwarded-for", "198.51.100.4"),
        ]);
        assert_eq!(
            client_ip(Some(ip("10.0.0.2")), &h, &proxies("10.0.0.0/8")),
            ip("198.51.100.4")
        );
    }

    #[test]
    fn trusted_peer_without_forwarded_for_uses_x_real_ip() {
        let h = headers(&[("x-real-ip", "198.51.100.4")]);
        assert_eq!(
            client_ip(Some(ip("10.0.0.2")), &h, &proxies("10.0.0.0/8")),
            ip("198.51.100.4")
        );
    }

    #[test]
    fn trusted_peer_without_headers_is_the_client() {
        assert_eq!(
            client_ip(
                Some(ip("10.0.0.2")),
                &HeaderMap::new(),
                &proxies("10.0.0.0/8")
            ),
            ip("10.0.0.2")
        );
    }

    #[test]
    fn ipv4_mapped_peer_is_canonicalized() {
        let h = headers(&[("x-forwarded-for", "198.51.100.4")]);
        assert_eq!(
            client_ip(Some(ip("::ffff:10.0.0.2")), &h, &proxies("10.0.0.2")),
            ip("198.51.100.4")
        );
        assert_eq!(
            client_ip(
                Some(ip("::ffff:203.0.113.7")),
                &h,
                &TrustedProxies::default()
            ),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn missing_peer_falls_back_to_one_shared_address() {
        let h = headers(&[("x-forwarded-for", "9.9.9.9")]);
        assert_eq!(
            client_ip(None, &h, &proxies("0.0.0.0/0")),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
    }

    #[test]
    fn parses_addresses_and_ranges() {
        let parsed = proxies(" 10.0.0.0/8 ,192.168.1.10, fd00::/8 ,");
        assert!(parsed.contains(ip("10.255.0.1")));
        assert!(parsed.contains(ip("192.168.1.10")));
        assert!(!parsed.contains(ip("192.168.1.11")));
        assert!(parsed.contains(ip("fd12::1")));
    }

    #[test]
    fn empty_string_trusts_no_proxy() {
        assert!(proxies("").is_empty());
        assert!(proxies(" , ").is_empty());
    }

    #[test]
    fn invalid_entry_is_reported() {
        assert_eq!(
            "10.0.0.0/8,10.0.0.0/33".parse::<TrustedProxies>(),
            Err(InvalidTrustedProxy("10.0.0.0/33".to_string()))
        );
        assert_eq!(
            "localhost".parse::<TrustedProxies>(),
            Err(InvalidTrustedProxy("localhost".to_string()))
        );
    }
}
