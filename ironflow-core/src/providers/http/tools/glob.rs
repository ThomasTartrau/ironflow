//! Tool for finding files by name pattern, confined to allowed directories.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Instant;

use globset::{GlobBuilder, GlobMatcher};
use ignore::WalkBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::task::spawn_blocking;

use super::confinement::AllowedRoots;
use super::input::{input_schema, parse_input};
use super::tool_trait::{Tool, ToolError, ToolOutput};
use super::walk::{MAX_RESULTS, SEARCH_TIMEOUT, confined_walker, walk_files};

/// Arguments of the `glob` tool. Field docs are the descriptions the model reads.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GlobInput {
    /// Glob relative to `path`, e.g. `**/*.rs` or `src/**/mod.rs`
    pattern: String,
    /// Absolute path of the directory to search. Defaults to the first searchable directory
    path: Option<String>,
}

/// Finds files whose path matches a glob pattern.
///
/// Returns the absolute paths of the matching files, sorted, one per line.
/// The pattern is matched against the path relative to the searched
/// directory: `*.rs` only matches at its top level, `**/*.rs` at any depth.
/// Runs in-process (crates `ignore` and `globset`), nothing is spawned.
///
/// # Security
///
/// The walk never leaves the directories given to
/// [`with_allowed_paths`](Self::with_allowed_paths): the requested path is
/// resolved to a canonical path (`..` and symbolic links resolved) and must
/// lie under a root, a relative path is refused, and a symbolic link met
/// during the walk whose target lies outside every root is skipped.
///
/// # Limits
///
/// `.git` directories and paths ignored by `.gitignore` are skipped. At most
/// 200 paths, 64 KiB of output and 10 seconds per call; a result cut short by
/// one of these bounds ends with an explicit `[truncated: ...]` line.
pub struct GlobTool {
    roots: AllowedRoots,
    description: String,
}

impl GlobTool {
    /// Create a `GlobTool` that can only list files under `roots`.
    ///
    /// The roots are resolved to canonical paths once, here. When the model
    /// gives no `path`, the first root is searched.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] if `roots` is empty, or if a root does not exist
    /// or is not a directory.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::PathBuf;
    ///
    /// use ironflow_core::providers::http::tools::glob::GlobTool;
    /// use ironflow_core::providers::http::tools::{ToolError, ToolRegistry};
    ///
    /// # fn example() -> Result<(), ToolError> {
    /// let registry = ToolRegistry::new()
    ///     .register(GlobTool::with_allowed_paths(vec![PathBuf::from("/srv/repo")])?);
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_allowed_paths(roots: Vec<PathBuf>) -> Result<Self, ToolError> {
        let roots = AllowedRoots::new(roots)?;
        let description = format!(
            "Find files by glob pattern, matched against the path relative to the searched directory \
             (`*.rs` = top level only, `**/*.rs` = any depth). Returns sorted absolute paths, at most {MAX_RESULTS}. \
             Skips .git and paths ignored by .gitignore. \
             Searchable directories: {}.",
            roots.describe()
        );
        Ok(Self { roots, description })
    }
}

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        input_schema::<GlobInput>()
    }

    fn read_only(&self) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let GlobInput { pattern, path } = match parse_input(input) {
                Ok(input) => input,
                Err(out) => return Ok(out),
            };

            if Path::new(&pattern).is_absolute() {
                return Ok(ToolOutput::error(format!(
                    "Glob '{pattern}' must be relative to 'path', pass the directory as 'path'"
                )));
            }
            let matcher = match GlobBuilder::new(&pattern).literal_separator(true).build() {
                Ok(glob) => glob.compile_matcher(),
                Err(e) => return Ok(ToolOutput::error(format!("Invalid glob '{pattern}': {e}"))),
            };

            let start = match self.roots.resolve_or_first(path.as_deref()) {
                Ok(start) => start,
                Err(message) => return Ok(ToolOutput::error(message)),
            };
            if !start.is_dir() {
                return Ok(ToolOutput::error(format!(
                    "'{}' is not a directory",
                    start.display()
                )));
            }

            let walker = confined_walker(&self.roots, &start);
            let deadline = Instant::now() + SEARCH_TIMEOUT;
            let text = spawn_blocking(move || find(walker, &start, &matcher, deadline))
                .await
                .map_err(|e| ToolError::new(format!("glob search task failed: {e}")))?;
            Ok(ToolOutput::success(text))
        })
    }
}

