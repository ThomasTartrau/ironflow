//! The auth proxy tokens of a K8s agent step stop working when the step ends,
//! whatever ends it: a normal end, or the worker dropping the step (run
//! cancelled, lease lost, run timed out) while its pod is created or runs.
//!
//! The real `kube` client talks to the fake API server (see [`fake_api`]); the
//! real auth proxy client talks to a fake admin API (wiremock).

use std::fmt::Debug;
use std::future::Future;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::select;
use tokio::time::{Duration, Instant, sleep, timeout};
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt as tracing_fmt;
use tracing_subscriber::fmt::MakeWriter;
use wiremock::matchers::{body_partial_json, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use ironflow_core::account::{AccountCredential, AccountSession, RateLimitRecorder};
use ironflow_core::auth_proxy::{ProxiedSecret, SecretInjection};
use ironflow_core::provider::{AgentConfig, AgentProvider, LogSink};
use ironflow_core::providers::claude::K8sEphemeralProvider;

use crate::fake_api::{FakeK8s, NS, collection, pod};

const ADMIN_KEY: &str = "0123456789abcdef0123456789abcdef";
const CLAUDE_TOKEN_ID: &str = "claude0123456789";
const SECRET_TOKEN_ID: &str = "secret0123456789";
const TEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest wait for a request: generous, the whole workspace suite may run
/// in parallel.
const WAIT_LIMIT: Duration = Duration::from_secs(10);

/// A fake auth proxy admin API issuing one token for the Claude credential
/// and one for the `GITHUB_TOKEN` proxied secret.
async fn fake_proxy() -> MockServer {
    let server = proxy_issuing_claude_token().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/tokens"))
        .and(body_partial_json(
            json!({ "credential": { "name": "GITHUB_TOKEN" } }),
        ))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "id": SECRET_TOKEN_ID, "token": "ifap_secret" })),
        )
        .mount(&server)
        .await;
    server
}

/// A fake auth proxy admin API issuing the Claude token and refusing the
/// `GITHUB_TOKEN` one.
async fn proxy_refusing_the_secret() -> MockServer {
    let server = proxy_issuing_claude_token().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/tokens"))
        .and(body_partial_json(
            json!({ "credential": { "name": "GITHUB_TOKEN" } }),
        ))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    server
}

async fn proxy_issuing_claude_token() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/tokens"))
        .and(body_partial_json(
            json!({ "credential": { "kind": "oauth_token" } }),
        ))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "id": CLAUDE_TOKEN_ID, "token": "ifap_claude" })),
        )
        .mount(&server)
        .await;
    server
}

/// Answer the revocation of `id` with `status`, expecting it exactly `times`.
async fn expect_revocation(proxy: &MockServer, id: &str, status: u16, times: u64) {
    Mock::given(method("DELETE"))
        .and(path(format!("/admin/v1/tokens/{id}")))
        .respond_with(ResponseTemplate::new(status))
        .expect(times)
        .mount(proxy)
        .await;
}

/// Wait until the requests `server` received satisfy `done`.
async fn wait_for(server: &MockServer, what: &str, done: impl Fn(&[Request]) -> bool) {
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        let requests = server.received_requests().await.expect("recorded");
        if done(&requests) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} never happened; requests: {:?}",
            requests
                .iter()
                .map(|r| format!("{} {}", r.method, r.url.path()))
                .collect::<Vec<_>>()
        );
        sleep(Duration::from_millis(20)).await;
    }
}

/// Number of `verb` requests to `path` in `requests`.
fn count(requests: &[Request], verb: &str, path: &str) -> usize {
    requests
        .iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == path)
        .count()
}

/// Wait until the proxy received the revocation of every id in `ids`.
async fn revoked(proxy: &MockServer, ids: &[&str]) {
    wait_for(proxy, "token revocation", |r| {
        ids.iter()
            .all(|id| count(r, "DELETE", &format!("/admin/v1/tokens/{id}")) > 0)
    })
    .await;
}

