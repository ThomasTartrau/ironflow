//! Worker-only Provider Account routes (WORKER_TOKEN auth).
//!
//! - `GET /api/v1/internal/provider-accounts/candidates?kind=` -- accounts a
//!   step may run under, with their windows and running steps.
//! - `POST /api/v1/internal/provider-accounts/{id}/observations` -- record
//!   the windows observed during an invocation.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use ironflow_engine::notify::{Event, ProviderAccountUsageUpdatedEvent};
use ironflow_store::entities::NewProviderAccountObservation;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Query of the candidates route.
#[derive(Debug, Deserialize)]
pub struct CandidatesQuery {
    /// Account kind the worker's provider can inject.
    pub kind: String,
}

/// List the candidate accounts of a kind. Worker-only.
pub async fn list_candidates(
    State(state): State<AppState>,
    Query(query): Query<CandidatesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let candidates = state
        .store
        .list_provider_account_candidates(query.kind)
        .await?;
    Ok(ok(candidates))
}

/// Record observed windows for an account and notify SSE clients. Worker-only.
pub async fn record_observation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(observation): Json<NewProviderAccountObservation>,
) -> Result<impl IntoResponse, ApiError> {
    let account = state
        .store
        .get_provider_account(id)
        .await?
        .ok_or_else(|| ApiError::ProviderAccountNotFound(id.to_string()))?;
    let windows = state
        .store
        .record_provider_account_observation(id, observation)
        .await?;
    // A send error only means no SSE client is listening.
    let _ = state.event_sender.send(Event::ProviderAccountUsageUpdated(
        ProviderAccountUsageUpdatedEvent {
            account_id: id,
            name: account.name,
            windows: windows.clone(),
            at: Utc::now(),
        },
    ));
    Ok(ok(windows))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use chrono::{TimeDelta, Utc};
    use serde_json::json;

    use crate::routes::provider_accounts::test_support::{
        Stub, call, create_account, state_with_stub,
    };

    const WORKER: &str = "Bearer test-worker-token";

    #[tokio::test]
    async fn internal_provider_accounts_candidates_and_observations() {
        let state = state_with_stub(Stub::Valid).await;
        let created = create_account(&state, "perso-max").await;
        let id = created["data"]["id"].as_str().unwrap().to_string();

        let (status, resp, _) = call(
            &state,
            "GET",
            "/api/v1/internal/provider-accounts/candidates?kind=claude_subscription",
            WORKER,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"][0]["account"]["name"], "perso-max");
        assert_eq!(resp["data"][0]["running_steps"], 0);

        let mut rx = state.event_sender.subscribe();
        let observed_at = Utc::now() + TimeDelta::seconds(1);
        let (status, resp, _) = call(
            &state,
            "POST",
            &format!("/api/v1/internal/provider-accounts/{id}/observations"),
            WORKER,
            Some(json!({
                "windows": [{
                    "window": "five_hour",
                    "utilization": 0.9,
                    "resets_at": null,
                    "status": "allowed_warning",
                    "model_scope": null,
                    "observed_at": observed_at,
                }],
                "auth_failed": false
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let five = resp["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["window"] == "five_hour")
            .unwrap()
            .clone();
        assert_eq!(five["status"], "allowed_warning");
        let event = rx.try_recv().expect("usage event published");
        assert_eq!(event.event_type(), "provider_account.usage_updated");
    }

    #[tokio::test]
    async fn internal_provider_accounts_require_worker_token() {
        let state = state_with_stub(Stub::Valid).await;
        let (status, _, _) = call(
            &state,
            "GET",
            "/api/v1/internal/provider-accounts/candidates?kind=claude_subscription",
            "Bearer wrong",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
