//! [`PodUp`] -- start a service pod and wait until it is ready, as a tracked
//! [`Operation`].
//!
//! A [`PodRun`] waits for its pod to end. A service pod (a headless browser,
//! a database, the app under test) must outlive the step that starts it:
//! [`PodUp`] returns as soon as the pod is `Ready`, with its IP, and leaves it
//! running for the next steps.

use std::time::Duration;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use k8s_openapi::api::core::v1::{Pod, PodStatus, Probe};
use kube::api::{Api, DeleteParams, LogParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, to_value};
use tokio::time::{Instant, sleep};

use super::{PodRun, Termination, truncate_logs};
use crate::error::k8s_external;

#[cfg(test)]
mod tests;

/// Default time a [`PodUp`] waits for its pod to be ready.
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(300);

/// Lines of the pod's log quoted in the error of a pod that never got ready.
const LOG_TAIL_LINES: i64 = 40;

/// Output of a [`PodUp`] operation: where to reach the ready pod.
///
/// # Examples
///
/// ```
/// use ironflow_ops_k8s::PodUpOutput;
/// use serde_json::{Error, from_str};
///
/// # fn example() -> Result<(), Error> {
/// let out: PodUpOutput = from_str(r#"{"pod":"browser","namespace":"qa","ip":"10.0.0.7"}"#)?;
/// assert_eq!(format!("{}:9222", out.ip), "10.0.0.7:9222");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodUpOutput {
    /// Name of the pod.
    pub pod: String,
    /// Namespace of the pod.
    pub namespace: String,
    /// The pod's IP (`status.podIP`).
    pub ip: String,
}

/// Start a service pod, wait until it is `Ready`, and leave it running.
///
/// The pod is the one the [`PodRun`] describes (image, command, volumes,
/// labels, sandbox, owner pod), with an optional
/// [`readiness_probe`](Self::readiness_probe). Without a probe, Kubernetes
/// declares the pod ready once its container runs.
///
/// # Lifetime and cleanup
///
/// `lifetime` is how long the service may run, the steps that use it
/// included. It replaces the [`timeout`](PodRun::timeout) of the [`PodRun`]:
/// the pod's `ironflow.io/expires-at` annotation is creation + `lifetime` +
/// [`expiry_margin`](PodRun::expiry_margin), past which the orphan reaper
/// deletes it. Like any [`PodRun`] pod, tag it with
/// [`LABEL_RUN_ID`](ironflow_core::provider::LABEL_RUN_ID) so that a retry of
/// the run deletes it; delete it yourself once the run no longer needs it.
/// [`PodRun::active_deadline_seconds`] adds a server-side bound.
///
/// `release_run` and the reaper belong to the Kubernetes agent provider and
/// only see its namespace: a pod in another namespace, or under another
/// provider, lives until a reaper of its namespace runs past `expires-at`.
/// Until then, a pod of the same name is still there and the next attempt
/// fails to create its own: put the run id in the pod name when nothing else
/// deletes it.
///
/// With the Kubernetes agent provider, `release_run` deletes the pods tagged
/// with the run each time the run executes again: a resume after an
/// approval, a signal, a human input or a crashed worker. The `PodUp` step is
/// then replayed from its stored output, the IP of a pod that is gone. Keep
/// the steps that use the service in the same execution as the `PodUp`, with
/// no such gate in between.
///
/// # Errors
///
/// A pod that ends, or is not ready within the
/// [`ready_timeout`](Self::ready_timeout), fails the step: the error says why
/// and quotes the last 40 lines of the pod's log, and the pod is deleted.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use ironflow_ops_k8s::{KubeClient, PodRun, PodUp};
/// use ironflow_core::provider::LABEL_RUN_ID;
/// use k8s_openapi::api::core::v1::{HTTPGetAction, Probe};
/// use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
/// use kube::Config;
///
/// # async fn example(run_id: &str) -> Result<(), ironflow_core::error::OperationError> {
/// let config = Config::infer().await.expect("kubeconfig");
/// let kube = KubeClient::from_config(config).await?;
/// let up = PodUp::new(
///     PodRun::new(&kube, "browser", "chromedp/headless-shell:140", "/headless-shell/headless-shell")
///         .namespace("qa")
///         .label(LABEL_RUN_ID, run_id),
///     Duration::from_secs(3600),
/// )
/// .readiness_probe(Probe {
///     http_get: Some(HTTPGetAction {
///         path: Some("/json/version".to_string()),
///         port: IntOrString::Int(9222),
///         ..Default::default()
///     }),
///     ..Default::default()
/// })
/// .ready_timeout(Duration::from_secs(120));
/// let browser = up.run().await?;
/// println!("DevTools on {}:9222", browser.ip);
/// # Ok(())
/// # }
/// ```
pub struct PodUp {
    run: PodRun,
    readiness_probe: Option<Probe>,
    ready_timeout: Duration,
}

