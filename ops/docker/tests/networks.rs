mod common;

use ironflow_ops_docker::networks::{NetworkCreate, NetworkInspect, NetworkList, NetworkRemove};

fn unique_name(prefix: &str) -> String {
    format!("ironflow-test-{prefix}-{}", std::process::id())
}

#[tokio::test]
async fn network_lifecycle() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let name = unique_name("net");

    let created = NetworkCreate::new(&client, &name).run(&ctx).await.unwrap();
    assert!(!created.id.is_empty());

    let inspect = NetworkInspect::new(&client, &name).run(&ctx).await.unwrap();
    assert!(inspect.data.is_object());

    let list = NetworkList::new(&client).run(&ctx).await.unwrap();
    let found = list.networks.iter().any(|n| n.name == name);
    assert!(found, "created network not found in list");

    NetworkRemove::new(&client, &name).run(&ctx).await.unwrap();
}
