//! Claude profile ConfigMaps copied into `~/.claude` before the agent starts.
//!
//! A ConfigMap key cannot contain `/`, so a profile with sub-directories
//! (`rules/`, `agents/`, `commands/`) takes one ConfigMap per directory. The
//! n-th ConfigMap is mounted read-only at [`profile_mount_path`]`(n)`, next to
//! the others and never inside another ConfigMap mount, then its keys are
//! copied into `~/.claude/<subdir>`: Claude Code must be able to write into
//! `~/.claude`, so the profile cannot be mounted in place.

use super::common::PROFILE_MOUNT_DIR;
use super::ephemeral::K8sEphemeralProvider;

impl K8sEphemeralProvider {
    /// Copy the keys of a ConfigMap (`CLAUDE.md`, `settings.json`) into
    /// `~/.claude` before the agent starts. Same as
    /// [`claude_profile_configmap_at`](Self::claude_profile_configmap_at)
    /// with an empty `subdir`.
    ///
    /// # Panics
    ///
    /// Panics if a ConfigMap is already mapped to `~/.claude` itself.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .claude_profile_configmap("claude-profile");
    /// ```
    pub fn claude_profile_configmap(self, name: &str) -> Self {
        self.claude_profile_configmap_at(name, "")
    }

    /// Copy the keys of a ConfigMap into `~/.claude/<subdir>` before the agent
    /// starts. Call it once per directory of the profile: a ConfigMap key
    /// cannot contain `/`, so `rules/*.md` takes its own ConfigMap.
    ///
    /// Each ConfigMap gets its own read-only mount, and only its keys are
    /// copied, never the `..data` entries of the volume. Profiles are copied
    /// in call order, then the credentials, which they cannot overwrite.
    ///
    /// # Panics
    ///
    /// Panics if [`validate_profile_subdir`] refuses `subdir` (absolute, `.`
    /// or `..` segment, char outside `[A-Za-z0-9._-]`), or if `subdir` is
    /// already mapped to a ConfigMap.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .claude_profile_configmap("claude-profile")
    ///     .claude_profile_configmap_at("claude-profile-rules", "rules");
    /// ```
    pub fn claude_profile_configmap_at(mut self, configmap: &str, subdir: &str) -> Self {
        if let Err(reason) = validate_profile_subdir(subdir) {
            panic!("invalid claude profile subdir '{subdir}': {reason}");
        }
        if let Some(taken) = self.claude_profiles.iter().find(|p| p.subdir == subdir) {
            panic!(
                "claude profile subdir '{subdir}' is already mapped to ConfigMap '{}'",
                taken.configmap
            );
        }
        self.claude_profiles.push(ClaudeProfile {
            configmap: configmap.to_string(),
            subdir: subdir.to_string(),
        });
        self
    }
}

/// A ConfigMap whose keys are copied into a directory of `~/.claude`.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::profile::ClaudeProfile;
///
/// let rules = ClaudeProfile {
///     configmap: "claude-profile-rules".to_string(),
///     subdir: "rules".to_string(),
/// };
/// assert_eq!(rules.subdir, "rules");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeProfile {
    /// Name of the ConfigMap, in the pod's namespace.
    pub configmap: String,
    /// Directory under `~/.claude` the keys are copied to; empty for
    /// `~/.claude` itself. Checked by [`validate_profile_subdir`].
    pub subdir: String,
}

