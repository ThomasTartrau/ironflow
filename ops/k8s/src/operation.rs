//! [`KubeOp`] -- wraps a Kubernetes verb as a tracked [`Operation`].

use std::fmt::Debug;

use async_trait::async_trait;
use either::Either;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use k8s_openapi::serde::de::DeserializeOwned;
use kube::Resource;
use kube::ResourceExt;
use kube::api::Api;
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::{kube_err, to_json};
use crate::verb;
use crate::verb::Verb;

/// A Kubernetes operation wrapped as an Ironflow [`Operation`].
///
/// Created via [`KubeClient::op`](crate::KubeClient::op). Implements
/// `Operation` so it can be passed to `WorkflowContext::operation()` for
/// step lifecycle tracking (step record, status transitions, duration,
/// output persistence).
///
/// Type parameters:
/// - `R`: the Kubernetes resource type (e.g. `Pod`, `Deployment`)
/// - `V`: the verb type (e.g. [`verb::List`], [`verb::Get`])
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::{KubeClient, KubeOp, verb};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use k8s_openapi::api::core::v1::Pod;
/// use kube::Config;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Config::infer().await?;
/// let kube = KubeClient::from_config(config).await?;
/// let pods = kube.namespaced::<Pod>("default");
/// let op = kube.op(pods, verb::List::default());
///
/// assert_eq!(op.kind(), "k8s");
/// # Ok(())
/// # }
/// ```
pub struct KubeOp<R, V>
where
    R: Resource,
{
    api: Api<R>,
    verb: V,
}

impl<R, V> KubeOp<R, V>
where
    R: Resource,
{
    pub(crate) fn new(api: Api<R>, verb: V) -> Self {
        Self { api, verb }
    }
}

/// Common trait bounds for Kubernetes resource types used in [`Operation`] impls.
trait KubeResource:
    Resource<DynamicType = ()> + Clone + DeserializeOwned + Serialize + Debug + Send + Sync + 'static
{
}
impl<T> KubeResource for T where
    T: Resource<DynamicType = ()>
        + Clone
        + DeserializeOwned
        + Serialize
        + Debug
        + Send
        + Sync
        + 'static
{
}

fn either_to_json<L: Serialize, R: Serialize>(
    either: Either<L, R>,
) -> Result<Value, OperationError> {
    match either {
        Either::Left(val) => to_json(&val),
        Either::Right(val) => to_json(&val),
    }
}

// -- List --

#[async_trait]
impl<R: KubeResource> Operation for KubeOp<R, verb::List> {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let list = self.api.list(&self.verb.params).await.map_err(kube_err)?;
        to_json(&list.items)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "verb": self.verb.name(),
            "resource": R::kind(&()),
        }))
    }
}

// -- Get --

#[async_trait]
impl<R: KubeResource> Operation for KubeOp<R, verb::Get> {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let obj = self.api.get(&self.verb.name).await.map_err(kube_err)?;
        to_json(&obj)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "verb": self.verb.name(),
            "resource": R::kind(&()),
            "name": self.verb.name,
        }))
    }
}

// -- Create --

#[async_trait]
impl<R: KubeResource> Operation for KubeOp<R, verb::Create<R>> {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let obj = self
            .api
            .create(&self.verb.params, &self.verb.data)
            .await
            .map_err(kube_err)?;
        to_json(&obj)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "verb": self.verb.name(),
            "resource": R::kind(&()),
        }))
    }
}

// -- Update --

#[async_trait]
impl<R: KubeResource> Operation for KubeOp<R, verb::Update<R>> {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let name = self.verb.data.name_any();
        if name.is_empty() {
            return Err(OperationError::Http {
                status: None,
                message: "resource metadata.name is required for update".to_string(),
            });
        }
        let obj = self
            .api
            .replace(&name, &self.verb.params, &self.verb.data)
            .await
            .map_err(kube_err)?;
        to_json(&obj)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "verb": self.verb.name(),
            "resource": R::kind(&()),
        }))
    }
}

// -- Patch --

#[async_trait]
impl<R, P> Operation for KubeOp<R, verb::Patch<P>>
where
    R: KubeResource,
    P: Serialize + Debug + Send + Sync + 'static,
{
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let obj = self
            .api
            .patch(&self.verb.name, &self.verb.params, &self.verb.patch)
            .await
            .map_err(kube_err)?;
        to_json(&obj)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "verb": self.verb.name(),
            "resource": R::kind(&()),
            "name": self.verb.name,
        }))
    }
}

