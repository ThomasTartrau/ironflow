//! Tests for [`PodRun::sandboxed`](crate::pod_run::PodRun::sandboxed): the
//! hardened manifest `build_pod` renders.

use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use super::SANDBOX_UID;
use crate::pod_run::tests::dummy_kube;
use crate::pod_run::{PodRun, SecuritySpec};

fn sandboxed() -> PodRun {
    PodRun::new(&dummy_kube(), "p1", "busybox", "echo hi").sandboxed()
}

#[tokio::test]
async fn build_pod_sandboxed_hardens_the_container() {
    let spec = sandboxed().build_pod().spec.unwrap();
    let sc = spec.containers[0].security_context.as_ref().unwrap();
    assert_eq!(sc.read_only_root_filesystem, Some(true));
    assert_eq!(
        sc.capabilities.as_ref().unwrap().drop.as_deref(),
        Some(["ALL".to_string()].as_slice())
    );
    assert_eq!(sc.seccomp_profile.as_ref().unwrap().type_, "RuntimeDefault");
    assert_eq!(sc.allow_privilege_escalation, Some(false));
    assert_eq!(sc.run_as_non_root, Some(true));
    assert_eq!(sc.run_as_user, Some(SANDBOX_UID));
    assert_eq!(sc.run_as_group, Some(SANDBOX_UID));
}

#[tokio::test]
async fn build_pod_sandboxed_mounts_no_service_account_token() {
    let spec = sandboxed().build_pod().spec.unwrap();
    assert_eq!(spec.automount_service_account_token, Some(false));
    let psc = spec.security_context.unwrap();
    assert_eq!(psc.run_as_non_root, Some(true));
    assert_eq!(psc.fs_group, Some(SANDBOX_UID));
}

#[tokio::test]
async fn build_pod_sandboxed_mounts_tmp_on_an_empty_dir() {
    let spec = sandboxed().build_pod().spec.unwrap();
    let volume = &spec.volumes.as_ref().unwrap()[0];
    assert_eq!(volume.name, "ironflow-tmp");
    assert_eq!(
        volume.empty_dir.as_ref().unwrap().size_limit,
        Some(Quantity("512Mi".to_string()))
    );
    let mount = &spec.containers[0].volume_mounts.as_ref().unwrap()[0];
    assert_eq!(mount.name, "ironflow-tmp");
    assert_eq!(mount.mount_path, "/tmp");
    assert_eq!(mount.read_only, None);
}

#[tokio::test]
async fn build_pod_sandboxed_honours_tmp_size_limit() {
    let spec = sandboxed().tmp_size_limit("2Gi").build_pod().spec.unwrap();
    assert_eq!(
        spec.volumes.unwrap()[0]
            .empty_dir
            .as_ref()
            .unwrap()
            .size_limit,
        Some(Quantity("2Gi".to_string()))
    );
}

#[tokio::test]
#[should_panic(expected = "tmp_size_limit requires a PodRun built with .sandboxed()")]
async fn tmp_size_limit_without_sandboxed_panics() {
    let _ = PodRun::new(&dummy_kube(), "p1", "busybox", "echo hi").tmp_size_limit("2Gi");
}

#[tokio::test]
async fn build_pod_sandboxed_keeps_the_uid_of_security() {
    let spec = PodRun::new(&dummy_kube(), "p1", "busybox", "echo hi")
        .security(SecuritySpec {
            run_as_user: 1000,
            run_as_group: 1500,
            fs_group: 2000,
        })
        .sandboxed()
        .build_pod()
        .spec
        .unwrap();
    let sc = spec.containers[0].security_context.as_ref().unwrap();
    assert_eq!(sc.run_as_user, Some(1000));
    assert_eq!(sc.run_as_group, Some(1500));
    assert_eq!(sc.read_only_root_filesystem, Some(true));
    assert_eq!(spec.security_context.unwrap().fs_group, Some(2000));
}

#[tokio::test]
async fn build_pod_sandboxed_lets_explicit_relaxations_win() {
    let spec = sandboxed()
        .automount_service_account_token(true)
        .allow_privilege_escalation(true)
        .build_pod()
        .spec
        .unwrap();
    assert_eq!(spec.automount_service_account_token, Some(true));
    let sc = spec.containers[0].security_context.as_ref().unwrap();
    assert_eq!(sc.allow_privilege_escalation, Some(true));
    assert_eq!(sc.read_only_root_filesystem, Some(true));
}

#[tokio::test]
async fn build_pod_sandboxed_with_stdin_mounts_both_volumes() {
    let spec = sandboxed().stdin("x").build_pod().spec.unwrap();
    let names: Vec<&str> = spec
        .volumes
        .as_ref()
        .unwrap()
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(names, ["ironflow-tmp", "ironflow-stdin"]);
}

#[tokio::test]
async fn build_pod_without_sandboxed_leaves_the_root_filesystem_writable() {
    let spec = PodRun::new(&dummy_kube(), "p1", "busybox", "echo hi")
        .allow_privilege_escalation(false)
        .build_pod()
        .spec
        .unwrap();
    let sc = spec.containers[0].security_context.as_ref().unwrap();
    assert_eq!(sc.read_only_root_filesystem, None);
    assert_eq!(sc.capabilities, None);
    assert_eq!(sc.seccomp_profile, None);
    assert_eq!(spec.automount_service_account_token, None);
}
