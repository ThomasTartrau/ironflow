//! SSE endpoint for real-time event streaming.

use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::stream::{Stream, StreamExt};
use serde::Deserialize;
use serde::de::{self, Deserializer};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::Event;

use crate::state::AppState;

/// Strongly-typed event kind matching [`Event`] variants.
///
/// Serializes to/from the same `snake_case` strings as
/// [`Event::event_type()`].
///
/// # Examples
///
/// ```
/// use ironflow_api::routes::events::EventKind;
///
/// let kind: EventKind = "run_status_changed".parse().unwrap();
/// assert_eq!(kind, EventKind::RunStatusChanged);
/// assert_eq!(kind.as_str(), "run_status_changed");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(serde::Serialize, utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", serde(rename_all = "snake_case"))]
pub enum EventKind {
    /// [`Event::RunCreated`]
    RunCreated,
    /// [`Event::RunStatusChanged`]
    RunStatusChanged,
    /// [`Event::RunFailed`]
    RunFailed,
    /// [`Event::StepCompleted`]
    StepCompleted,
    /// [`Event::StepFailed`]
    StepFailed,
    /// [`Event::ApprovalRequested`]
    ApprovalRequested,
    /// [`Event::ApprovalGranted`]
    ApprovalGranted,
    /// [`Event::ApprovalRejected`]
    ApprovalRejected,
    /// [`Event::UserSignedIn`]
    UserSignedIn,
    /// [`Event::UserSignedUp`]
    UserSignedUp,
    /// [`Event::UserSignedOut`]
    UserSignedOut,
}

impl EventKind {
    /// Returns the wire-format string for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunCreated => Event::RUN_CREATED,
            Self::RunStatusChanged => Event::RUN_STATUS_CHANGED,
            Self::RunFailed => Event::RUN_FAILED,
            Self::StepCompleted => Event::STEP_COMPLETED,
            Self::StepFailed => Event::STEP_FAILED,
            Self::ApprovalRequested => Event::APPROVAL_REQUESTED,
            Self::ApprovalGranted => Event::APPROVAL_GRANTED,
            Self::ApprovalRejected => Event::APPROVAL_REJECTED,
            Self::UserSignedIn => Event::USER_SIGNED_IN,
            Self::UserSignedUp => Event::USER_SIGNED_UP,
            Self::UserSignedOut => Event::USER_SIGNED_OUT,
        }
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EventKind {
    type Err = InvalidEventKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "run_created" => Ok(Self::RunCreated),
            "run_status_changed" => Ok(Self::RunStatusChanged),
            "run_failed" => Ok(Self::RunFailed),
            "step_completed" => Ok(Self::StepCompleted),
            "step_failed" => Ok(Self::StepFailed),
            "approval_requested" => Ok(Self::ApprovalRequested),
            "approval_granted" => Ok(Self::ApprovalGranted),
            "approval_rejected" => Ok(Self::ApprovalRejected),
            "user_signed_in" => Ok(Self::UserSignedIn),
            "user_signed_up" => Ok(Self::UserSignedUp),
            "user_signed_out" => Ok(Self::UserSignedOut),
            _ => Err(InvalidEventKind(s.to_string())),
        }
    }
}

/// Error returned when parsing an unknown event kind string.
#[derive(Debug, Clone)]
pub struct InvalidEventKind(pub String);

impl fmt::Display for InvalidEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown event kind: {}", self.0)
    }
}

impl std::error::Error for InvalidEventKind {}

/// Deserialize a comma-separated string into `Option<Vec<EventKind>>`.
fn deserialize_comma_event_kinds<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<EventKind>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(raw) => {
            let kinds: Result<Vec<EventKind>, _> = raw
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(EventKind::from_str)
                .collect();
            kinds.map(Some).map_err(de::Error::custom)
        }
    }
}

/// Query parameters for the SSE events endpoint.
///
/// Both fields are optional. When set, only matching events are streamed.
///
/// # Examples
///
/// ```
/// use ironflow_api::routes::events::EventsQuery;
///
/// let query = EventsQuery {
///     run_id: None,
///     types: None,
/// };
/// ```
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Only stream events related to this run.
    pub run_id: Option<Uuid>,
    /// Comma-separated list of event types to include (e.g. `?types=run_status_changed,step_completed`).
    #[serde(default, deserialize_with = "deserialize_comma_event_kinds")]
    pub types: Option<Vec<EventKind>>,
}

/// Extract the `run_id` from an event, if the variant carries one.
fn event_run_id(event: &Event) -> Option<Uuid> {
    match event {
        Event::RunCreated { run_id, .. }
        | Event::RunStatusChanged { run_id, .. }
        | Event::RunFailed { run_id, .. }
        | Event::StepCompleted { run_id, .. }
        | Event::StepFailed { run_id, .. }
        | Event::ApprovalRequested { run_id, .. }
        | Event::ApprovalGranted { run_id, .. }
        | Event::ApprovalRejected { run_id, .. } => Some(*run_id),
        Event::UserSignedIn { .. } | Event::UserSignedUp { .. } | Event::UserSignedOut { .. } => {
            None
        }
    }
}

