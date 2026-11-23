//! Claude Code sessions of a step kept on a PersistentVolumeClaim the
//! workflow already uses (K8s ephemeral provider only).
//!
//! [`SessionsVolume`] travels in [`PodSettings::sessions`](super::PodSettings::sessions)
//! and is set with [`AgentConfig::sessions_pvc`].

use serde::{Deserialize, Serialize};

use super::{AgentConfig, validate_pvc_sub_path};

/// Directory of a PersistentVolumeClaim holding the Claude Code sessions of
/// a step, one sub-directory per run.
///
/// The K8s ephemeral provider mounts `<dir>/<run id>` of the claim on
/// `~/.claude/projects`, so a later step of the same run finds the session
/// in another pod, and a pod never sees the sessions of another run.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::SessionsVolume;
///
/// let sessions = SessionsVolume {
///     claim_name: "workspace".to_string(),
///     dir: "claude-sessions".to_string(),
/// };
/// assert!(sessions.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsVolume {
    /// Name of the claim in the pod's namespace.
    pub claim_name: String,
    /// Directory of the claim holding one sub-directory per run.
    pub dir: String,
}

impl SessionsVolume {
    /// Check that the claim is named and that `dir` passes
    /// [`validate_pvc_sub_path`].
    ///
    /// # Errors
    ///
    /// Returns the reason as a message when the volume is refused.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::SessionsVolume;
    ///
    /// let sessions = SessionsVolume {
    ///     claim_name: "workspace".to_string(),
    ///     dir: "../x".to_string(),
    /// };
    /// assert!(sessions.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if self.claim_name.is_empty() {
            return Err("sessions volume needs a non-empty claim_name".to_string());
        }
        validate_pvc_sub_path(&self.dir)
    }

    /// `subPath` of the claim mounted for the run `run_id`: `<dir>/<run_id>`.
    ///
    /// # Errors
    ///
    /// Returns the reason as a message when the resulting path is refused
    /// by [`validate_pvc_sub_path`] (an empty run id, or one holding `/`
    /// segments such as `..`).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::SessionsVolume;
    ///
    /// let sessions = SessionsVolume {
    ///     claim_name: "workspace".to_string(),
    ///     dir: "claude-sessions".to_string(),
    /// };
    /// assert_eq!(sessions.run_sub_path("run-1")?, "claude-sessions/run-1");
    /// assert!(sessions.run_sub_path("..").is_err());
    /// # Ok::<(), String>(())
    /// ```
    pub fn run_sub_path(&self, run_id: &str) -> Result<String, String> {
        if run_id.contains('/') {
            return Err(format!("run id '{run_id}' must not contain '/'"));
        }
        let sub_path = format!("{}/{run_id}", self.dir);
        validate_pvc_sub_path(&sub_path)?;
        Ok(sub_path)
    }
}

impl<Tools, Schema> AgentConfig<Tools, Schema> {
    /// Keep the Claude Code sessions of the step in `dir` of the
    /// PersistentVolumeClaim `claim` (K8s ephemeral provider only, sandboxed).
    ///
    /// The provider mounts `<dir>/<run id>` of the claim on
    /// `~/.claude/projects`: a later step of the same run that resumes the
    /// session ([`AgentConfig::resume`]) finds it in another pod, and a pod
    /// never sees the sessions of another run. The claim can be one the
    /// workflow already mounts elsewhere, by sub-path only: a mount of its
    /// root, or of a parent of `dir`, would show every run's sessions and is
    /// refused. Overrides the provider's `sessions_volume` for this step.
    ///
    /// # Panics
    ///
    /// Panics when [`SessionsVolume::validate`] refuses the volume: an empty
    /// `claim`, or a `dir` refused by [`validate_pvc_sub_path`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("implement").sessions_pvc("workspace", "claude-sessions");
    /// let sessions = config.pod.sessions.as_ref().map(|s| s.dir.as_str());
    /// assert_eq!(sessions, Some("claude-sessions"));
    /// ```
    pub fn sessions_pvc(mut self, claim: &str, dir: &str) -> Self {
        let sessions = SessionsVolume {
            claim_name: claim.to_string(),
            dir: dir.to_string(),
        };
        if let Err(reason) = sessions.validate() {
            panic!("invalid sessions_pvc: {reason}");
        }
        self.pod.sessions = Some(sessions);
        self
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;
    use crate::provider::PodSettings;

    #[test]
    fn sessions_pvc_sets_the_step_sessions() {
        let config = AgentConfig::new("x").sessions_pvc("workspace", "sessions");
        assert_eq!(
            config.pod.sessions,
            Some(SessionsVolume {
                claim_name: "workspace".to_string(),
                dir: "sessions".to_string(),
            })
        );
        assert!(!config.pod.is_empty());
    }

    #[test]
    #[should_panic(expected = "sessions volume needs a non-empty claim_name")]
    fn sessions_pvc_empty_claim_panics() {
        let _ = AgentConfig::new("x").sessions_pvc("", "sessions");
    }

    #[test]
    #[should_panic(expected = "invalid sessions_pvc")]
    fn sessions_pvc_parent_dir_panics() {
        let _ = AgentConfig::new("x").sessions_pvc("workspace", "../sessions");
    }

    #[test]
    #[should_panic(expected = "invalid sessions_pvc")]
    fn sessions_pvc_absolute_dir_panics() {
        let _ = AgentConfig::new("x").sessions_pvc("workspace", "/sessions");
    }

    #[test]
    fn run_sub_path_appends_the_run_id() {
        let sessions = SessionsVolume {
            claim_name: "workspace".to_string(),
            dir: "a/b".to_string(),
        };
        assert_eq!(sessions.run_sub_path("r1").unwrap(), "a/b/r1");
    }

    #[test]
    fn run_sub_path_refuses_a_run_id_escaping_the_dir() {
        let sessions = SessionsVolume {
            claim_name: "workspace".to_string(),
            dir: "sessions".to_string(),
        };
        for run_id in ["", "..", ".", "../other", "x/y"] {
            assert!(sessions.run_sub_path(run_id).is_err(), "{run_id:?}");
        }
    }

    #[test]
    fn pod_settings_without_sessions_serialize_as_before() {
        let json = to_value(PodSettings::default()).unwrap();
        assert!(json.get("sessions").is_none());
        let back: PodSettings = from_value(json!({})).unwrap();
        assert_eq!(back.sessions, None);
    }

    #[test]
    fn pod_settings_sessions_round_trip() {
        let config = AgentConfig::new("x").sessions_pvc("workspace", "sessions");
        let json = to_value(&config.pod).unwrap();
        let back: PodSettings = from_value(json).unwrap();
        assert_eq!(back, config.pod);
    }
}
