//! [`PodRun::sandboxed`]: a hardened pod for a command that is not trusted.

use k8s_openapi::api::core::v1::{
    Capabilities, EmptyDirVolumeSource, SeccompProfile, SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use super::{PodRun, SecuritySpec};

#[cfg(test)]
mod tests;

/// Uid, gid and fsGroup of a [`PodRun::sandboxed`] pod without
/// [`PodRun::security`]: the uid of the sandboxed agent pods.
pub const SANDBOX_UID: i64 = 10001;

/// Default size limit of the `emptyDir` backing `/tmp`.
const DEFAULT_TMP_SIZE_LIMIT: &str = "512Mi";

/// Name of the `emptyDir` volume backing `/tmp`.
const TMP_VOLUME: &str = "ironflow-tmp";

/// Settings of a [`PodRun::sandboxed`] pod.
#[derive(Debug, Clone)]
pub(crate) struct Sandbox {
    /// Size limit of the `emptyDir` backing `/tmp`.
    tmp_size_limit: String,
}

impl Default for Sandbox {
    fn default() -> Self {
        Self {
            tmp_size_limit: DEFAULT_TMP_SIZE_LIMIT.to_string(),
        }
    }
}

impl PodRun {
    /// Harden the pod, as `K8sEphemeralProvider::sandboxed` does for agents.
    ///
    /// * read-only root filesystem, all capabilities dropped, seccomp
    ///   `RuntimeDefault`, no privilege escalation;
    /// * `runAsNonRoot`, as uid/gid/fsGroup [`SANDBOX_UID`] unless
    ///   [`security`](Self::security) sets them;
    /// * `/tmp` on an `emptyDir` of 512Mi ([`tmp_size_limit`](Self::tmp_size_limit));
    /// * no service account token.
    ///
    /// A later [`automount_service_account_token`](Self::automount_service_account_token)
    /// or [`allow_privilege_escalation`](Self::allow_privilege_escalation)
    /// call overrides its default. Any other path the command writes to must
    /// be a volume: [`volume`](Self::volume) with an `emptyDir`, or a
    /// [`pvc`](Self::pvc).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::pod_run::PodRun;
    /// # use ironflow_ops_k8s::KubeClient;
    ///
    /// # fn example(kube: &KubeClient) {
    /// let run = PodRun::new(kube, "script", "python:3.13-slim", "python /scripts/extract.py")
    ///     .sandboxed()
    ///     .stdin("{\"invoice\": 42}");
    /// # let _ = run;
    /// # }
    /// ```
    #[must_use]
    pub fn sandboxed(mut self) -> Self {
        self.sandbox = Some(Sandbox::default());
        self
    }

    /// Set the size limit of the `emptyDir` backing `/tmp` (default `512Mi`).
    ///
    /// Past it, the kubelet evicts the pod.
    ///
    /// # Panics
    ///
    /// Panics if the run was not built with [`sandboxed`](Self::sandboxed).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::pod_run::PodRun;
    /// # use ironflow_ops_k8s::KubeClient;
    ///
    /// # fn example(kube: &KubeClient) {
    /// let run = PodRun::new(kube, "convert", "pandoc/core:3.6", "pandoc -o /tmp/out.pdf")
    ///     .sandboxed()
    ///     .tmp_size_limit("2Gi");
    /// # let _ = run;
    /// # }
    /// ```
    #[must_use]
    pub fn tmp_size_limit(mut self, limit: &str) -> Self {
        let sandbox = self
            .sandbox
            .as_mut()
            .expect("tmp_size_limit requires a PodRun built with .sandboxed()");
        sandbox.tmp_size_limit = limit.to_string();
        self
    }

    /// [`security`](Self::security), else the [`SANDBOX_UID`] of a sandboxed
    /// pod.
    pub(crate) fn effective_security(&self) -> Option<SecuritySpec> {
        self.security.or_else(|| {
            self.sandbox.as_ref().map(|_| SecuritySpec {
                run_as_user: SANDBOX_UID,
                run_as_group: SANDBOX_UID,
                fs_group: SANDBOX_UID,
            })
        })
    }
}

impl Sandbox {
    /// Lock down a container `securityContext`, keeping an explicit
    /// `allowPrivilegeEscalation`.
    pub(crate) fn harden(sc: &mut SecurityContext) {
        sc.read_only_root_filesystem = Some(true);
        sc.allow_privilege_escalation.get_or_insert(false);
        sc.capabilities = Some(Capabilities {
            drop: Some(vec!["ALL".to_string()]),
            ..Default::default()
        });
        sc.seccomp_profile = Some(SeccompProfile {
            type_: "RuntimeDefault".to_string(),
            ..Default::default()
        });
    }

    /// The `emptyDir` backing `/tmp`, writable under a read-only root, and its
    /// mount.
    pub(crate) fn tmp_volume(&self) -> (Volume, VolumeMount) {
        let volume = Volume {
            name: TMP_VOLUME.to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                size_limit: Some(Quantity(self.tmp_size_limit.clone())),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mount = VolumeMount {
            name: TMP_VOLUME.to_string(),
            mount_path: "/tmp".to_string(),
            ..Default::default()
        };
        (volume, mount)
    }
}
