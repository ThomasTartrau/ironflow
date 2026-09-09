//! High-level helpers for common multi-step Kubernetes operations.
//!
//! These helpers compose lower-level [`kube::Api`] calls into common
//! workflows that would otherwise require multiple API calls and
//! careful orchestration.
//!
//! # Available helpers
//!
//! | Helper | Description |
//! |--------|-------------|
//! | [`Rollout`] | Manage rollouts for Deployments, StatefulSets, and DaemonSets |
//! | [`Drain`] | Cordon a node and evict its pods |
//! | [`TriggerCronJob`] | Create a one-off Job from a CronJob template |

use chrono::Utc;
use ironflow_core::error::OperationError;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Node, Pod};
use k8s_openapi::api::policy::v1::Eviction;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::ResourceExt;
use kube::api::{Api, ListParams, Patch, PatchParams, PostParams};
use serde_json::json;

use crate::KubeClient;
use crate::error::kube_err;

/// Rollout management for workloads (Deployment, StatefulSet, DaemonSet).
///
/// Provides pause, resume, restart, and undo operations that mirror
/// `kubectl rollout` commands.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::{KubeClient, helpers::Rollout};
/// use kube::Config;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let config = Config::infer().await.expect("kubeconfig");
/// let kube = KubeClient::from_config(config).await?;
/// let rollout = Rollout::deployment(&kube, "default");
/// rollout.restart("my-deployment").await?;
/// # Ok(())
/// # }
/// ```
pub struct Rollout<R> {
    api: Api<R>,
}

impl Rollout<Deployment> {
    /// Create a rollout helper for Deployments in the given namespace.
    pub fn deployment(client: &KubeClient, namespace: &str) -> Self {
        Self {
            api: client.namespaced(namespace),
        }
    }
}

impl Rollout<StatefulSet> {
    /// Create a rollout helper for StatefulSets in the given namespace.
    pub fn stateful_set(client: &KubeClient, namespace: &str) -> Self {
        Self {
            api: client.namespaced(namespace),
        }
    }
}

impl Rollout<DaemonSet> {
    /// Create a rollout helper for DaemonSets in the given namespace.
    pub fn daemon_set(client: &KubeClient, namespace: &str) -> Self {
        Self {
            api: client.namespaced(namespace),
        }
    }
}

macro_rules! impl_rollout {
    ($resource:ty) => {
        impl Rollout<$resource> {
            /// Pause the rollout of a workload.
            ///
            /// Equivalent to `kubectl rollout pause`.
            ///
            /// # Errors
            ///
            /// Returns [`OperationError::Http`] if the patch fails.
            pub async fn pause(&self, name: &str) -> Result<(), OperationError> {
                let patch = json!({ "spec": { "paused": true } });
                self.api
                    .patch(name, &PatchParams::default(), &Patch::Strategic(patch))
                    .await
                    .map_err(kube_err)?;
                Ok(())
            }

            /// Resume a paused rollout.
            ///
            /// Equivalent to `kubectl rollout resume`.
            ///
            /// # Errors
            ///
            /// Returns [`OperationError::Http`] if the patch fails.
            pub async fn resume(&self, name: &str) -> Result<(), OperationError> {
                let patch = json!({ "spec": { "paused": false } });
                self.api
                    .patch(name, &PatchParams::default(), &Patch::Strategic(patch))
                    .await
                    .map_err(kube_err)?;
                Ok(())
            }

            /// Trigger a rolling restart by patching the pod template annotation.
            ///
            /// Equivalent to `kubectl rollout restart`.
            ///
            /// # Errors
            ///
            /// Returns [`OperationError::Http`] if the patch fails.
            pub async fn restart(&self, name: &str) -> Result<(), OperationError> {
                let now = Utc::now().to_rfc3339();
                let patch = json!({
                    "spec": {
                        "template": {
                            "metadata": {
                                "annotations": {
                                    "kubectl.kubernetes.io/restartedAt": now
                                }
                            }
                        }
                    }
                });
                self.api
                    .patch(name, &PatchParams::default(), &Patch::Strategic(patch))
                    .await
                    .map_err(kube_err)?;
                Ok(())
            }
        }
    };
}

impl_rollout!(Deployment);
impl_rollout!(StatefulSet);
impl_rollout!(DaemonSet);

/// Drain a Kubernetes node by cordoning it and evicting its pods.
///
/// Mirrors the behavior of `kubectl drain --ignore-daemonsets --delete-emptydir-data`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::{KubeClient, helpers::Drain};
/// use kube::Config;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let config = Config::infer().await.expect("kubeconfig");
/// let kube = KubeClient::from_config(config).await?;
/// let drain = Drain::new(&kube);
/// drain.execute("worker-node-1").await?;
/// # Ok(())
/// # }
/// ```
pub struct Drain {
    nodes: Api<Node>,
    client: kube::Client,
}

