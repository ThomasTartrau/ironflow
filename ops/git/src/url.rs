//! Validation and normalization of the remote URLs handed to the git CLI.

use ironflow_core::error::OperationError;

use crate::helpers::{external_error, redact_url};

/// Default port of each URL scheme, dropped by [`normalize_url`].
const DEFAULT_PORTS: &[(&str, &str)] = &[
    ("http", "80"),
    ("https", "443"),
    ("ssh", "22"),
    ("git", "9418"),
];

/// Reject a remote URL that the git CLI could misread or that carries
/// credentials, before any process is spawned.
///
/// Rejected: an empty URL, a URL starting with `-` (read as an option), a
/// control character (newline, NUL...), a transport-helper form such as
/// `ext::sh -c ...`, and credentials embedded in the URL. The error message
/// never shows those credentials.
pub(crate) fn validate_remote_url(url: &str) -> Result<(), OperationError> {
    let refuse = |why: &str| -> Result<(), OperationError> {
        Err(external_error(format!(
            "refusing remote URL `{}`: {why}",
            redact_url(url).escape_debug()
        )))
    };
    if url.is_empty() {
        return refuse("it is empty");
    }
    if url.starts_with('-') {
        return refuse("it starts with `-`");
    }
    if url.chars().any(char::is_control) {
        return refuse("it contains a control character");
    }
    let first_slash = url.find('/').unwrap_or(url.len());
    if url.find("::").is_some_and(|idx| idx < first_slash) {
        return refuse("transport helpers are not allowed");
    }
    if let Some((scheme, rest)) = url.split_once("://") {
        let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
        if let Some((userinfo, _)) = authority.rsplit_once('@') {
            let scheme = scheme.to_ascii_lowercase();
            if scheme == "http" || scheme == "https" || userinfo.contains(':') {
                return refuse(
                    "credentials in the URL are not allowed, use `token_secret` or `token` instead",
                );
            }
        }
    }
    Ok(())
}

/// Bring a remote URL to a canonical form, so that two spellings of the
/// same remote compare equal.
///
/// The scheme and host are lowercased, the userinfo and the default port are
/// dropped, the scp-like form `user@host:path` becomes `ssh://host/path`, and
/// trailing `/` and `.git` are stripped. The case of the path is kept.
pub(crate) fn normalize_url(url: &str) -> String {
    let url = url.trim();
    let normalized = match url.split_once("://") {
        Some((scheme, rest)) => normalize_scheme_url(scheme, rest),
        None => match scp_parts(url) {
            Some((host, path)) if path.starts_with('/') => {
                normalize_scheme_url("ssh", &format!("{host}{path}"))
            }
            Some((host, path)) => normalize_scheme_url("ssh", &format!("{host}/{path}")),
            None => url.to_string(),
        },
    };
    let trimmed = normalized.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    trimmed.trim_end_matches('/').to_string()
}

/// Split the scp-like form `[user@]host:path` into its host (with the user)
/// and path, or `None` if `url` is not in that form.
fn scp_parts(url: &str) -> Option<(&str, &str)> {
    let colon = url.find(':')?;
    let first_slash = url.find('/').unwrap_or(url.len());
    if colon < first_slash {
        Some((&url[..colon], &url[colon + 1..]))
    } else {
        None
    }
}

/// Normalize `scheme://[userinfo@]host[:port]/path` given as its scheme and
/// the part after `://`.
fn normalize_scheme_url(scheme: &str, rest: &str) -> String {
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    // An IPv6 host is bracketed and holds `:`, so the port is only looked
    // for after the closing bracket.
    let port_from = host_port.rfind(']').map_or(0, |idx| idx + 1);
    let (host, port) = match host_port[port_from..].rfind(':') {
        Some(idx) => {
            let idx = port_from + idx;
            (&host_port[..idx], Some(&host_port[idx + 1..]))
        }
        None => (host_port, None),
    };
    let default_port = DEFAULT_PORTS
        .iter()
        .find(|(s, _)| *s == scheme)
        .map(|(_, p)| *p);
    let port = match port {
        Some(p) if p.is_empty() || Some(p) == default_port => String::new(),
        Some(p) => format!(":{p}"),
        None => String::new(),
    };
    format!("{scheme}://{}{port}{path}", host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_spellings_normalize_to_the_same_url() {
        for (a, b) in [
            ("https://GitLab.com/g/r.git", "https://gitlab.com/g/r"),
            ("https://oauth2:t@gitlab.com/g/r", "https://gitlab.com/g/r"),
            ("https://gitlab.com:443/g/r", "https://gitlab.com/g/r/"),
            ("git@gitlab.com:g/r.git", "ssh://git@gitlab.com:22/g/r"),
            ("/srv/repo.git/", "/srv/repo"),
            ("  http://h:8080/r.git/  ", "http://h:8080/r"),
            ("http://[::1]:80/r", "http://[::1]/r"),
        ] {
            assert_eq!(normalize_url(a), normalize_url(b), "{a} vs {b}");
        }
    }

    #[test]
    fn different_remotes_do_not_normalize_to_the_same_url() {
        for (a, b) in [
            ("http://gitlab.com/g/r", "https://gitlab.com/g/r"),
            ("https://gitlab.com/g/r", "https://github.com/g/r"),
            ("https://gitlab.com/g/r", "https://gitlab.com/g/other"),
            ("https://gitlab.com/G/r", "https://gitlab.com/g/r"),
            ("http://127.0.0.1:8080/r", "http://127.0.0.1:8081/r"),
        ] {
            assert_ne!(normalize_url(a), normalize_url(b), "{a} vs {b}");
        }
    }

    #[test]
    fn validate_accepts_plain_urls_and_paths() {
        for url in [
            "https://gitlab.com/g/r.git",
            "git@gitlab.com:g/r.git",
            "ssh://git@gitlab.com/g/r.git",
            "/srv/repo.git",
            "http://[::1]:8080/r",
        ] {
            validate_remote_url(url).unwrap();
        }
    }

    #[test]
    fn validate_rejects_credentials_without_showing_them() {
        let message = validate_remote_url("https://oauth2:s3cret@h/r")
            .unwrap_err()
            .to_string();
        assert!(!message.contains("s3cret"), "got: {message}");
        assert!(message.contains("token_secret"), "got: {message}");

        let message = validate_remote_url("https://s3cret@h/r")
            .unwrap_err()
            .to_string();
        assert!(!message.contains("s3cret"), "got: {message}");

        validate_remote_url("ssh://user:s3cret@h/r").unwrap_err();
    }

    #[test]
    fn validate_rejects_option_like_control_chars_helpers_and_empty() {
        for url in [
            "-uhttps://h/r",
            "https://h/r\nx",
            "ext::sh -c touch",
            "",
            "a\0b",
        ] {
            assert!(validate_remote_url(url).is_err(), "accepted: {url:?}");
        }
    }
}