/// `GET /api/v1/events` -- Server-Sent Events stream.
///
/// Streams domain events in real time. Supports optional filtering:
/// - `?run_id=<uuid>` -- only events for that run
/// - `?types=run_status_changed,step_completed` -- only those event types
///
/// Each SSE message has:
/// - `event:` set to the event type (e.g. `run_status_changed`)
/// - `data:` JSON-serialized event payload
///
/// A keep-alive comment is sent every 30 seconds.
///
/// # Errors
///
/// Returns 401 if the request is not authenticated.
pub async fn events(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let receiver = state.event_sender.subscribe();
    let type_filter = query.types;

    let stream = BroadcastStream::new(receiver).filter_map(move |result: Result<Event, _>| {
        let type_filter = type_filter.clone();
        let run_id_filter = query.run_id;
        async move {
            let event = result.ok()?;

            if let Some(ref rid) = run_id_filter
                && event_run_id(&event) != Some(*rid)
            {
                return None;
            }

            if let Some(ref kinds) = type_filter {
                let event_type = event.event_type();
                if !kinds.iter().any(|k| k.as_str() == event_type) {
                    return None;
                }
            }

            let data = serde_json::to_string(&event).ok()?;
            let sse_event = SseEvent::default().event(event.event_type()).data(data);

            Some(Ok::<_, Infallible>(sse_event))
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(30)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::routing::get;
    use chrono::Utc;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::RunStatus;
    use rust_decimal::Decimal;
    use tokio::io::AsyncBufReadExt;
    use tokio::io::BufReader;
    use tokio::net::TcpListener;
    use tokio::sync::broadcast;
    use tokio::time::{sleep, timeout};
    use uuid::Uuid;

    use super::events;
    use crate::state::AppState;

    fn test_state() -> AppState {
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
        AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn sample_run_event(run_id: Uuid) -> Event {
        Event::RunStatusChanged {
            run_id,
            workflow_name: "deploy".to_string(),
            from: RunStatus::Running,
            to: RunStatus::Completed,
            error: None,
            cost_usd: Decimal::ZERO,
            duration_ms: 1000,
            at: Utc::now(),
        }
    }

    fn sample_user_event() -> Event {
        Event::UserSignedIn {
            user_id: Uuid::now_v7(),
            username: "alice".to_string(),
            at: Utc::now(),
        }
    }

    fn make_auth_token(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    /// Start a real TCP server and return (address, sender, auth header).
    async fn start_sse_server(state: AppState) -> (String, broadcast::Sender<Event>, String) {
        let sender = state.event_sender.clone();
        let auth = make_auth_token(&state);
        let app = Router::new()
            .route("/events", get(events))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, sender, auth)
    }

    /// Connect to the SSE endpoint with auth and return a line reader.
    async fn connect_sse(addr: &str, query: &str, auth: &str) -> BufReader<tokio::net::TcpStream> {
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (reader, mut writer) = stream.into_split();

        use tokio::io::AsyncWriteExt;
        writer
            .write_all(
                format!(
                    "GET /events{query} HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\nAuthorization: {auth}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        BufReader::new(reader.reunite(writer).unwrap())
    }

    /// Read all available data from the SSE stream until `needle` is found
    /// in the accumulated text, or timeout.
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
    async fn sse_stream_receives_events() {
        let state = test_state();
        let (addr, sender, auth) = start_sse_server(state).await;
        let mut reader = connect_sse(&addr, "", &auth).await;

        sleep(Duration::from_millis(50)).await;

        let run_id = Uuid::now_v7();
        sender.send(sample_run_event(run_id)).unwrap();

        let text =
            read_until_contains(&mut reader, &run_id.to_string(), Duration::from_secs(5)).await;

        assert!(text.contains("run_status_changed"));
        assert!(text.contains(&run_id.to_string()));
    }

    #[tokio::test]
    async fn sse_filters_by_run_id() {
        let state = test_state();
        let (addr, sender, auth) = start_sse_server(state).await;

        let target_run = Uuid::now_v7();
        let other_run = Uuid::now_v7();

        let mut reader = connect_sse(&addr, &format!("?run_id={target_run}"), &auth).await;
        sleep(Duration::from_millis(50)).await;

        sender.send(sample_run_event(other_run)).unwrap();
        sender.send(sample_run_event(target_run)).unwrap();

        let text =
            read_until_contains(&mut reader, &target_run.to_string(), Duration::from_secs(5)).await;

        assert!(text.contains(&target_run.to_string()));
        assert!(!text.contains(&other_run.to_string()));
    }

    #[tokio::test]
    async fn sse_filters_by_event_type() {
        let state = test_state();
        let (addr, sender, auth) = start_sse_server(state).await;

        let mut reader = connect_sse(&addr, "?types=user_signed_in", &auth).await;
        sleep(Duration::from_millis(50)).await;

        let run_id = Uuid::now_v7();
        sender.send(sample_run_event(run_id)).unwrap();
        sender.send(sample_user_event()).unwrap();

        let text = read_until_contains(&mut reader, "user_signed_in", Duration::from_secs(5)).await;

        assert!(text.contains("user_signed_in"));
        assert!(!text.contains("run_status_changed"));
    }

    #[tokio::test]
    async fn sse_returns_correct_content_type() {
        let state = test_state();
        let (addr, _sender, auth) = start_sse_server(state).await;
        let mut reader = connect_sse(&addr, "", &auth).await;

        let text =
            read_until_contains(&mut reader, "text/event-stream", Duration::from_secs(5)).await;

        assert!(text.contains("text/event-stream"));
    }

    #[tokio::test]
    async fn sse_rejects_unauthenticated() {
        let state = test_state();
        let (addr, _sender, _auth) = start_sse_server(state).await;
        // Connect without auth header
        let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let (reader, mut writer) = stream.into_split();

        use tokio::io::AsyncWriteExt;
        writer
            .write_all(
                format!(
                    "GET /events HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let mut buf_reader = BufReader::new(reader.reunite(writer).unwrap());
        let text = read_until_contains(&mut buf_reader, "401", Duration::from_secs(5)).await;

        assert!(text.contains("401"));
    }
}
