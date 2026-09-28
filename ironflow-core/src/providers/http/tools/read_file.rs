//! Tool for reading local files.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use serde_json::{Value, json};
use tokio::fs;

use super::tool_trait::{Tool, ToolError, ToolOutput};

/// Maximum file size to read (10 MB).
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Reads a local file and returns its contents as text.
///
/// Supports optional `offset` and `limit` parameters for reading
/// specific line ranges from large files.
///
/// # Security
///
/// An optional `allowed_paths` list restricts which directories the tool
/// can access. Both the requested path and every configured allowed root
/// are resolved to their canonical form (following symlinks and resolving
/// `.`/`..`) before being compared, so a request cannot escape an allowed
/// root through a `..` component or a symlink that points outside it. A
/// relative `file_path` is rejected outright when any root is configured,
/// since resolving it implicitly against the worker's current directory
/// would bypass the restriction. When no root is configured (see
/// [`unrestricted`](Self::unrestricted)), every path the worker process
/// can read is accessible.
pub struct ReadFileTool {
    allowed_paths: Vec<PathBuf>,
}

impl ReadFileTool {
    /// Create a `ReadFileTool` with no path restrictions.
    ///
    /// # Security
    ///
    /// This grants the model access to the entire filesystem readable by
    /// the worker process. Prefer [`with_allowed_paths`](Self::with_allowed_paths)
    /// to restrict access, or call [`unrestricted`](Self::unrestricted) if
    /// full access is genuinely intended -- it has the same behavior as
    /// this constructor but says so at the call site.
    #[deprecated(
        note = "call `with_allowed_paths` to restrict access, or `unrestricted` if full filesystem access is intended"
    )]
    pub fn new() -> Self {
        Self::unrestricted()
    }

    /// Create a `ReadFileTool` with no path restrictions.
    ///
    /// # Security
    ///
    /// Grants the model access to the entire filesystem readable by the
    /// worker process. Use [`with_allowed_paths`](Self::with_allowed_paths)
    /// whenever the set of readable directories can be bounded.
    pub fn unrestricted() -> Self {
        Self {
            allowed_paths: Vec::new(),
        }
    }

    /// Create a `ReadFileTool` restricted to the given directories.
    ///
    /// Each path is canonicalized once, at construction time: this resolves
    /// symlinks and `.`/`..` components in the roots themselves, so the
    /// per-request check in [`resolve_allowed_path`](Self::resolve_allowed_path)
    /// always compares two canonical paths.
    ///
    /// # Panics
    ///
    /// Panics if any given path does not exist or cannot be resolved. A
    /// missing root is a construction error, not a restriction that gets
    /// silently dropped.
    pub fn with_allowed_paths(paths: Vec<PathBuf>) -> Self {
        let allowed_paths = paths
            .into_iter()
            .map(|path| {
                std::fs::canonicalize(&path).unwrap_or_else(|err| {
                    panic!(
                        "ReadFileTool: allowed path '{}' does not exist or cannot be resolved: {}",
                        path.display(),
                        err
                    )
                })
            })
            .collect();
        Self { allowed_paths }
    }

    /// Resolve `file_path` to its canonical form and check it against
    /// `allowed_paths`.
    ///
    /// Returns the canonical [`PathBuf`] to use for every subsequent
    /// filesystem operation on success, so the path that was checked is
    /// the exact path that gets read -- no window between check and use.
    async fn resolve_allowed_path(&self, file_path: &str) -> Result<PathBuf, String> {
        let requested = Path::new(file_path);

        if !self.allowed_paths.is_empty() && requested.is_relative() {
            return Err(Self::access_denied_message(file_path));
        }

        let canonical = fs::canonicalize(requested)
            .await
            .map_err(|_| Self::access_denied_message(file_path))?;

        let allowed = self.allowed_paths.is_empty()
            || self
                .allowed_paths
                .iter()
                .any(|root| canonical.starts_with(root));

        if allowed {
            Ok(canonical)
        } else {
            Err(Self::access_denied_message(file_path))
        }
    }

    fn access_denied_message(file_path: &str) -> String {
        format!(
            "Access denied: path '{}' is outside allowed directories",
            file_path
        )
    }
}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::unrestricted()
    }
}

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a local file. Returns the file content as text."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-based)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["file_path"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let file_path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::new("missing 'file_path' parameter"))?;

            let canonical_path = match self.resolve_allowed_path(file_path).await {
                Ok(path) => path,
                Err(message) => return Ok(ToolOutput::error(message)),
            };

            let metadata = match fs::metadata(&canonical_path).await {
                Ok(m) => m,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Cannot read '{}': {}",
                        file_path, e
                    )));
                }
            };

            if !metadata.is_file() {
                return Ok(ToolOutput::error(format!("'{}' is not a file", file_path)));
            }

            if metadata.len() > MAX_FILE_SIZE {
                return Ok(ToolOutput::error(format!(
                    "File '{}' is too large ({} bytes, max {})",
                    file_path,
                    metadata.len(),
                    MAX_FILE_SIZE
                )));
            }

            let content = match fs::read_to_string(&canonical_path).await {
                Ok(c) => c,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to read '{}': {}",
                        file_path, e
                    )));
                }
            };

            let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = input
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let lines: Vec<&str> = content.lines().collect();
            let selected: Vec<&str> = match limit {
                Some(lim) => lines.into_iter().skip(offset).take(lim).collect(),
                None => lines.into_iter().skip(offset).collect(),
            };

            Ok(ToolOutput::success(selected.join("\n")))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::json;
    use tempfile::NamedTempFile;
    use tempfile::tempdir;

    use super::*;

    fn create_temp_file(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("failed to create temp file");
        f.write_all(content.as_bytes())
            .expect("failed to write temp file");
        f.flush().expect("failed to flush temp file");
        f
    }

    #[tokio::test]
    async fn read_file_success() {
        let file = create_temp_file("line 1\nline 2\nline 3");
        let tool = ReadFileTool::unrestricted();
        let result = tool
            .execute(json!({"file_path": file.path().to_str().expect("path")}))
            .await
            .expect("should succeed");
        assert!(!result.is_error);
        assert!(result.content.contains("line 1"));
        assert!(result.content.contains("line 3"));
    }

    #[tokio::test]
    async fn read_file_with_offset_and_limit() {
        let file = create_temp_file("a\nb\nc\nd\ne");
        let tool = ReadFileTool::unrestricted();
        let result = tool
            .execute(json!({
                "file_path": file.path().to_str().expect("path"),
                "offset": 1,
                "limit": 2
            }))
            .await
            .expect("should succeed");
        assert!(!result.is_error);
        assert_eq!(result.content, "b\nc");
    }

    #[tokio::test]
    async fn read_file_not_found() {
        let tool = ReadFileTool::unrestricted();
        let result = tool
            .execute(json!({"file_path": "/tmp/nonexistent_ironflow_test_file_xyz"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Access denied"));
    }

    #[tokio::test]
    async fn read_file_path_restriction() {
        let root = tempdir().expect("failed to create temp dir");
        let outside = create_temp_file("secret data");
        let tool = ReadFileTool::with_allowed_paths(vec![root.path().to_path_buf()]);
        let result = tool
            .execute(json!({"file_path": outside.path().to_str().expect("path")}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Access denied"));
    }

    #[tokio::test]
    async fn read_file_missing_param() {
        let tool = ReadFileTool::unrestricted();
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_directory_returns_error() {
        let tool = ReadFileTool::unrestricted();
        let result = tool
            .execute(json!({"file_path": "/tmp"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("is not a file"));
    }

    #[test]
    fn read_file_tool_is_read_only() {
        assert!(ReadFileTool::unrestricted().read_only());
    }

    #[test]
    #[allow(deprecated)]
    fn read_file_new_is_deprecated_alias_for_unrestricted() {
        let tool = ReadFileTool::new();
        assert!(tool.allowed_paths.is_empty());
    }

    #[test]
    fn read_file_default_is_unrestricted() {
        let tool = ReadFileTool::default();
        assert!(tool.allowed_paths.is_empty());
    }

    #[tokio::test]
    async fn read_file_path_traversal_via_dotdot_denied() {
        let base = tempdir().expect("failed to create temp dir");
        let root = base.path().join("racine");
        std::fs::create_dir(&root).expect("failed to create root dir");
        let outside_file = base.path().join("fichier_hors_racine");
        std::fs::write(&outside_file, "secret").expect("failed to write outside file");

        let tool = ReadFileTool::with_allowed_paths(vec![root.clone()]);
        let traversal_path = root.join("..").join("fichier_hors_racine");
        let result = tool
            .execute(json!({"file_path": traversal_path.to_str().expect("path")}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Access denied"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_file_symlink_escaping_root_denied() {
        let base = tempdir().expect("failed to create temp dir");
        let root = base.path().join("root");
        std::fs::create_dir(&root).expect("failed to create root dir");
        let secret = base.path().join("secret.txt");
        std::fs::write(&secret, "top secret").expect("failed to write secret file");
        let link = root.join("escape_link");
        std::os::unix::fs::symlink(&secret, &link).expect("failed to create symlink");

        let tool = ReadFileTool::with_allowed_paths(vec![root.clone()]);
        let result = tool
            .execute(json!({"file_path": link.to_str().expect("path")}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Access denied"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_file_symlink_inside_root_accepted() {
        let base = tempdir().expect("failed to create temp dir");
        let root = base.path().join("root");
        std::fs::create_dir(&root).expect("failed to create root dir");
        let target = root.join("target.txt");
        std::fs::write(&target, "hello from target").expect("failed to write target file");
        let link = root.join("internal_link");
        std::os::unix::fs::symlink(&target, &link).expect("failed to create symlink");

        let tool = ReadFileTool::with_allowed_paths(vec![root.clone()]);
        let result = tool
            .execute(json!({"file_path": link.to_str().expect("path")}))
            .await
            .expect("should succeed");
        assert!(!result.is_error);
        assert_eq!(result.content, "hello from target");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn read_file_proc_self_environ_denied_with_root() {
        let root = tempdir().expect("failed to create temp dir");
        let tool = ReadFileTool::with_allowed_paths(vec![root.path().to_path_buf()]);
        let result = tool
            .execute(json!({"file_path": "/proc/self/environ"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Access denied"));
    }

    #[tokio::test]
    async fn read_file_relative_path_denied_when_roots_configured() {
        let root = tempdir().expect("failed to create temp dir");
        let tool = ReadFileTool::with_allowed_paths(vec![root.path().to_path_buf()]);
        let result = tool
            .execute(json!({"file_path": "some/relative/path.txt"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Access denied"));
    }
}
