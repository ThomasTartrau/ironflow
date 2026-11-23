use std::collections::BTreeMap;

use serde_json::Value;

use crate::provider::{LABEL_RUN_ID, PodVolumeSource, PvcVolume, ReadOnlyVolume, SessionsVolume};
use crate::providers::claude::k8s::common::tests::{
    find_by_name, hardened_config, hardening_err, pod_json, ro, sandboxed, step_pvc,
};
use crate::providers::claude::k8s::common::{PodHardening, SANDBOX_HOME, SandboxSettings};
use crate::providers::claude::k8s::profile::ClaudeProfile;

use super::*;

const RUN_A: &str = "0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d";
const RUN_B: &str = "0192f0c1-7d2e-7a4b-9c3d-ffffffffffff";

fn step_sessions() -> SessionsVolume {
    SessionsVolume {
        claim_name: "workspace".to_string(),
        dir: "claude-sessions".to_string(),
    }
}

fn run_labels(run_id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(LABEL_RUN_ID.to_string(), run_id.to_string())])
}

fn resolved(run_id: &str) -> SessionsMount {
    resolve_sessions(None, true, Some(&step_sessions()), &run_labels(run_id))
        .unwrap()
        .unwrap()
}

fn mounts(pod: &Value) -> &Vec<Value> {
    pod["spec"]["containers"][0]["volumeMounts"]
        .as_array()
        .unwrap()
}

/// Sandboxed hardening mounting `workspace` at `<dir>/<RUN_A>` on the sessions path.
fn step_hardening<'a>(sandbox: &'a SandboxSettings, sub_path: &'a str) -> PodHardening<'a> {
    PodHardening {
        sessions_claim: Some("workspace"),
        sessions_sub_path: Some(sub_path),
        ..sandboxed(sandbox)
    }
}

#[test]
fn step_sessions_mount_the_run_directory_of_the_claim() {
    assert_eq!(
        resolved(RUN_A),
        SessionsMount {
            claim_name: "workspace".to_string(),
            sub_path: Some(format!("claude-sessions/{RUN_A}")),
        }
    );
}

#[test]
fn two_runs_get_two_session_directories() {
    assert_ne!(resolved(RUN_A).sub_path, resolved(RUN_B).sub_path);
}

#[test]
fn step_sessions_win_over_the_provider_sessions_volume() {
    let mount = resolve_sessions(
        Some("claude-sessions"),
        true,
        Some(&step_sessions()),
        &run_labels(RUN_A),
    )
    .unwrap()
    .unwrap();
    assert_eq!(mount.claim_name, "workspace");
}

#[test]
fn provider_sessions_volume_without_step_sessions_is_unchanged() {
    let mount = resolve_sessions(Some("claude-sessions"), true, None, &run_labels(RUN_A))
        .unwrap()
        .unwrap();
    assert_eq!(
        mount,
        SessionsMount {
            claim_name: "claude-sessions".to_string(),
            sub_path: None,
        }
    );
    assert_eq!(
        resolve_sessions(None, true, None, &run_labels(RUN_A)).unwrap(),
        None
    );
}

#[test]
fn step_sessions_on_a_provider_that_is_not_sandboxed_fail() {
    let err = resolve_sessions(None, false, Some(&step_sessions()), &run_labels(RUN_A))
        .unwrap_err()
        .to_string();
    assert!(err.contains("sandboxed"), "{err}");
}

#[test]
fn step_sessions_without_run_label_fail() {
    let err = resolve_sessions(None, true, Some(&step_sessions()), &BTreeMap::new())
        .unwrap_err()
        .to_string();
    assert!(err.contains(LABEL_RUN_ID), "{err}");
}

#[test]
fn step_sessions_with_a_run_id_escaping_the_dir_fail() {
    for run_id in ["..", "../other", "a/b"] {
        let result = resolve_sessions(None, true, Some(&step_sessions()), &run_labels(run_id));
        assert!(result.is_err(), "{run_id:?}");
    }
}

#[test]
fn step_sessions_mounted_on_the_sessions_path_after_home() {
    let sandbox = SandboxSettings::default();
    let sub_path = format!("claude-sessions/{RUN_A}");
    let pod = pod_json(&hardened_config(step_hardening(&sandbox, &sub_path)));

    let volume = find_by_name(&pod["spec"]["volumes"], "ironflow-sessions").unwrap();
    assert_eq!(volume["persistentVolumeClaim"]["claimName"], "workspace");
    let mounts = mounts(&pod);
    let position = |name: &str| mounts.iter().position(|m| m["name"] == name).unwrap();
    let sessions = &mounts[position("ironflow-sessions")];
    assert_eq!(sessions["mountPath"], SESSIONS_MOUNT_PATH);
    assert_eq!(sessions["subPath"], sub_path.as_str());
    assert!(sessions.get("readOnly").is_none());
    assert!(position("ironflow-home") < position("ironflow-sessions"));
}

