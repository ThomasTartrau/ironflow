//! Masks secrets in text an ops crate sends to a third party (a GitLab note,
//! a merge request description, a chat message).
//!
//! Two layers: the values of the secrets the caller knows, then the shape of
//! the tokens an agent or a command may print (`sk-ant-...`, `glpat-...`).

use std::fmt;

/// Replacement for every masked secret.
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::redact::{REDACTED, Redactor};
///
/// let masked = Redactor::new(["hunter2hunter2"]).redact("hunter2hunter2");
/// assert_eq!(masked.as_deref(), Some(REDACTED));
/// ```
pub const REDACTED: &str = "[REDACTED]";

/// Known values shorter than this are ignored, to avoid masking common substrings.
const MIN_SECRET_LEN: usize = 8;

/// Token prefixes matched by shape.
const TOKEN_PREFIXES: [&str; 2] = ["sk-ant-", "glpat-"];

/// A prefix followed by fewer token characters than this is not a token.
const MIN_TOKEN_RUN: usize = 20;

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Masks known secret values and token-shaped strings in text.
///
/// Known values are trimmed, values shorter than 8 characters are ignored,
/// and the longest values are masked first so that a secret containing
/// another one is masked whole. Then every `sk-ant-` or `glpat-` prefix
/// followed by at least 20 token characters (`[A-Za-z0-9_-]`) is masked.
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::redact::Redactor;
///
/// let redactor = Redactor::new(["hunter2hunter2"]);
/// let masked = redactor.redact("token is hunter2hunter2");
/// assert_eq!(masked.as_deref(), Some("token is [REDACTED]"));
/// assert_eq!(redactor.redact("nothing secret"), None);
/// ```
#[derive(Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    /// Build a redactor that masks `secrets` and token-shaped strings.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_common::redact::Redactor;
    ///
    /// let redactor = Redactor::new(["first-secret", "second-secret"]);
    /// assert!(redactor.redact("first-secret").is_some());
    /// ```
    pub fn new<I, S>(secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::default().with_secrets(secrets)
    }

    /// Add `secrets` to the values to mask.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_common::redact::Redactor;
    ///
    /// let redactor = Redactor::default().with_secrets(["added-later-secret"]);
    /// assert!(redactor.redact("added-later-secret").is_some());
    /// ```
    pub fn with_secrets<I, S>(mut self, secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.secrets.extend(
            secrets
                .into_iter()
                .map(|s| s.as_ref().trim().to_string())
                .filter(|s| s.len() >= MIN_SECRET_LEN),
        );
        // Equal values must be adjacent for `dedup`: break length ties on the value.
        self.secrets
            .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        self.secrets.dedup();
        self
    }

    /// Mask the known secrets and token-shaped strings in `text`.
    ///
    /// Returns `None` when nothing was masked, so the caller knows whether
    /// the text carried a secret.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_common::redact::Redactor;
    ///
    /// let redactor = Redactor::default();
    /// let masked = redactor.redact("key: glpat-abcdefghij0123456789");
    /// assert_eq!(masked.as_deref(), Some("key: [REDACTED]"));
    /// ```
    pub fn redact(&self, text: &str) -> Option<String> {
        let mut masked = false;
        let mut current = text.to_string();
        for value in &self.secrets {
            if current.contains(value.as_str()) {
                current = current.replace(value.as_str(), REDACTED);
                masked = true;
            }
        }

        let bytes = current.as_bytes();
        let mut out = String::with_capacity(current.len());
        let mut last = 0;
        let mut i = 0;
        while i < bytes.len() {
            let rest = &bytes[i..];
            if let Some(prefix) = TOKEN_PREFIXES
                .iter()
                .find(|p| rest.starts_with(p.as_bytes()))
            {
                let run = rest[prefix.len()..]
                    .iter()
                    .take_while(|b| is_token_byte(**b))
                    .count();
                if run >= MIN_TOKEN_RUN {
                    // `i` sits on an ASCII prefix byte and the run is ASCII,
                    // so both slice bounds are char boundaries.
                    out.push_str(&current[last..i]);
                    out.push_str(REDACTED);
                    i += prefix.len() + run;
                    last = i;
                    masked = true;
                    continue;
                }
            }
            i += 1;
        }
        out.push_str(&current[last..]);

        masked.then_some(out)
    }
}

