//! SSE endpoint for real-time event streaming.

use std::convert::Infallible;
use std::str::FromStr;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::stream::{Stream, StreamExt};
use serde::Deserialize;
use serde::de::{self, Deserializer};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::authorization::AuthorizeExt;
use crate::error::ApiError;
use crate::state::AppState;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::Event;
use ironflow_store::entities::ApiKeyScope;

pub use ironflow_store::entities::EventKind;

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
/// Provider Account events reach an admin session, or an API key holding the
/// `admin` scope whose owner is an admin.
///
/// # Errors
///
/// - 401 if the request is not authenticated
/// - 403 `INSUFFICIENT_SCOPE` if the API key lacks `runs_read`
pub async fn events(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    auth.require(ApiKeyScope::RunsRead, false)?;
    let receiver = state.event_sender.subscribe();
    // Provider Account events describe admin-only resources, which an API
    // key reaches only through the `admin` scope.
    let sees_provider_account_events = auth.require(ApiKeyScope::Admin, true).is_ok();
    let type_filter = query.types;

    let stream = BroadcastStream::new(receiver).filter_map(move |result: Result<Event, _>| {
        let type_filter = type_filter.clone();
        let run_id_filter = query.run_id;
        async move {
            let event = result.ok()?;

            if !sees_provider_account_events
                && matches!(
                    event,
                    Event::ProviderAccountUpdated(_) | Event::ProviderAccountUsageUpdated(_)
                )
            {
                return None;
            }

            if let Some(ref rid) = run_id_filter
                && event.run_id() != Some(*rid)
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

    let sse = Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(30)));
    Ok(sse)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::routing::get;
    use chrono::Utc;
    use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::{
        Event, ProviderAccountChange, ProviderAccountUpdatedEvent, RunStatusChangedEvent,
        UserSignedInEvent,
    };
    use ironflow_store::entities::{ApiKeyScope, NewApiKey, NewUser};
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
    use crate::routes::test_helpers::create_user_auth_header;
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
        Event::RunStatusChanged(RunStatusChangedEvent {
            run_id,
            workflow_name: "deploy".to_string(),
            from: RunStatus::Running,
            to: RunStatus::Completed,
            error: None,
            cost_usd: Decimal::ZERO,
            duration_ms: 1000,
            labels: HashMap::new(),
            at: Utc::now(),
        })
    }

    fn sample_user_event() -> Event {
        Event::UserSignedIn(UserSignedInEvent {
            user_id: Uuid::now_v7(),
            username: "alice".to_string(),
            at: Utc::now(),
        })
    }

    fn sample_provider_account_event() -> Event {
        Event::ProviderAccountUpdated(ProviderAccountUpdatedEvent {
            account_id: Uuid::now_v7(),
            name: "acct".into(),
            change: ProviderAccountChange::Updated,
            at: Utc::now(),
        })
    }

    /// A `Bearer` header for an API key holding `scopes`, owned by a fresh
    /// user who is an admin when `owner_is_admin` is set.
    ///
    /// The key is written to the store directly, so a member may hold scopes
    /// the `/api-keys` route would refuse them.
    async fn api_key_header(
        state: &AppState,
        owner_is_admin: bool,
        scopes: Vec<ApiKeyScope>,
    ) -> String {
        let suffix = Uuid::now_v7().simple().to_string();
        let owner = state
            .store
            .create_user(NewUser {
                email: format!("owner-{suffix}@example.com"),
                username: format!("owner-{suffix}"),
                password_hash: "argon2hash".to_string(),
                is_admin: Some(owner_is_admin),
            })
            .await
            .unwrap();
        let raw_key = format!("irfl_{}rest-of-secret-key", &suffix[24..]);
        let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];
        state
            .store
            .create_api_key(NewApiKey {
                user_id: owner.id,
                name: format!("key-{suffix}"),
                key_hash: password::hash(&raw_key).unwrap(),
                key_prefix: prefix.to_string(),
                scopes,
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();
        format!("Bearer {raw_key}")
    }

    /// Start a real TCP server and return (address, sender, auth header).
    async fn start_sse_server(state: AppState) -> (String, broadcast::Sender<Event>, String) {
        let sender = state.event_sender.clone();
        let auth = create_user_auth_header(&state, "testuser", false).await;
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

    /// Wait until the SSE handler has subscribed to the broadcast channel.
    ///
    /// Sending before a receiver exists makes `broadcast::Sender::send` return
    /// `SendError`, so a fixed sleep is racy on slow runners. Poll the receiver
    /// count instead.
    async fn wait_for_subscriber(sender: &broadcast::Sender<Event>) {
        let ready = timeout(Duration::from_secs(5), async {
            while sender.receiver_count() == 0 {
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(ready.is_ok(), "SSE handler did not subscribe in time");
    }

    #[tokio::test]
    async fn sse_stream_receives_events() {
        let state = test_state();
        let (addr, sender, auth) = start_sse_server(state).await;
        let mut reader = connect_sse(&addr, "", &auth).await;

        wait_for_subscriber(&sender).await;

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
        wait_for_subscriber(&sender).await;

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
        wait_for_subscriber(&sender).await;

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

    /// Stream a provider account event, then a run event, to a client
    /// authenticated with `auth`, and assert only the run event arrives.
    ///
    /// Both events go through the same channel in order: had the provider
    /// account event not been filtered out, it would arrive first.
    async fn assert_provider_account_events_hidden(state: AppState, auth: &str) {
        let (addr, sender, _jwt) = start_sse_server(state).await;
        let mut reader = connect_sse(&addr, "", auth).await;
        wait_for_subscriber(&sender).await;

        let run_id = Uuid::now_v7();
        sender.send(sample_provider_account_event()).unwrap();
        sender.send(sample_run_event(run_id)).unwrap();

        let text =
            read_until_contains(&mut reader, &run_id.to_string(), Duration::from_secs(5)).await;

        assert!(!text.contains("provider_account"));
    }

    #[tokio::test]
    async fn sse_scope_hides_provider_account_events_from_a_runs_read_key() {
        let state = test_state();
        let auth = api_key_header(&state, true, vec![ApiKeyScope::RunsRead]).await;
        assert_provider_account_events_hidden(state, &auth).await;
    }

    #[tokio::test]
    async fn sse_scope_hides_provider_account_events_from_an_admin_scope_key_of_a_member() {
        let state = test_state();
        let auth = api_key_header(&state, false, vec![ApiKeyScope::Admin]).await;
        assert_provider_account_events_hidden(state, &auth).await;
    }

    #[tokio::test]
    async fn sse_scope_streams_provider_account_events_to_an_admin_scope_key() {
        let state = test_state();
        let auth = api_key_header(&state, true, vec![ApiKeyScope::Admin]).await;
        let (addr, sender, _jwt) = start_sse_server(state).await;
        let mut reader = connect_sse(&addr, "", &auth).await;
        wait_for_subscriber(&sender).await;

        sender.send(sample_provider_account_event()).unwrap();

        let text =
            read_until_contains(&mut reader, "provider_account", Duration::from_secs(5)).await;

        assert!(text.contains("provider_account"));
    }

    #[tokio::test]
    async fn sse_scope_rejects_a_key_without_runs_read() {
        let state = test_state();
        let auth = api_key_header(&state, true, vec![ApiKeyScope::StatsRead]).await;
        let (addr, _sender, _jwt) = start_sse_server(state).await;
        let mut reader = connect_sse(&addr, "", &auth).await;

        let text = read_until_contains(&mut reader, "\r\n", Duration::from_secs(5)).await;

        assert!(text.starts_with("HTTP/1.1 403"), "status line: {text}");
    }
}
