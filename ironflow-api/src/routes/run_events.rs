//! SSE endpoint for per-run workflow event streaming.
//!
//! A stream past the caps of [`SseLimits`](crate::sse_limiter::SseLimits) is
//! refused with `429 SSE_CONNECTION_LIMIT`. The stream of a run ends once the
//! run reaches a terminal status; a run already terminal gets a stream that
//! ends at once.

use std::convert::Infallible;
use std::pin::Pin;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::stream::{Stream, StreamExt, empty};
use serde::Deserialize;
use serde::de::{self, Deserializer};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::authorization::AuthorizeExt;
use crate::error::ApiError;
use crate::state::AppState;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::WorkflowEvent;
use ironflow_store::entities::ApiKeyScope;

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
                WorkflowEvent::INPUT_REQUIRED,
                WorkflowEvent::AGENT_STEP_TOKENS_USED,
                WorkflowEvent::AGENT_STEP_RESUMED,
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
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<RunEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    auth.require(ApiKeyScope::RunsRead, false)?;
    let run = state.get_run_or_404(id).await?;
    let connection = state.sse_limiter.try_acquire(auth.user_id)?;

    let type_filter = query.types;

    let stream: Pin<Box<dyn Stream<Item = Result<SseEvent, Infallible>> + Send>> = match state
        .event_bus
    {
        // A terminal run publishes nothing more, and its bus channel has been
        // released: subscribing would re-create a channel nobody removes.
        Some(ref bus) if !run.status.state.is_terminal() => {
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
        _ => Box::pin(empty()),
    };

    // The stream owns the slot: axum drops it when the client disconnects.
    let stream = stream.map(move |item| {
        let _held = &connection;
        item
    });

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
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::{
        Event, WorkflowAgentStepResumedEvent, WorkflowEvent, WorkflowEventBus,
        WorkflowStepCompletedEvent, WorkflowStepStartedEvent,
    };
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::broadcast;
    use tokio::time::{sleep, timeout};
    use uuid::Uuid;

    use super::run_events;
    use crate::routes::test_helpers::create_user_auth_header;
    use crate::sse_limiter::SseLimits;
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
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run()
            .id
    }

    async fn start_sse_server(state: AppState) -> (String, String) {
        let auth = create_user_auth_header(&state, "testuser", false).await;
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

    async fn connect_sse(addr: &str, path: &str, auth: &str) -> BufReader<TcpStream> {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (reader, mut writer) = stream.into_split();

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

    /// Reads and discards the HTTP status line and headers, blocking until
    /// the blank line that terminates them.
    ///
    /// By the time a client can read any response bytes, `run_events` has
    /// already called `bus.subscribe`, so this is a deterministic
    /// synchronization point for tests that publish events right after
    /// connecting -- unlike a fixed `sleep`, it cannot race under load.
    async fn wait_for_response_headers(reader: &mut BufReader<TcpStream>) {
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await.unwrap();
            if n == 0 || line == "\r\n" || line == "\n" {
                break;
            }
        }
    }

    async fn read_until_contains(
        reader: &mut BufReader<TcpStream>,
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
        wait_for_response_headers(&mut reader).await;

        bus.publish(
            run_id,
            WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                step_name: "build".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            }),
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

        let stream = TcpStream::connect(&addr).await.unwrap();
        let (reader, mut writer) = stream.into_split();

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
        wait_for_response_headers(&mut reader).await;

        bus.publish(
            run_id,
            WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                step_name: "build".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            }),
        );
        bus.publish(
            run_id,
            WorkflowEvent::StepCompleted(WorkflowStepCompletedEvent {
                step_name: "build".to_string(),
                step_index: 0,
                duration_ms: 1234,
                output_summary: None,
            }),
        );

        let text = read_until_contains(&mut reader, "step_completed", Duration::from_secs(5)).await;

        assert!(text.contains("step_completed"));
        assert!(!text.contains("event: step_started"));
    }

    #[tokio::test]
    async fn filters_by_agent_step_resumed_event_type() {
        let (state, bus) = test_state_with_bus();
        let run_id = create_run(&state).await;
        let (addr, auth) = start_sse_server(state).await;

        let mut reader = connect_sse(
            &addr,
            &format!("/{run_id}/events?types=agent_step_resumed"),
            &auth,
        )
        .await;
        wait_for_response_headers(&mut reader).await;

        bus.publish(
            run_id,
            WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                step_name: "review".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            }),
        );
        bus.publish(
            run_id,
            WorkflowEvent::AgentStepResumed(WorkflowAgentStepResumedEvent {
                step_name: "review".to_string(),
                step_index: 0,
                session_id: "0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d".to_string(),
                timestamp: Utc::now(),
            }),
        );

        let text = read_until_contains(
            &mut reader,
            "0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d",
            Duration::from_secs(5),
        )
        .await;

        assert!(text.contains("event: agent_step_resumed"));
        assert!(!text.contains("event: step_started"));
    }

    #[tokio::test]
    async fn events_isolated_between_runs() {
        let (state, bus) = test_state_with_bus();
        let run_a = create_run(&state).await;
        let run_b = create_run(&state).await;
        let (addr, auth) = start_sse_server(state).await;

        let mut reader_a = connect_sse(&addr, &format!("/{run_a}/events"), &auth).await;
        wait_for_response_headers(&mut reader_a).await;

        bus.publish(
            run_b,
            WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                step_name: "only-for-b".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            }),
        );
        bus.publish(
            run_a,
            WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                step_name: "only-for-a".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            }),
        );

        let text = read_until_contains(&mut reader_a, "only-for-a", Duration::from_secs(5)).await;

        assert!(text.contains("only-for-a"));
        assert!(!text.contains("only-for-b"));
    }

    /// Read the HTTP status line of a response.
    async fn status_line(reader: &mut BufReader<TcpStream>) -> String {
        read_until_contains(reader, "\r\n", Duration::from_secs(5)).await
    }

    /// Read the headers left after the status line, then the body they announce.
    async fn read_body(reader: &mut BufReader<TcpStream>) -> String {
        timeout(Duration::from_secs(5), async {
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if line.is_empty() || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).await.unwrap();
            String::from_utf8(body).unwrap()
        })
        .await
        .expect("timeout reading the response body")
    }

    fn limited_state(per_principal: usize) -> (AppState, WorkflowEventBus) {
        let (state, bus) = test_state_with_bus();
        let state = state.with_sse_limits(SseLimits {
            per_principal: Some(per_principal),
            global: None,
        });
        (state, bus)
    }

    #[tokio::test]
    async fn run_events_returns_429_past_the_per_principal_limit() {
        let (state, _bus) = limited_state(2);
        let run_id = create_run(&state).await;
        let other_auth = create_user_auth_header(&state, "otheruser", false).await;
        let (addr, auth) = start_sse_server(state).await;
        let path = format!("/{run_id}/events");

        let mut first = connect_sse(&addr, &path, &auth).await;
        assert!(status_line(&mut first).await.starts_with("HTTP/1.1 200"));
        let mut second = connect_sse(&addr, &path, &auth).await;
        assert!(status_line(&mut second).await.starts_with("HTTP/1.1 200"));

        let mut third = connect_sse(&addr, &path, &auth).await;
        let status = status_line(&mut third).await;
        assert!(status.starts_with("HTTP/1.1 429"), "status line: {status}");
        let body = read_body(&mut third).await;
        assert!(body.contains("SSE_CONNECTION_LIMIT"), "body: {body}");

        let mut other = connect_sse(&addr, &path, &other_auth).await;
        let status = status_line(&mut other).await;
        assert!(
            status.starts_with("HTTP/1.1 200"),
            "another user keeps its own quota: {status}"
        );
    }

    #[tokio::test]
    async fn run_events_frees_the_slot_when_the_client_disconnects() {
        let (state, _bus) = limited_state(1);
        let run_id = create_run(&state).await;
        let (addr, auth) = start_sse_server(state).await;
        let path = format!("/{run_id}/events");

        let mut first = connect_sse(&addr, &path, &auth).await;
        assert!(status_line(&mut first).await.starts_with("HTTP/1.1 200"));
        let mut refused = connect_sse(&addr, &path, &auth).await;
        assert!(status_line(&mut refused).await.starts_with("HTTP/1.1 429"));

        drop(first);

        let reopened = timeout(Duration::from_secs(5), async {
            loop {
                let mut retry = connect_sse(&addr, &path, &auth).await;
                if status_line(&mut retry).await.starts_with("HTTP/1.1 200") {
                    return retry;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(reopened.is_ok(), "the slot was not freed after disconnect");
    }

    #[tokio::test]
    async fn run_events_unknown_run_is_404_before_the_limit() {
        let (state, _bus) = limited_state(0);
        let (addr, auth) = start_sse_server(state).await;

        let unknown = Uuid::nil();
        let mut reader = connect_sse(&addr, &format!("/{unknown}/events"), &auth).await;

        let status = status_line(&mut reader).await;
        assert!(status.starts_with("HTTP/1.1 404"), "status line: {status}");
    }

    #[tokio::test]
    async fn run_events_on_terminal_run_ends_without_creating_a_channel() {
        let (state, bus) = test_state_with_bus();
        let run_id = create_run(&state).await;
        state
            .store
            .update_run_status(run_id, RunStatus::Running)
            .await
            .unwrap();
        state
            .store
            .update_run_status(run_id, RunStatus::Completed)
            .await
            .unwrap();
        let (addr, auth) = start_sse_server(state).await;

        let mut stream = TcpStream::connect(&addr).await.unwrap();
        stream
            .write_all(
                format!(
                    "GET /{run_id}/events HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\nAuthorization: {auth}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let mut response = String::new();
        timeout(Duration::from_secs(5), stream.read_to_string(&mut response))
            .await
            .expect("the stream of a terminal run must end")
            .unwrap();

        assert!(response.starts_with("HTTP/1.1 200"), "response: {response}");
        assert!(!bus.has_channel(run_id));
    }

    #[tokio::test]
    async fn run_events_on_live_run_subscribes_to_the_bus() {
        let (state, bus) = test_state_with_bus();
        let run_id = create_run(&state).await;
        let (addr, auth) = start_sse_server(state).await;

        let mut reader = connect_sse(&addr, &format!("/{run_id}/events"), &auth).await;
        wait_for_response_headers(&mut reader).await;

        assert!(bus.has_channel(run_id));
    }
}
