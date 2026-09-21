//! [`Assignee`] — the user or group an approval gate is assigned to.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Who an approval gate is currently assigned to: an individual user or a group.
///
/// The distinction is carried in the type rather than in a free-form string, so
/// callers cannot confuse a user with a group. On the wire and in the store the
/// value is a single prefixed string — `user:{name}` for [`Assignee::User`] and
/// `group:{name}` for [`Assignee::Group`] — which keeps the database column a
/// plain `TEXT` and the OpenAPI type a plain `string`.
///
/// The distinction is advisory: it drives notification routing and audit, not
/// authorization. Whether an approver is allowed to resolve a gate is decided by
/// the API layer, not by this type.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::Assignee;
///
/// let alice = Assignee::user("alice");
/// assert_eq!(alice.to_string(), "user:alice");
/// assert_eq!(alice.name(), "alice");
///
/// let sre: Assignee = "group:sre-oncall".parse()?;
/// assert_eq!(sre, Assignee::group("sre-oncall"));
/// assert!(sre.is_group());
/// # Ok::<(), ironflow_store::entities::AssigneeParseError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Assignee {
    /// An individual user, identified by an opaque name.
    User(String),
    /// A group, identified by an opaque name.
    Group(String),
}

impl Assignee {
    /// Build a [`Assignee::User`] from any string-like value.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::Assignee;
    ///
    /// assert_eq!(Assignee::user("alice").to_string(), "user:alice");
    /// ```
    pub fn user(name: impl Into<String>) -> Self {
        Assignee::User(name.into())
    }

    /// Build a [`Assignee::Group`] from any string-like value.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::Assignee;
    ///
    /// assert_eq!(Assignee::group("sre").to_string(), "group:sre");
    /// ```
    pub fn group(name: impl Into<String>) -> Self {
        Assignee::Group(name.into())
    }

    /// The bare name, without the `user:`/`group:` prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::Assignee;
    ///
    /// assert_eq!(Assignee::group("sre").name(), "sre");
    /// ```
    pub fn name(&self) -> &str {
        match self {
            Assignee::User(name) | Assignee::Group(name) => name,
        }
    }

    /// Whether this assignee is an individual user.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::Assignee;
    ///
    /// assert!(Assignee::user("alice").is_user());
    /// assert!(!Assignee::group("sre").is_user());
    /// ```
    pub fn is_user(&self) -> bool {
        matches!(self, Assignee::User(_))
    }

    /// Whether this assignee is a group.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::Assignee;
    ///
    /// assert!(Assignee::group("sre").is_group());
    /// assert!(!Assignee::user("alice").is_group());
    /// ```
    pub fn is_group(&self) -> bool {
        matches!(self, Assignee::Group(_))
    }
}

impl fmt::Display for Assignee {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Assignee::User(name) => write!(f, "user:{name}"),
            Assignee::Group(name) => write!(f, "group:{name}"),
        }
    }
}

/// Error returned when a string cannot be parsed into an [`Assignee`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssigneeParseError {
    /// The string carried no `user:`/`group:` prefix.
    #[error("assignee must be prefixed with `user:` or `group:`, got `{0}`")]
    MissingPrefix(String),
    /// The name after the prefix was empty.
    #[error("assignee name must not be empty")]
    EmptyName,
}

impl FromStr for Assignee {
    type Err = AssigneeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let assignee = if let Some(name) = s.strip_prefix("user:") {
            Assignee::User(name.to_string())
        } else if let Some(name) = s.strip_prefix("group:") {
            Assignee::Group(name.to_string())
        } else {
            return Err(AssigneeParseError::MissingPrefix(s.to_string()));
        };

        if assignee.name().is_empty() {
            return Err(AssigneeParseError::EmptyName);
        }

        Ok(assignee)
    }
}

impl From<Assignee> for String {
    fn from(assignee: Assignee) -> Self {
        assignee.to_string()
    }
}

impl TryFrom<String> for Assignee {
    type Error = AssigneeParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_and_group_render_with_prefix() {
        assert_eq!(Assignee::user("alice").to_string(), "user:alice");
        assert_eq!(Assignee::group("sre").to_string(), "group:sre");
    }

    #[test]
    fn round_trips_through_string() {
        for value in [Assignee::user("alice"), Assignee::group("sre-oncall")] {
            let text = value.to_string();
            let parsed: Assignee = text.parse().expect("parse");
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn round_trips_through_json() {
        let value = Assignee::group("sre-oncall");
        let json = serde_json::to_string(&value).expect("serialize");
        assert_eq!(json, "\"group:sre-oncall\"");
        let back: Assignee = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, value);
    }

    #[test]
    fn missing_prefix_is_rejected() {
        assert_eq!(
            "sre-oncall".parse::<Assignee>(),
            Err(AssigneeParseError::MissingPrefix("sre-oncall".to_string()))
        );
    }

    #[test]
    fn empty_name_is_rejected() {
        assert_eq!(
            "user:".parse::<Assignee>(),
            Err(AssigneeParseError::EmptyName)
        );
        assert_eq!(
            "group:".parse::<Assignee>(),
            Err(AssigneeParseError::EmptyName)
        );
    }

    #[test]
    fn deserialize_rejects_unprefixed() {
        assert!(serde_json::from_str::<Assignee>("\"sre-oncall\"").is_err());
    }

    #[test]
    fn name_strips_the_prefix() {
        assert_eq!(Assignee::user("alice").name(), "alice");
        assert_eq!(Assignee::group("sre").name(), "sre");
    }
}
