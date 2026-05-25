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
/// can access. When empty, all paths are allowed.
pub struct ReadFileTool {
    allowed_paths: Vec<PathBuf>,
}

impl ReadFileTool {
    /// Create a `ReadFileTool` with no path restrictions.
    pub fn new() -> Self {
        Self {
            allowed_paths: Vec::new(),
        }
    }

    /// Create a `ReadFileTool` restricted to the given directories.
    ///
    /// Any read attempt outside these directories will return an error
    /// to the model.
    pub fn with_allowed_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            allowed_paths: paths,
        }
    }

    fn is_path_allowed(&self, path: &Path) -> bool {
        if self.allowed_paths.is_empty() {
            return true;
        }
        self.allowed_paths
            .iter()
            .any(|allowed| path.starts_with(allowed))
    }
}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::new()
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

    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let file_path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::new("missing 'file_path' parameter"))?;

            let path = PathBuf::from(file_path);

            if !self.is_path_allowed(&path) {
                return Ok(ToolOutput::error(format!(
                    "Access denied: path '{}' is outside allowed directories",
                    file_path
                )));
            }

            let metadata = match fs::metadata(&path).await {
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

            let content = match fs::read_to_string(&path).await {
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
        let tool = ReadFileTool::new();
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
        let tool = ReadFileTool::new();
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
        let tool = ReadFileTool::new();
        let result = tool
            .execute(json!({"file_path": "/tmp/nonexistent_ironflow_test_file_xyz"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Cannot read"));
    }

    #[tokio::test]
    async fn read_file_path_restriction() {
        let file = create_temp_file("secret data");
        let tool = ReadFileTool::with_allowed_paths(vec![PathBuf::from("/nonexistent_dir")]);
        let result = tool
            .execute(json!({"file_path": file.path().to_str().expect("path")}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Access denied"));
    }

    #[tokio::test]
    async fn read_file_missing_param() {
        let tool = ReadFileTool::new();
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_directory_returns_error() {
        let tool = ReadFileTool::new();
        let result = tool
            .execute(json!({"file_path": "/tmp"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("is not a file"));
    }
}
