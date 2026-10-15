//! `TRUSTED_PROXIES` and `TRUSTED_PROXY_HEADER`: the reverse proxies whose
//! forwarding header the rate limiters believe, and which header that is.

use crate::rate_limit::{ClientIpHeader, TrustedProxies};

/// Parse the raw `TRUSTED_PROXIES` and `TRUSTED_PROXY_HEADER` values.
///
/// Unset or blank `TRUSTED_PROXIES` trusts no proxy; an invalid entry is
/// pushed to `errors` and no proxy is trusted. Unset `TRUSTED_PROXY_HEADER`
/// reads `X-Forwarded-For`; an invalid name is pushed to `errors`.
pub(super) fn parse_trusted_proxies(
    raw: Option<&str>,
    raw_header: Option<&str>,
    errors: &mut Vec<String>,
) -> TrustedProxies {
    let proxies = match raw {
        None => TrustedProxies::default(),
        Some(raw) => raw.parse().unwrap_or_else(|err| {
            errors.push(format!("TRUSTED_PROXIES: {err}"));
            TrustedProxies::default()
        }),
    };
    let header = match raw_header.map(str::parse::<ClientIpHeader>) {
        None => ClientIpHeader::default(),
        Some(Ok(header)) => header,
        Some(Err(err)) => {
            errors.push(format!("TRUSTED_PROXY_HEADER: {err}"));
            ClientIpHeader::default()
        }
    };
    proxies.with_header(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_trusts_no_proxy() {
        let mut errors = Vec::new();
        assert!(parse_trusted_proxies(None, None, &mut errors).is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn addresses_and_ranges_are_parsed() {
        let mut errors = Vec::new();
        let proxies = parse_trusted_proxies(Some("10.0.0.0/8, 172.17.0.1"), None, &mut errors);
        assert!(errors.is_empty());
        assert!(proxies.contains("10.4.5.6".parse().unwrap()));
        assert!(proxies.contains("172.17.0.1".parse().unwrap()));
        assert!(!proxies.contains("172.17.0.2".parse().unwrap()));
    }

    #[test]
    fn invalid_entry_is_a_config_error() {
        let mut errors = Vec::new();
        let proxies = parse_trusted_proxies(Some("10.0.0.0/8, ingress"), None, &mut errors);
        assert!(proxies.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("TRUSTED_PROXIES"), "{}", errors[0]);
        assert!(errors[0].contains("\"ingress\""), "{}", errors[0]);
    }

    #[test]
    fn header_defaults_to_x_forwarded_for() {
        let mut errors = Vec::new();
        let proxies = parse_trusted_proxies(Some("10.0.0.0/8"), None, &mut errors);
        assert!(errors.is_empty());
        assert_eq!(proxies.header(), ClientIpHeader::XForwardedFor);
    }

    #[test]
    fn x_real_ip_header_is_read() {
        let mut errors = Vec::new();
        let proxies = parse_trusted_proxies(Some("10.0.0.0/8"), Some("X-Real-IP"), &mut errors);
        assert!(errors.is_empty());
        assert_eq!(proxies.header(), ClientIpHeader::XRealIp);
        assert!(proxies.contains("10.4.5.6".parse().unwrap()));
    }

    #[test]
    fn invalid_header_is_a_config_error() {
        let mut errors = Vec::new();
        let proxies = parse_trusted_proxies(Some("10.0.0.0/8"), Some("forwarded"), &mut errors);
        assert_eq!(proxies.header(), ClientIpHeader::XForwardedFor);
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].starts_with("TRUSTED_PROXY_HEADER"),
            "{}",
            errors[0]
        );
        assert!(errors[0].contains("\"forwarded\""), "{}", errors[0]);
    }
}
