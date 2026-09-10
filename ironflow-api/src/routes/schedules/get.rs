//! `GET /api/v1/schedules/{id}` -- Get a schedule by ID.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;

use crate::entities::ScheduleResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get a schedule by ID.
///
/// # Errors
///
/// - 401 if not authenticated
/// - 404 if the schedule does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/schedules/{id}",
        tags = ["schedules"],
        params(("id" = Uuid, Path, description = "Schedule ID")),
        responses(
            (status = 200, description = "Schedule detail", body = ScheduleResponse),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Schedule not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_schedule(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let schedule = state
        .store
        .find_schedule_by_id(id)
        .await?
        .ok_or(ApiError::ScheduleNotFound(id))?;

    Ok(ok(ScheduleResponse::from(schedule)))
}
