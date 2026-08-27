//! SSE endpoint for per-run workflow event streaming.

use std::convert::Infallible;
use std::pin::Pin;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::stream::{Stream, StreamExt};
use serde::Deserialize;
use serde::de::{self, Deserializer};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::WorkflowEvent;

/// Deserialize a comma-separated string into `Option<Vec<String>>`.
fn deserialize_comma_strings<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(raw) => {
            let all_types = [
                WorkflowEvent::STEP_STARTED,
                WorkflowEvent::STEP_COMPLETED,
                WorkflowEvent::STEP_FAILED,
                WorkflowEvent::APPROVAL_REQUIRED,
                WorkflowEvent::AGENT_STEP_TOKENS_USED,
            ];

            let kinds: Vec<String> = raw
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| {
                    if all_types.contains(&s) {
                        Ok(s.to_string())
                    } else {
                        Err(de::Error::custom(format!(
                            "unknown workflow event type: {s}"
                        )))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;

            Ok(Some(kinds))
        }
    }
}

/// Query parameters for the per-run SSE events endpoint.
///
/// # Examples
///
/// ```
/// use ironflow_api::routes::run_events::RunEventsQuery;
///
/// let query = RunEventsQuery { types: None };
/// ```
#[derive(Debug, Deserialize)]
pub struct RunEventsQuery {
    /// Comma-separated list of workflow event types to include
    /// (e.g. `?types=step_started,step_completed`).
    #[serde(default, deserialize_with = "deserialize_comma_strings")]
    pub types: Option<Vec<String>>,
}

