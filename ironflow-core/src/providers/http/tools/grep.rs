//! Tool for searching file contents, confined to allowed directories.

use std::borrow::Cow;
use std::fs::File;
use std::future::Future;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Instant;

use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::task::spawn_blocking;

use super::confinement::AllowedRoots;
use super::input::{input_schema, parse_input};
use super::tool_trait::{Tool, ToolError, ToolOutput};
use super::walk::{
    BoundedOutput, MAX_RESULTS, SEARCH_TIMEOUT, Truncation, confined_walker, walk_files,
};

/// Longest line content returned for one match, in bytes.
const MAX_LINE_BYTES: usize = 512;

/// Leading bytes inspected to detect a binary file.
const BINARY_PROBE_BYTES: usize = 8 * 1024;

/// Arguments of the `grep` tool. Field docs are the descriptions the model reads.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GrepInput {
    /// Regular expression to search for, matched against each line
    pattern: String,
    /// Absolute path of the file or directory to search. Defaults to the first searchable directory
    path: Option<String>,
    /// Only search files matching this glob, gitignore-style and relative to `path`, e.g. `*.rs` or `src/**/*.ts`
    glob: Option<String>,
    /// Match case-insensitively. Defaults to false
    case_insensitive: Option<bool>,
}

/// Searches file contents with a regular expression, like a confined `rg`.
///
/// Returns one `path:line:content` entry per matching line, with an absolute
/// path that the `read_file` tool accepts as is.
/// Runs in-process (crates `ignore` and `regex`): nothing is spawned and `rg`
/// does not need to be installed on the worker.
///
/// # Security
///
/// The search never leaves the directories given to
/// [`with_allowed_paths`](Self::with_allowed_paths): the requested path is
/// resolved to a canonical path (`..` and symbolic links resolved) and must
/// lie under a root, a relative path is refused, and a symbolic link met
/// during the walk whose target lies outside every root is skipped.
///
/// # Limits
///
/// `.git` directories, binary files and paths ignored by `.gitignore` are
/// skipped. At most 200 matching lines, 64 KiB of output and 10 seconds per
/// call; a result cut short by one of these bounds ends with an explicit
/// `[truncated: ...]` line.
pub struct GrepTool {
    roots: AllowedRoots,
    description: String,
}

impl GrepTool {
    /// Create a `GrepTool` that can only search under `roots`.
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
    /// use ironflow_core::providers::http::tools::grep::GrepTool;
    /// use ironflow_core::providers::http::tools::{ToolError, ToolRegistry};
    ///
    /// # fn example() -> Result<(), ToolError> {
    /// let registry = ToolRegistry::new()
    ///     .register(GrepTool::with_allowed_paths(vec![PathBuf::from("/srv/repo")])?);
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_allowed_paths(roots: Vec<PathBuf>) -> Result<Self, ToolError> {
        let roots = AllowedRoots::new(roots)?;
        let description = format!(
            "Search file contents with a regular expression (Rust regex syntax), line by line. \
             Returns one `path:line:content` entry per matching line, at most {MAX_RESULTS} lines. \
             Skips .git, binary files and paths ignored by .gitignore. \
             Searchable directories: {}.",
            roots.describe()
        );
        Ok(Self { roots, description })
    }
}

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        input_schema::<GrepInput>()
    }

    fn read_only(&self) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let GrepInput {
                pattern,
                path,
                glob,
                case_insensitive,
            } = match parse_input(input) {
                Ok(input) => input,
                Err(out) => return Ok(out),
            };

            let regex = match RegexBuilder::new(&pattern)
                .case_insensitive(case_insensitive.unwrap_or(false))
                .build()
            {
                Ok(regex) => regex,
                Err(e) => {
                    return Ok(ToolOutput::error(format!("Invalid regex '{pattern}': {e}")));
                }
            };

            let start = match self.roots.resolve_or_first(path.as_deref()) {
                Ok(start) => start,
                Err(message) => return Ok(ToolOutput::error(message)),
            };

            let mut walker = confined_walker(&self.roots, &start);
            if let Some(glob) = glob {
                let base = if start.is_dir() {
                    start.as_path()
                } else {
                    start.parent().unwrap_or(&start)
                };
                let mut overrides = OverrideBuilder::new(base);
                let built = overrides.add(&glob).and_then(|builder| builder.build());
                match built {
                    Ok(overrides) => {
                        walker.overrides(overrides);
                    }
                    Err(e) => return Ok(ToolOutput::error(format!("Invalid glob '{glob}': {e}"))),
                }
            }

            let deadline = Instant::now() + SEARCH_TIMEOUT;
            let text = spawn_blocking(move || search(walker, &regex, deadline))
                .await
                .map_err(|e| ToolError::new(format!("grep search task failed: {e}")))?;
            Ok(ToolOutput::success(text))
        })
    }
}

