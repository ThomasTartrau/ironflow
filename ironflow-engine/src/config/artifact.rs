//! Artifact declarations carried by a step config.
//!
//! A step declares the files it *produces* ([`ArtifactOutput`]) and the ones it
//! *consumes* ([`ArtifactInput`]). Both are serialized with the step input, so
//! they survive a round-trip through the store and are visible on the dashboard.

use serde::{Deserialize, Serialize};

/// A file the step promises to produce.
///
/// `pattern` is a glob resolved against the step's working directory. When the
/// step succeeds and the pattern matches nothing, the step fails with
/// [`MissingArtifact`](crate::error::EngineError::MissingArtifact): a declared
/// output that never appeared is a broken contract.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::ArtifactOutput;
///
/// let output = ArtifactOutput::new("target/report.html");
/// assert_eq!(output.pattern, "target/report.html");
/// assert!(output.content_type.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactOutput {
    /// Glob pattern, relative to the step's working directory.
    pub pattern: String,
    /// MIME type to record. Guessed from the file name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

impl ArtifactOutput {
    /// Declare an output whose MIME type is guessed from the file name.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ArtifactOutput;
    ///
    /// let output = ArtifactOutput::new("dist/*.js");
    /// assert_eq!(output.pattern, "dist/*.js");
    /// ```
    pub fn new(pattern: &str) -> Self {
        Self {
            pattern: pattern.to_string(),
            content_type: None,
        }
    }

    /// Declare an output with an explicit MIME type.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ArtifactOutput;
    ///
    /// let output = ArtifactOutput::typed("data", "application/json");
    /// assert_eq!(output.content_type.as_deref(), Some("application/json"));
    /// ```
    pub fn typed(pattern: &str, content_type: &str) -> Self {
        Self {
            pattern: pattern.to_string(),
            content_type: Some(content_type.to_string()),
        }
    }
}

/// An artifact the step wants placed in its working directory before it runs.
///
/// Resolved within the current run and attempt, among steps positioned strictly
/// before the consumer. When several steps share `step`, the one closest to the
/// consumer wins. No match fails the step with
/// [`ArtifactNotFound`](crate::error::EngineError::ArtifactNotFound).
///
/// A sub-workflow never sees its parent's artifacts: pass what it needs through
/// the payload instead.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::ArtifactInput;
///
/// let input = ArtifactInput::new("build", "report.html");
/// assert_eq!(input.destination(), "report.html");
///
/// let renamed = ArtifactInput::new("build", "report.html").at("inputs/report.html");
/// assert_eq!(renamed.destination(), "inputs/report.html");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactInput {
    /// Name of the step that produced the artifact.
    pub step: String,
    /// Name of the artifact.
    pub name: String,
    /// Where to write it, relative to the working directory.
    ///
    /// Defaults to [`name`](ArtifactInput::name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest: Option<String>,
}

impl ArtifactInput {
    /// Consume `name` as produced by the step called `step`.
    pub fn new(step: &str, name: &str) -> Self {
        Self {
            step: step.to_string(),
            name: name.to_string(),
            dest: None,
        }
    }

    /// Write the artifact to `dest` instead of its own name.
    pub fn at(mut self, dest: &str) -> Self {
        self.dest = Some(dest.to_string());
        self
    }

    /// Path the artifact is written to, relative to the working directory.
    pub fn destination(&self) -> &str {
        self.dest.as_deref().unwrap_or(&self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_defaults_to_a_guessed_type() {
        assert!(ArtifactOutput::new("a.html").content_type.is_none());
    }

    #[test]
    fn output_keeps_an_explicit_type() {
        let output = ArtifactOutput::typed("a", "text/csv");
        assert_eq!(output.content_type.as_deref(), Some("text/csv"));
    }

    #[test]
    fn output_serde_omits_an_absent_content_type() {
        let json = serde_json::to_string(&ArtifactOutput::new("a.html")).expect("serialize");
        assert!(!json.contains("content_type"));
    }

    #[test]
    fn output_serde_roundtrips() {
        let output = ArtifactOutput::typed("a", "text/csv");
        let json = serde_json::to_string(&output).expect("serialize");
        let parsed: ArtifactOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, output);
    }

    #[test]
    fn input_destination_defaults_to_the_artifact_name() {
        assert_eq!(ArtifactInput::new("build", "a.txt").destination(), "a.txt");
    }

    #[test]
    fn input_destination_honours_an_override() {
        assert_eq!(
            ArtifactInput::new("build", "a.txt")
                .at("in/a.txt")
                .destination(),
            "in/a.txt"
        );
    }

    #[test]
    fn input_serde_roundtrips() {
        let input = ArtifactInput::new("build", "a.txt").at("in/a.txt");
        let json = serde_json::to_string(&input).expect("serialize");
        let parsed: ArtifactInput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, input);
    }

    #[test]
    fn input_deserializes_a_payload_without_dest() {
        let parsed: ArtifactInput =
            serde_json::from_str(r#"{"step":"build","name":"a.txt"}"#).expect("deserialize");
        assert_eq!(parsed.destination(), "a.txt");
    }
}