#[test]
fn step_sessions_reuse_the_volume_of_a_claim_already_mounted() {
    let sandbox = SandboxSettings::default();
    let sub_path = format!("claude-sessions/{RUN_A}");
    let repo = vec![step_pvc("workspace", "/workspace", Some("repo"), false)];
    let pod = pod_json(&hardened_config(PodHardening {
        step_pvc_volumes: &repo,
        ..step_hardening(&sandbox, &sub_path)
    }));

    let claims: Vec<&Value> = pod["spec"]["volumes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["persistentVolumeClaim"]["claimName"] == "workspace")
        .collect();
    assert_eq!(claims.len(), 1, "the claim is declared once");
    let name = claims[0]["name"].as_str().unwrap();
    let on_claim: Vec<&Value> = mounts(&pod).iter().filter(|m| m["name"] == name).collect();
    assert_eq!(on_claim.len(), 2);
    let sessions = on_claim
        .iter()
        .find(|m| m["mountPath"] == SESSIONS_MOUNT_PATH)
        .unwrap();
    assert_eq!(sessions["subPath"], sub_path.as_str());
}

#[test]
fn provider_sessions_volume_keeps_its_own_volume_next_to_a_provider_mount() {
    let sandbox = SandboxSettings::default();
    let pvcs = vec![("claude-sessions".to_string(), "/data".to_string())];
    let mut config = hardened_config(PodHardening {
        sessions_claim: Some("claude-sessions"),
        ..sandboxed(&sandbox)
    });
    config.pvc_volumes = &pvcs;
    let pod = pod_json(&config);
    let sessions = find_by_name(&pod["spec"]["volumes"], "ironflow-sessions").unwrap();
    assert_eq!(
        sessions["persistentVolumeClaim"]["claimName"],
        "claude-sessions"
    );
    let mount = find_by_name(
        &pod["spec"]["containers"][0]["volumeMounts"],
        "ironflow-sessions",
    )
    .unwrap();
    assert!(mount.get("subPath").is_none());
}

#[test]
fn step_sessions_refuse_the_claim_mounted_whole_by_the_provider() {
    let sandbox = SandboxSettings::default();
    let sub_path = format!("claude-sessions/{RUN_A}");
    let pvcs = vec![("workspace".to_string(), "/workspace".to_string())];
    let mut config = hardened_config(step_hardening(&sandbox, &sub_path));
    config.pvc_volumes = &pvcs;
    let err = hardening_err(&config);
    assert!(err.contains("'/workspace', which shows them"), "{err}");
}

#[test]
fn step_sessions_refuse_a_step_mount_showing_every_run() {
    let sandbox = SandboxSettings::default();
    let sub_path = format!("team/claude-sessions/{RUN_A}");
    for exposing in [None, Some("team"), Some("team/claude-sessions")] {
        let mounts = vec![step_pvc("workspace", "/workspace", exposing, false)];
        let config = hardened_config(PodHardening {
            step_pvc_volumes: &mounts,
            ..step_hardening(&sandbox, &sub_path)
        });
        let err = hardening_err(&config);
        assert!(err.contains("which shows them"), "{exposing:?}: {err}");
    }
}

#[test]
fn step_sessions_accept_a_step_mount_beside_the_sessions() {
    let sandbox = SandboxSettings::default();
    let sub_path = format!("team/claude-sessions/{RUN_A}");
    for beside in ["team/repo", "team/claude-sessions-old", "other"] {
        let mounts = vec![step_pvc("workspace", "/workspace", Some(beside), false)];
        let config = hardened_config(PodHardening {
            step_pvc_volumes: &mounts,
            ..step_hardening(&sandbox, &sub_path)
        });
        assert!(build_pod_spec_ok(&config), "{beside}");
    }
}

#[test]
fn step_sessions_refuse_a_read_only_mount_of_the_claim_root() {
    let sandbox = SandboxSettings::default();
    let sub_path = format!("claude-sessions/{RUN_A}");
    let volumes: Vec<ReadOnlyVolume> = vec![ro(
        PodVolumeSource::PersistentVolumeClaim {
            claim_name: "workspace".to_string(),
        },
        "/data",
    )];
    let config = hardened_config(PodHardening {
        read_only_volumes: &volumes,
        ..step_hardening(&sandbox, &sub_path)
    });
    assert!(hardening_err(&config).contains("'/data', which shows them"));
}

#[test]
fn step_sessions_reserve_the_sessions_path() {
    let sandbox = SandboxSettings::default();
    let sub_path = format!("claude-sessions/{RUN_A}");
    let other: Vec<PvcVolume> = vec![step_pvc("other", SESSIONS_MOUNT_PATH, None, false)];
    let config = hardened_config(PodHardening {
        step_pvc_volumes: &other,
        ..step_hardening(&sandbox, &sub_path)
    });
    assert!(hardening_err(&config).contains("duplicate"));
}

#[test]
fn step_sessions_refuse_a_profile_copied_into_projects() {
    let sandbox = SandboxSettings::default();
    let sub_path = format!("claude-sessions/{RUN_A}");
    let profiles = vec![ClaudeProfile {
        configmap: "profile".to_string(),
        subdir: "projects".to_string(),
    }];
    let config = hardened_config(PodHardening {
        claude_profiles: &profiles,
        ..step_hardening(&sandbox, &sub_path)
    });
    assert!(hardening_err(&config).contains("overlaps the sessions volume"));
}

#[test]
fn sessions_path_stays_under_home() {
    assert!(SESSIONS_MOUNT_PATH.starts_with(SANDBOX_HOME));
}

fn build_pod_spec_ok(config: &PodConfig<'_>) -> bool {
    super::super::common::build_pod_spec(config).is_ok()
}
