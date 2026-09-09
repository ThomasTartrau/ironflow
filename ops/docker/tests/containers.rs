mod common;

use ironflow_ops_docker::containers::{
    ContainerCreate, ContainerInspect, ContainerList, ContainerRemove, ContainerStart,
    ContainerStop,
};

fn unique_name(prefix: &str) -> String {
    format!("ironflow-test-{prefix}-{}", std::process::id())
}

#[tokio::test]
async fn container_lifecycle() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let name = unique_name("lifecycle");

    let created = ContainerCreate::new(&client, &name, "alpine:latest")
        .cmd(vec!["sleep".to_string(), "300".to_string()])
        .run(&ctx)
        .await
        .unwrap();
    assert!(!created.id.is_empty());

    ContainerStart::new(&client, &name).run(&ctx).await.unwrap();

    let inspect = ContainerInspect::new(&client, &name)
        .run(&ctx)
        .await
        .unwrap();
    assert!(inspect.data.is_object());
    let state = inspect.data["State"]["Running"].as_bool();
    assert_eq!(state, Some(true));

    ContainerStop::new(&client, &name)
        .timeout(1)
        .run(&ctx)
        .await
        .unwrap();

    ContainerRemove::new(&client, &name)
        .run(&ctx)
        .await
        .unwrap();
}

#[tokio::test]
async fn container_list_includes_created() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let name = unique_name("list");

    ContainerCreate::new(&client, &name, "alpine:latest")
        .run(&ctx)
        .await
        .unwrap();

    let list = ContainerList::new(&client).all().run(&ctx).await.unwrap();
    let found = list
        .containers
        .iter()
        .any(|c| c.names.iter().any(|n| n.contains(&name)));
    assert!(found, "created container not found in list");

    ContainerRemove::new(&client, &name)
        .force()
        .run(&ctx)
        .await
        .unwrap();
}

#[tokio::test]
async fn error_nonexistent_container() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();

    let err = ContainerInspect::new(&client, "nonexistent-container-12345")
        .run(&ctx)
        .await;
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("docker error") || msg.contains("404") || msg.contains("No such container"),
        "unexpected error: {msg}"
    );
}