/// Walk every file under the walker and collect matching lines until a
/// bound or `deadline` is reached.
fn search(walker: WalkBuilder, regex: &Regex, deadline: Instant) -> String {
    walk_files(walker, deadline, "No matches found", |path, out| {
        search_file(path, regex, deadline, out)
    })
}

/// Append the matching lines of one file to `out`. Returns `false` when the
/// whole search must stop.
fn search_file(path: &Path, regex: &Regex, deadline: Instant, out: &mut BoundedOutput) -> bool {
    let Ok(file) = File::open(path) else {
        return true;
    };
    let mut reader = BufReader::with_capacity(BINARY_PROBE_BYTES, file);
    match reader.fill_buf() {
        Ok(head) if !head.contains(&0) => {}
        _ => return true,
    }

    let mut line = Vec::new();
    let mut line_number = 0usize;
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return true,
            Ok(_) => {}
        }
        // A NUL byte past the probed head still marks the file as binary.
        if line.contains(&0) {
            return true;
        }
        if Instant::now() >= deadline {
            out.truncate(Truncation::Timeout);
            return false;
        }
        line_number += 1;
        let text = String::from_utf8_lossy(trim_line_ending(&line));
        if regex.is_match(&text)
            && !out.push(&format!("{}:{line_number}:{}", path.display(), clip(&text)))
        {
            return false;
        }
    }
}

