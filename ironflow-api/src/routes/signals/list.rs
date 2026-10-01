//! `GET /api/v1/signals` -- List received signals.

use axum::extract::{Query, State};
use axum::response::IntoResponse;

use ironflow_auth::extractor::{AuthMethod, Authenticated};
use ironflow_store::entities::{ApiKeyScope, SignalFilter};

use crate::entities::{ListSignalsQuery, SignalResponse};
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// List received signals, newest first, paginated.
///
/// Any signed-in user may list signals. An API key needs the `runs_read`
/// scope.
///
/// # Errors
///
/// - 401 if not authenticated
/// - 403 if the API key lacks `runs_read`
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/signals",
        tags = ["signals"],
        params(ListSignalsQuery),
        responses(
            (status = 200, description = "Paginated list of signals", body = Vec<SignalResponse>),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Insufficient scope")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_signals(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<ListSignalsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if let AuthMethod::ApiKey { scopes, .. } = &auth.method
        && !ApiKeyScope::has_permission(scopes, &ApiKeyScope::RunsRead)
    {
        return Err(ApiError::InsufficientScope);
    }

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let filter = SignalFilter {
        name: query.name,
        key: query.key,
    };

    let result = state.store.list_signals(filter, page, per_page).await?;
    let items: Vec<SignalResponse> = result.items.into_iter().map(SignalResponse::from).collect();

    Ok(ok_paged(items, page, per_page, result.total))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;
    use serde_json::{Value, json};

    use ironflow_store::entities::NewSignal;

    use crate::routes::signals::test_support::{api_key_header, call, jwt_header, test_state};

    use super::*;

    /// `GET uri` through a router serving only this route.
    async fn get_signals(state: &AppState, uri: &str, auth: &str) -> (StatusCode, Value) {
        let router = Router::new()
            .route("/", get(list_signals))
            .with_state(state.clone());
        call(router, "GET", uri, Some(auth), None).await
    }

    async fn insert(state: &AppState, name: &str, key: &str) {
        state
            .store
            .insert_signal(NewSignal {
                name: name.to_string(),
                key: key.to_string(),
                payload: json!({"n": 1}),
                idempotency_id: None,
            })
            .await
            .expect("insert signal");
    }

    #[tokio::test]
    async fn list_signals_filters_by_name_and_key() {
        let (state, _admin, member) = test_state().await;
        insert(&state, "demo.done", "k1").await;
        insert(&state, "demo.done", "k2").await;
        insert(&state, "other", "k1").await;
        let auth = jwt_header(&member, &state);

        let (status, body) = get_signals(&state, "/?name=demo.done&key=k1", &auth).await;
        assert_eq!(status, StatusCode::OK);
        let items = body["data"].as_array().expect("array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "demo.done");
        assert_eq!(items[0]["key"], "k1");
        assert_eq!(items[0]["payload"], json!({"n": 1}));
        assert_eq!(body["meta"]["total"], 1);

        let (status, body) = get_signals(&state, "/?name=demo.done", &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"].as_array().expect("array").len(), 2);
    }

    #[tokio::test]
    async fn list_signals_paginates() {
        let (state, admin, _member) = test_state().await;
        for key in ["k1", "k2", "k3"] {
            insert(&state, "demo.done", key).await;
        }
        let auth = jwt_header(&admin, &state);

        let (status, body) = get_signals(&state, "/?per_page=2&page=2", &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"].as_array().expect("array").len(), 1);
        assert_eq!(body["meta"]["total"], 3);
    }

    #[tokio::test]
    async fn list_signals_api_key_without_runs_read_returns_403() {
        let (state, admin, _member) = test_state().await;
        let auth = api_key_header(&admin, vec![ApiKeyScope::SignalsSend], &state).await;
        let (status, _) = get_signals(&state, "/", &auth).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn list_signals_api_key_with_runs_read_succeeds() {
        let (state, admin, _member) = test_state().await;
        insert(&state, "demo.done", "k1").await;
        let auth = api_key_header(&admin, vec![ApiKeyScope::RunsRead], &state).await;
        let (status, body) = get_signals(&state, "/", &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"].as_array().expect("array").len(), 1);
    }
}