/// Collect the files under `start` whose relative path matches `matcher`,
/// until a bound or `deadline` is reached. The walk yields entries in
/// file-name order, depth first, so the paths come out sorted.
fn find(walker: WalkBuilder, start: &Path, matcher: &GlobMatcher, deadline: Instant) -> String {
    walk_files(walker, deadline, "No files found", |path, out| {
        let Ok(relative) = path.strip_prefix(start) else {
            return true;
        };
        !matcher.is_match(relative) || out.push(&path.display().to_string())
    })
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir, create_dir_all, write};

    use serde_json::json;
    use tempfile::{TempDir, tempdir};

    use super::*;

    /// A temporary root, canonicalized so expected paths match the output.
    fn root() -> (TempDir, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().canonicalize().expect("canonical");
        (dir, path)
    }

    fn tool(root: &Path) -> GlobTool {
        GlobTool::with_allowed_paths(vec![root.to_path_buf()]).expect("tool")
    }

    async fn run(tool: &GlobTool, input: Value) -> ToolOutput {
        tool.execute(input).await.expect("tool should not abort")
    }

    fn lines(paths: &[PathBuf]) -> String {
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn glob_returns_sorted_absolute_paths() {
        let (_dir, root) = root();
        create_dir(root.join("src")).expect("mkdir");
        write(root.join("b.rs"), "").expect("write");
        write(root.join("a.rs"), "").expect("write");
        write(root.join("src/c.rs"), "").expect("write");
        write(root.join("README.md"), "").expect("write");
        let tool = tool(&root);

        let top = run(&tool, json!({"pattern": "*.rs"})).await;
        assert!(!top.is_error, "{}", top.content);
        assert_eq!(top.content, lines(&[root.join("a.rs"), root.join("b.rs")]));

        let deep = run(&tool, json!({"pattern": "**/*.rs"})).await;
        assert_eq!(
            deep.content,
            lines(&[root.join("a.rs"), root.join("b.rs"), root.join("src/c.rs")])
        );
    }

    #[tokio::test]
    async fn glob_without_match_says_so() {
        let (_dir, root) = root();
        write(root.join("a.txt"), "").expect("write");
        let out = run(&tool(&root), json!({"pattern": "*.rs"})).await;
        assert!(!out.is_error);
        assert_eq!(out.content, "No files found");
    }

    #[tokio::test]
    async fn glob_is_relative_to_the_given_path() {
        let (_dir, root) = root();
        create_dir_all(root.join("crates/core")).expect("mkdir");
        write(root.join("crates/core/lib.rs"), "").expect("write");
        write(root.join("top.rs"), "").expect("write");

        let out = run(
            &tool(&root),
            json!({"pattern": "core/*.rs", "path": root.join("crates").to_str().expect("utf8")}),
        )
        .await;
        assert_eq!(out.content, lines(&[root.join("crates/core/lib.rs")]));
    }

    #[tokio::test]
    async fn glob_refuses_parent_traversal_out_of_the_root() {
        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("root");
        create_dir(&root).expect("mkdir");
        write(outer.path().join("secret.rs"), "").expect("write");

        let out = run(
            &tool(&root),
            json!({"pattern": "*.rs", "path": root.join("..").to_str().expect("utf8")}),
        )
        .await;
        assert!(out.is_error);
        assert!(
            out.content.contains("outside allowed directories"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("secret.rs"), "{}", out.content);
    }

    #[tokio::test]
    async fn glob_refuses_a_parent_traversal_in_the_pattern() {
        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("root");
        create_dir(&root).expect("mkdir");
        write(outer.path().join("secret.rs"), "").expect("write");

        let out = run(&tool(&root), json!({"pattern": "../*.rs"})).await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "No files found");
    }

    #[tokio::test]
    async fn glob_refuses_a_relative_path() {
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({"pattern": "*.rs", "path": "src"})).await;
        assert!(out.is_error);
        assert!(out.content.contains("relative"), "{}", out.content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn glob_skips_symlinks_escaping_the_root() {
        use std::os::unix::fs::symlink;

        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("root");
        let secret = outer.path().join("secret");
        create_dir(&root).expect("mkdir");
        create_dir(&secret).expect("mkdir");
        write(secret.join("key.rs"), "").expect("write");
        symlink(&secret, root.join("escape_dir")).expect("symlink");
        symlink(secret.join("key.rs"), root.join("escape_file.rs")).expect("symlink");
        write(root.join("own.rs"), "").expect("write");

        let out = run(&tool(&root), json!({"pattern": "**/*.rs"})).await;
        assert!(out.content.contains("own.rs"), "{}", out.content);
        assert!(!out.content.contains("escape"), "{}", out.content);
        assert!(!out.content.contains("key.rs"), "{}", out.content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn glob_follows_symlinks_inside_the_root() {
        use std::os::unix::fs::symlink;

        let (_dir, root) = root();
        create_dir(root.join("real")).expect("mkdir");
        write(root.join("real/a.rs"), "").expect("write");
        symlink(root.join("real"), root.join("alias")).expect("symlink");

        let out = run(&tool(&root), json!({"pattern": "**/*.rs"})).await;
        assert_eq!(
            out.content,
            lines(&[root.join("alias/a.rs"), root.join("real/a.rs")])
        );
    }

    #[tokio::test]
    async fn glob_respects_gitignore_and_skips_git() {
        let (_dir, root) = root();
        write(root.join(".gitignore"), "target/\n").expect("write");
        create_dir(root.join("target")).expect("mkdir");
        write(root.join("target/gen.rs"), "").expect("write");
        create_dir(root.join(".git")).expect("mkdir");
        write(root.join(".git/hook.rs"), "").expect("write");
        create_dir(root.join(".github")).expect("mkdir");
        write(root.join(".github/ci.rs"), "").expect("write");
        write(root.join("kept.rs"), "").expect("write");

        let out = run(&tool(&root), json!({"pattern": "**/*.rs"})).await;
        assert_eq!(
            out.content,
            lines(&[root.join(".github/ci.rs"), root.join("kept.rs")])
        );
    }

    #[tokio::test]
    async fn glob_lists_binary_files_by_name() {
        let (_dir, root) = root();
        write(root.join("logo.png"), b"\x89PNG\0\0").expect("write");
        let out = run(&tool(&root), json!({"pattern": "*.png"})).await;
        assert_eq!(out.content, lines(&[root.join("logo.png")]));
    }

    #[tokio::test]
    async fn glob_stops_at_200_paths_and_says_so() {
        let (_dir, root) = root();
        for i in 0..250 {
            write(root.join(format!("f{i:03}.txt")), "").expect("write");
        }

        let out = run(&tool(&root), json!({"pattern": "*.txt"})).await;
        let paths: Vec<&str> = out
            .content
            .lines()
            .filter(|l| l.ends_with(".txt"))
            .collect();
        assert_eq!(paths.len(), 200);
        assert!(paths.windows(2).all(|w| w[0] < w[1]), "not sorted");
        assert!(
            out.content
                .lines()
                .last()
                .is_some_and(|l| l.starts_with("[truncated: stopped at 200 results")),
            "{}",
            out.content
        );
    }

    #[test]
    fn glob_stops_when_the_deadline_has_passed() {
        let (_dir, root) = root();
        write(root.join("a.rs"), "").expect("write");
        let tool = tool(&root);
        let matcher = GlobBuilder::new("*.rs")
            .literal_separator(true)
            .build()
            .expect("glob")
            .compile_matcher();

        let text = find(
            confined_walker(&tool.roots, &root),
            &root,
            &matcher,
            Instant::now(),
        );
        assert!(
            text.starts_with("[truncated: search stopped after 10 s"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn glob_reports_an_invalid_pattern_to_the_model() {
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({"pattern": "[z-a]"})).await;
        assert!(out.is_error);
        assert!(
            out.content.starts_with("Invalid glob '[z-a]'"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn glob_reports_an_absolute_pattern_to_the_model() {
        let (_dir, root) = root();
        let pattern = format!("{}/*.rs", root.display());
        let out = run(&tool(&root), json!({"pattern": pattern})).await;
        assert!(out.is_error);
        assert!(
            out.content.contains("relative to 'path'"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn glob_reports_a_file_path_to_the_model() {
        let (_dir, root) = root();
        let file = root.join("a.rs");
        write(&file, "").expect("write");
        let out = run(
            &tool(&root),
            json!({"pattern": "*.rs", "path": file.to_str().expect("utf8")}),
        )
        .await;
        assert!(out.is_error);
        assert!(out.content.contains("not a directory"), "{}", out.content);
    }

    #[tokio::test]
    async fn glob_reports_a_missing_pattern_to_the_model() {
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({})).await;
        assert!(out.is_error);
        assert_eq!(out.content, "Invalid arguments: missing field `pattern`");
    }

    #[tokio::test]
    async fn glob_reports_an_unknown_parameter_to_the_model() {
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({"pattern": "*.rs", "recursive": true})).await;
        assert!(out.is_error);
        assert!(
            out.content.contains("unknown field `recursive`"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn glob_treats_a_null_path_as_absent() {
        let (_dir, root) = root();
        write(root.join("a.rs"), "").expect("write");
        let out = run(&tool(&root), json!({"pattern": "*.rs", "path": null})).await;
        assert_eq!(out.content, lines(&[root.join("a.rs")]));
    }

    #[test]
    fn glob_schema_is_what_the_model_reads() {
        let (_dir, root) = root();
        assert_eq!(
            tool(&root).parameters_schema(),
            json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob relative to `path`, e.g. `**/*.rs` or `src/**/mod.rs`"
                    },
                    "path": {
                        "type": "string",
                        "description": "Absolute path of the directory to search. Defaults to the first searchable directory"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            })
        );
    }

    #[test]
    fn glob_construction_fails_on_a_missing_root() {
        let (_dir, root) = root();
        assert!(GlobTool::with_allowed_paths(vec![root.join("missing")]).is_err());
        assert!(GlobTool::with_allowed_paths(Vec::new()).is_err());
    }

    #[test]
    fn glob_description_lists_the_roots() {
        let (_dir, root) = root();
        assert!(tool(&root).description().contains(&*root.to_string_lossy()));
    }

    #[test]
    fn glob_tool_is_read_only() {
        let (_dir, root) = root();
        assert!(tool(&root).read_only());
    }
}
