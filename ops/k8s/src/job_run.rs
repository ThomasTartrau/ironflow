//! [`JobRun`] -- run a command to completion via an ephemeral `batch/v1` Job.

use std::time::Duration;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, PodTemplateSpec, SecurityContext};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, ListParams, LogParams, PostParams, PropagationPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

use crate::KubeClient;
use crate::error::k8s_external;
use crate::pod_run::{PvcMount, build_pvc_volumes};

#[cfg(test)]
mod tests;

/// Default wall-clock timeout for a [`JobRun`] if none is configured.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// Default interval between Job status polls.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Output of a [`JobRun`] operation.
///
/// `success` is `true` only when the Job reached the `Complete` condition.
/// A Job that exhausted its retries (`Failed` condition) reports
/// `success: false` with `phase == "Failed"` -- this is **not** an error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRunOutput {
    /// `true` when the Job completed successfully.
    pub success: bool,
    /// Logs of the Job's pod, captured once the Job reached a terminal state.
    pub logs: String,
    /// Terminal phase: `"Succeeded"` (Job `Complete`) or `"Failed"`.
    pub phase: String,
}

/// Run a shell command to completion via an ephemeral `batch/v1` Job, wait for
/// the Job to finish, collect the pod logs, and delete the Job.
///
/// The command runs via `/bin/sh -c` in a pod with `restartPolicy: Never`.
/// Unlike [`PodRun`](crate::pod_run::PodRun), a Job can retry the pod up to
/// `backoff_limit` times before it is marked `Failed`.
///
/// # Error vs. non-zero exit
///
/// A Job that ran but ultimately failed (`Failed` condition) returns
/// `Ok(JobRunOutput { success: false, .. })`. An [`OperationError::External`]
/// with `origin: "kubernetes"` is returned only for infrastructure failures:
/// Job creation, the wait loop, or a wall-clock timeout.
///
/// The Job is deleted (best-effort, with `Background` propagation so its pods
/// are removed too) on every path, including timeout and wait errors.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::{KubeClient, job_run::JobRun};
/// use kube::Config;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let config = Config::infer().await.expect("kubeconfig");
/// let kube = KubeClient::from_config(config).await?;
/// let output = JobRun::new(&kube, "migrate", "migrate:latest", "migrate up")
///     .namespace("ci")
///     .backoff_limit(2)
///     .run()
///     .await?;
/// assert!(output.success || !output.phase.is_empty());
/// # Ok(())
/// # }
/// ```
pub struct JobRun {
    client: kube::Client,
    name: String,
    namespace: String,
    image: String,
    command: String,
    backoff_limit: i32,
    pvcs: Vec<PvcMount>,
    automount_service_account_token: Option<bool>,
    allow_privilege_escalation: Option<bool>,
    timeout: Duration,
    poll_interval: Duration,
}