fn trim_line_ending(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn clip(line: &str) -> Cow<'_, str> {
    if line.len() <= MAX_LINE_BYTES {
        Cow::Borrowed(line)
    } else {
        let end = line.floor_char_boundary(MAX_LINE_BYTES);
        Cow::Owned(format!("{} [line truncated]", &line[..end]))
    }
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

    fn tool(root: &Path) -> GrepTool {
        GrepTool::with_allowed_paths(vec![root.to_path_buf()]).expect("tool")
    }

    async fn run(tool: &GrepTool, input: Value) -> ToolOutput {
        tool.execute(input).await.expect("tool should not abort")
    }

    #[tokio::test]
    async fn grep_returns_path_line_and_content() {
        let (_dir, root) = root();
        let file = root.join("main.rs");
        write(&file, "fn main() {}\nlet needle = 1;\n").expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, format!("{}:2:let needle = 1;", file.display()));
    }

    #[tokio::test]
    async fn grep_without_match_says_so() {
        let (_dir, root) = root();
        write(root.join("a.txt"), "hay\n").expect("write");
        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert!(!out.is_error);
        assert_eq!(out.content, "No matches found");
    }

    #[tokio::test]
    async fn grep_is_case_sensitive_unless_asked() {
        let (_dir, root) = root();
        write(root.join("a.txt"), "NEEDLE\n").expect("write");
        let tool = tool(&root);

        let sensitive = run(&tool, json!({"pattern": "needle"})).await;
        assert_eq!(sensitive.content, "No matches found");

        let insensitive = run(
            &tool,
            json!({"pattern": "needle", "case_insensitive": true}),
        )
        .await;
        assert!(
            insensitive.content.ends_with(":1:NEEDLE"),
            "{}",
            insensitive.content
        );
    }

    #[tokio::test]
    async fn grep_glob_filters_files() {
        let (_dir, root) = root();
        create_dir(root.join("src")).expect("mkdir");
        write(root.join("src/lib.rs"), "needle\n").expect("write");
        write(root.join("notes.md"), "needle\n").expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle", "glob": "*.rs"})).await;
        assert!(out.content.contains("lib.rs:1:needle"), "{}", out.content);
        assert!(!out.content.contains("notes.md"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_searches_the_given_path_only() {
        let (_dir, root) = root();
        create_dir(root.join("a")).expect("mkdir");
        create_dir(root.join("b")).expect("mkdir");
        write(root.join("a/x.txt"), "needle\n").expect("write");
        write(root.join("b/y.txt"), "needle\n").expect("write");

        let out = run(
            &tool(&root),
            json!({"pattern": "needle", "path": root.join("b").to_str().expect("utf8")}),
        )
        .await;
        assert!(out.content.contains("y.txt"), "{}", out.content);
        assert!(!out.content.contains("x.txt"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_accepts_a_single_file_path() {
        let (_dir, root) = root();
        let file = root.join("one.txt");
        write(&file, "a\nneedle\n").expect("write");
        let out = run(
            &tool(&root),
            json!({"pattern": "needle", "path": file.to_str().expect("utf8")}),
        )
        .await;
        assert_eq!(out.content, format!("{}:2:needle", file.display()));
    }

    #[tokio::test]
    async fn grep_refuses_parent_traversal_out_of_the_root() {
        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("root");
        create_dir(&root).expect("mkdir");
        write(outer.path().join("secret.txt"), "needle secret\n").expect("write");

        let traversal = root.join("..");
        let out = run(
            &tool(&root),
            json!({"pattern": "needle", "path": traversal.to_str().expect("utf8")}),
        )
        .await;
        assert!(out.is_error);
        assert!(
            out.content.contains("outside allowed directories"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("secret"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_refuses_a_relative_path() {
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({"pattern": "needle", "path": "src"})).await;
        assert!(out.is_error);
        assert!(out.content.contains("relative"), "{}", out.content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grep_skips_symlinks_escaping_the_root() {
        use std::os::unix::fs::symlink;

        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("root");
        let secret = outer.path().join("secret");
        create_dir(&root).expect("mkdir");
        create_dir(&secret).expect("mkdir");
        write(secret.join("key.txt"), "needle secret\n").expect("write");
        symlink(&secret, root.join("escape_dir")).expect("symlink");
        symlink(secret.join("key.txt"), root.join("escape_file.txt")).expect("symlink");
        write(root.join("own.txt"), "needle own\n").expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("needle own"), "{}", out.content);
        assert!(!out.content.contains("secret"), "{}", out.content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grep_follows_symlinks_inside_the_root() {
        use std::os::unix::fs::symlink;

        let (_dir, root) = root();
        create_dir(root.join("real")).expect("mkdir");
        write(root.join("real/a.txt"), "needle\n").expect("write");
        symlink(root.join("real"), root.join("alias")).expect("symlink");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert!(
            out.content.contains("alias/a.txt:1:needle"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("real/a.txt:1:needle"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn grep_respects_gitignore_outside_a_git_repository() {
        let (_dir, root) = root();
        write(root.join(".gitignore"), "ignored.txt\ntarget/\n").expect("write");
        write(root.join("ignored.txt"), "needle\n").expect("write");
        create_dir(root.join("target")).expect("mkdir");
        write(root.join("target/out.txt"), "needle\n").expect("write");
        write(root.join("kept.txt"), "needle\n").expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert!(out.content.contains("kept.txt"), "{}", out.content);
        assert!(!out.content.contains("ignored.txt"), "{}", out.content);
        assert!(!out.content.contains("out.txt"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_skips_git_but_searches_other_hidden_files() {
        let (_dir, root) = root();
        create_dir_all(root.join(".git/refs")).expect("mkdir");
        write(root.join(".git/config"), "needle\n").expect("write");
        write(root.join(".gitlab-ci.yml"), "needle\n").expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert!(
            out.content.contains(".gitlab-ci.yml:1:needle"),
            "{}",
            out.content
        );
        assert!(!out.content.contains(".git/"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_skips_binary_files() {
        let (_dir, root) = root();
        write(root.join("blob.bin"), b"needle\0\x01\x02needle\n").expect("write");
        write(root.join("text.txt"), "needle\n").expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert!(out.content.contains("text.txt"), "{}", out.content);
        assert!(!out.content.contains("blob.bin"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_skips_a_binary_file_whose_first_line_is_text() {
        // The NUL byte sits on line 2: only the head probe can reject line 1.
        let (_dir, root) = root();
        write(root.join("exe"), b"needle header\n\0\x7fELF\n").expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert_eq!(out.content, "No matches found");
    }

    #[tokio::test]
    async fn grep_stops_at_200_matches_and_says_so() {
        let (_dir, root) = root();
        let lines: String = (0..250).map(|i| format!("needle {i}\n")).collect();
        write(root.join("many.txt"), lines).expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        let matches = out
            .content
            .lines()
            .filter(|l| l.contains(":needle "))
            .count();
        assert_eq!(matches, 200);
        assert!(out.content.contains("needle 199"), "{}", out.content);
        assert!(!out.content.contains("needle 200"), "{}", out.content);
        assert!(
            out.content
                .lines()
                .last()
                .is_some_and(|l| l.starts_with("[truncated: stopped at 200 results")),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn grep_stops_at_64_kib_and_says_so() {
        let (_dir, root) = root();
        let long = "x".repeat(490);
        let lines: String = (0..150).map(|i| format!("needle {i} {long}\n")).collect();
        write(root.join("wide.txt"), lines).expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        let (results, notice) = out.content.rsplit_once('\n').expect("notice line");
        assert!(results.len() <= 64 * 1024, "{}", results.len());
        assert!(
            notice.starts_with("[truncated: output limit of 64 KiB"),
            "{notice}"
        );
    }

    #[tokio::test]
    async fn grep_clips_very_long_lines() {
        let (_dir, root) = root();
        write(root.join("min.js"), format!("needle{}\n", "é".repeat(2000))).expect("write");

        let out = run(&tool(&root), json!({"pattern": "needle"})).await;
        assert!(out.content.len() < 1024, "{}", out.content.len());
        assert!(out.content.ends_with("[line truncated]"), "{}", out.content);
    }

    #[test]
    fn grep_stops_when_the_deadline_has_passed() {
        let (_dir, root) = root();
        write(root.join("a.txt"), "needle\n").expect("write");
        let tool = tool(&root);
        let regex = Regex::new("needle").expect("regex");

        let text = search(confined_walker(&tool.roots, &root), &regex, Instant::now());
        assert!(
            text.starts_with("[truncated: search stopped after 10 s"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn grep_reports_an_invalid_regex_to_the_model() {
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({"pattern": "(unclosed"})).await;
        assert!(out.is_error);
        assert!(
            out.content.starts_with("Invalid regex '(unclosed'"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn grep_reports_an_invalid_glob_to_the_model() {
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({"pattern": "x", "glob": "[z-a]"})).await;
        assert!(out.is_error);
        assert!(
            out.content.starts_with("Invalid glob '[z-a]'"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn grep_reports_a_mistyped_parameter_to_the_model() {
        let (_dir, root) = root();
        let out = run(
            &tool(&root),
            json!({"pattern": "x", "case_insensitive": "yes"}),
        )
        .await;
        assert!(out.is_error);
        assert_eq!(
            out.content,
            "Invalid arguments: case_insensitive: invalid type: string \"yes\", expected a boolean"
        );
    }

    #[tokio::test]
    async fn grep_reports_a_missing_pattern_to_the_model() {
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({})).await;
        assert!(out.is_error);
        assert_eq!(out.content, "Invalid arguments: missing field `pattern`");
    }

    #[tokio::test]
    async fn grep_reports_an_unknown_parameter_to_the_model() {
        // Silently ignoring `ignore_case` would run a case-sensitive search
        // the model believes is not.
        let (_dir, root) = root();
        let out = run(&tool(&root), json!({"pattern": "x", "ignore_case": true})).await;
        assert!(out.is_error);
        assert!(
            out.content.contains("unknown field `ignore_case`"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn grep_treats_null_parameters_as_absent() {
        let (_dir, root) = root();
        let file = root.join("a.txt");
        write(&file, "needle\n").expect("write");
        let out = run(
            &tool(&root),
            json!({"pattern": "needle", "path": null, "glob": null, "case_insensitive": null}),
        )
        .await;
        assert_eq!(out.content, format!("{}:1:needle", file.display()));
    }

    #[test]
    fn grep_schema_is_what_the_model_reads() {
        let (_dir, root) = root();
        assert_eq!(
            tool(&root).parameters_schema(),
            json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regular expression to search for, matched against each line"
                    },
                    "path": {
                        "type": "string",
                        "description": "Absolute path of the file or directory to search. Defaults to the first searchable directory"
                    },
                    "glob": {
                        "type": "string",
                        "description": "Only search files matching this glob, gitignore-style and relative to `path`, e.g. `*.rs` or `src/**/*.ts`"
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Match case-insensitively. Defaults to false"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            })
        );
    }

    #[test]
    fn grep_construction_fails_on_a_missing_root() {
        let (_dir, root) = root();
        assert!(GrepTool::with_allowed_paths(vec![root.join("missing")]).is_err());
        assert!(GrepTool::with_allowed_paths(Vec::new()).is_err());
    }

    #[test]
    fn grep_description_lists_the_roots() {
        let (_dir, root) = root();
        assert!(tool(&root).description().contains(&*root.to_string_lossy()));
    }

    #[test]
    fn grep_tool_is_read_only() {
        let (_dir, root) = root();
        assert!(tool(&root).read_only());
    }
}
