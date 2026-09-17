//! [`PodRun`] -- run a command in an ephemeral pod as a tracked [`Operation`].

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use k8s_openapi::api::core::v1::{
    Container, LocalObjectReference, PersistentVolumeClaimVolumeSource, Pod, PodSecurityContext,
    PodSpec, ResourceRequirements, SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, LogParams, PostParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

use crate::KubeClient;
use crate::error::k8s_external;

#[cfg(test)]
mod tests;

/// Default wall-clock timeout for a [`PodRun`] if none is configured.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Default interval between pod status polls.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// CPU and memory requests/limits for a [`PodRun`] container.
///
/// Values are Kubernetes quantity strings (e.g. `"100m"`, `"256Mi"`). Each
/// field is optional; only the set ones are emitted into the pod spec.
#[derive(Debug, Clone, Default)]
pub struct ResourceSpec {
    /// CPU request (e.g. `"100m"`).
    pub cpu_request: Option<String>,
    /// CPU limit (e.g. `"500m"`).
    pub cpu_limit: Option<String>,
    /// Memory request (e.g. `"128Mi"`).
    pub memory_request: Option<String>,
    /// Memory limit (e.g. `"512Mi"`).
    pub memory_limit: Option<String>,
}

/// Security context for a [`PodRun`]: run as a non-root user with an explicit
/// uid/gid and an fsGroup for mounted volumes.
#[derive(Debug, Clone, Copy)]
pub struct SecuritySpec {
    /// `runAsUser` (container-level).
    pub run_as_user: i64,
    /// `runAsGroup` (container-level).
    pub run_as_group: i64,
    /// `fsGroup` (pod-level), applied to mounted volume ownership.
    pub fs_group: i64,
}

/// A PersistentVolumeClaim mounted into the [`PodRun`] container.
#[derive(Debug, Clone)]
pub struct PvcMount {
    /// Name of the PersistentVolumeClaim to mount.
    pub claim: String,
    /// Path at which the volume is mounted inside the container.
    pub mount_path: String,
}

/// Output of a [`PodRun`] operation.
///
/// `success` is `true` only when the pod reached the `Succeeded` phase, i.e.
/// the container's command exited zero. A pod whose command failed reports
/// `success: false` with `phase == "Failed"` -- this is **not** an error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodRunOutput {
    /// `true` when `phase == "Succeeded"`.
    pub success: bool,
    /// Container logs captured once the pod reached a terminal phase.
    pub logs: String,
    /// Terminal pod phase (`"Succeeded"` or `"Failed"`).
    pub phase: String,
}

/// Run a shell command in an ephemeral pod, wait for completion, collect its
/// logs, and delete the pod.
///
/// The command is executed via `/bin/sh -c`. The pod uses `restartPolicy:
/// Never` so it runs exactly once.
///
/// # Error vs. non-zero exit
///
/// A pod that ran but whose command exited non-zero (phase `Failed`) returns
/// `Ok(PodRunOutput { success: false, .. })`. An [`OperationError::External`]
/// with `origin: "kubernetes"` is returned only for infrastructure failures:
/// client init, pod creation, the wait loop, or a wall-clock timeout.
///
/// The pod is deleted (best-effort) on every path, including timeout and wait
/// errors.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::{KubeClient, pod_run::PodRun};
/// use kube::Config;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let config = Config::infer().await.expect("kubeconfig");
/// let kube = KubeClient::from_config(config).await?;
/// let output = PodRun::new(&kube, "run-tests", "rust:1.94", "cargo test")
///     .namespace("ci")
///     .run()
///     .await?;
/// assert!(output.success || !output.logs.is_empty());
/// # Ok(())
/// # }
/// ```
pub struct PodRun {
    client: kube::Client,
    name: String,
    namespace: String,
    image: String,
    command: String,
    working_dir: Option<String>,
    node_selector: BTreeMap<String, String>,
    image_pull_secret: Option<String>,
    pvc: Option<PvcMount>,
    resources: Option<ResourceSpec>,
    security: Option<SecuritySpec>,
    automount_service_account_token: Option<bool>,
    allow_privilege_escalation: Option<bool>,
    timeout: Duration,
    poll_interval: Duration,
}

