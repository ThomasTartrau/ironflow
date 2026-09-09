mod common;

use ironflow_ops_docker::images::{ImageInspect, ImageList, ImagePull, ImageRemove, ImageTag};

#[tokio::test]
async fn image_pull_and_inspect() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();

    ImagePull::new(&client, "alpine:latest")
        .run(&ctx)
        .await
        .unwrap();

    let inspect = ImageInspect::new(&client, "alpine:latest")
        .run(&ctx)
        .await
        .unwrap();
    assert!(inspect.data.is_object());
}

#[tokio::test]
async fn image_list_contains_alpine() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();

    ImagePull::new(&client, "alpine:latest")
        .run(&ctx)
        .await
        .unwrap();

    let list = ImageList::new(&client).run(&ctx).await.unwrap();
    let found = list
        .images
        .iter()
        .any(|i| i.repo_tags.iter().any(|t| t.contains("alpine")));
    assert!(found, "alpine not found in image list");
}

#[tokio::test]
async fn image_tag_and_remove_tag() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();
    let tag_name = format!("ironflow-test-tag:{}", std::process::id());

    ImagePull::new(&client, "alpine:latest")
        .run(&ctx)
        .await
        .unwrap();

    ImageTag::new(
        &client,
        "alpine:latest",
        "ironflow-test-tag",
        format!("{}", std::process::id()),
    )
    .run(&ctx)
    .await
    .unwrap();

    let _ = ImageRemove::new(&client, &tag_name).run(&ctx).await;
}

#[tokio::test]
async fn error_nonexistent_image() {
    if !common::docker_available().await {
        eprintln!("Docker not available, skipping");
        return;
    }
    let (client, ctx) = common::require_docker();

    let err = ImageInspect::new(&client, "nonexistent-image:impossible-tag-12345")
        .run(&ctx)
        .await;
    assert!(err.is_err());
}
