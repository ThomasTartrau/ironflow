//! `GET /api/v1/stats/history` -- Time-bucketed historical statistics.

use std::collections::BTreeMap;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use chrono::{DateTime, Duration, Utc};
use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::{
    HistoryGranularity, HistoryPeriod, StatsHistoryBucket, StatsHistoryFilter,
};

use crate::entities::{StatsHistoryBucketResponse, StatsHistoryQuery, StatsHistoryResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get time-bucketed historical statistics for trend charts.
///
/// Returns run counts for every status, the success rate, duration metrics
/// and cost per time bucket. Runs are bucketed by creation time and counted
/// under their current status. Buckets use UTC boundaries (weeks start on
/// Monday) and cover the whole period: buckets without runs are zero-filled.
///
/// Accepts `period` and `granularity`, plus the same filters as
/// `GET /api/v1/stats` and `GET /api/v1/runs` (`workflow` as a
/// case-insensitive substring, `status`, `has_steps`, `label`, `created_by`).
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/stats/history",
        tags = ["stats"],
        params(StatsHistoryQuery),
        responses(
            (status = 200, description = "Historical statistics", body = StatsHistoryResponse),
            (status = 400, description = "Invalid period, granularity or filter"),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_stats_history(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<StatsHistoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let period = params.period.unwrap_or_default();
    let granularity = params
        .granularity
        .unwrap_or_else(|| period.default_granularity());

    let filter = StatsHistoryFilter {
        workflow_name: params.workflow.clone(),
        status: params.status,
        has_steps: params.has_steps,
        labels: params.parse_labels(),
        created_by_user_id: params.created_by,
        period,
        granularity,
    };

    let buckets = state.store.get_stats_history(filter).await?;
    let buckets = fill_buckets(buckets, Utc::now(), period, granularity);

    let response = StatsHistoryResponse {
        period,
        granularity,
        workflow: params.workflow,
        buckets: buckets
            .into_iter()
            .map(StatsHistoryBucketResponse::from)
            .collect(),
    };

    Ok(ok(response))
}

/// Zero-fill the bucket grid of `period` ending at `now`.
///
/// The grid runs from the bucket containing `now - period` to the bucket
/// containing `now`, inclusive. Store buckets replace the empty ones at the
/// same time; store buckets outside the grid (clock drift between the store
/// and this route) are kept. The result is sorted by time ascending.
fn fill_buckets(
    buckets: Vec<StatsHistoryBucket>,
    now: DateTime<Utc>,
    period: HistoryPeriod,
    granularity: HistoryGranularity,
) -> Vec<StatsHistoryBucket> {
    let first = granularity.bucket_start(now - Duration::hours(period.hours()));
    let last = granularity.bucket_start(now);
    let step = Duration::seconds(granularity.seconds());

    let mut grid: BTreeMap<DateTime<Utc>, StatsHistoryBucket> = BTreeMap::new();
    let mut time = first;
    while time <= last {
        grid.insert(
            time,
            StatsHistoryBucket {
                time,
                ..StatsHistoryBucket::default()
            },
        );
        time += step;
    }
    for bucket in buckets {
        grid.insert(bucket.time, bucket);
    }
    grid.into_values().collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use chrono::{Datelike, Timelike, Weekday};
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{
        NewRun, NewStep, NewUser, RunActor, RunStatus, StepKind, TriggerKind, step_trace_id,
    };
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
    use rust_decimal::Decimal;
    use serde_json::{Value as JsonValue, from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::routes::test_helpers::create_terminal_run;

    const COUNTERS: [&str; 9] = [
        "completed",
        "warning",
        "failed",
        "cancelled",
        "pending",
        "running",
        "retrying",
        "awaiting_approval",
        "sleeping",
    ];

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn new_run(labels: HashMap<String, String>, created_by: Option<RunActor>) -> NewRun {
        NewRun {
            created_by,
            workflow_name: "wf".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels,
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        }
    }

    /// Create a run and drive it to `status` through valid FSM transitions.
    async fn create_run_in_status(store: &InMemoryStore, req: NewRun, status: RunStatus) {
        let run = store.create_run(req).await.unwrap().into_run();
        let path: &[RunStatus] = match status {
            RunStatus::Pending => &[],
            RunStatus::Running => &[RunStatus::Running],
            RunStatus::Retrying => &[RunStatus::Running, RunStatus::Retrying],
            RunStatus::AwaitingApproval => &[RunStatus::Running, RunStatus::AwaitingApproval],
            RunStatus::Sleeping => &[RunStatus::Running, RunStatus::Sleeping],
            RunStatus::Completed => &[RunStatus::Running, RunStatus::Completed],
            RunStatus::Warning => &[RunStatus::Running, RunStatus::Warning],
            RunStatus::Failed => &[RunStatus::Running, RunStatus::Failed],
            RunStatus::Cancelled => &[RunStatus::Cancelled],
        };
        for next in path {
            store.update_run_status(run.id, *next).await.unwrap();
        }
    }

    async fn fetch(store: Arc<InMemoryStore>, uri: &str) -> (StatusCode, JsonValue) {
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/", get(get_stats_history))
            .with_state(state);

        let req = Request::builder()
            .uri(uri)
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = if body.is_empty() {
            JsonValue::Null
        } else {
            from_slice(&body).unwrap_or(JsonValue::Null)
        };
        (status, json_val)
    }

    fn buckets(json_val: &JsonValue) -> &Vec<JsonValue> {
        json_val["data"]["buckets"].as_array().unwrap()
    }

    fn total(json_val: &JsonValue, key: &str) -> u64 {
        buckets(json_val)
            .iter()
            .map(|b| b[key].as_u64().unwrap())
            .sum()
    }

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn empty_store_returns_zero_filled_grid() {
        let (status, json_val) = fetch(Arc::new(InMemoryStore::new()), "/?period=24h").await;
        assert_eq!(status, StatusCode::OK);

        let buckets = buckets(&json_val);
        assert!(buckets.len() == 24 || buckets.len() == 25);
        for b in buckets {
            for key in COUNTERS {
                assert_eq!(b[key], 0, "{key} should be zero");
            }
            assert!(b["success_rate_percent"].is_null());
            assert!(b.get("running").is_some());
            assert!(b.get("pending").is_some());
            assert!(b.get("awaiting_approval").is_some());
        }
    }

    #[tokio::test]
    async fn running_run_is_counted_in_current_bucket() {
        let store = Arc::new(InMemoryStore::new());
        create_run_in_status(&store, new_run(HashMap::new(), None), RunStatus::Running).await;

        let (status, json_val) = fetch(store, "/?period=24h").await;
        assert_eq!(status, StatusCode::OK);

        assert_eq!(total(&json_val, "running"), 1);
        let last = buckets(&json_val).last().unwrap();
        assert_eq!(last["running"], 1);
    }

    #[tokio::test]
    async fn every_status_has_its_own_counter() {
        let store = Arc::new(InMemoryStore::new());
        for status in [
            RunStatus::Pending,
            RunStatus::Running,
            RunStatus::Retrying,
            RunStatus::AwaitingApproval,
            RunStatus::Sleeping,
            RunStatus::Completed,
            RunStatus::Warning,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            create_run_in_status(&store, new_run(HashMap::new(), None), status).await;
        }

        let (status, json_val) = fetch(store, "/?period=7d").await;
        assert_eq!(status, StatusCode::OK);

        for key in COUNTERS {
            assert_eq!(total(&json_val, key), 1, "{key} should be 1");
        }
        let last = buckets(&json_val).last().unwrap();
        // completed is strict: the warning run is not counted in it.
        assert_eq!(last["completed"], 1);
        assert_eq!(last["warning"], 1);
        let rate = last["success_rate_percent"].as_f64().unwrap();
        assert!((rate - 200.0 / 3.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn filters_by_status() {
        let store = Arc::new(InMemoryStore::new());
        create_run_in_status(&store, new_run(HashMap::new(), None), RunStatus::Failed).await;
        create_run_in_status(&store, new_run(HashMap::new(), None), RunStatus::Completed).await;
        create_run_in_status(&store, new_run(HashMap::new(), None), RunStatus::Running).await;

        let (status, json_val) = fetch(store, "/?period=24h&status=failed").await;
        assert_eq!(status, StatusCode::OK);

        assert_eq!(total(&json_val, "failed"), 1);
        assert_eq!(total(&json_val, "completed"), 0);
        assert_eq!(total(&json_val, "running"), 0);
        assert!(buckets(&json_val).len() >= 24);
    }

    #[tokio::test]
    async fn filters_by_label() {
        let store = Arc::new(InMemoryStore::new());
        let prod = HashMap::from([("env".to_string(), "prod".to_string())]);
        let staging = HashMap::from([("env".to_string(), "staging".to_string())]);
        create_run_in_status(&store, new_run(prod, None), RunStatus::Completed).await;
        create_run_in_status(&store, new_run(staging, None), RunStatus::Completed).await;

        let (status, json_val) = fetch(store, "/?period=24h&label=env:prod").await;
        assert_eq!(status, StatusCode::OK);

        assert_eq!(total(&json_val, "completed"), 1);
    }

    #[tokio::test]
    async fn filters_by_created_by() {
        let store = Arc::new(InMemoryStore::new());
        let alice = store
            .create_user(NewUser {
                email: "alice@example.com".to_string(),
                username: "alice".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();
        let bob = store
            .create_user(NewUser {
                email: "bob@example.com".to_string(),
                username: "bob".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();
        for user_id in [alice.id, bob.id] {
            let actor = Some(RunActor::User { user_id });
            create_run_in_status(&store, new_run(HashMap::new(), actor), RunStatus::Pending).await;
        }

        let uri = format!("/?period=24h&created_by={}", alice.id);
        let (status, json_val) = fetch(store, &uri).await;
        assert_eq!(status, StatusCode::OK);

        assert_eq!(total(&json_val, "pending"), 1);
    }

    #[tokio::test]
    async fn filters_by_has_steps() {
        let store = Arc::new(InMemoryStore::new());
        let _empty = create_terminal_run(store.as_ref(), "empty-wf", RunStatus::Completed).await;
        let r = create_terminal_run(store.as_ref(), "busy-wf", RunStatus::Completed).await;
        store
            .create_step(NewStep {
                run_id: r.id,
                trace_id: step_trace_id(r.id, "build", 0),
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .unwrap();

        let (status, json_val) = fetch(store.clone(), "/?period=24h&has_steps=true").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(total(&json_val, "completed"), 1);

        let (status, json_val) = fetch(store, "/?period=24h").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(total(&json_val, "completed"), 2);
    }

    #[tokio::test]
    async fn invalid_filters_are_rejected() {
        let store = Arc::new(InMemoryStore::new());
        let (status, _) = fetch(store.clone(), "/?created_by=not-a-uuid").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = fetch(store, "/?status=unknown").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn ninety_days_uses_monday_weeks() {
        let store = Arc::new(InMemoryStore::new());
        create_run_in_status(&store, new_run(HashMap::new(), None), RunStatus::Completed).await;

        let (status, json_val) = fetch(store, "/?period=90d").await;
        assert_eq!(status, StatusCode::OK);

        let buckets = buckets(&json_val);
        assert!(buckets.len() == 13 || buckets.len() == 14);
        for b in buckets {
            let time: DateTime<Utc> = b["time"].as_str().unwrap().parse().unwrap();
            assert_eq!(time.weekday(), Weekday::Mon);
            assert_eq!(time.hour(), 0);
            assert_eq!(time.minute(), 0);
        }
        assert_eq!(total(&json_val, "completed"), 1);
    }

    #[test]
    fn fill_buckets_hourly_grid_is_complete_and_sorted() {
        let now = utc("2026-09-24T13:45:00Z");
        let filled = fill_buckets(
            Vec::new(),
            now,
            HistoryPeriod::TwentyFourHours,
            HistoryGranularity::OneHour,
        );

        assert_eq!(filled.len(), 25);
        assert_eq!(filled[0].time, utc("2026-09-23T13:00:00Z"));
        assert_eq!(filled[24].time, utc("2026-09-24T13:00:00Z"));
        assert!(filled.windows(2).all(|w| w[0].time < w[1].time));
    }

    #[test]
    fn fill_buckets_daily_grid() {
        let now = utc("2026-09-24T13:45:00Z");
        let filled = fill_buckets(
            Vec::new(),
            now,
            HistoryPeriod::SevenDays,
            HistoryGranularity::OneDay,
        );

        assert_eq!(filled.len(), 8);
        assert_eq!(filled[0].time, utc("2026-09-17T00:00:00Z"));
        assert_eq!(filled[7].time, utc("2026-09-24T00:00:00Z"));
    }

    #[test]
    fn fill_buckets_weekly_grid() {
        let now = utc("2026-09-24T13:45:00Z");
        let filled = fill_buckets(
            Vec::new(),
            now,
            HistoryPeriod::NinetyDays,
            HistoryGranularity::OneWeek,
        );

        // now - 90d = 2026-06-26 (Friday), whose week starts on 2026-06-22.
        assert_eq!(filled.len(), 14);
        assert_eq!(filled[0].time, utc("2026-06-22T00:00:00Z"));
        assert_eq!(filled[13].time, utc("2026-09-21T00:00:00Z"));
        assert!(filled.iter().all(|b| b.time.weekday() == Weekday::Mon));
    }

    #[test]
    fn fill_buckets_preserves_store_values() {
        let now = utc("2026-09-24T13:45:00Z");
        let store_bucket = StatsHistoryBucket {
            time: utc("2026-09-24T10:00:00Z"),
            completed: 3,
            running: 2,
            failed: 1,
            avg_duration_ms: 1500,
            total_cost_usd: Decimal::new(42, 2),
            ..StatsHistoryBucket::default()
        };
        let filled = fill_buckets(
            vec![store_bucket],
            now,
            HistoryPeriod::TwentyFourHours,
            HistoryGranularity::OneHour,
        );

        assert_eq!(filled.len(), 25);
        let b = filled
            .iter()
            .find(|b| b.time == utc("2026-09-24T10:00:00Z"))
            .unwrap();
        assert_eq!(b.completed, 3);
        assert_eq!(b.running, 2);
        assert_eq!(b.failed, 1);
        assert_eq!(b.avg_duration_ms, 1500);
        assert_eq!(b.total_cost_usd, Decimal::new(42, 2));
        let zeros = filled.iter().filter(|b| b.completed == 0).count();
        assert_eq!(zeros, 24);
    }

    #[test]
    fn fill_buckets_keeps_out_of_grid_store_bucket() {
        let now = utc("2026-09-24T13:45:00Z");
        let drifted = StatsHistoryBucket {
            time: utc("2026-09-24T14:00:00Z"),
            pending: 1,
            ..StatsHistoryBucket::default()
        };
        let filled = fill_buckets(
            vec![drifted],
            now,
            HistoryPeriod::TwentyFourHours,
            HistoryGranularity::OneHour,
        );

        assert_eq!(filled.len(), 26);
        let last = filled.last().unwrap();
        assert_eq!(last.time, utc("2026-09-24T14:00:00Z"));
        assert_eq!(last.pending, 1);
        assert!(filled.windows(2).all(|w| w[0].time < w[1].time));
    }
}
