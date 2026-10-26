//! Upload route missing from the `gitlab` crate:
//! `POST /projects/:id/uploads` (multipart).

use std::borrow::Cow;

use gitlab::api::common::NameOrId;
use gitlab::api::endpoint_prelude::{BodyError, Endpoint, Method};
use ironflow_core::error::OperationError;

/// Multipart boundary. The [`Endpoint`] trait needs a `&'static str` content type, so the
/// boundary is fixed and [`UploadFile::new`] refuses any content that contains it.
const BOUNDARY: &str = "ironflow-ops-gitlab-upload-boundary-7f3a9c1e";

/// Content type of the upload body, carrying [`BOUNDARY`].
const CONTENT_TYPE: &str =
    "multipart/form-data; boundary=ironflow-ops-gitlab-upload-boundary-7f3a9c1e";

/// Fallback file name when sanitizing leaves nothing.
const FALLBACK_FILENAME: &str = "upload";

/// Upload a file to a project, to reference it from an issue, note or merge request.
///
/// Maps to `POST /projects/:id/uploads`. The answer is GitLab's JSON (`alt`, `url`,
/// `full_path`, `markdown`): deserialize it with `json::<T>()` into a struct and paste the
/// `markdown` field in a note body.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::uploads::UploadFile;
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let upload = UploadFile::new(NameOrId::from(42), "screenshot.png", vec![0x89, 0x50])?;
/// let op = gitlab.op(upload);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Building the upload fails if the content contains the multipart boundary marker. Running
/// it returns an error if the request fails or the response cannot be deserialized (e.g. the
/// project does not exist).
pub struct UploadFile {
    /// The project to upload the file to.
    pub project: NameOrId<'static>,
    filename: String,
    content: Vec<u8>,
}

impl UploadFile {
    /// Build an upload. The file name is sanitized: only its last path component is kept
    /// and every character other than ASCII alphanumerics, `.`, `-` and `_` becomes `_`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_gitlab::endpoints::uploads::UploadFile;
    /// use gitlab::api::common::NameOrId;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let upload = UploadFile::new(NameOrId::from("group/project"), "../shot.png", b"data".to_vec())?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] (without status) if `content` contains the multipart
    /// boundary marker, which would end the part early and break the encoding.
    pub fn new(
        project: NameOrId<'static>,
        filename: &str,
        content: Vec<u8>,
    ) -> Result<Self, OperationError> {
        let marker = format!("--{BOUNDARY}");
        if contains_subslice(&content, marker.as_bytes()) {
            return Err(OperationError::Http {
                status: None,
                message: "upload content contains the multipart boundary marker".to_string(),
            });
        }
        Ok(Self {
            project,
            filename: safe_filename(filename),
            content,
        })
    }
}

impl Endpoint for UploadFile {
    fn method(&self) -> Method {
        Method::POST
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!("projects/{}/uploads", self.project).into()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, BodyError> {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
            self.filename,
        )
        .into_bytes();
        body.extend_from_slice(&self.content);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        Ok(Some((CONTENT_TYPE, body)))
    }
}

/// Whether `haystack` contains `needle` as a contiguous run of bytes.
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Reduce a client supplied file name to a safe one for a `Content-Disposition` header.
fn safe_filename(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mapped: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = mapped.trim_start_matches('.');
    if trimmed.is_empty() {
        FALLBACK_FILENAME.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_plain_name() {
        assert_eq!(safe_filename("shot-1_a.png"), "shot-1_a.png");
    }

    #[test]
    fn keeps_last_component_of_unix_and_windows_paths() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename("C:\\dir\\file.txt"), "file.txt");
    }

    #[test]
    fn replaces_quotes_crlf_and_unicode() {
        assert_eq!(safe_filename("a\"b\r\nc.png"), "a_b__c.png");
        assert_eq!(safe_filename("é.png"), "_.png");
    }

    #[test]
    fn trims_leading_dots() {
        assert_eq!(safe_filename("..hidden"), "hidden");
    }

    #[test]
    fn falls_back_when_empty_or_only_dots() {
        assert_eq!(safe_filename(""), "upload");
        assert_eq!(safe_filename("..."), "upload");
        assert_eq!(safe_filename("dir/"), "upload");
    }
}
