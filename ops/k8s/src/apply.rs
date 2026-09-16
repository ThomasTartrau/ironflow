//! Server-side apply of [`ApplyConfigMap`] and [`ApplySecret`] as tracked
//! [`Operation`]s.
//!
//! Both operations upsert a resource from a key/value map via Kubernetes
//! server-side apply (`kubectl apply --server-side`). Repeated applies with
//! the same field manager are idempotent.

use std::collections::BTreeMap;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, Patch, PatchParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::KubeClient;
use crate::error::k8s_external;

#[cfg(test)]
mod tests;

/// Field manager used for server-side apply of these operations.
const FIELD_MANAGER: &str = "ironflow";

/// Output of an [`ApplyConfigMap`] or [`ApplySecret`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyOutput {
    /// Name of the applied resource.
    pub name: String,
    /// Namespace of the applied resource.
    pub namespace: String,
}

/// Server-side apply a `ConfigMap` from a key/value map.
///
/// # Examples
///
/// ```no_run
/// use std::collections::BTreeMap;
/// use ironflow_ops_k8s::{KubeClient, apply::ApplyConfigMap};
/// use kube::Config;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let config = Config::infer().await.expect("kubeconfig");
/// let kube = KubeClient::from_config(config).await?;
/// let mut data = BTreeMap::new();
/// data.insert("LOG_LEVEL".to_string(), "info".to_string());
/// let out = ApplyConfigMap::new(&kube, "ci", "app-config", data).run().await?;
/// assert_eq!(out.name, "app-config");
/// # Ok(())
/// # }
/// ```
pub struct ApplyConfigMap {
    client: kube::Client,
    name: String,
    namespace: String,
    data: BTreeMap<String, String>,
}

impl ApplyConfigMap {
    /// Create a config-map apply operation from a key/value map.
    pub fn new(
        client: &KubeClient,
        namespace: &str,
        name: &str,
        data: BTreeMap<String, String>,
    ) -> Self {
        Self {
            client: client.client().clone(),
            name: name.to_string(),
            namespace: namespace.to_string(),
            data,
        }
    }

    /// Build the [`ConfigMap`] manifest. Pure: performs no I/O.
    pub fn build_config_map(&self) -> ConfigMap {
        ConfigMap {
            metadata: ObjectMeta {
                name: Some(self.name.clone()),
                namespace: Some(self.namespace.clone()),
                ..Default::default()
            },
            data: Some(self.data.clone()),
            ..Default::default()
        }
    }

    /// Server-side apply the ConfigMap.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] with `origin: "kubernetes"` if the
    /// apply request fails.
    pub async fn run(&self) -> Result<ApplyOutput, OperationError> {
        let api: Api<ConfigMap> = Api::namespaced(self.client.clone(), &self.namespace);
        let body = apply_body(&self.build_config_map(), "v1", "ConfigMap")?;
        api.patch(
            &self.name,
            &PatchParams::apply(FIELD_MANAGER),
            &Patch::Apply(body),
        )
        .await
        .map_err(k8s_external)?;
        Ok(ApplyOutput {
            name: self.name.clone(),
            namespace: self.namespace.clone(),
        })
    }
}

#[async_trait]
impl Operation for ApplyConfigMap {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        serde_json::to_value(self.run().await?).map_err(k8s_external)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "namespace": self.namespace,
            "name": self.name,
            "keys": self.data.keys().collect::<Vec<_>>(),
        }))
    }
}

impl TypedOperation for ApplyConfigMap {
    type Output = ApplyOutput;
}

/// Server-side apply a `Secret` from a key/value map.
///
/// Values are placed in `stringData` (plaintext at apply time, stored
/// base64-encoded by Kubernetes). [`Operation::input`] exposes only the keys,
/// never the values, so secret material never reaches the step's input log.
///
/// # Examples
///
/// ```no_run
/// use std::collections::BTreeMap;
/// use ironflow_ops_k8s::{KubeClient, apply::ApplySecret};
/// use kube::Config;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let config = Config::infer().await.expect("kubeconfig");
/// let kube = KubeClient::from_config(config).await?;
/// let mut data = BTreeMap::new();
/// data.insert("API_TOKEN".to_string(), "s3cr3t".to_string());
/// let out = ApplySecret::new(&kube, "ci", "app-secrets", data).run().await?;
/// assert_eq!(out.name, "app-secrets");
/// # Ok(())
/// # }
/// ```
pub struct ApplySecret {
    client: kube::Client,
    name: String,
    namespace: String,
    data: BTreeMap<String, String>,
}

impl ApplySecret {
    /// Create a secret apply operation from a key/value map.
    pub fn new(
        client: &KubeClient,
        namespace: &str,
        name: &str,
        data: BTreeMap<String, String>,
    ) -> Self {
        Self {
            client: client.client().clone(),
            name: name.to_string(),
            namespace: namespace.to_string(),
            data,
        }
    }

    /// Build the [`Secret`] manifest with values in `stringData`. Pure: no I/O.
    pub fn build_secret(&self) -> Secret {
        Secret {
            metadata: ObjectMeta {
                name: Some(self.name.clone()),
                namespace: Some(self.namespace.clone()),
                ..Default::default()
            },
            string_data: Some(self.data.clone()),
            type_: Some("Opaque".to_string()),
            ..Default::default()
        }
    }

    /// Server-side apply the Secret.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] with `origin: "kubernetes"` if the
    /// apply request fails.
    pub async fn run(&self) -> Result<ApplyOutput, OperationError> {
        let api: Api<Secret> = Api::namespaced(self.client.clone(), &self.namespace);
        let body = apply_body(&self.build_secret(), "v1", "Secret")?;
        api.patch(
            &self.name,
            &PatchParams::apply(FIELD_MANAGER),
            &Patch::Apply(body),
        )
        .await
        .map_err(k8s_external)?;
        Ok(ApplyOutput {
            name: self.name.clone(),
            namespace: self.namespace.clone(),
        })
    }
}

#[async_trait]
impl Operation for ApplySecret {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        serde_json::to_value(self.run().await?).map_err(k8s_external)
    }

    fn input(&self) -> Option<Value> {
        // Keys only -- never the secret values.
        Some(json!({
            "namespace": self.namespace,
            "name": self.name,
            "keys": self.data.keys().collect::<Vec<_>>(),
        }))
    }
}

impl TypedOperation for ApplySecret {
    type Output = ApplyOutput;
}

/// Serialize a typed resource and inject `apiVersion`/`kind`, which
/// server-side apply requires but k8s-openapi types omit.
fn apply_body<T: Serialize>(
    resource: &T,
    api_version: &str,
    kind: &str,
) -> Result<Value, OperationError> {
    let mut body = serde_json::to_value(resource).map_err(k8s_external)?;
    body["apiVersion"] = json!(api_version);
    body["kind"] = json!(kind);
    Ok(body)
}
