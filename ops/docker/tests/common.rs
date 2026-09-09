use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, OperationContext};
use ironflow_ops_docker::DockerClient;

pub fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

pub fn client() -> Option<DockerClient> {
    DockerClient::connect_local().ok()
}

#[allow(dead_code)]
pub fn require_docker() -> (DockerClient, OperationContext) {
    let c = client().expect("Docker daemon not available - skipping");
    (c, ctx())
}

pub async fn docker_available() -> bool {
    if let Some(client) = client() {
        use ironflow_ops_docker::system::SystemPing;
        SystemPing::new(&client).run(&ctx()).await.is_ok()
    } else {
        false
    }
}
