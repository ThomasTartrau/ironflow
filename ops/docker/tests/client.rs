mod common;

use ironflow_ops_docker::DockerClient;

#[tokio::test]
async fn connect_local_and_ping() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let client = DockerClient::connect_local().unwrap();
    let ctx = common::ctx();
    use ironflow_ops_docker::system::SystemPing;
    let result = SystemPing::new(&client).run(&ctx).await.unwrap();
    assert_eq!(result.response, "OK");
}

#[test]
fn debug_does_not_leak_internals() {
    if let Ok(client) = DockerClient::connect_local() {
        let debug = format!("{client:?}");
        assert!(debug.contains("DockerClient"));
        assert!(!debug.contains("docker.sock"));
    }
}
