//! `GET /api/v1/internal/runs/:id/descendants` — Non-terminal descendants of a
//! run (raw store entities).

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// List the non-terminal sub-workflow descendants of a run, oldest first.
///
/// Lets a worker close the children a stopped run leaves behind. An unknown
/// run has no descendant: the list is empty, not a 404.
pub async fn list_descendants(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let descendants = state.store.list_active_descendants(id).await?;
    Ok(ok(descendants))
}