impl JobRun {
    /// Create a job-run operation. `command` is executed via `/bin/sh -c`.
    ///
    /// Defaults: namespace `"default"`, `backoff_limit` 0, a 600s wall-clock
    /// timeout, and a 2s poll interval.
    pub fn new(client: &KubeClient, name: &str, image: &str, command: &str) -> Self {
        Self {
            client: client.client().clone(),
            name: name.to_string(),
            namespace: "default".to_string(),
            image: image.to_string(),
            command: command.to_string(),
            backoff_limit: 0,
            pvcs: Vec::new(),
            automount_service_account_token: None,
            allow_privilege_escalation: None,
            timeout: DEFAULT_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    /// Set the namespace the Job is created in.
    #[must_use]
    pub fn namespace(mut self, namespace: &str) -> Self {
        self.namespace = namespace.to_string();
        self
    }

    /// Set the number of retries before the Job is marked `Failed`.
    #[must_use]
    pub fn backoff_limit(mut self, backoff_limit: i32) -> Self {
        self.backoff_limit = backoff_limit;
        self
    }

    /// Mount a PersistentVolumeClaim at the given path in the Job's pod.
    ///
    /// Additive: every call appends one volume and its mount, so a Job's pod can
    /// carry several PVCs at once. The first call keeps the volume name
    /// `"workspace"`; each further call gets a unique deterministic name
    /// (`"workspace-1"`, `"workspace-2"`, ...), matching
    /// [`PodRun::pvc`](crate::pod_run::PodRun::pvc).
    #[must_use]
    pub fn pvc(mut self, claim: &str, mount_path: &str) -> Self {
        self.pvcs.push(PvcMount {
            claim: claim.to_string(),
            mount_path: mount_path.to_string(),
        });
        self
    }

    /// Set `automountServiceAccountToken` on the Job's pod template.
    ///
    /// Pass `false` to prevent the default ServiceAccount token from being
    /// mounted into the Job's pods, hardening them against token exfiltration.
    /// Opt-in: if this builder is never called the field is left absent and
    /// Kubernetes applies its default (mount the token).
    #[must_use]
    pub fn automount_service_account_token(mut self, automount: bool) -> Self {
        self.automount_service_account_token = Some(automount);
        self
    }

    /// Set the container's `SecurityContext.allowPrivilegeEscalation` in the
    /// Job's pod template.
    ///
    /// Pass `false` to forbid a process from gaining more privileges than its
    /// parent, hardening the container. Opt-in: if this builder is never called
    /// the field is left absent and Kubernetes applies its default.
    #[must_use]
    pub fn allow_privilege_escalation(mut self, allow: bool) -> Self {
        self.allow_privilege_escalation = Some(allow);
        self
    }

    /// Set the wall-clock timeout for the whole run.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the interval between Job status polls.
    #[must_use]
    pub fn poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Build the [`Job`] manifest for this run. Pure: performs no I/O.
    ///
    /// Exposed for testing the manifest without a cluster.
    pub fn build_job(&self) -> Job {
        let mut container = Container {
            name: self.name.clone(),
            image: Some(self.image.clone()),
            command: Some(vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                self.command.clone(),
            ]),
            ..Default::default()
        };

        // Container SecurityContext is built only when the hardening toggle is
        // set, otherwise left absent (opt-in, no regression for existing jobs).
        if self.allow_privilege_escalation.is_some() {
            container.security_context = Some(SecurityContext {
                allow_privilege_escalation: self.allow_privilege_escalation,
                ..Default::default()
            });
        }

        let (volumes, volume_mounts) = build_pvc_volumes(&self.pvcs);
        if !volume_mounts.is_empty() {
            container.volume_mounts = Some(volume_mounts);
        }

        Job {
            metadata: ObjectMeta {
                name: Some(self.name.clone()),
                namespace: Some(self.namespace.clone()),
                ..Default::default()
            },
            spec: Some(JobSpec {
                backoff_limit: Some(self.backoff_limit),
                template: PodTemplateSpec {
                    metadata: None,
                    spec: Some(PodSpec {
                        containers: vec![container],
                        restart_policy: Some("Never".to_string()),
                        volumes: if volumes.is_empty() {
                            None
                        } else {
                            Some(volumes)
                        },
                        automount_service_account_token: self.automount_service_account_token,
                        ..Default::default()
                    }),
                },
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create the Job, wait for completion, collect the pod logs, delete the Job.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] with `origin: "kubernetes"` if the
    /// Job cannot be created, the wait loop fails, or the wall-clock timeout is
    /// exceeded. A Job that ran and failed is **not** an error; it is reported
    /// via [`JobRunOutput::success`].
    pub async fn run(&self) -> Result<JobRunOutput, OperationError> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);

        jobs.create(&PostParams::default(), &self.build_job())
            .await
            .map_err(k8s_external)?;

        let waited = timeout(self.timeout, self.wait_terminal(&jobs)).await;

        // Background propagation deletes the Job's pods along with the Job.
        let dp = DeleteParams {
            propagation_policy: Some(PropagationPolicy::Background),
            ..Default::default()
        };
        let _ = jobs.delete(&self.name, &dp).await;

        match waited {
            Err(_elapsed) => Err(k8s_external(format!(
                "job '{}' did not finish within {:?}",
                self.name, self.timeout
            ))),
            Ok(Err(e)) => Err(e),
            Ok(Ok((phase, logs))) => Ok(JobRunOutput {
                success: phase == "Succeeded",
                logs,
                phase,
            }),
        }
    }

    /// Poll the Job until a terminal condition, then collect its pod logs.
    async fn wait_terminal(&self, jobs: &Api<Job>) -> Result<(String, String), OperationError> {
        loop {
            let job = jobs.get(&self.name).await.map_err(k8s_external)?;
            if let Some(phase) = terminal_phase(&job) {
                let logs = self.collect_logs().await;
                return Ok((phase, logs));
            }
            sleep(self.poll_interval).await;
        }
    }

    /// Best-effort collection of the Job's pod logs. Returns an empty string if
    /// no pod is found or its logs cannot be read.
    async fn collect_logs(&self) -> String {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let lp = ListParams::default().labels(&format!("job-name={}", self.name));
        let Ok(list) = pods.list(&lp).await else {
            return String::new();
        };
        let Some(pod_name) = list.items.first().and_then(|p| p.metadata.name.clone()) else {
            return String::new();
        };
        pods.logs(&pod_name, &LogParams::default())
            .await
            .unwrap_or_default()
    }
}

/// Derive a terminal phase from a Job's conditions, or `None` if still running.
///
/// A `Complete`/`True` condition maps to `"Succeeded"`, a `Failed`/`True`
/// condition to `"Failed"`.
fn terminal_phase(job: &Job) -> Option<String> {
    let conditions = job.status.as_ref()?.conditions.as_ref()?;
    conditions.iter().find_map(|c| {
        if c.status != "True" {
            return None;
        }
        match c.type_.as_str() {
            "Complete" => Some("Succeeded".to_string()),
            "Failed" => Some("Failed".to_string()),
            _ => None,
        }
    })
}

#[async_trait]
impl Operation for JobRun {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let output = self.run().await?;
        serde_json::to_value(&output).map_err(k8s_external)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "image": self.image,
            "namespace": self.namespace,
            "name": self.name,
            "command": self.command,
            "backoff_limit": self.backoff_limit,
        }))
    }
}

impl TypedOperation for JobRun {
    type Output = JobRunOutput;
}