/// Check that `subdir` can receive a Claude profile.
///
/// Empty means `~/.claude` itself. Otherwise `subdir` is relative: segments
/// of `[A-Za-z0-9._-]` separated by `/`, none of them empty, `.` or `..`.
///
/// # Errors
///
/// Returns why `subdir` is refused.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::profile::validate_profile_subdir;
///
/// assert!(validate_profile_subdir("").is_ok());
/// assert!(validate_profile_subdir("skills/review").is_ok());
/// assert!(validate_profile_subdir("../etc").is_err());
/// ```
pub fn validate_profile_subdir(subdir: &str) -> Result<(), String> {
    if subdir.is_empty() {
        return Ok(());
    }
    if subdir.starts_with('/') {
        return Err("must be relative to ~/.claude".to_string());
    }
    for segment in subdir.split('/') {
        match segment {
            "" => return Err("contains an empty segment".to_string()),
            "." | ".." => return Err(format!("contains a '{segment}' segment")),
            _ => {}
        }
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
        if !segment.chars().all(allowed) {
            return Err(format!(
                "segment '{segment}' has a char outside [A-Za-z0-9._-]"
            ));
        }
    }
    Ok(())
}

/// Directory the n-th profile ConfigMap is mounted at:
/// `/etc/ironflow/claude-profile/<index>`.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::profile::profile_mount_path;
///
/// assert_eq!(profile_mount_path(1), "/etc/ironflow/claude-profile/1");
/// ```
pub fn profile_mount_path(index: usize) -> String {
    format!("{PROFILE_MOUNT_DIR}/{index}")
}

/// Build a shell prefix that copies each profile mounted at
/// [`profile_mount_path`] into its directory of `~/.claude`.
///
/// Only the ConfigMap keys are copied: the `..data` link and the
/// `..<timestamp>` directory of a ConfigMap volume are skipped. Kubernetes
/// refuses keys starting with `..`, so no key is lost, `.mcp.json` included.
/// A failed copy aborts the pod command. Returns an empty string when
/// `profiles` is empty.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::profile::{ClaudeProfile, build_profile_copy_prefix};
///
/// let profiles = [ClaudeProfile {
///     configmap: "claude-profile".to_string(),
///     subdir: String::new(),
/// }];
/// assert!(build_profile_copy_prefix(&profiles).ends_with("&& "));
/// assert!(build_profile_copy_prefix(&[]).is_empty());
/// ```
pub fn build_profile_copy_prefix(profiles: &[ClaudeProfile]) -> String {
    copy_prefix(PROFILE_MOUNT_DIR, profiles)
}

/// [`build_profile_copy_prefix`] for profiles mounted under `mount_root`.
fn copy_prefix(mount_root: &str, profiles: &[ClaudeProfile]) -> String {
    profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let src = shell_quote(&format!("{mount_root}/{index}"));
            let dest = match profile.subdir.as_str() {
                "" => r#""$HOME/.claude""#.to_string(),
                subdir => format!(r#""$HOME/.claude/"{}"#, shell_quote(subdir)),
            };
            // `.[!.]*` matches dotfile keys but not `..data` nor `..<timestamp>`.
            format!(
                r#"mkdir -p {dest} && for f in {src}/* {src}/.[!.]*; do [ -e "$f" ] || continue; cp -L "$f" {dest}/ || exit 1; done && "#
            )
        })
        .collect()
}

