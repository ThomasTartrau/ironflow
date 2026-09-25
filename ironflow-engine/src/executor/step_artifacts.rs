//! [`StepArtifacts`] -- the artifacts a step can hand out as handles.

use glob::{MatchOptions, Pattern};
use uuid::Uuid;

use crate::config::{ArtifactOutput, ArtifactRef};
use crate::error::EngineError;

use super::StepOutput;

/// Which artifacts a step can hand out as [`ArtifactRef`] handles.
///
/// The workflow context fills it once the step has run: the step name, its
/// record when one exists (not while planning), and the file-name part of the
/// outputs it declared. A [`StepOutput`] built anywhere else carries the empty
/// default and hands out nothing.
///
/// # Examples
///
/// ```
/// use ironflow_engine::executor::StepArtifacts;
///
/// let artifacts = StepArtifacts::default();
/// assert_eq!(artifacts.step_name(), "");
/// assert!(artifacts.declared().is_empty());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepArtifacts {
    step_name: String,
    step_id: Option<Uuid>,
    declared: Vec<String>,
}

impl StepArtifacts {
    /// Artifacts of the step `step_name`, recorded as `step_id`, which
    /// declared `outputs`.
    pub(crate) fn new(step_name: &str, step_id: Option<Uuid>, outputs: &[ArtifactOutput]) -> Self {
        Self {
            step_name: step_name.to_string(),
            step_id,
            declared: outputs
                .iter()
                .map(|output| file_name_part(&output.pattern).to_string())
                .collect(),
        }
    }

    /// Name of the step, empty when the output does not come from a step.
    pub fn step_name(&self) -> &str {
        &self.step_name
    }

    /// Record of the step, `None` while planning.
    pub fn step_id(&self) -> Option<Uuid> {
        self.step_id
    }

    /// File-name patterns of the outputs the step declared.
    pub fn declared(&self) -> &[String] {
        &self.declared
    }
}

impl StepOutput {
    /// Handle on an artifact this step declared, to feed a later step.
    ///
    /// `name` is the file name the artifact is stored under, without its
    /// directory. It must match the file-name part of one of the step's
    /// [`output`](crate::config::ShellConfig::output) patterns, so a typo fails
    /// here rather than in the step that consumes it.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ArtifactNotDeclared`] when no declared output
    /// covers `name`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let build = ctx
    ///     .shell("build", ShellConfig::new("cargo build").output("target/*.log"))
    ///     .await?;
    /// let log = build.artifact("build.log")?;
    /// assert!(build.artifact("build.txt").is_err());
    ///
    /// ctx.shell("archive", ShellConfig::new("gzip build.log").input(&log)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn artifact(&self, name: &str) -> Result<ArtifactRef, EngineError> {
        let options = MatchOptions {
            require_literal_separator: true,
            ..MatchOptions::new()
        };
        let declared = self.artifacts.declared.iter().any(|pattern| {
            Pattern::new(pattern).is_ok_and(|pattern| pattern.matches_with(name, options))
        });
        if !declared {
            return Err(EngineError::ArtifactNotDeclared {
                step: self.artifacts.step_name.clone(),
                name: name.to_string(),
            });
        }
        Ok(ArtifactRef::new(&self.artifacts.step_name, name))
    }
}

/// The part of an output pattern that names the file: artifacts are stored
/// under the file name alone.
fn file_name_part(pattern: &str) -> &str {
    pattern
        .rsplit_once('/')
        .map_or(pattern, |(_, file_name)| file_name)
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use serde_json::Value;

    use super::*;

    fn output_declaring(patterns: &[&str]) -> StepOutput {
        let outputs: Vec<ArtifactOutput> =
            patterns.iter().map(|p| ArtifactOutput::new(p)).collect();
        StepOutput {
            output: Value::Null,
            duration_ms: 0,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
            artifacts: StepArtifacts::new("build", None, &outputs),
        }
    }

    #[test]
    fn a_literal_declaration_hands_out_its_file_name() {
        let output = output_declaring(&["target/report.html"]);
        let handle = output.artifact("report.html").expect("declared");
        assert_eq!(handle, ArtifactRef::new("build", "report.html"));
    }

    #[test]
    fn a_glob_declaration_hands_out_every_matching_name() {
        let output = output_declaring(&["logs/*.log", "report.html"]);
        assert!(output.artifact("build.log").is_ok());
        assert!(output.artifact("report.html").is_ok());
        assert!(output.artifact("build.txt").is_err());
    }

    #[test]
    fn a_name_with_a_directory_never_matches() {
        let output = output_declaring(&["*.log"]);
        assert!(matches!(
            output.artifact("nested/build.log"),
            Err(EngineError::ArtifactNotDeclared { .. })
        ));
    }

    #[test]
    fn an_undeclared_name_reports_the_step() {
        let err = output_declaring(&["report.html"])
            .artifact("report.htm")
            .expect_err("typo");
        assert!(matches!(
            err,
            EngineError::ArtifactNotDeclared { ref step, ref name }
                if step == "build" && name == "report.htm"
        ));
    }

    #[test]
    fn an_output_built_outside_a_step_hands_out_nothing() {
        let mut output = output_declaring(&[]);
        output.artifacts = StepArtifacts::default();
        assert!(output.artifact("anything").is_err());
    }

    #[test]
    fn unicode_file_names_match() {
        let output = output_declaring(&["out/*.html"]);
        assert!(output.artifact("rapport-été.html").is_ok());
    }

    #[test]
    fn file_name_part_drops_the_directories() {
        assert_eq!(file_name_part("target/release/app"), "app");
        assert_eq!(file_name_part("report.html"), "report.html");
        assert_eq!(file_name_part("**/*.log"), "*.log");
    }
}
