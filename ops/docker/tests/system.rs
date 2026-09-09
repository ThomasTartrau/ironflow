mod common;

use ironflow_ops_docker::system::{SystemDf, SystemInfo, SystemPing, SystemVersion};

#[tokio::test]
async fn system_ping() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let result = SystemPing::new(&client).run(&ctx).await.unwrap();
    assert_eq!(result.response, "OK");
}

#[tokio::test]
async fn system_version() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let result = SystemVersion::new(&client).run(&ctx).await.unwrap();
    assert!(result.data.is_object());
}

#[tokio::test]
async fn system_info() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let result = SystemInfo::new(&client).run(&ctx).await.unwrap();
    assert!(result.data.is_object());
}

#[tokio::test]
async fn system_df() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let result = SystemDf::new(&client).run(&ctx).await.unwrap();
    assert!(result.data.is_object());
}