impl PodUp {
    /// Start the pod `run` describes, for at most `lifetime`.
    ///
    /// Defaults: no readiness probe, a 300s
    /// [`ready_timeout`](Self::ready_timeout), the pod polled every
    /// [`poll_interval`](PodRun::poll_interval) of `run`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_ops_k8s::{PodRun, PodUp};
    /// # use ironflow_ops_k8s::KubeClient;
    ///
    /// # fn example(kube: &KubeClient) {
    /// let up = PodUp::new(
    ///     PodRun::new(kube, "db", "postgres:17", "docker-entrypoint.sh postgres"),
    ///     Duration::from_secs(1800),
    /// );
    /// # let _ = up;
    /// # }
    /// ```
    pub fn new(run: PodRun, lifetime: Duration) -> Self {
        Self {
            run: run.timeout(lifetime),
            readiness_probe: None,
            ready_timeout: DEFAULT_READY_TIMEOUT,
        }
    }

    /// Set the container's `readinessProbe`: the pod is ready once it passes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_ops_k8s::{PodRun, PodUp};
    /// use k8s_openapi::api::core::v1::{ExecAction, Probe};
    /// # use ironflow_ops_k8s::KubeClient;
    ///
    /// # fn example(kube: &KubeClient) {
    /// let up = PodUp::new(
    ///     PodRun::new(kube, "db", "postgres:17", "docker-entrypoint.sh postgres"),
    ///     Duration::from_secs(1800),
    /// )
    /// .readiness_probe(Probe {
    ///     exec: Some(ExecAction {
    ///         command: Some(vec!["pg_isready".to_string()]),
    ///     }),
    ///     period_seconds: Some(2),
    ///     ..Default::default()
    /// });
    /// # let _ = up;
    /// # }
    /// ```
    #[must_use]
    pub fn readiness_probe(mut self, probe: Probe) -> Self {
        self.readiness_probe = Some(probe);
        self
    }

    /// Set how long [`run`](Self::run) waits for the pod to be ready
    /// (default 300s).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_ops_k8s::{PodRun, PodUp};
    /// # use ironflow_ops_k8s::KubeClient;
    ///
    /// # fn example(kube: &KubeClient) {
    /// let up = PodUp::new(PodRun::new(kube, "svc", "nginx:1.29", "nginx -g 'daemon off;'"), Duration::from_secs(600))
    ///     .ready_timeout(Duration::from_secs(60));
    /// # let _ = up;
    /// # }
    /// ```
    #[must_use]
    pub fn ready_timeout(mut self, timeout: Duration) -> Self {
        self.ready_timeout = timeout;
        self
    }

    /// Build the [`Pod`] manifest: the [`PodRun`] pod with the readiness
    /// probe. Pure: performs no I/O.
    ///
    /// Exposed for testing the manifest without a cluster.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_ops_k8s::{PodRun, PodUp};
    /// # use ironflow_ops_k8s::KubeClient;
    ///
    /// # fn example(kube: &KubeClient) {
    /// let pod = PodUp::new(PodRun::new(kube, "svc", "nginx:1.29", "nginx"), Duration::from_secs(600))
    ///     .build_pod();
    /// assert_eq!(pod.metadata.name.as_deref(), Some("svc"));
    /// # }
    /// ```
    pub fn build_pod(&self) -> Pod {
        let mut pod = self.run.build_pod();
        // `build_pod` creates exactly one container.
        if let Some(container) = pod
            .spec
            .as_mut()
            .and_then(|spec| spec.containers.first_mut())
        {
            container.readiness_probe = self.readiness_probe.clone();
        }
        pod
    }

