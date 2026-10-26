//! Scratch volume of a [`PodRun`](super::PodRun), see `PodRun::scratch_volume`.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    EmptyDirVolumeSource, EphemeralVolumeSource, PersistentVolumeClaimSpec,
    PersistentVolumeClaimTemplate, Volume, VolumeMount, VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

/// Build the `(Volume, VolumeMount)` pair of a scratch volume.
///
/// Without `storage_class` the volume is an `emptyDir` bounded by `size`. With
/// one it is a generic ephemeral volume: a `ReadWriteOnce` claim of `size`
/// created with the pod and deleted with it.
///
/// # Panics
///
/// Panics if `size` or `storage_class` (when given) is blank.
pub(crate) fn scratch_volume(
    name: &str,
    mount_path: &str,
    size: &str,
    storage_class: Option<&str>,
) -> (Volume, VolumeMount) {
    assert!(
        !size.trim().is_empty(),
        "scratch volume size must not be blank"
    );
    if let Some(class) = storage_class {
        assert!(
            !class.trim().is_empty(),
            "scratch volume storage class must not be blank"
        );
    }
    let mut volume = Volume {
        name: name.to_string(),
        ..Default::default()
    };
    match storage_class {
        None => {
            volume.empty_dir = Some(EmptyDirVolumeSource {
                size_limit: Some(Quantity(size.to_string())),
                ..Default::default()
            });
        }
        Some(class) => {
            volume.ephemeral = Some(EphemeralVolumeSource {
                volume_claim_template: Some(PersistentVolumeClaimTemplate {
                    metadata: None,
                    spec: PersistentVolumeClaimSpec {
                        access_modes: Some(vec!["ReadWriteOnce".to_string()]),
                        storage_class_name: Some(class.to_string()),
                        resources: Some(VolumeResourceRequirements {
                            requests: Some(BTreeMap::from([(
                                "storage".to_string(),
                                Quantity(size.to_string()),
                            )])),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                }),
            });
        }
    }
    let mount = VolumeMount {
        name: name.to_string(),
        mount_path: mount_path.to_string(),
        ..Default::default()
    };
    (volume, mount)
}