/// Wait until the step sent the creation of its pod.
async fn pod_posted(fake: &FakeK8s) {
    wait_for(&fake.server, "the pod creation", |r| {
        count(r, "POST", &collection("pods")) > 0
    })
    .await;
}

/// Wait until the step waits on its pod: the fake API was asked about pods.
async fn pod_awaited(fake: &FakeK8s) {
    wait_for(&fake.server, "the wait on the pod", |r| {
        count(r, "GET", &collection("pods")) > 0
    })
    .await;
}

/// Run `step` until `reached` completes, then drop it, as the worker drops a
/// run it abandons. The drop returns at once: it never waits for the proxy.
async fn drop_when<T: Debug>(step: impl Future<Output = T>, reached: impl Future<Output = ()>) {
    let mut step = Box::pin(step);
    select! {
        ended = &mut step => panic!("the step ended on its own: {ended:?}"),
        () = reached => {}
    }
    let started = Instant::now();
    drop(step);
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "dropping the step waited {:?}",
        started.elapsed()
    );
}

fn provider(fake: &FakeK8s, proxy: &MockServer) -> K8sEphemeralProvider {
    K8sEphemeralProvider::sandboxed("img:v1")
        .namespace(NS)
        .cluster_config(fake.cluster_config())
        .auth_proxy(&format!("{}/", proxy.uri()))
        .auth_proxy_admin_key(ADMIN_KEY)
        .proxied_secret(ProxiedSecret {
            name: "GITHUB_TOKEN".to_string(),
            env: "GITHUB_TOKEN".to_string(),
            value: "ghp_real".to_string(),
            injection: SecretInjection::Bearer,
            hosts: vec!["api.github.com".to_string()],
        })
}

fn config() -> AgentConfig {
    let mut config = AgentConfig::new("review the diff");
    config.account = Some(AccountSession::new(
        AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-test".to_string()),
        RateLimitRecorder::default(),
    ));
    config
}

/// Accept the creation of any pod, after `delay`.
async fn accept_pods(fake: &FakeK8s, delay: Duration) {
    Mock::given(method("POST"))
        .and(path(collection("pods")))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(pod("created", "agent", &[], "Pending", None, None))
                .set_delay(delay),
        )
        .mount(&fake.server)
        .await;
}

/// Answer the wait on any pod with a pod in `phase`, and a watch that never
/// reports a change in time.
async fn pods_in_phase(fake: &FakeK8s, phase: &str) {
    Mock::given(method("GET"))
        .and(path(collection("pods")))
        .and(query_param("watch", "true"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&fake.server)
        .await;
    Mock::given(method("GET"))
        .and(path(collection("pods")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "v1",
            "kind": "List",
            "metadata": { "resourceVersion": "2" },
            "items": [pod("agent-pod", "agent", &[], phase, None, None)],
        })))
        .mount(&fake.server)
        .await;
}

/// Answer the read of any pod with a running pod, and its log stream with
/// nothing until the test is over: the agent is still working.
async fn streaming_pods(fake: &FakeK8s) {
    Mock::given(method("GET"))
        .and(path_regex(format!("^{}/[^/]+$", collection("pods"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(pod(
            "agent-pod",
            "agent",
            &[],
            "Running",
            None,
            None,
        )))
        .mount(&fake.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(format!("^{}/[^/]+/log$", collection("pods"))))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(60)))
        .mount(&fake.server)
        .await;
}

/// Wait until the step streams the logs of its pod.
async fn logs_streamed(fake: &FakeK8s) {
    wait_for(&fake.server, "the log stream", |r| {
        r.iter()
            .any(|r| r.method.as_str() == "GET" && r.url.path().ends_with("/log"))
    })
    .await;
}

/// Answer the revocation of `id` only after `delay`, expecting it once.
async fn slow_revocation(proxy: &MockServer, id: &str, delay: Duration) {
    Mock::given(method("DELETE"))
        .and(path(format!("/admin/v1/tokens/{id}")))
        .respond_with(ResponseTemplate::new(204).set_delay(delay))
        .expect(1)
        .mount(proxy)
        .await;
}

/// A log sink that drops every line.
struct Discard;

impl LogSink for Discard {
    fn log(&self, _stream: &str, _line: &str) {}
}

/// What the code under test logs, captured in memory.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Logs {
    type Writer = Logs;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Answer the logs and the deletion of any pod.
async fn finished_pods(fake: &FakeK8s) {
    Mock::given(method("GET"))
        .and(path_regex(format!("^{}/[^/]+/log$", collection("pods"))))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&fake.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(format!("^{}/[^/]+$", collection("pods"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "v1", "kind": "Status", "status": "Success", "metadata": {}
        })))
        .mount(&fake.server)
        .await;
}

