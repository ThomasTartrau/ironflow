//! `TRUSTED_PROXIES`: the reverse proxies whose forwarding headers the rate
//! limiters believe.

use crate::rate_limit::TrustedProxies;

/// Parse the raw `TRUSTED_PROXIES` value. Unset or blank trusts no proxy.
/// An invalid entry is pushed to `errors` and no proxy is trusted.
pub(super) fn parse_trusted_proxies(raw: Option<&str>, errors: &mut Vec<String>) -> TrustedProxies {
    let Some(raw) = raw else {
        return TrustedProxies::default();
    };
    raw.parse().unwrap_or_else(|err| {
        errors.push(format!("TRUSTED_PROXIES: {err}"));
        TrustedProxies::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_trusts_no_proxy() {
        let mut errors = Vec::new();
        assert!(parse_trusted_proxies(None, &mut errors).is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn addresses_and_ranges_are_parsed() {
        let mut errors = Vec::new();
        let proxies = parse_trusted_proxies(Some("10.0.0.0/8, 172.17.0.1"), &mut errors);
        assert!(errors.is_empty());
        assert!(proxies.contains("10.4.5.6".parse().unwrap()));
        assert!(proxies.contains("172.17.0.1".parse().unwrap()));
        assert!(!proxies.contains("172.17.0.2".parse().unwrap()));
    }

    #[test]
    fn invalid_entry_is_a_config_error() {
        let mut errors = Vec::new();
        let proxies = parse_trusted_proxies(Some("10.0.0.0/8, ingress"), &mut errors);
        assert!(proxies.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("TRUSTED_PROXIES"), "{}", errors[0]);
        assert!(errors[0].contains("\"ingress\""), "{}", errors[0]);
    }
}