// -- Delete --

#[async_trait]
impl<R: KubeResource> Operation for KubeOp<R, verb::Delete> {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let result = self
            .api
            .delete(&self.verb.name, &self.verb.params)
            .await
            .map_err(kube_err)?;
        either_to_json(result)
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "verb": self.verb.name(),
            "resource": R::kind(&()),
            "name": self.verb.name,
        }))
    }
}

// -- DeleteCollection --

#[async_trait]
impl<R: KubeResource> Operation for KubeOp<R, verb::DeleteCollection> {
    fn kind(&self) -> &str {
        "k8s"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let result = self
            .api
            .delete_collection(&self.verb.delete_params, &self.verb.list_params)
            .await
            .map_err(kube_err)?;
        match result {
            Either::Left(list) => to_json(&list.items),
            Either::Right(status) => to_json(&status),
        }
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "verb": self.verb.name(),
            "resource": R::kind(&()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;

    use http::{Request, Response};
    use hyper::body::Bytes;
    use ironflow_core::operation::{NoopSecretResolver, OperationContext};
    use k8s_openapi::api::core::v1::Pod;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use serde_json::json;
    use tower::service_fn;

    use super::*;

    fn dummy_client() -> kube::Client {
        let svc = service_fn(|_req: Request<kube::client::Body>| async {
            Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
                b"{}",
            ))))
        });
        kube::Client::new(svc, "default")
    }

    fn json_client(body: &'static str) -> kube::Client {
        let svc = service_fn(move |_req: Request<kube::client::Body>| async move {
            Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
                body.as_bytes(),
            ))))
        });
        kube::Client::new(svc, "default")
    }

    fn op_ctx() -> OperationContext {
        OperationContext::new(Arc::new(NoopSecretResolver))
    }

    // -- kind() and input() --

    #[tokio::test]
    async fn kind_returns_k8s() {
        let api: Api<Pod> = Api::namespaced(dummy_client(), "default");
        let op = KubeOp::new(api, verb::List::default());
        assert_eq!(op.kind(), "k8s");
    }

    #[tokio::test]
    async fn input_returns_metadata() {
        let api: Api<Pod> = Api::namespaced(dummy_client(), "default");
        let op = KubeOp::new(api, verb::Get::new("my-pod"));
        let input = op.input().unwrap();
        assert_eq!(input["verb"], "get");
        assert_eq!(input["resource"], "Pod");
        assert_eq!(input["name"], "my-pod");
    }

    #[tokio::test]
    async fn input_list_has_no_name() {
        let api: Api<Pod> = Api::namespaced(dummy_client(), "default");
        let op = KubeOp::new(api, verb::List::default());
        let input = op.input().unwrap();
        assert_eq!(input["verb"], "list");
        assert_eq!(input["resource"], "Pod");
        assert!(input.get("name").is_none());
    }

    #[tokio::test]
    async fn input_create_has_resource_kind() {
        let api: Api<Pod> = Api::namespaced(dummy_client(), "default");
        let op = KubeOp::new(api, verb::Create::with_data(Pod::default()));
        let input = op.input().unwrap();
        assert_eq!(input["verb"], "create");
        assert_eq!(input["resource"], "Pod");
    }

    #[tokio::test]
    async fn input_delete_has_name() {
        let api: Api<Pod> = Api::namespaced(dummy_client(), "default");
        let op = KubeOp::new(api, verb::Delete::new("to-delete"));
        let input = op.input().unwrap();
        assert_eq!(input["verb"], "delete");
        assert_eq!(input["name"], "to-delete");
    }

    #[tokio::test]
    async fn input_patch_has_name() {
        let api: Api<Pod> = Api::namespaced(dummy_client(), "default");
        let op = KubeOp::new(
            api,
            verb::Patch::new(
                "my-pod",
                kube::api::PatchParams::default(),
                kube::api::Patch::Merge(json!({})),
            ),
        );
        let input = op.input().unwrap();
        assert_eq!(input["verb"], "patch");
        assert_eq!(input["name"], "my-pod");
    }

    #[tokio::test]
    async fn input_delete_collection_has_no_name() {
        let api: Api<Pod> = Api::namespaced(dummy_client(), "default");
        let op = KubeOp::new(api, verb::DeleteCollection::default());
        let input = op.input().unwrap();
        assert_eq!(input["verb"], "delete_collection");
        assert!(input.get("name").is_none());
    }

    // -- execute() --

    #[tokio::test]
    async fn list_execute_returns_items_array() {
        let body = r#"{"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"1"},"items":[{"metadata":{"name":"pod-1","namespace":"default"},"spec":{"containers":[]},"status":{}}]}"#;
        let client = json_client(body);
        let api: Api<Pod> = Api::namespaced(client, "default");
        let op = KubeOp::new(api, verb::List::default());

        let ctx = op_ctx();
        let result = op.execute(&ctx).await.unwrap();
        let items = result.as_array().expect("should be array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["metadata"]["name"], "pod-1");
    }

    #[tokio::test]
    async fn get_execute_returns_object() {
        let body = r#"{"kind":"Pod","apiVersion":"v1","metadata":{"name":"my-pod","namespace":"default"},"spec":{"containers":[]},"status":{}}"#;
        let client = json_client(body);
        let api: Api<Pod> = Api::namespaced(client, "default");
        let op = KubeOp::new(api, verb::Get::new("my-pod"));

        let ctx = op_ctx();
        let result = op.execute(&ctx).await.unwrap();
        assert_eq!(result["metadata"]["name"], "my-pod");
    }

    #[tokio::test]
    async fn create_execute_returns_created_object() {
        let body = r#"{"kind":"Pod","apiVersion":"v1","metadata":{"name":"new-pod","namespace":"default"},"spec":{"containers":[]},"status":{}}"#;
        let client = json_client(body);
        let api: Api<Pod> = Api::namespaced(client, "default");
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("new-pod".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let op = KubeOp::new(api, verb::Create::with_data(pod));

        let ctx = op_ctx();
        let result = op.execute(&ctx).await.unwrap();
        assert_eq!(result["metadata"]["name"], "new-pod");
    }

    #[tokio::test]
    async fn update_execute_returns_replaced_object() {
        let body = r#"{"kind":"Pod","apiVersion":"v1","metadata":{"name":"existing","namespace":"default"},"spec":{"containers":[]},"status":{}}"#;
        let client = json_client(body);
        let api: Api<Pod> = Api::namespaced(client, "default");
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("existing".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let op = KubeOp::new(api, verb::Update::with_data(pod));

        let ctx = op_ctx();
        let result = op.execute(&ctx).await.unwrap();
        assert_eq!(result["metadata"]["name"], "existing");
    }

    #[tokio::test]
    async fn update_execute_rejects_missing_name() {
        let client = dummy_client();
        let api: Api<Pod> = Api::namespaced(client, "default");
        let pod = Pod::default();
        let op = KubeOp::new(api, verb::Update::with_data(pod));

        let ctx = op_ctx();
        let err = op.execute(&ctx).await.unwrap_err();
        assert!(
            err.to_string().contains("metadata.name is required"),
            "expected name-required error, got: {err}"
        );
    }

    #[tokio::test]
    async fn patch_execute_returns_patched_object() {
        let body = r#"{"kind":"Pod","apiVersion":"v1","metadata":{"name":"my-pod","namespace":"default","labels":{"env":"prod"}},"spec":{"containers":[]},"status":{}}"#;
        let client = json_client(body);
        let api: Api<Pod> = Api::namespaced(client, "default");
        let op = KubeOp::new(
            api,
            verb::Patch::new(
                "my-pod",
                kube::api::PatchParams::default(),
                kube::api::Patch::Merge(json!({"metadata":{"labels":{"env":"prod"}}})),
            ),
        );

        let ctx = op_ctx();
        let result = op.execute(&ctx).await.unwrap();
        assert_eq!(result["metadata"]["labels"]["env"], "prod");
    }

    #[tokio::test]
    async fn delete_execute_returns_object_or_status() {
        let body = r#"{"kind":"Pod","apiVersion":"v1","metadata":{"name":"gone","namespace":"default"},"spec":{"containers":[]},"status":{}}"#;
        let client = json_client(body);
        let api: Api<Pod> = Api::namespaced(client, "default");
        let op = KubeOp::new(api, verb::Delete::new("gone"));

        let ctx = op_ctx();
        let result = op.execute(&ctx).await.unwrap();
        assert!(result.is_object());
    }

    #[tokio::test]
    async fn delete_collection_execute_returns_array_or_status() {
        let body =
            r#"{"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"1"},"items":[]}"#;
        let client = json_client(body);
        let api: Api<Pod> = Api::namespaced(client, "default");
        let op = KubeOp::new(api, verb::DeleteCollection::default());

        let ctx = op_ctx();
        let result = op.execute(&ctx).await.unwrap();
        assert!(result.is_array());
    }
}
