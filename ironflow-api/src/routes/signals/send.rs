//! `POST /api/v1/signals` -- Send a signal to the runs waiting for it.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;

use ironflow_auth::extractor::{AuthMethod, Authenticated};
use ironflow_engine::error::EngineError;
use ironflow_store::entities::{ApiKeyScope, NewSignal};

use crate::entities::{SendSignalRequest, SignalDeliveryResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Maximum length of a signal name, key or idempotency ID.
const MAX_SIGNAL_FIELD_LEN: usize = 255;

/// Check that the caller may send signals.
///
/// A JWT session must belong to an admin. An API key must carry the
/// `signals_send` (or `admin`) scope.
fn authorize(auth: &Authenticated) -> Result<(), ApiError> {
    match &auth.method {
        AuthMethod::Jwt { is_admin, .. } => {
            if !is_admin {
                return Err(ApiError::Forbidden);
            }
        }
        AuthMethod::ApiKey { scopes, .. } => {
            if !ApiKeyScope::has_permission(scopes, &ApiKeyScope::SignalsSend) {
                return Err(ApiError::InsufficientScope);
            }
        }
    }
    Ok(())
}

/// Reject an empty or oversized signal field.
fn validate_field(field: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::BadRequest(format!("{field} must not be empty")));
    }
    if value.chars().count() > MAX_SIGNAL_FIELD_LEN {
        return Err(ApiError::BadRequest(format!(
            "{field} must be at most {MAX_SIGNAL_FIELD_LEN} characters"
        )));
    }
    Ok(())
}

/// Send a signal.
///
/// The signal is stored, then every run waiting for its `(name, key)` with a
/// payload schema it matches is resumed. A waiting run whose schema the
/// payload does not match keeps waiting and is listed under `rejected`. The
/// signal is stored even when nobody waits for it yet, so a run that opens its
/// wait later still finds it.
///
/// Sending again with the same `idempotency_id` returns 200 with
/// `duplicate: true` and delivers nothing.
///
/// # Errors
///
/// - 400 if `name` or `key` is empty or longer than 255 characters, or if
///   `idempotency_id` is empty or longer than 255 characters
/// - 401 if not authenticated
/// - 403 if the caller is not an admin, or the API key lacks `signals_send`
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/signals",
        tags = ["signals"],
        request_body(content = SendSignalRequest, description = "Signal to deliver"),
        responses(
            (status = 200, description = "Signal stored and delivered", body = SignalDeliveryResponse),
            (status = 400, description = "Invalid signal"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden or insufficient scope")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn send_signal(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<SendSignalRequest>,
) -> Result<impl IntoResponse, ApiError> {
    authorize(&auth)?;
    validate_field("name", &req.name)?;
    validate_field("key", &req.key)?;
    if let Some(idempotency_id) = &req.idempotency_id {
        validate_field("idempotency_id", idempotency_id)?;
    }

    let delivery = state
        .engine
        .deliver_signal(NewSignal {
            name: req.name,
            key: req.key,
            payload: req.payload,
            idempotency_id: req.idempotency_id,
        })
        .await
        .map_err(|e| match e {
            EngineError::InvalidSignal(msg) => ApiError::BadRequest(msg),
            EngineError::Store(err) => ApiError::from(err),
            other => ApiError::Internal(other.to_string()),
        })?;

    Ok(ok(SignalDeliveryResponse::from(delivery)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::post;
    use serde_json::{Value, json};

    use ironflow_store::entities::SignalFilter;

    use crate::routes::signals::test_support::{api_key_header, call, jwt_header, test_state};

    use super::*;

    /// `POST /` with `body` through a router serving only this route.
    async fn post_signal(state: &AppState, auth: Option<&str>, body: Value) -> (StatusCode, Value) {
        let router = Router::new()
            .route("/", post(send_signal))
            .with_state(state.clone());
        call(router, "POST", "/", auth, Some(body)).await
    }

    fn demo_signal() -> Value {
        json!({"name": "demo.done", "key": "k1", "payload": {}})
    }

    #[tokio::test]
    async fn send_signal_without_waiter_returns_200() {
        let (state, admin, _member) = test_state().await;
        let auth = jwt_header(&admin, &state);

        let (status, body) = post_signal(&state, Some(&auth), demo_signal()).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["duplicate"], false);
        assert_eq!(body["data"]["resumed"], json!([]));
        assert_eq!(body["data"]["rejected"], json!([]));
        let stored = state
            .store
            .list_signals(SignalFilter::default(), 1, 20)
            .await
            .expect("list");
        assert_eq!(stored.total, 1);
        assert_eq!(body["data"]["signal_id"], json!(stored.items[0].id));
    }

    #[tokio::test]
    async fn send_signal_unauthenticated_returns_401() {
        let (state, _admin, _member) = test_state().await;
        let (status, _) = post_signal(&state, None, demo_signal()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn send_signal_member_jwt_returns_403() {
        let (state, _admin, member) = test_state().await;
        let auth = jwt_header(&member, &state);
        let (status, _) = post_signal(&state, Some(&auth), demo_signal()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn send_signal_api_key_without_scope_returns_403() {
        let (state, admin, _member) = test_state().await;
        let auth = api_key_header(&admin, vec![ApiKeyScope::RunsRead], &state).await;
        let (status, body) = post_signal(&state, Some(&auth), demo_signal()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["code"], "INSUFFICIENT_SCOPE");
    }

    #[tokio::test]
    async fn send_signal_with_signals_send_scope_succeeds() {
        let (state, admin, _member) = test_state().await;
        let auth = api_key_header(&admin, vec![ApiKeyScope::SignalsSend], &state).await;
        let (status, body) = post_signal(&state, Some(&auth), demo_signal()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["duplicate"], false);
        assert_eq!(body["data"]["resumed"], json!([]));
    }

    #[tokio::test]
    async fn send_signal_empty_key_returns_400() {
        let (state, admin, _member) = test_state().await;
        let auth = jwt_header(&admin, &state);
        let body = json!({"name": "demo.done", "key": "  ", "payload": {}});
        let (status, _) = post_signal(&state, Some(&auth), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn send_signal_invalid_fields_return_400() {
        let (state, admin, _member) = test_state().await;
        let auth = jwt_header(&admin, &state);
        let long = "x".repeat(MAX_SIGNAL_FIELD_LEN + 1);
        let cases = [
            json!({"name": "", "key": "k1", "payload": {}}),
            json!({"name": long, "key": "k1", "payload": {}}),
            json!({"name": "demo.done", "key": long, "payload": {}}),
            json!({"name": "demo.done", "key": "k1", "payload": {}, "idempotency_id": ""}),
            json!({"name": "demo.done", "key": "k1", "payload": {}, "idempotency_id": long}),
        ];
        for case in cases {
            let (status, _) = post_signal(&state, Some(&auth), case.clone()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{case}");
        }
    }

    #[tokio::test]
    async fn send_signal_duplicate_returns_duplicate_true() {
        let (state, admin, _member) = test_state().await;
        let auth = jwt_header(&admin, &state);
        let body = json!({
            "name": "demo.done",
            "key": "k1",
            "payload": {},
            "idempotency_id": "delivery-1"
        });

        let (status, first) = post_signal(&state, Some(&auth), body.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["data"]["duplicate"], false);

        let (status, second) = post_signal(&state, Some(&auth), body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second["data"]["duplicate"], true);
        assert_eq!(second["data"]["signal_id"], first["data"]["signal_id"]);
    }
}