impl Drain {
    /// Create a drain helper using the given client.
    pub fn new(kube: &KubeClient) -> Self {
        Self {
            nodes: kube.all(),
            client: kube.client().clone(),
        }
    }

    /// Cordon and drain a node.
    ///
    /// 1. Marks the node as unschedulable (cordon).
    /// 2. Lists all non-DaemonSet pods on the node.
    /// 3. Evicts each pod.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if any API call fails.
    pub async fn execute(&self, node_name: &str) -> Result<Vec<String>, OperationError> {
        let patch = json!({ "spec": { "unschedulable": true } });
        self.nodes
            .patch(node_name, &PatchParams::default(), &Patch::Strategic(patch))
            .await
            .map_err(kube_err)?;

        let pods: Api<Pod> = Api::all(self.client.clone());
        let lp = ListParams::default().fields(&format!("spec.nodeName={node_name}"));
        let pod_list = pods.list(&lp).await.map_err(kube_err)?;

        let mut evicted = Vec::new();
        for pod in &pod_list.items {
            let pod_name = pod.name_any();
            let ns = pod.namespace().unwrap_or_else(|| "default".to_string());

            if is_daemonset_pod(pod) {
                continue;
            }

            let ns_pods: Api<Pod> = Api::namespaced(self.client.clone(), &ns);
            let eviction = Eviction {
                metadata: ObjectMeta {
                    name: Some(pod_name.clone()),
                    namespace: Some(ns),
                    ..Default::default()
                },
                delete_options: None,
            };

            let _: Eviction = ns_pods
                .create_subresource("eviction", &pod_name, &PostParams::default(), &eviction)
                .await
                .map_err(kube_err)?;

            evicted.push(pod_name);
        }

        Ok(evicted)
    }

    /// Uncordon a node (mark as schedulable again).
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the patch fails.
    pub async fn uncordon(&self, node_name: &str) -> Result<(), OperationError> {
        let patch = json!({ "spec": { "unschedulable": false } });
        self.nodes
            .patch(node_name, &PatchParams::default(), &Patch::Strategic(patch))
            .await
            .map_err(kube_err)?;
        Ok(())
    }
}

fn is_daemonset_pod(pod: &Pod) -> bool {
    pod.metadata
        .owner_references
        .as_ref()
        .is_some_and(|refs| refs.iter().any(|r| r.kind == "DaemonSet"))
}

/// Create a one-off Job from a CronJob template.
///
/// Equivalent to `kubectl create job --from=cronjob/<name>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::{KubeClient, helpers::TriggerCronJob};
/// use kube::Config;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let config = Config::infer().await.expect("kubeconfig");
/// let kube = KubeClient::from_config(config).await?;
/// let trigger = TriggerCronJob::new(&kube, "default");
/// let job = trigger.run("my-cronjob", "manual-run-1").await?;
/// # Ok(())
/// # }
/// ```
pub struct TriggerCronJob {
    cronjobs: Api<CronJob>,
    jobs: Api<Job>,
}

impl TriggerCronJob {
    /// Create a trigger helper for CronJobs in the given namespace.
    pub fn new(kube: &KubeClient, namespace: &str) -> Self {
        Self {
            cronjobs: kube.namespaced(namespace),
            jobs: kube.namespaced(namespace),
        }
    }

    /// Fetch the CronJob and create a Job from its template.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the CronJob cannot be fetched,
    /// the Job cannot be created, or the CronJob has no job template spec.
    pub async fn run(&self, cronjob_name: &str, job_name: &str) -> Result<Job, OperationError> {
        let cj = self.cronjobs.get(cronjob_name).await.map_err(kube_err)?;

        let job_template = cj
            .spec
            .as_ref()
            .and_then(|s| s.job_template.spec.clone())
            .ok_or_else(|| OperationError::Http {
                status: None,
                message: format!("CronJob '{cronjob_name}' has no job template spec"),
            })?;

        let job = Job {
            metadata: ObjectMeta {
                name: Some(job_name.to_string()),
                annotations: Some(
                    [(
                        "cronjob.kubernetes.io/instantiate".to_string(),
                        "manual".to_string(),
                    )]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
            spec: Some(job_template),
            ..Default::default()
        };

        let created = self
            .jobs
            .create(&PostParams::default(), &job)
            .await
            .map_err(kube_err)?;

        Ok(created)
    }
}