impl PodRun {
    /// Create a pod-run operation. `command` is executed via `/bin/sh -c`.
    ///
    /// Defaults: namespace `"default"`, a 300s wall-clock timeout, and a 2s
    /// poll interval.
    pub fn new(client: &KubeClient, name: &str, image: &str, command: &str) -> Self {
        Self {
            client: client.client().clone(),
            name: name.to_string(),
            namespace: "default".to_string(),
            image: image.to_string(),
            command: command.to_string(),
            working_dir: None,
            node_selector: BTreeMap::new(),
            image_pull_secret: None,
            pvc: None,
            resources: None,
            security: None,
            automount_service_account_token: None,
            allow_privilege_escalation: None,
            timeout: DEFAULT_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    /// Set the namespace the pod is created in.
    #[must_use]
    pub fn namespace(mut self, namespace: &str) -> Self {
        self.namespace = namespace.to_string();
        self
    }

    /// Set the container's working directory.
    #[must_use]
    pub fn working_dir(mut self, working_dir: &str) -> Self {
        self.working_dir = Some(working_dir.to_string());
        self
    }

    /// Add a `nodeSelector` label the pod must match to be scheduled.
    #[must_use]
    pub fn node_selector(mut self, key: &str, value: &str) -> Self {
        self.node_selector
            .insert(key.to_string(), value.to_string());
        self
    }

    /// Set an `imagePullSecret` used to pull the container image.
    #[must_use]
    pub fn image_pull_secret(mut self, secret_name: &str) -> Self {
        self.image_pull_secret = Some(secret_name.to_string());
        self
    }

    /// Mount a PersistentVolumeClaim at the given path.
    #[must_use]
    pub fn pvc(mut self, claim: &str, mount_path: &str) -> Self {
        self.pvc = Some(PvcMount {
            claim: claim.to_string(),
            mount_path: mount_path.to_string(),
        });
        self
    }

    /// Set CPU/memory requests and limits.
    #[must_use]
    pub fn resources(mut self, resources: ResourceSpec) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Run the container as a non-root user with the given uid/gid and fsGroup.
    #[must_use]
    pub fn security(mut self, security: SecuritySpec) -> Self {
        self.security = Some(security);
        self
    }

    /// Set `PodSpec.automountServiceAccountToken`.
    ///
    /// Pass `false` to prevent the default ServiceAccount token from being
    /// mounted into the pod, hardening it against token exfiltration. Opt-in:
    /// if this builder is never called the field is left absent and Kubernetes
    /// applies its default (mount the token).
    #[must_use]
    pub fn automount_service_account_token(mut self, automount: bool) -> Self {
        self.automount_service_account_token = Some(automount);
        self
    }

    /// Set the container's `SecurityContext.allowPrivilegeEscalation`.
    ///
    /// Pass `false` to forbid a process from gaining more privileges than its
    /// parent, hardening the container. Opt-in: if this builder is never called
    /// the field is left absent and Kubernetes applies its default.
    #[must_use]
    pub fn allow_privilege_escalation(mut self, allow: bool) -> Self {
        self.allow_privilege_escalation = Some(allow);
        self
    }

    /// Set the wall-clock timeout for the whole run (create -> terminal phase).
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the interval between pod status polls.
    #[must_use]
    pub fn poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Build the [`Pod`] manifest for this run. Pure: performs no I/O.
    ///
    /// Exposed for testing the manifest without a cluster.
    pub fn build_pod(&self) -> Pod {
        let mut container = Container {
            name: self.name.clone(),
            image: Some(self.image.clone()),
            command: Some(vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                self.command.clone(),
            ]),
            working_dir: self.working_dir.clone(),
            ..Default::default()
        };

        if let Some(res) = &self.resources {
            container.resources = Some(build_resources(res));
        }

        // Build a container SecurityContext when either the non-root spec or the
        // privilege-escalation toggle is set, merging both so neither overwrites
        // the other. Left absent entirely when neither is set (opt-in, no
        // regression for existing callers).
        if self.security.is_some() || self.allow_privilege_escalation.is_some() {
            let mut sc = SecurityContext {
                allow_privilege_escalation: self.allow_privilege_escalation,
                ..Default::default()
            };
            if let Some(sec) = &self.security {
                sc.run_as_non_root = Some(true);
                sc.run_as_user = Some(sec.run_as_user);
                sc.run_as_group = Some(sec.run_as_group);
            }
            container.security_context = Some(sc);
        }

        let mut volumes = Vec::new();
        if let Some(pvc) = &self.pvc {
            container.volume_mounts = Some(vec![VolumeMount {
                name: "workspace".to_string(),
                mount_path: pvc.mount_path.clone(),
                ..Default::default()
            }]);
            volumes.push(Volume {
                name: "workspace".to_string(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: pvc.claim.clone(),
                    read_only: None,
                }),
                ..Default::default()
            });
        }

        let node_selector = if self.node_selector.is_empty() {
            None
        } else {
            Some(self.node_selector.clone())
        };

        let image_pull_secrets = self
            .image_pull_secret
            .as_ref()
            .map(|name| vec![LocalObjectReference { name: name.clone() }]);

        let security_context = self.security.map(|sec| PodSecurityContext {
            run_as_non_root: Some(true),
            fs_group: Some(sec.fs_group),
            ..Default::default()
        });

        Pod {
            metadata: ObjectMeta {
                name: Some(self.name.clone()),
                namespace: Some(self.namespace.clone()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![container],
                restart_policy: Some("Never".to_string()),
                node_selector,
                image_pull_secrets,
                volumes: if volumes.is_empty() {
                    None
                } else {
                    Some(volumes)
                },
                security_context,
                automount_service_account_token: self.automount_service_account_token,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create the pod, wait for it to finish, collect its logs, and delete it.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] with `origin: "kubernetes"` if the
    /// pod cannot be created, the wait loop fails, or the wall-clock timeout is
    /// exceeded. A command that ran and exited non-zero is **not** an error;
    /// it is reported via [`PodRunOutput::success`].
    pub async fn run(&self) -> Result<PodRunOutput, OperationError> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);

        pods.create(&PostParams::default(), &self.build_pod())
            .await
            .map_err(k8s_external)?;

        // Wait for a terminal phase, bounded by the wall-clock timeout. The pod
        // is deleted best-effort on every path below, including timeout.
        let waited = timeout(self.timeout, self.wait_terminal(&pods)).await;

        let _ = pods.delete(&self.name, &DeleteParams::default()).await;

        match waited {
            Err(_elapsed) => Err(k8s_external(format!(
                "pod '{}' did not finish within {:?}",
                self.name, self.timeout
            ))),
            Ok(Err(e)) => Err(e),
            Ok(Ok((phase, logs))) => Ok(PodRunOutput {
                success: phase == "Succeeded",
                logs,
                phase,
            }),
        }
    }

    /// Poll the pod until it reaches a terminal phase, then collect its logs.
    async fn wait_terminal(&self, pods: &Api<Pod>) -> Result<(String, String), OperationError> {
        loop {
            let pod = pods.get(&self.name).await.map_err(k8s_external)?;
            let phase = pod.status.and_then(|s| s.phase).unwrap_or_default();

            if phase == "Succeeded" || phase == "Failed" {
                // Logs are best-effort: a terminal pod with no readable log
                // stream still yields a valid result.
                let logs = pods
                    .logs(&self.name, &LogParams::default())
                    .await
                    .unwrap_or_default();
                return Ok((phase, logs));
            }

            sleep(self.poll_interval).await;
        }
    }
}

/// Translate a [`ResourceSpec`] into a [`ResourceRequirements`], omitting empty
/// requests/limits maps.
fn build_resources(spec: &ResourceSpec) -> ResourceRequirements {
    let mut requests = BTreeMap::new();
    let mut limits = BTreeMap::new();

    if let Some(cpu) = &spec.cpu_request {
        requests.insert("cpu".to_string(), Quantity(cpu.clone()));
    }
    if let Some(mem) = &spec.memory_request {
        requests.insert("memory".to_string(), Quantity(mem.clone()));
    }
    if let Some(cpu) = &spec.cpu_limit {
        limits.insert("cpu".to_string(), Quantity(cpu.clone()));
    }
    if let Some(mem) = &spec.memory_limit {
        limits.insert("memory".to_string(), Quantity(mem.clone()));
    }

    ResourceRequirements {
        requests: if requests.is_empty() {
            None
        } else {
            Some(requests)
        },
        limits: if limits.is_empty() {
            None
        } else {
            Some(limits)
        },
        ..Default::default()
    }
}

#[async_trait]
impl Operation for PodRun {
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
        }))
    }
}

impl TypedOperation for PodRun {
    type Output = PodRunOutput;
}
