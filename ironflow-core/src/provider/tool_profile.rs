//! [`ToolProfile`] -- the name of a set of tools a provider exposes to a step.

use std::borrow::Cow;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

/// The name of a tool profile, shared by the provider that registers it and
/// the steps that select it.
///
/// Declare each profile once as a constant and use that constant on both
/// sides: [`HttpAgentProvider::with_tool_profile`](crate::providers::http::HttpAgentProvider::with_tool_profile)
/// and [`AgentConfig::tool_profile`](super::AgentConfig::tool_profile). A
/// misspelled constant does not compile, where a misspelled string would only
/// fail the step at run time.
///
/// It serializes as its plain name.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::{AgentConfig, ToolProfile};
///
/// const BUG: ToolProfile = ToolProfile::new("bug");
///
/// let config = AgentConfig::new("Find the root cause").tool_profile(BUG);
/// assert_eq!(config.tool_profile, Some(BUG));
/// assert_eq!(BUG.to_string(), "bug");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ToolProfile(Cow<'static, str>);

impl ToolProfile {
    /// Name a tool profile.
    ///
    /// # Panics
    ///
    /// Panics if `name` is empty. In a `const`, that is a compile error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::ToolProfile;
    ///
    /// const SUGGESTION: ToolProfile = ToolProfile::new("suggestion");
    /// assert_eq!(SUGGESTION.as_str(), "suggestion");
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_core::provider::ToolProfile;
    ///
    /// let _ = ToolProfile::new("");
    /// ```
    pub const fn new(name: &'static str) -> Self {
        assert!(!name.is_empty(), "tool profile name must not be empty");
        Self(Cow::Borrowed(name))
    }

    /// The profile name.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::ToolProfile;
    ///
    /// assert_eq!(ToolProfile::new("bug").as_str(), "bug");
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ToolProfile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        if name.is_empty() {
            return Err(D::Error::custom("tool profile name must not be empty"));
        }
        Ok(Self(Cow::Owned(name)))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;

    const BUG: ToolProfile = ToolProfile::new("bug");

    #[test]
    fn serializes_as_its_name() {
        assert_eq!(to_value(BUG).unwrap(), json!("bug"));
    }

    #[test]
    fn deserialized_profile_equals_the_constant() {
        let back: ToolProfile = from_value(json!("bug")).unwrap();
        assert_eq!(back, BUG);
    }

    #[test]
    fn empty_name_is_rejected_on_deserialize() {
        let err = from_value::<ToolProfile>(json!("")).unwrap_err();
        assert_eq!(err.to_string(), "tool profile name must not be empty");
    }

    #[test]
    fn unicode_name_is_kept_verbatim() {
        let profile = ToolProfile::new("débogage");
        assert_eq!(profile.as_str(), "débogage");
        assert_eq!(to_value(&profile).unwrap(), json!("débogage"));
    }
}
