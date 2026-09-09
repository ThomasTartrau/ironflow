mod common;

use ironflow_ops_docker::volumes::{VolumeCreate, VolumeInspect, VolumeList, VolumeRemove};

fn unique_name(prefix: &str) -> String {
    format!("ironflow-test-{prefix}-{}", std::process::id())
}

#[tokio::test]
async fn volume_lifecycle() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let name = unique_name("vol");

    let created = VolumeCreate::new(&client, &name).run(&ctx).await.unwrap();
    assert_eq!(created.name, name);
    assert!(!created.mountpoint.is_empty());

    let inspect = VolumeInspect::new(&client, &name).run(&ctx).await.unwrap();
    assert_eq!(inspect.name, name);

    let list = VolumeList::new(&client).run(&ctx).await.unwrap();
    let found = list.volumes.iter().any(|v| v.name == name);
    assert!(found, "created volume not found in list");

    VolumeRemove::new(&client, &name).run(&ctx).await.unwrap();
}
