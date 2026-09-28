//! Named tool profiles: which [`ToolRegistry`] a step is allowed to use.
//!
//! A provider holds an optional default registry (set with
//! [`HttpAgentProvider::with_tools`](crate::providers::http::HttpAgentProvider::with_tools))
//! and any number of named ones (set with
//! [`HttpAgentProvider::with_tool_profile`](crate::providers::http::HttpAgentProvider::with_tool_profile)).
//! Each invocation selects exactly one of them, or none.

use std::collections::BTreeMap;

use crate::error::AgentError;
use crate::provider::ToolProfile;

use super::ToolRegistry;

/// The tool registries a provider can expose, keyed by profile name.
#[derive(Default)]
pub(crate) struct ToolProfiles {
    default: Option<ToolRegistry>,
    named: BTreeMap<ToolProfile, ToolRegistry>,
}

/// The tools selected for one invocation.
pub(crate) enum ToolSelection<'a> {
    /// A named profile the step asked for.
    Named(&'a ToolProfile, &'a ToolRegistry),
    /// The default registry, for a step that asked for no profile.
    Default(&'a ToolRegistry),
    /// No tools at all.
    Empty,
}

impl ToolProfiles {
    /// Replace the default registry.
    pub(crate) fn set_default(&mut self, registry: ToolRegistry) {
        self.default = Some(registry);
    }

    /// Register a named profile.
    ///
    /// # Panics
    ///
    /// Panics if `profile` is already registered.
    pub(crate) fn insert(&mut self, profile: ToolProfile, registry: ToolRegistry) {
        assert!(
            !self.named.contains_key(&profile),
            "tool profile '{profile}' already registered"
        );
        self.named.insert(profile, registry);
    }

    /// Returns `true` when the provider has neither default tools nor profiles.
    pub(crate) fn is_empty(&self) -> bool {
        self.default.is_none() && self.named.is_empty()
    }

    /// Select the registry for a step asking for `profile`.
    ///
    /// Without a profile, the default registry is used if there is one. A
    /// named profile is never replaced by another one or by the default.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::UnknownToolProfile`] if `profile` is not registered.
    pub(crate) fn select(
        &self,
        profile: Option<&ToolProfile>,
    ) -> Result<ToolSelection<'_>, AgentError> {
        match profile {
            Some(name) => self
                .named
                .get_key_value(name)
                .map(|(key, registry)| ToolSelection::Named(key, registry))
                .ok_or_else(|| AgentError::UnknownToolProfile {
                    profile: name.to_string(),
                    available: self.named.keys().map(ToolProfile::to_string).collect(),
                }),
            None => Ok(self
                .default
                .as_ref()
                .map_or(ToolSelection::Empty, ToolSelection::Default)),
        }
    }
}

impl<'a> ToolSelection<'a> {
    /// The selected registry, if any.
    pub(crate) fn registry(&self) -> Option<&'a ToolRegistry> {
        match self {
            ToolSelection::Named(_, registry) | ToolSelection::Default(registry) => Some(registry),
            ToolSelection::Empty => None,
        }
    }

    /// The profile name as shown in logs.
    pub(crate) fn label(&self) -> String {
        match self {
            ToolSelection::Named(name, _) => format!("tool profile '{name}'"),
            ToolSelection::Default(_) => "default tool profile".to_string(),
            ToolSelection::Empty => "no tool profile".to_string(),
        }
    }

    /// One line naming the profile and the tools it exposes, for the run trace.
    pub(crate) fn describe(&self) -> String {
        let names = self
            .registry()
            .map(ToolRegistry::tool_names)
            .unwrap_or_default();
        match names.len() {
            0 => format!("{}: no tools exposed", self.label()),
            1 => format!("{}: 1 tool exposed ({})", self.label(), names[0]),
            n => format!("{}: {n} tools exposed ({})", self.label(), names.join(", ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_without_profile_or_default_is_empty() {
        let profiles = ToolProfiles::default();
        assert!(profiles.is_empty());
        let selection = profiles.select(None).unwrap();
        assert!(selection.registry().is_none());
        assert_eq!(selection.describe(), "no tool profile: no tools exposed");
    }

    #[test]
    fn select_named_profile_with_no_tools() {
        let mut profiles = ToolProfiles::default();
        let empty = ToolProfile::new("empty");
        profiles.insert(empty.clone(), ToolRegistry::new());
        assert!(!profiles.is_empty());
        let selection = profiles.select(Some(&empty)).unwrap();
        assert_eq!(
            selection.describe(),
            "tool profile 'empty': no tools exposed"
        );
    }

    #[test]
    fn unknown_profile_lists_sorted_names() {
        let mut profiles = ToolProfiles::default();
        profiles.insert(ToolProfile::new("zeta"), ToolRegistry::new());
        profiles.insert(ToolProfile::new("alpha"), ToolRegistry::new());
        let err = profiles
            .select(Some(&ToolProfile::new("beta")))
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "unknown tool profile 'beta' (registered profiles: alpha, zeta)"
        );
    }
}
