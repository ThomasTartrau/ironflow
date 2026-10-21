//! [`PodRun::stdin`]: the input of the command, projected into a file by the
//! downward API and redirected onto its stdin.
//!
//! The input lives in an annotation of the pod rather than in an environment
//! variable: Linux refuses to start a process with an environment string over
//! 128 KiB, and a variable leaks into the environment of every child process.
//! It needs no extra RBAC and no object to clean up, unlike a ConfigMap.

use ironflow_core::error::OperationError;
use k8s_openapi::api::core::v1::{
    DownwardAPIVolumeFile, DownwardAPIVolumeSource, ObjectFieldSelector, Volume, VolumeMount,
};

use super::PodRun;
use crate::error::k8s_external;

#[cfg(test)]
mod tests;

/// Annotation of the pod holding the input set with [`PodRun::stdin`].
pub const STDIN_ANNOTATION: &str = "ironflow.io/stdin";

/// Largest input [`PodRun::stdin`] accepts, in bytes: 255 KiB.
///
/// Kubernetes caps the annotations of an object at 256 KiB, keys and values
/// together; 1 KiB is left for the key and the other annotations of the pod.
pub const MAX_STDIN_BYTES: usize = 255 * 1024;

/// Name of the downward API volume carrying the input.
const STDIN_VOLUME: &str = "ironflow-stdin";

/// Directory the input volume is mounted at, read-only.
const STDIN_DIR: &str = "/var/run/ironflow";

/// Path of the input file, under [`STDIN_DIR`].
const STDIN_FILE: &str = "stdin";

impl PodRun {
    /// Write `input` on the command's stdin.
    ///
    /// The input is stored in the [`STDIN_ANNOTATION`] annotation of the pod,
    /// projected read-only at `/var/run/ironflow/stdin` by a downward API
    /// volume, and the command starts with `exec < /var/run/ironflow/stdin`.
    /// The command reads it as a local process reads a pipe, and sees EOF at
    /// its end. Without this call, the command's stdin is empty, as before.
    ///
    /// The input is visible to whoever can read the pod (`kubectl get pod -o
    /// yaml`): pass secrets through a Secret, not here. At most
    /// [`MAX_STDIN_BYTES`]: [`run`](Self::run) refuses a larger input before
    /// creating the pod.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::pod_run::PodRun;
    /// # use ironflow_ops_k8s::KubeClient;
    ///
    /// # async fn example(kube: &KubeClient) -> Result<(), ironflow_core::error::OperationError> {
    /// let output = PodRun::new(kube, "count", "busybox", "wc -l")
    ///     .stdin("one\ntwo\n")
    ///     .run()
    ///     .await?;
    /// assert_eq!(output.exit_code, Some(0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn stdin(mut self, input: impl Into<String>) -> Self {
        self.stdin = Some(input.into());
        self
    }

    /// Refuse an input Kubernetes would reject in the annotations of the pod.
    pub(crate) fn check_stdin(&self) -> Result<(), OperationError> {
        match &self.stdin {
            Some(input) if input.len() > MAX_STDIN_BYTES => Err(k8s_external(format!(
                "stdin of pod '{}' is {} bytes, over the {MAX_STDIN_BYTES} bytes a pod annotation can hold",
                self.name,
                input.len()
            ))),
            _ => Ok(()),
        }
    }
}

/// `command` with its stdin read from the input file.
///
/// `exec <` on a line of its own redirects the stdin of the shell itself, so
/// the command is left as written: no quoting, and its exit code is the
/// shell's.
pub(crate) fn stdin_command(command: &str) -> String {
    format!("exec < {STDIN_DIR}/{STDIN_FILE}\n{command}")
}

/// The downward API volume projecting [`STDIN_ANNOTATION`], and its mount.
pub(crate) fn stdin_volume() -> (Volume, VolumeMount) {
    let volume = Volume {
        name: STDIN_VOLUME.to_string(),
        downward_api: Some(DownwardAPIVolumeSource {
            items: Some(vec![DownwardAPIVolumeFile {
                path: STDIN_FILE.to_string(),
                field_ref: Some(ObjectFieldSelector {
                    field_path: format!("metadata.annotations['{STDIN_ANNOTATION}']"),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mount = VolumeMount {
        name: STDIN_VOLUME.to_string(),
        mount_path: STDIN_DIR.to_string(),
        read_only: Some(true),
        ..Default::default()
    };
    (volume, mount)
}