    /// Create the pod and wait until it is ready, then return where to reach
    /// it. The pod keeps running.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] with `origin: "kubernetes"`, and
    /// creates no pod, under the conditions of [`PodRun::run`] (oversized
    /// stdin, invalid owner pod, creation refused). Once the pod exists, the
    /// same error is returned, and the pod deleted, when it ends before being
    /// ready, is not ready within the [`ready_timeout`](Self::ready_timeout),
    /// or cannot be read: the message gives the reason (phase, reason and
    /// exit code; or the reason its container is waiting, such as
    /// `ImagePullBackOff`) and the last 40 lines of its log, bounded by
    /// [`PodRun::max_log_bytes`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_ops_k8s::{KubeClient, PodRun, PodUp};
    /// use kube::Config;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let kube = KubeClient::from_config(Config::infer().await.expect("kubeconfig")).await?;
    /// let up = PodUp::new(PodRun::new(&kube, "svc", "nginx:1.29", "nginx -g 'daemon off;'"), Duration::from_secs(600));
    /// let svc = up.run().await?;
    /// println!("http://{}:80", svc.ip);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn run(&self) -> Result<PodUpOutput, OperationError> {
        let pods = self.run.create(&self.build_pod()).await?;
        let reason = match self.wait_ready(&pods).await {
            Ok(ip) => {
                return Ok(PodUpOutput {
                    pod: self.run.name.clone(),
                    namespace: self.run.namespace.clone(),
                    ip,
                });
            }
            Err(reason) => reason,
        };
        let logs = self.log_tail(&pods).await;
        let deleted = match pods.delete(&self.run.name, &DeleteParams::default()).await {
            Ok(_) => String::new(),
            Err(e) => format!(" (not deleted: {e}; the reaper deletes it once expired)"),
        };
        Err(k8s_external(format!(
            "{reason}{deleted}\n--- last {LOG_TAIL_LINES} lines of its log ---\n{logs}"
        )))
    }

    /// Poll the pod until it is ready and has an IP. `Err` holds why it
    /// never will.
    async fn wait_ready(&self, pods: &Api<Pod>) -> Result<String, String> {
        let name = &self.run.name;
        // `None`: a timeout past what an `Instant` holds never elapses.
        let deadline = Instant::now().checked_add(self.ready_timeout);
        loop {
            let pod = pods
                .get(name)
                .await
                .map_err(|e| format!("failed to read pod '{name}': {e}"))?;
            let status = pod.status.unwrap_or_default();
            if let Some(end) = Termination::of(&status, name) {
                return Err(format!(
                    "pod '{name}' ended before getting ready: {}",
                    describe(&end)
                ));
            }
            if let Some(ip) = ready_ip(&status) {
                return Ok(ip);
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                let waiting = waiting_reason(&status, name)
                    .map(|reason| format!(", its container is waiting: {reason}"))
                    .unwrap_or_default();
                return Err(format!(
                    "pod '{name}' was not ready within {:?}{waiting}",
                    self.ready_timeout
                ));
            }
            sleep(self.run.poll_interval).await;
        }
    }

    /// The last [`LOG_TAIL_LINES`] lines of the pod's log, cut to
    /// [`PodRun::max_log_bytes`].
    async fn log_tail(&self, pods: &Api<Pod>) -> String {
        let params = LogParams {
            tail_lines: Some(LOG_TAIL_LINES),
            ..LogParams::default()
        };
        match pods.logs(&self.run.name, &params).await {
            Ok(logs) if logs.is_empty() => "(no logs)".to_string(),
            Ok(logs) => truncate_logs(logs, self.run.max_log_bytes).0,
            Err(e) => format!("(logs unavailable: {e})"),
        }
    }
}

/// The pod's IP once its `Ready` condition is `True`.
fn ready_ip(status: &PodStatus) -> Option<String> {
    let ready = status
        .conditions
        .iter()
        .flatten()
        .any(|condition| condition.type_ == "Ready" && condition.status == "True");
    if ready { status.pod_ip.clone() } else { None }
}

/// `reason: message` of the container `container` while it is waiting.
fn waiting_reason(status: &PodStatus, container: &str) -> Option<String> {
    let waiting = status
        .container_statuses
        .iter()
        .flatten()
        .find(|s| s.name == container)?
        .state
        .as_ref()?
        .waiting
        .as_ref()?;
    let reason = waiting.reason.as_deref()?;
    Some(match &waiting.message {
        Some(message) => format!("{reason}: {message}"),
        None => reason.to_string(),
    })
}

/// `phase P, reason R, exit code C`, leaving out what is unknown.
fn describe(end: &Termination) -> String {
    let mut parts = vec![format!("phase {}", end.phase)];
    if let Some(reason) = &end.reason {
        parts.push(format!("reason {reason}"));
    }
    if let Some(code) = end.exit_code {
        parts.push(format!("exit code {code}"));
    }
    parts.join(", ")
}

#[async_trait]
impl Operation for PodUp {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let output = self.run().await?;
        to_value(&output).map_err(k8s_external)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "image": self.run.image,
            "namespace": self.run.namespace,
            "name": self.run.name,
            "command": self.run.command,
            "ready_timeout_secs": self.ready_timeout.as_secs(),
            "lifetime_secs": self.run.timeout.as_secs(),
        }))
    }
}

impl TypedOperation for PodUp {
    type Output = PodUpOutput;
}