/// `GET /api/v1/runs/{id}/events` -- per-run Server-Sent Events stream.
///
/// Streams [`WorkflowEvent`]s for a specific workflow run in real time.
/// Supports optional filtering via `?types=step_started,step_completed`.
///
/// Each SSE message has:
/// - `event:` set to the event type (e.g. `step_started`)
/// - `data:` JSON-serialized event payload
///
/// A keep-alive comment is sent every 30 seconds.
///
/// # Errors
///
/// Returns 401 if the request is not authenticated.
/// Returns 404 if the run does not exist.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/runs/{id}/events",
        tags = ["runs"],
        params(
            ("id" = Uuid, Path, description = "Run ID"),
            ("types" = Option<String>, Query, description = "Comma-separated workflow event types to filter (e.g. step_started,step_completed)")
        ),
        responses(
            (status = 200, description = "SSE stream of workflow events"),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn run_events(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<RunEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    state.get_run_or_404(id).await?;

    let type_filter = query.types;

    let stream: Pin<Box<dyn Stream<Item = Result<SseEvent, Infallible>> + Send>> = match state
        .event_bus
    {
        Some(ref bus) => {
            let receiver = bus.subscribe(id);

            Box::pin(BroadcastStream::new(receiver).filter_map(
                move |result: Result<WorkflowEvent, _>| {
                    let type_filter = type_filter.clone();
                    async move {
                        let event = result.ok()?;

                        if let Some(ref kinds) = type_filter {
                            let event_type = event.event_type();
                            if !kinds.iter().any(|k| k == event_type) {
                                return None;
                            }
                        }

                        let data = serde_json::to_string(&event).ok()?;
                        let sse_event = SseEvent::default().event(event.event_type()).data(data);

                        Some(Ok::<_, Infallible>(sse_event))
                    }
                },
            ))
        }
        None => Box::pin(futures_util::stream::empty()),
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(30))))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::routing::get;
    use chrono::Utc;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::{Event, WorkflowEvent, WorkflowEventBus};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use serde_json::json;
    use tokio::io::AsyncBufReadExt;
    use tokio::io::BufReader;
    use tokio::net::TcpListener;
    use tokio::sync::broadcast;
    use tokio::time::{sleep, timeout};
    use uuid::Uuid;

    use super::run_events;
    use crate::state::AppState;

    fn test_state_with_bus() -> (AppState, WorkflowEventBus) {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(16);
        let bus = WorkflowEventBus::new();
        let state = AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
        .with_event_bus(bus.clone());
        (state, bus)
    }

    fn make_auth_token(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    async fn create_run(state: &AppState) -> Uuid {
        state
            .store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run()
            .id
    }

    async fn start_sse_server(state: AppState) -> (String, String) {
        let auth = make_auth_token(&state);
        let app = Router::new()
            .route("/{id}/events", get(run_events))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, auth)
    }

    async fn connect_sse(addr: &str, path: &str, auth: &str) -> BufReader<tokio::net::TcpStream> {
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (reader, mut writer) = stream.into_split();

        use tokio::io::AsyncWriteExt;
        writer
            .write_all(
                format!(
                    "GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\nAuthorization: {auth}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        BufReader::new(reader.reunite(writer).unwrap())
    }

    async fn read_until_contains(
        reader: &mut BufReader<tokio::net::TcpStream>,
        needle: &str,
        dur: Duration,
    ) -> String {
        let mut accumulated = String::new();
        let result = timeout(dur, async {
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).await.unwrap();
                if n == 0 {
                    break;
                }
                accumulated.push_str(&line);
                if accumulated.contains(needle) {
                    break;
                }
            }
        })
        .await;
        if result.is_err() {
            panic!("timeout waiting for '{needle}' in SSE stream. Data so far:\n{accumulated}");
        }
        accumulated
    }

    #[tokio::test]
    async fn sse_stream_receives_workflow_events() {
        let (state, bus) = test_state_with_bus();
        let run_id = create_run(&state).await;
        let (addr, auth) = start_sse_server(state).await;

        let mut reader = connect_sse(&addr, &format!("/{run_id}/events"), &auth).await;
        sleep(Duration::from_millis(50)).await;

        bus.publish(
            run_id,
            WorkflowEvent::StepStarted {
                step_name: "build".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            },
        );

        let text = read_until_contains(&mut reader, "build", Duration::from_secs(5)).await;

        assert!(text.contains("event: step_started"));
        assert!(text.contains("build"));
    }

    #[tokio::test]
    async fn returns_404_for_unknown_run() {
        let (state, _bus) = test_state_with_bus();
        let (addr, auth) = start_sse_server(state).await;

        let unknown = Uuid::nil();
        let mut reader = connect_sse(&addr, &format!("/{unknown}/events"), &auth).await;

        let text = read_until_contains(&mut reader, "404", Duration::from_secs(5)).await;
        assert!(text.contains("404"));
    }

    #[tokio::test]
    async fn rejects_unauthenticated() {
        let (state, _bus) = test_state_with_bus();
        let run_id = create_run(&state).await;
        let (addr, _auth) = start_sse_server(state).await;

        let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let (reader, mut writer) = stream.into_split();

        use tokio::io::AsyncWriteExt;
        writer
            .write_all(
                format!(
                    "GET /{run_id}/events HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let mut buf_reader = BufReader::new(reader.reunite(writer).unwrap());
        let text = read_until_contains(&mut buf_reader, "401", Duration::from_secs(5)).await;
        assert!(text.contains("401"));
    }

    #[tokio::test]
    async fn filters_by_event_type() {
        let (state, bus) = test_state_with_bus();
        let run_id = create_run(&state).await;
        let (addr, auth) = start_sse_server(state).await;

        let mut reader = connect_sse(
            &addr,
            &format!("/{run_id}/events?types=step_completed"),
            &auth,
        )
        .await;
        sleep(Duration::from_millis(50)).await;

        bus.publish(
            run_id,
            WorkflowEvent::StepStarted {
                step_name: "build".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            },
        );
        bus.publish(
            run_id,
            WorkflowEvent::StepCompleted {
                step_name: "build".to_string(),
                step_index: 0,
                duration_ms: 1234,
                output_summary: None,
            },
        );

        let text = read_until_contains(&mut reader, "step_completed", Duration::from_secs(5)).await;

        assert!(text.contains("step_completed"));
        assert!(!text.contains("event: step_started"));
    }

    #[tokio::test]
    async fn events_isolated_between_runs() {
        let (state, bus) = test_state_with_bus();
        let run_a = create_run(&state).await;
        let run_b = create_run(&state).await;
        let (addr, auth) = start_sse_server(state).await;

        let mut reader_a = connect_sse(&addr, &format!("/{run_a}/events"), &auth).await;
        sleep(Duration::from_millis(50)).await;

        bus.publish(
            run_b,
            WorkflowEvent::StepStarted {
                step_name: "only-for-b".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            },
        );
        bus.publish(
            run_a,
            WorkflowEvent::StepStarted {
                step_name: "only-for-a".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            },
        );

        let text = read_until_contains(&mut reader_a, "only-for-a", Duration::from_secs(5)).await;

        assert!(text.contains("only-for-a"));
        assert!(!text.contains("only-for-b"));
    }
}