/// Quote a string for safe inclusion in a `sh -c` argument.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir_all, read_dir, read_to_string, write};
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::*;

    fn profile(configmap: &str, subdir: &str) -> ClaudeProfile {
        ClaudeProfile {
            configmap: configmap.to_string(),
            subdir: subdir.to_string(),
        }
    }

    /// Lay out `dir` the way the kubelet lays out a ConfigMap volume: the
    /// keys in a timestamped `..<ts>` directory, a `..data` link to it, and
    /// one link per key through `..data`.
    fn fake_configmap_mount(dir: &Path, keys: &[(&str, &str)]) {
        let stamp = "..2026_09_28_12_00_00.123456789";
        create_dir_all(dir.join(stamp)).unwrap();
        for (key, content) in keys {
            write(dir.join(stamp).join(key), content).unwrap();
        }
        symlink(stamp, dir.join("..data")).unwrap();
        for (key, _) in keys {
            symlink(format!("..data/{key}"), dir.join(key)).unwrap();
        }
    }

    /// Run `prefix` with a real `sh` and `HOME` set to `home`, then return
    /// the sorted entries of `home/.claude/<subdir>`.
    fn run_copy(prefix: &str, home: &Path, subdir: &str) -> Vec<String> {
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("{prefix}true"))
            .env("HOME", home)
            .status()
            .unwrap();
        assert!(status.success(), "copy prefix failed: {prefix}");
        let mut names: Vec<String> = read_dir(home.join(".claude").join(subdir))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn k8s_profile_copy_skips_configmap_internals() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("profiles");
        fake_configmap_mount(
            &root.join("0"),
            &[("CLAUDE.md", "root"), (".mcp.json", "{}")],
        );
        let home = tmp.path().join("home");

        let prefix = copy_prefix(root.to_str().unwrap(), &[profile("claude-profile", "")]);
        let copied = run_copy(&prefix, &home, "");

        assert_eq!(copied, vec![".mcp.json", "CLAUDE.md"]);
        let claude_md = read_to_string(home.join(".claude/CLAUDE.md")).unwrap();
        assert_eq!(claude_md, "root");
    }

    #[test]
    fn k8s_profile_copy_places_each_configmap_in_its_subdir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("profiles");
        fake_configmap_mount(&root.join("0"), &[("CLAUDE.md", "root")]);
        fake_configmap_mount(&root.join("1"), &[("rust.md", "r"), ("security.md", "s")]);
        fake_configmap_mount(&root.join("2"), &[("SKILL.md", "k")]);
        let home = tmp.path().join("home");
        let profiles = [
            profile("claude-profile", ""),
            profile("claude-profile-rules", "rules"),
            profile("claude-profile-review", "skills/review"),
        ];

        let prefix = copy_prefix(root.to_str().unwrap(), &profiles);

        assert_eq!(
            run_copy(&prefix, &home, "rules"),
            vec!["rust.md", "security.md"]
        );
        assert_eq!(run_copy(&prefix, &home, "skills/review"), vec!["SKILL.md"]);
        assert_eq!(
            run_copy(&prefix, &home, ""),
            vec!["CLAUDE.md", "rules", "skills"]
        );
        let rust = read_to_string(home.join(".claude/rules/rust.md")).unwrap();
        assert_eq!(rust, "r");
    }

    #[test]
    fn k8s_profile_copy_failure_aborts_the_command() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("profiles");
        fake_configmap_mount(&root.join("0"), &[("rust.md", "r")]);
        // A root key named like the directory created just before: `cp`
        // cannot overwrite a directory with a file.
        fake_configmap_mount(&root.join("1"), &[("rules", "a file, not a directory")]);
        let home = tmp.path().join("home");
        let profiles = [profile("rules", "rules"), profile("root", "")];

        let prefix = copy_prefix(root.to_str().unwrap(), &profiles);
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("{prefix}echo agent-started"))
            .env("HOME", &home)
            .output()
            .unwrap();

        assert!(!status.status.success());
        let stdout = String::from_utf8_lossy(&status.stdout);
        assert!(!stdout.contains("agent-started"), "{stdout}");
    }

    #[test]
    fn k8s_profile_copy_prefix_is_empty_without_profiles() {
        assert_eq!(build_profile_copy_prefix(&[]), "");
    }

    #[test]
    fn k8s_profile_copy_prefix_reads_the_mount_paths() {
        let prefix = build_profile_copy_prefix(&[profile("a", ""), profile("b", "rules")]);
        assert!(prefix.contains(&format!("'{}'/*", profile_mount_path(0))));
        assert!(prefix.contains(&format!("'{}'/*", profile_mount_path(1))));
        assert!(prefix.contains(r#""$HOME/.claude/"'rules'/"#), "{prefix}");
    }

    #[test]
    fn k8s_profile_mount_paths_are_siblings() {
        let first = profile_mount_path(0);
        let second = profile_mount_path(1);
        assert_ne!(first, second);
        assert!(!second.starts_with(&format!("{first}/")));
        assert!(first.starts_with(&format!("{PROFILE_MOUNT_DIR}/")));
    }

    #[test]
    fn k8s_profile_configmap_maps_to_claude_home() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1").claude_profile_configmap("cp");
        assert_eq!(provider.claude_profiles, vec![profile("cp", "")]);
        let prefix = provider.home_setup_prefix();
        assert!(
            prefix.contains(r#"mkdir -p "$HOME/.claude" && "#),
            "{prefix}"
        );
    }

    #[test]
    fn k8s_profile_configmap_at_keeps_call_order() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .claude_profile_configmap("cp")
            .claude_profile_configmap_at("cp-rules", "rules")
            .claude_profile_configmap_at("cp", "agents");
        let expected = vec![
            profile("cp", ""),
            profile("cp-rules", "rules"),
            profile("cp", "agents"),
        ];
        assert_eq!(provider.claude_profiles, expected);
    }

    #[test]
    fn k8s_profile_copied_before_credentials() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .claude_profile_configmap_at("cp-rules", "rules")
            .oauth_credentials_from_secret("claude-credentials", "credentials.json");
        let prefix = provider.home_setup_prefix();
        let copy = prefix.find("cp -L").expect("profile copy");
        let credentials = prefix.find(".credentials.json").expect("credentials");
        assert!(copy < credentials, "{prefix}");
    }

    #[test]
    fn k8s_profile_absent_leaves_only_credentials() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .oauth_credentials_from_secret("claude-credentials", "credentials.json");
        let prefix = provider.home_setup_prefix();
        assert!(!prefix.contains("cp -L"), "{prefix}");
        assert!(prefix.starts_with("mkdir -p \"$HOME/.claude\" && printf"));
    }

    #[test]
    #[should_panic(expected = "invalid claude profile subdir '../etc': contains a '..' segment")]
    fn k8s_profile_configmap_at_rejects_parent_segment() {
        let _ =
            K8sEphemeralProvider::sandboxed("img:v1").claude_profile_configmap_at("cp", "../etc");
    }

    #[test]
    #[should_panic(expected = "invalid claude profile subdir '/rules': must be relative")]
    fn k8s_profile_configmap_at_rejects_absolute_subdir() {
        let _ =
            K8sEphemeralProvider::sandboxed("img:v1").claude_profile_configmap_at("cp", "/rules");
    }

    #[test]
    #[should_panic(expected = "claude profile subdir 'rules' is already mapped to ConfigMap 'a'")]
    fn k8s_profile_configmap_at_rejects_duplicate_subdir() {
        let _ = K8sEphemeralProvider::sandboxed("img:v1")
            .claude_profile_configmap_at("a", "rules")
            .claude_profile_configmap_at("b", "rules");
    }

    #[test]
    #[should_panic(expected = "claude profile subdir '' is already mapped to ConfigMap 'a'")]
    fn k8s_profile_configmap_twice_is_a_duplicate() {
        let _ = K8sEphemeralProvider::new("img:v1")
            .claude_profile_configmap("a")
            .claude_profile_configmap("b");
    }

    #[test]
    fn k8s_profile_subdir_accepts_relative_paths() {
        for subdir in ["", "rules", "skills/review", "a.b_c-d", ".hidden"] {
            assert!(validate_profile_subdir(subdir).is_ok(), "{subdir}");
        }
    }

    #[test]
    fn k8s_profile_subdir_rejects_escapes_and_odd_chars() {
        let cases = [
            ("/rules", "relative"),
            ("..", "'..' segment"),
            ("rules/../..", "'..' segment"),
            (".", "'.' segment"),
            ("rules/", "empty segment"),
            ("a//b", "empty segment"),
            ("a b", "outside"),
            ("$(id)", "outside"),
            ("it's", "outside"),
        ];
        for (subdir, reason) in cases {
            let err = validate_profile_subdir(subdir).unwrap_err();
            assert!(err.contains(reason), "{subdir}: {err}");
        }
    }
}
