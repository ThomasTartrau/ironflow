//! Worker capabilities sent as query parameters on the internal pick and
//! pending-count routes.

use ironflow_store::entities::{WorkerCapabilities, normalize_worker_tags, validate_worker_tags};

use crate::error::ApiError;

/// Parse the `workflows` and `tags` query parameters a worker sends.
///
/// Both are comma-separated lists. When neither is present the worker
/// predates routing: `None` is returned and it takes every run. When only
/// `tags` is present the worker accepts any workflow. An empty `tags` value
/// means a worker carrying no tag: it only takes runs that require none.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] when a tag is invalid (see
/// [`validate_worker_tags`]).
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::parse_worker_capabilities;
///
/// # fn example() -> Result<(), ironflow_api::error::ApiError> {
/// assert!(parse_worker_capabilities(None, None)?.is_none());
///
/// let caps = parse_worker_capabilities(Some("deploy,build"), Some("region:eu,gpu"))?
///     .expect("capabilities were sent");
/// assert_eq!(caps.workflows, Some(vec!["build".to_string(), "deploy".to_string()]));
/// assert_eq!(caps.tags, vec!["gpu".to_string(), "region:eu".to_string()]);
/// # Ok(())
/// # }
/// ```
pub fn parse_worker_capabilities(
    workflows: Option<&str>,
    tags: Option<&str>,
) -> Result<Option<WorkerCapabilities>, ApiError> {
    if workflows.is_none() && tags.is_none() {
        return Ok(None);
    }

    let workflows = workflows.map(|raw| normalize_worker_tags(split_list(raw)));
    let tags: Vec<String> = tags.map(split_list).unwrap_or_default();
    validate_worker_tags(&tags).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Some(WorkerCapabilities::new(
        workflows,
        normalize_worker_tags(tags),
    )))
}

/// Split a comma-separated list, dropping blank entries.
fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn absent_params_mean_a_legacy_worker() {
        assert!(parse_worker_capabilities(None, None).unwrap().is_none());
    }

    #[test]
    fn both_params_are_parsed() {
        let caps = parse_worker_capabilities(Some("deploy, build"), Some("gpu,region:eu"))
            .unwrap()
            .unwrap();
        assert_eq!(caps.workflows, Some(strings(&["build", "deploy"])));
        assert_eq!(caps.tags, strings(&["gpu", "region:eu"]));
    }

    #[test]
    fn tags_without_workflows_accept_any_workflow() {
        let caps = parse_worker_capabilities(None, Some("gpu"))
            .unwrap()
            .unwrap();
        assert!(caps.workflows.is_none());
        assert_eq!(caps.tags, strings(&["gpu"]));
    }

    #[test]
    fn empty_tags_mean_a_worker_without_tags() {
        let caps = parse_worker_capabilities(Some("deploy"), Some(""))
            .unwrap()
            .unwrap();
        assert_eq!(caps.workflows, Some(strings(&["deploy"])));
        assert!(caps.tags.is_empty());
    }

    #[test]
    fn empty_workflows_take_no_workflow() {
        let caps = parse_worker_capabilities(Some(""), None).unwrap().unwrap();
        assert_eq!(caps.workflows, Some(Vec::new()));
        assert!(caps.tags.is_empty());
    }

    #[test]
    fn duplicate_tags_are_dropped() {
        let caps = parse_worker_capabilities(None, Some("gpu,gpu, gpu,,arm"))
            .unwrap()
            .unwrap();
        assert_eq!(caps.tags, strings(&["arm", "gpu"]));
    }

    #[test]
    fn invalid_tag_is_a_bad_request() {
        let err = parse_worker_capabilities(None, Some("gpu,two words")).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(ref msg) if msg.contains("two words")));
    }

    #[test]
    fn too_long_tag_is_a_bad_request() {
        let long = "a".repeat(65);
        let err = parse_worker_capabilities(None, Some(&long)).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }
}
