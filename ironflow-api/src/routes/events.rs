//! SSE endpoint for real-time event streaming.

use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use chrono::Utc;
use futures_util::stream::{Stream, StreamExt, unfold};
use serde::Deserialize;
use serde::de::{self, Deserializer};
use serde_json::{json, to_string};
use tokio::select;
use tokio::sync::broadcast;
use tokio::time::{Instant, Interval, MissedTickBehavior, interval_at};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tracing::warn;
use uuid::Uuid;

use crate::authorization::AuthorizeExt;
use crate::error::ApiError;
use crate::state::AppState;
use ironflow_auth::extractor::{AuthMethod, Authenticated};
use ironflow_engine::notify::Event;
use ironflow_store::entities::ApiKeyScope;
use ironflow_store::error::StoreError;
use ironflow_store::store::Store;

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

/// Whether a caller that is not an admin may receive `event`.
///
/// An allow-list of run events. The match has no wildcard, so a new event
/// kind does not compile until it is classified here.
fn visible_to_non_admin(event: &Event) -> bool {
    match event {
        Event::RunCreated(_)
        | Event::RunStatusChanged(_)
        | Event::RunFailed(_)
        | Event::RunBudgetExceeded(_)
        | Event::RetryForced(_)
        | Event::StepCompleted(_)
        | Event::StepFailed(_)
        | Event::ApprovalRequested(_)
        | Event::ApprovalGranted(_)
        | Event::ApprovalRejected(_)
        | Event::ApprovalEscalated(_)
        | Event::LogLine(_)
        | Event::SignalAwaited(_)
        | Event::SignalReceived(_) => true,
        Event::UserSignedIn(_)
        | Event::UserSignedUp(_)
        | Event::UserSignedOut(_)
        | Event::ProviderAccountUpdated(_)
        | Event::ProviderAccountUsageUpdated(_)
        | Event::ScheduleOccurrencesMissed(_)
        | Event::WorkflowPaused(_)
        | Event::WorkflowResumed(_) => false,
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
/// A caller that is not an admin (a member session, a key owned by a member,
/// or any key without the `admin` scope) only receives run events:
/// `run_created`, `run_status_changed`, `run_failed`, `run_budget_exceeded`,
/// `retry_forced`, `step_completed`, `step_failed`, `approval_requested`,
/// `approval_granted`, `approval_rejected`, `approval_escalated`, `log_line`,
/// `signal_awaited` and `signal_received`. User sign-in, sign-up and sign-out,
/// Provider Account, schedule and workflow pause/resume events reach only an admin session, or an
/// API key holding the `admin` scope whose owner is an admin.
///
/// A client that reads slower than events are published loses the oldest
/// ones. It then receives a `lagged` event, whatever its filters, with
/// `data: {"missed": n}`: the number of events it missed. Its view is stale
/// and it should reload its state.
///
/// The principal is revalidated every 30 seconds. The stream is closed when
/// the API key is revoked, disabled, expired or lacks `runs_read`, or when
/// the user or the key owner is deleted, and it falls back to the non-admin
/// event filter when the user is demoted or the key loses its `admin` scope.
/// Within the interval, a revocation still lets events through.
///
/// # Errors
///
/// - 401 if the request is not authenticated
/// - 403 `INSUFFICIENT_SCOPE` if the API key lacks `runs_read`
/// - 429 `SSE_CONNECTION_LIMIT` if the caller (user or API key owner) or the
///   server already holds as many event streams as
///   [`SseLimits`](crate::sse_limiter::SseLimits) allows
pub async fn events(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    auth.require(ApiKeyScope::RunsRead, false)?;
    let connection = state.sse_limiter.try_acquire(auth.user_id)?;
    let receiver = state.event_sender.subscribe();
    // Account and user events describe admin-only resources: only an admin
    // session, or an `admin`-scope key owned by an admin, sees them.
    let access = if auth.require(ApiKeyScope::Admin, true).is_ok() {
        Access::Admin
    } else {
        Access::Member
    };
    let revalidation = Revalidation {
        store: state.store.clone(),
        principal: Principal::of(&auth),
        every: REVALIDATE_INTERVAL,
    };
    let stream = event_stream(receiver, query.run_id, query.types, access, revalidation);
    // The stream owns the slot: axum drops it when the client disconnects,
    // or when the stream ends because its principal lost access.
    let stream = stream.map(move |item| {
        let _held = &connection;
        item
    });

    let sse = Sse::new(stream).keep_alive(KeepAlive::new().interval(REVALIDATE_INTERVAL));
    Ok(sse)
}

/// How often an open stream sends a keep-alive and re-reads its principal's rights.
const REVALIDATE_INTERVAL: Duration = Duration::from_secs(30);

/// SSE event name sent to a client that fell behind and missed events.
const LAGGED_EVENT: &str = "lagged";

/// What an open stream's principal may still see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// The principal lost access: the stream ends.
    Closed,
    /// Only the events [`visible_to_non_admin`] lets through.
    Member,
    /// Every event.
    Admin,
}

/// Who owns an open stream, enough to re-read their rights from the store.
#[derive(Debug, Clone)]
enum Principal {
    /// A JWT session of `user_id`.
    Session { user_id: Uuid },
    /// The API key `key_id`, whose owner is read from the key itself.
    ApiKey { key_id: Uuid },
}

impl Principal {
    /// The principal behind `auth`.
    fn of(auth: &Authenticated) -> Self {
        match &auth.method {
            AuthMethod::Jwt { .. } => Principal::Session {
                user_id: auth.user_id,
            },
            AuthMethod::ApiKey { key_id, .. } => Principal::ApiKey { key_id: *key_id },
        }
    }

    /// What the principal may see now, read from `store`.
    ///
    /// Mirrors the connect-time checks: a key that was deleted, disabled,
    /// has expired or no longer holds `runs_read`, or whose owner (or the
    /// session's user) was deleted, closes the stream. Admin events need an
    /// admin user, and for a key the `admin` scope as well.
    async fn access(&self, store: &dyn Store) -> Result<Access, StoreError> {
        match self {
            Principal::Session { user_id } => {
                let access = match store.find_user_by_id(*user_id).await? {
                    None => Access::Closed,
                    Some(user) if user.is_admin => Access::Admin,
                    Some(_) => Access::Member,
                };
                Ok(access)
            }
            Principal::ApiKey { key_id } => {
                let Some(key) = store.find_api_key_by_id(*key_id).await? else {
                    return Ok(Access::Closed);
                };
                let expired = key.expires_at.is_some_and(|at| at < Utc::now());
                if !key.is_active
                    || expired
                    || !ApiKeyScope::has_permission(&key.scopes, &ApiKeyScope::RunsRead)
                {
                    return Ok(Access::Closed);
                }
                let Some(owner) = store.find_user_by_id(key.user_id).await? else {
                    return Ok(Access::Closed);
                };
                let admin_scope = ApiKeyScope::has_permission(&key.scopes, &ApiKeyScope::Admin);
                if owner.is_admin && admin_scope {
                    Ok(Access::Admin)
                } else {
                    Ok(Access::Member)
                }
            }
        }
    }
}

/// How an open stream re-reads its principal's rights.
struct Revalidation {
    /// Where the principal's rights are read from.
    store: Arc<dyn Store>,
    /// Whose rights are read.
    principal: Principal,
    /// Time between two reads. The first one happens one interval after connect.
    every: Duration,
}

/// The state [`event_stream`] carries from one item to the next.
struct StreamState {
    receiver: BroadcastStream<Event>,
    run_id_filter: Option<Uuid>,
    type_filter: Option<Vec<EventKind>>,
    access: Access,
    revalidation: Revalidation,
    ticker: Interval,
}

impl StreamState {
    /// Re-read the principal's rights. Returns `false` when the stream must end.
    ///
    /// A store error closes the stream too: the client's `EventSource`
    /// reconnects and is authenticated again.
    async fn revalidate(&mut self) -> bool {
        let store = self.revalidation.store.as_ref();
        match self.revalidation.principal.access(store).await {
            Ok(Access::Closed) => false,
            Ok(access) => {
                self.access = access;
                true
            }
            Err(e) => {
                warn!(error = %e, "failed to revalidate SSE principal, closing the stream");
                false
            }
        }
    }

    /// The SSE message for `result`, or `None` when the filters drop it.
    fn render(&self, result: Result<Event, BroadcastStreamRecvError>) -> Option<SseEvent> {
        let event = match result {
            Ok(event) => event,
            Err(BroadcastStreamRecvError::Lagged(missed)) => {
                let data = json!({ "missed": missed }).to_string();
                return Some(SseEvent::default().event(LAGGED_EVENT).data(data));
            }
        };

        if self.access != Access::Admin && !visible_to_non_admin(&event) {
            return None;
        }

        if let Some(ref rid) = self.run_id_filter
            && event.run_id() != Some(*rid)
        {
            return None;
        }

        if let Some(ref kinds) = self.type_filter {
            let event_type = event.event_type();
            if !kinds.iter().any(|k| k.as_str() == event_type) {
                return None;
            }
        }

        let data = to_string(&event).ok()?;
        Some(SseEvent::default().event(event.event_type()).data(data))
    }
}

/// Turn the broadcast `receiver` into the SSE stream a client asked for.
///
/// A receiver that falls behind the channel capacity loses the oldest
/// events: the client gets a [`LAGGED_EVENT`] carrying how many it missed.
/// That event bypasses the filters, since which events were lost is unknown.
///
/// The principal's rights are re-read every `revalidation.every`: the stream
/// ends when they are gone, and `access` follows a demotion or a promotion.
fn event_stream(
    receiver: broadcast::Receiver<Event>,
    run_id_filter: Option<Uuid>,
    type_filter: Option<Vec<EventKind>>,
    access: Access,
    revalidation: Revalidation,
) -> impl Stream<Item = Result<SseEvent, Infallible>> {
    let mut ticker = interval_at(Instant::now() + revalidation.every, revalidation.every);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let state = StreamState {
        receiver: BroadcastStream::new(receiver),
        run_id_filter,
        type_filter,
        access,
        revalidation,
        ticker,
    };

    unfold(state, |mut state| async move {
        loop {
            // Biased so a due revalidation runs before the next event is
            // delivered; the ticker fires once per interval, so events never starve.
            select! {
                biased;
                _ = state.ticker.tick() => {
                    if !state.revalidate().await {
                        return None;
                    }
                }
                result = state.receiver.next() => {
                    if let Some(sse_event) = state.render(result?) {
                        return Some((Ok::<_, Infallible>(sse_event), state));
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::body::Body;
    use axum::response::IntoResponse;
    use axum::response::sse::Sse;
    use axum::routing::get;
    use chrono::Utc;
    use http_body_util::BodyExt;
    use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::{
        Event, ProviderAccountChange, ProviderAccountUpdatedEvent, RunStatusChangedEvent,
        UserSignedInEvent, UserSignedOutEvent, UserSignedUpEvent,
    };
    use ironflow_store::entities::{ApiKeyScope, ApiKeyUpdate, NewApiKey, NewUser, User};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::RunStatus;
    use rust_decimal::Decimal;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::broadcast;
    use tokio::time::{sleep, timeout};
    use uuid::Uuid;

    use super::{
        Access, EventKind, Principal, REVALIDATE_INTERVAL, Revalidation, event_stream, events,
        visible_to_non_admin,
    };
    use crate::routes::test_helpers::create_user_auth_header;
    use crate::sse_limiter::SseLimits;
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

    fn sample_user_signed_up_event() -> Event {
        Event::UserSignedUp(UserSignedUpEvent {
            user_id: Uuid::now_v7(),
            username: "bob".to_string(),
            at: Utc::now(),
        })
    }

    fn sample_user_signed_out_event() -> Event {
        Event::UserSignedOut(UserSignedOutEvent {
            user_id: Uuid::now_v7(),
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
        let owner = stored_user(state, owner_is_admin).await;
        let (_, header) = stored_key(state, &owner, scopes).await;
        header
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
    async fn connect_sse(addr: &str, query: &str, auth: &str) -> BufReader<TcpStream> {
        let stream = TcpStream::connect(addr).await.unwrap();
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
        // User events only reach an admin.
        let admin = create_user_auth_header(&state, "admin", true).await;
        let (addr, sender, _member) = start_sse_server(state).await;

        let mut reader = connect_sse(&addr, "?types=user_signed_in", &admin).await;
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
        let stream = TcpStream::connect(&addr).await.unwrap();
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

    /// Stream the three user events, then a run event, to a client
    /// authenticated with `auth`, and assert only the run event arrives.
    async fn assert_user_events_hidden(state: AppState, auth: Option<&str>) {
        let (addr, sender, member) = start_sse_server(state).await;
        let mut reader = connect_sse(&addr, "", auth.unwrap_or(&member)).await;
        wait_for_subscriber(&sender).await;

        let run_id = Uuid::now_v7();
        sender.send(sample_user_event()).unwrap();
        sender.send(sample_user_signed_up_event()).unwrap();
        sender.send(sample_user_signed_out_event()).unwrap();
        sender.send(sample_run_event(run_id)).unwrap();

        let text =
            read_until_contains(&mut reader, &run_id.to_string(), Duration::from_secs(5)).await;

        assert!(!text.contains("user_signed_in"), "stream: {text}");
        assert!(!text.contains("user_signed_up"), "stream: {text}");
        assert!(!text.contains("user_signed_out"), "stream: {text}");
        assert!(text.contains("run_status_changed"));
    }

    #[tokio::test]
    async fn sse_hides_user_sign_in_sign_up_and_sign_out_events_from_a_member() {
        assert_user_events_hidden(test_state(), None).await;
    }

    #[tokio::test]
    async fn sse_hides_user_events_from_a_runs_read_key_of_an_admin() {
        let state = test_state();
        let auth = api_key_header(&state, true, vec![ApiKeyScope::RunsRead]).await;
        assert_user_events_hidden(state, Some(&auth)).await;
    }

    #[tokio::test]
    async fn sse_streams_user_events_to_an_admin() {
        let state = test_state();
        let admin = create_user_auth_header(&state, "admin", true).await;
        let (addr, sender, _member) = start_sse_server(state).await;
        let mut reader = connect_sse(&addr, "", &admin).await;
        wait_for_subscriber(&sender).await;

        sender.send(sample_user_event()).unwrap();

        let text = read_until_contains(&mut reader, "user_signed_in", Duration::from_secs(5)).await;

        assert!(text.contains("user_signed_in"));
    }

    #[test]
    fn visible_to_non_admin_allows_only_run_events() {
        assert!(visible_to_non_admin(&sample_run_event(Uuid::now_v7())));
        assert!(!visible_to_non_admin(&sample_user_event()));
        assert!(!visible_to_non_admin(&sample_user_signed_up_event()));
        assert!(!visible_to_non_admin(&sample_user_signed_out_event()));
        assert!(!visible_to_non_admin(&sample_provider_account_event()));
    }

    /// A channel of capacity 2 whose receiver already missed 3 of the 5
    /// events sent: the lag is deterministic, the stream is not polled yet.
    fn lagging_receiver() -> (broadcast::Sender<Event>, broadcast::Receiver<Event>) {
        let (sender, receiver) = broadcast::channel::<Event>(2);
        for _ in 0..5 {
            sender.send(sample_run_event(Uuid::now_v7())).unwrap();
        }
        (sender, receiver)
    }

    /// Read the SSE body until `needle` shows up, or time out.
    async fn read_body_until(body: &mut Body, needle: &str) -> String {
        let mut accumulated = String::new();
        let result = timeout(Duration::from_secs(5), async {
            while !accumulated.contains(needle) {
                let Some(frame) = body.frame().await else {
                    break;
                };
                if let Ok(data) = frame.unwrap().into_data() {
                    accumulated.push_str(&String::from_utf8_lossy(&data));
                }
            }
        })
        .await;
        assert!(
            result.is_ok(),
            "timeout waiting for '{needle}'. Data so far:\n{accumulated}"
        );
        accumulated
    }

    /// A revalidation of a principal no store knows, every
    /// [`REVALIDATE_INTERVAL`]: it never fires within a test.
    fn idle_revalidation() -> Revalidation {
        Revalidation {
            store: test_state().store,
            principal: Principal::Session {
                user_id: Uuid::now_v7(),
            },
            every: REVALIDATE_INTERVAL,
        }
    }

    #[tokio::test]
    async fn lagged_event_is_sent_when_the_subscriber_falls_behind() {
        let (sender, receiver) = lagging_receiver();
        let stream = event_stream(receiver, None, None, Access::Member, idle_revalidation());
        let response = Sse::new(stream).into_response();
        let mut body = response.into_body();

        let text = read_body_until(&mut body, "\"missed\"").await;
        assert!(text.contains("lagged"), "got {text}");
        assert!(text.contains("\"missed\":3"), "got {text}");

        // The stream goes on after the notice.
        let run_id = Uuid::now_v7();
        sender.send(sample_run_event(run_id)).unwrap();
        let text = read_body_until(&mut body, &run_id.to_string()).await;
        assert!(text.contains("run_status_changed"), "got {text}");
    }

    #[tokio::test]
    async fn lagged_event_bypasses_the_run_and_type_filters() {
        let (_sender, receiver) = lagging_receiver();
        let stream = event_stream(
            receiver,
            Some(Uuid::now_v7()),
            Some(vec![EventKind::UserSignedIn]),
            Access::Member,
            idle_revalidation(),
        );
        let mut body = Sse::new(stream).into_response().into_body();

        let text = read_body_until(&mut body, "\"missed\"").await;
        assert!(text.contains("lagged"), "got {text}");
        assert!(!text.contains("run_status_changed"), "got {text}");
    }

    /// Revalidation period of the stream tests: short, so no test waits 30 s.
    const FAST_REVALIDATION: Duration = Duration::from_millis(50);

    /// Store a user, admin when `is_admin` is set.
    async fn stored_user(state: &AppState, is_admin: bool) -> User {
        let suffix = Uuid::now_v7().simple().to_string();
        state
            .store
            .create_user(NewUser {
                email: format!("user-{suffix}@example.com"),
                username: format!("user-{suffix}"),
                password_hash: "argon2hash".to_string(),
                is_admin: Some(is_admin),
            })
            .await
            .unwrap()
    }

    /// Store an API key of `owner` holding `scopes`, returning its ID and a
    /// `Bearer` header for it.
    async fn stored_key(
        state: &AppState,
        owner: &User,
        scopes: Vec<ApiKeyScope>,
    ) -> (Uuid, String) {
        let suffix = Uuid::now_v7().simple().to_string();
        let raw_key = format!("irfl_{}rest-of-secret-key", &suffix[24..]);
        let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];
        let key = state
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
        (key.id, format!("Bearer {raw_key}"))
    }

    /// The SSE body of a stream opened by `principal` with `access`,
    /// revalidated every [`FAST_REVALIDATION`].
    fn revalidated_body(state: &AppState, access: Access, principal: Principal) -> Body {
        let revalidation = Revalidation {
            store: state.store.clone(),
            principal,
            every: FAST_REVALIDATION,
        };
        let receiver = state.event_sender.subscribe();
        let stream = event_stream(receiver, None, None, access, revalidation);
        Sse::new(stream).into_response().into_body()
    }

    /// Send a run event and read it back, proving the stream is open.
    async fn assert_stream_open(state: &AppState, body: &mut Body) {
        let run_id = Uuid::now_v7();
        state.event_sender.send(sample_run_event(run_id)).unwrap();
        read_body_until(body, &run_id.to_string()).await;
    }

    /// Wait until the stream ends, or fail after 5 seconds.
    async fn assert_stream_ends(body: &mut Body) {
        let ended = timeout(Duration::from_secs(5), async {
            while let Some(frame) = body.frame().await {
                frame.unwrap();
            }
        })
        .await;
        assert!(ended.is_ok(), "the stream is still open");
    }

    /// Send a user event then a run event, and return what the stream
    /// delivers up to the run event.
    async fn user_then_run_event(state: &AppState, body: &mut Body) -> String {
        let run_id = Uuid::now_v7();
        state.event_sender.send(sample_user_event()).unwrap();
        state.event_sender.send(sample_run_event(run_id)).unwrap();
        read_body_until(body, &run_id.to_string()).await
    }

    #[tokio::test]
    async fn sse_stream_closes_when_the_api_key_is_revoked() {
        let state = test_state();
        let owner = stored_user(&state, true).await;
        let (key_id, _) = stored_key(&state, &owner, vec![ApiKeyScope::RunsRead]).await;
        let mut body = revalidated_body(&state, Access::Member, Principal::ApiKey { key_id });
        assert_stream_open(&state, &mut body).await;

        state.store.delete_api_key(key_id).await.unwrap();

        assert_stream_ends(&mut body).await;
    }

    #[tokio::test]
    async fn sse_stream_closes_when_the_api_key_is_disabled() {
        let state = test_state();
        let owner = stored_user(&state, true).await;
        let (key_id, _) = stored_key(&state, &owner, vec![ApiKeyScope::RunsRead]).await;
        let mut body = revalidated_body(&state, Access::Member, Principal::ApiKey { key_id });
        assert_stream_open(&state, &mut body).await;

        state
            .store
            .update_api_key(
                key_id,
                ApiKeyUpdate {
                    is_active: Some(false),
                    ..ApiKeyUpdate::default()
                },
            )
            .await
            .unwrap();

        assert_stream_ends(&mut body).await;
    }

    #[tokio::test]
    async fn sse_stream_closes_when_the_api_key_expires() {
        let state = test_state();
        let owner = stored_user(&state, true).await;
        let (key_id, _) = stored_key(&state, &owner, vec![ApiKeyScope::RunsRead]).await;
        let mut body = revalidated_body(&state, Access::Member, Principal::ApiKey { key_id });
        assert_stream_open(&state, &mut body).await;

        state
            .store
            .update_api_key(
                key_id,
                ApiKeyUpdate {
                    expires_at: Some(Some(Utc::now())),
                    ..ApiKeyUpdate::default()
                },
            )
            .await
            .unwrap();

        assert_stream_ends(&mut body).await;
    }

    #[tokio::test]
    async fn sse_stream_closes_when_the_api_key_loses_runs_read() {
        let state = test_state();
        let owner = stored_user(&state, true).await;
        let (key_id, _) = stored_key(&state, &owner, vec![ApiKeyScope::RunsRead]).await;
        let mut body = revalidated_body(&state, Access::Member, Principal::ApiKey { key_id });
        assert_stream_open(&state, &mut body).await;

        state
            .store
            .update_api_key(
                key_id,
                ApiKeyUpdate {
                    scopes: Some(vec![ApiKeyScope::StatsRead]),
                    ..ApiKeyUpdate::default()
                },
            )
            .await
            .unwrap();

        assert_stream_ends(&mut body).await;
    }

    #[tokio::test]
    async fn sse_stream_closes_when_the_owner_is_deleted() {
        let state = test_state();
        let owner = stored_user(&state, true).await;
        let (key_id, _) = stored_key(&state, &owner, vec![ApiKeyScope::RunsRead]).await;
        let mut body = revalidated_body(&state, Access::Member, Principal::ApiKey { key_id });
        assert_stream_open(&state, &mut body).await;

        state.store.delete_user(owner.id).await.unwrap();

        assert_stream_ends(&mut body).await;
    }

    #[tokio::test]
    async fn sse_stream_closes_when_the_session_user_is_deleted() {
        let state = test_state();
        let user = stored_user(&state, false).await;
        let mut body = revalidated_body(
            &state,
            Access::Member,
            Principal::Session { user_id: user.id },
        );
        assert_stream_open(&state, &mut body).await;

        state.store.delete_user(user.id).await.unwrap();

        assert_stream_ends(&mut body).await;
    }

    #[tokio::test]
    async fn sse_stream_of_a_demoted_admin_falls_back_to_member_filtering() {
        let state = test_state();
        let admin = stored_user(&state, true).await;
        let mut body = revalidated_body(
            &state,
            Access::Admin,
            Principal::Session { user_id: admin.id },
        );
        let text = user_then_run_event(&state, &mut body).await;
        assert!(text.contains("user_signed_in"), "stream: {text}");

        state.store.update_user_role(admin.id, false).await.unwrap();
        sleep(FAST_REVALIDATION * 3).await;

        let text = user_then_run_event(&state, &mut body).await;
        assert!(!text.contains("user_signed_in"), "stream: {text}");
        assert!(text.contains("run_status_changed"), "stream: {text}");
    }

    #[tokio::test]
    async fn sse_stream_of_a_demoted_admin_key_falls_back_to_member_filtering() {
        let state = test_state();
        let admin = stored_user(&state, true).await;
        let (key_id, _) = stored_key(
            &state,
            &admin,
            vec![ApiKeyScope::RunsRead, ApiKeyScope::Admin],
        )
        .await;
        let mut body = revalidated_body(&state, Access::Admin, Principal::ApiKey { key_id });
        let text = user_then_run_event(&state, &mut body).await;
        assert!(text.contains("user_signed_in"), "stream: {text}");

        // The demotion strips the `admin` scope and keeps `runs_read`.
        state.store.update_user_role(admin.id, false).await.unwrap();
        sleep(FAST_REVALIDATION * 3).await;

        let text = user_then_run_event(&state, &mut body).await;
        assert!(!text.contains("user_signed_in"), "stream: {text}");
        assert!(text.contains("run_status_changed"), "stream: {text}");
    }

    #[tokio::test]
    async fn sse_stream_of_a_demoted_admin_with_an_admin_only_key_is_closed() {
        let state = test_state();
        let admin = stored_user(&state, true).await;
        let (key_id, _) = stored_key(&state, &admin, vec![ApiKeyScope::Admin]).await;
        let mut body = revalidated_body(&state, Access::Admin, Principal::ApiKey { key_id });
        assert_stream_open(&state, &mut body).await;

        // Left without any scope, the key is disabled.
        state.store.update_user_role(admin.id, false).await.unwrap();

        assert_stream_ends(&mut body).await;
    }

    #[tokio::test]
    async fn sse_stream_keeps_flowing_while_the_principal_is_unchanged() {
        let state = test_state();
        let admin = stored_user(&state, true).await;
        let mut body = revalidated_body(
            &state,
            Access::Admin,
            Principal::Session { user_id: admin.id },
        );

        for _ in 0..3 {
            sleep(FAST_REVALIDATION * 2).await;
            let text = user_then_run_event(&state, &mut body).await;
            assert!(text.contains("user_signed_in"), "stream: {text}");
        }
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

    fn limited_state(per_principal: usize) -> AppState {
        test_state().with_sse_limits(SseLimits {
            per_principal: Some(per_principal),
            global: None,
        })
    }

    #[tokio::test]
    async fn events_returns_429_past_the_per_principal_limit() {
        let state = limited_state(2);
        let other_auth = create_user_auth_header(&state, "otheruser", false).await;
        let (addr, _sender, auth) = start_sse_server(state).await;

        let mut first = connect_sse(&addr, "", &auth).await;
        assert!(status_line(&mut first).await.starts_with("HTTP/1.1 200"));
        let mut second = connect_sse(&addr, "", &auth).await;
        assert!(status_line(&mut second).await.starts_with("HTTP/1.1 200"));

        let mut third = connect_sse(&addr, "", &auth).await;
        let status = status_line(&mut third).await;
        assert!(status.starts_with("HTTP/1.1 429"), "status line: {status}");
        let body = read_body(&mut third).await;
        assert!(body.contains("SSE_CONNECTION_LIMIT"), "body: {body}");

        let mut other = connect_sse(&addr, "", &other_auth).await;
        let status = status_line(&mut other).await;
        assert!(
            status.starts_with("HTTP/1.1 200"),
            "another user keeps its own quota: {status}"
        );
    }

    #[tokio::test]
    async fn events_frees_the_slot_when_the_client_disconnects() {
        let state = limited_state(1);
        let (addr, _sender, auth) = start_sse_server(state).await;

        let mut first = connect_sse(&addr, "", &auth).await;
        assert!(status_line(&mut first).await.starts_with("HTTP/1.1 200"));
        let mut refused = connect_sse(&addr, "", &auth).await;
        assert!(status_line(&mut refused).await.starts_with("HTTP/1.1 429"));

        drop(first);

        let reopened = timeout(Duration::from_secs(5), async {
            loop {
                let mut retry = connect_sse(&addr, "", &auth).await;
                if status_line(&mut retry).await.starts_with("HTTP/1.1 200") {
                    return retry;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(reopened.is_ok(), "the slot was not freed after disconnect");
    }
}