/// The worker path: it always streams the logs of an agent step.
#[tokio::test]
async fn k8s_step_dropped_while_its_pod_runs_revokes_its_proxy_tokens() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let proxy = fake_proxy().await;
        expect_revocation(&proxy, CLAUDE_TOKEN_ID, 204, 1).await;
        expect_revocation(&proxy, SECRET_TOKEN_ID, 204, 1).await;
        accept_pods(&fake, Duration::ZERO).await;
        pods_in_phase(&fake, "Running").await;
        streaming_pods(&fake).await;

        let provider = provider(&fake, &proxy);
        let config = config();
        let step = provider.invoke_with_logs(&config, Arc::new(Discard));
        drop_when(step, logs_streamed(&fake)).await;

        revoked(&proxy, &[CLAUDE_TOKEN_ID, SECRET_TOKEN_ID]).await;
        proxy.verify().await;
    })
    .await
    .expect("test timed out");
}

/// Library mode: `invoke`, without log streaming.
#[tokio::test]
async fn k8s_step_dropped_while_waiting_for_its_pod_revokes_its_proxy_tokens() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let proxy = fake_proxy().await;
        expect_revocation(&proxy, CLAUDE_TOKEN_ID, 204, 1).await;
        expect_revocation(&proxy, SECRET_TOKEN_ID, 204, 1).await;
        accept_pods(&fake, Duration::ZERO).await;
        pods_in_phase(&fake, "Running").await;

        let provider = provider(&fake, &proxy);
        let config = config();
        drop_when(provider.invoke(&config), pod_awaited(&fake)).await;

        revoked(&proxy, &[CLAUDE_TOKEN_ID, SECRET_TOKEN_ID]).await;
        proxy.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_step_dropped_while_its_pod_is_created_revokes_its_proxy_tokens() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let proxy = fake_proxy().await;
        expect_revocation(&proxy, CLAUDE_TOKEN_ID, 204, 1).await;
        expect_revocation(&proxy, SECRET_TOKEN_ID, 204, 1).await;
        // The pod creation outlasts the test: the step is still creating it
        // when dropped.
        accept_pods(&fake, Duration::from_secs(60)).await;

        let provider = provider(&fake, &proxy);
        let config = config();
        drop_when(provider.invoke(&config), pod_posted(&fake)).await;

        revoked(&proxy, &[CLAUDE_TOKEN_ID, SECRET_TOKEN_ID]).await;
        proxy.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_step_that_ends_revokes_each_proxy_token_once() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let proxy = fake_proxy().await;
        expect_revocation(&proxy, CLAUDE_TOKEN_ID, 204, 1).await;
        expect_revocation(&proxy, SECRET_TOKEN_ID, 204, 1).await;
        accept_pods(&fake, Duration::ZERO).await;
        pods_in_phase(&fake, "Failed").await;
        finished_pods(&fake).await;

        let provider = provider(&fake, &proxy);
        // The empty logs fail the step: what matters is that it ended.
        let _ended = provider.invoke(&config()).await;

        // A second revocation would come from a detached task: give it room
        // to show up. Asserting an absence needs a window.
        sleep(Duration::from_millis(300)).await;
        proxy.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_step_dropped_during_its_revocation_still_revokes_every_token() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let proxy = fake_proxy().await;
        slow_revocation(&proxy, CLAUDE_TOKEN_ID, Duration::from_secs(1)).await;
        expect_revocation(&proxy, SECRET_TOKEN_ID, 204, 1).await;
        accept_pods(&fake, Duration::ZERO).await;
        pods_in_phase(&fake, "Failed").await;
        finished_pods(&fake).await;

        let provider = provider(&fake, &proxy);
        let config = config();
        // Dropped once the first revocation is in flight.
        drop_when(
            provider.invoke(&config),
            revoked(&proxy, &[CLAUDE_TOKEN_ID]),
        )
        .await;

        revoked(&proxy, &[SECRET_TOKEN_ID]).await;
        proxy.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_step_that_ends_logs_a_failed_revocation_and_does_not_wait_for_the_proxy() {
    timeout(TEST_TIMEOUT, async {
        let logs = Logs::default();
        let subscriber = tracing_fmt()
            .with_writer(logs.clone())
            .with_max_level(Level::WARN)
            .with_ansi(false)
            .finish();
        let _logs = set_default(subscriber);

        let fake = FakeK8s::start().await;
        let proxy = fake_proxy().await;
        expect_revocation(&proxy, CLAUDE_TOKEN_ID, 500, 1).await;
        // A proxy that never answers: the step must not wait for it.
        slow_revocation(&proxy, SECRET_TOKEN_ID, Duration::from_secs(60)).await;
        accept_pods(&fake, Duration::ZERO).await;
        pods_in_phase(&fake, "Failed").await;
        finished_pods(&fake).await;

        let provider = provider(&fake, &proxy);
        let started = Instant::now();
        let _ended = provider.invoke(&config()).await;
        // Above the 5 s bound plus the pod launch under a loaded suite, far
        // below the 30 s an unbounded wait on the silent proxy would take.
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the step waited {:?} for the proxy",
            started.elapsed()
        );

        let text = logs.text();
        assert!(
            text.contains("auth proxy token revocation failed"),
            "no warning logged: {text}"
        );
        assert!(
            text.contains("the step ends without it"),
            "no warning about the revocation still running: {text}"
        );
        revoked(&proxy, &[CLAUDE_TOKEN_ID, SECRET_TOKEN_ID]).await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_step_whose_secret_token_is_refused_revokes_its_claude_token() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let proxy = proxy_refusing_the_secret().await;
        expect_revocation(&proxy, CLAUDE_TOKEN_ID, 204, 1).await;

        let provider = provider(&fake, &proxy);
        let ended = provider.invoke(&config()).await;
        assert!(ended.is_err(), "the step ran without its secret: {ended:?}");

        assert_eq!(count_posts(&fake).await, 0, "a pod was created");
        proxy.verify().await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn k8s_step_whose_pod_is_refused_revokes_its_proxy_tokens() {
    timeout(TEST_TIMEOUT, async {
        let fake = FakeK8s::start().await;
        let proxy = fake_proxy().await;
        expect_revocation(&proxy, CLAUDE_TOKEN_ID, 204, 1).await;
        expect_revocation(&proxy, SECRET_TOKEN_ID, 204, 1).await;
        Mock::given(method("POST"))
            .and(path(collection("pods")))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "apiVersion": "v1",
                "kind": "Status",
                "status": "Failure",
                "message": "refused by the fake API server",
                "reason": "Forbidden",
                "code": 403,
            })))
            .mount(&fake.server)
            .await;

        let provider = provider(&fake, &proxy);
        let ended = provider.invoke(&config()).await;
        assert!(ended.is_err(), "the step ran without its pod: {ended:?}");

        proxy.verify().await;
    })
    .await
    .expect("test timed out");
}

/// Number of pods the step asked the fake API to create.
async fn count_posts(fake: &FakeK8s) -> usize {
    let requests = fake.server.received_requests().await.expect("recorded");
    count(&requests, "POST", &collection("pods"))
}