impl fmt::Debug for Redactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Redactor")
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{REDACTED, Redactor};

    #[test]
    fn masks_a_known_secret_value() {
        let redactor = Redactor::new(["hunter2hunter2"]);
        assert_eq!(
            redactor.redact("token is hunter2hunter2 ok").as_deref(),
            Some("token is [REDACTED] ok")
        );
    }

    #[test]
    fn masks_every_occurrence_and_the_longest_value_first() {
        let redactor = Redactor::new(["supersecret1", "supersecret1-extended"]);
        assert_eq!(
            redactor
                .redact("a supersecret1 b supersecret1-extended c supersecret1")
                .as_deref(),
            Some("a [REDACTED] b [REDACTED] c [REDACTED]")
        );
    }

    #[test]
    fn longest_first_holds_across_with_secrets_calls() {
        let redactor = Redactor::new(["supersecret1"]).with_secrets(["supersecret1-extended"]);
        assert_eq!(
            redactor.redact("x supersecret1-extended y").as_deref(),
            Some("x [REDACTED] y")
        );
    }

    #[test]
    fn duplicate_secrets_are_kept_once() {
        let redactor =
            Redactor::new(["bbbbbbbb", "aaaaaaaa", "bbbbbbbb"]).with_secrets(["aaaaaaaa"]);
        assert_eq!(format!("{redactor:?}"), "Redactor { secrets: 2 }");
    }

    #[test]
    fn masks_a_trimmed_secret_value() {
        let redactor = Redactor::new(["  padded-secret-value\n"]);
        assert_eq!(
            redactor.redact("v=padded-secret-value").as_deref(),
            Some("v=[REDACTED]")
        );
    }

    #[test]
    fn masks_the_sk_ant_shape() {
        let redactor = Redactor::default();
        assert_eq!(
            redactor
                .redact("summary: `sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUv`.")
                .as_deref(),
            Some("summary: `[REDACTED]`.")
        );
    }

    #[test]
    fn masks_the_glpat_shape_inside_a_url() {
        let redactor = Redactor::default();
        assert_eq!(
            redactor
                .redact("https://oauth2:glpat-abcdefghij0123456789@gitlab.com/x")
                .as_deref(),
            Some("https://oauth2:[REDACTED]@gitlab.com/x")
        );
    }

    #[test]
    fn masks_a_token_of_exactly_the_minimum_run() {
        let redactor = Redactor::default();
        assert_eq!(
            redactor.redact("sk-ant-01234567890123456789").as_deref(),
            Some(REDACTED)
        );
    }

    #[test]
    fn leaves_a_prefix_with_a_short_run_alone() {
        let redactor = Redactor::default();
        assert_eq!(
            redactor.redact("the sk-ant-abc prefix and sk-ant-0123456789012345678 (19)"),
            None
        );
    }

    #[test]
    fn ignores_short_or_blank_secrets() {
        let redactor = Redactor::new(["abc", "", "   ", "1234567"]);
        assert_eq!(redactor.redact("short abc and 1234567 and   blank"), None);
    }

    #[test]
    fn keeps_unicode_around_a_masked_token() {
        let redactor = Redactor::default();
        let masked = redactor
            .redact("été \u{1F680} glpat-abcdefghij0123456789 écrit - fin")
            .unwrap();
        assert_eq!(masked, "été \u{1F680} [REDACTED] écrit - fin");
        assert_eq!(redactor.redact(&masked), None);
    }

    #[test]
    fn masks_a_unicode_secret_value() {
        let redactor = Redactor::new(["mot-de-passe-éàç"]);
        assert_eq!(
            redactor.redact("p: mot-de-passe-éàç!").as_deref(),
            Some("p: [REDACTED]!")
        );
    }

    #[test]
    fn returns_none_for_clean_text() {
        let redactor = Redactor::new(["not-present-value"]);
        assert_eq!(
            redactor.redact("plain note with `code` and https://gitlab.com/x"),
            None
        );
    }

    #[test]
    fn returns_none_for_empty_text() {
        let redactor = Redactor::new(["some-secret-value"]);
        assert_eq!(redactor.redact(""), None);
    }

    #[test]
    fn debug_does_not_leak_secret_values() {
        let redactor = Redactor::new(["do-not-print-me"]);
        let debug = format!("{redactor:?}");
        assert!(!debug.contains("do-not-print-me"), "{debug}");
    }
}
