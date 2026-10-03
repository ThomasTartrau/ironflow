//! Functional tests of `template update` against real local Git repositories:
//! a registry (`index.toml`) and a template repository tagged `v0.1.0` and
//! `v0.2.0`, reached through `file://` URLs.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ironflow_cli::commands::template::{TemplateArgs, TemplateCommands, execute};
use ironflow_templates::lockfile::{InstalledEntry, LOCKFILE_NAME, LockFile};
use serial_test::serial;
use tempfile::TempDir;

const INSTALL_PATH: &str = "src/workflows/hello";
const PROJECT_CARGO: &str = "[package]\nname = \"workflows\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nironflow-engine = \"0.1.0\"\n";

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_all(dir: &Path, message: &str, tag: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-m", message]);
    git(dir, &["tag", tag]);
}

fn manifest(version: &str, min_ironflow: Option<&str>) -> String {
    let min = min_ironflow
        .map(|v| format!("min_ironflow_version = \"{v}\"\n"))
        .unwrap_or_default();
    let deps = if version == "0.1.0" {
        ""
    } else {
        "\n[dependencies]\nglobset = \"0.4\"\n"
    };
    format!(
        "[template]\nname = \"hello\"\ndescription = \"Hello\"\nversion = \"{version}\"\n{min}{deps}"
    )
}

/// A template repository with `v0.1.0`, and `v0.2.0` when `with_v2` is set.
/// `v0.2.0` modifies `mod.rs`, adds `new.rs` and removes `old.rs`.
fn write_template_repo(root: &Path, with_v2: bool, min_ironflow: Option<&str>) {
    fs::create_dir_all(root.join("src")).unwrap();
    git(root, &["init", "-q"]);
    fs::write(root.join("template.toml"), manifest("0.1.0", None)).unwrap();
    fs::write(root.join("src/mod.rs"), "// v1\n").unwrap();
    fs::write(root.join("src/plan.rs"), "// plan\n").unwrap();
    fs::write(root.join("src/old.rs"), "// old\n").unwrap();
    commit_all(root, "v1", "v0.1.0");

    if with_v2 {
        fs::write(root.join("template.toml"), manifest("0.2.0", min_ironflow)).unwrap();
        fs::write(root.join("src/mod.rs"), "// v2\n").unwrap();
        fs::write(root.join("src/new.rs"), "// new\n").unwrap();
        fs::remove_file(root.join("src/old.rs")).unwrap();
        commit_all(root, "v2", "v0.2.0");
    }
}

fn write_registry(root: &Path, template_repo: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q"]);
    fs::write(
        root.join("index.toml"),
        format!(
            "[[templates]]\nname = \"hello\"\ndescription = \"Hello\"\nrepo = \"file://{}\"\n",
            template_repo.display()
        ),
    )
    .unwrap();
    commit_all(root, "registry", "registry-v1");
}

/// A project that installed `hello` at `installed_version`, with one local
/// file the template does not ship.
fn write_project(project: &Path, cargo_toml: &str, installed_version: &str, repo: &Path) {
    let installed = project.join(INSTALL_PATH);
    fs::create_dir_all(&installed).unwrap();
    fs::write(project.join("Cargo.toml"), cargo_toml).unwrap();
    fs::write(installed.join("mod.rs"), "// v1\n").unwrap();
    fs::write(installed.join("plan.rs"), "// plan\n").unwrap();
    fs::write(installed.join("old.rs"), "// old\n").unwrap();
    fs::write(installed.join("local.rs"), "// local only\n").unwrap();

    let mut lock = LockFile::default();
    lock.record_install(InstalledEntry {
        name: "hello".to_string(),
        version: installed_version.to_string(),
        repo: format!("file://{}", repo.display()),
        installed_at: "2026-01-01".to_string(),
        path: INSTALL_PATH.to_string(),
    });
    lock.save(&project.join(LOCKFILE_NAME)).unwrap();
}

struct Fixture {
    _tmp: TempDir,
    project: PathBuf,
    registry_url: String,
}

fn fixture(with_v2: bool, min_ironflow: Option<&str>, installed_version: &str) -> Fixture {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let registry = tmp.path().join("registry");
    let project = tmp.path().join("project");
    fs::create_dir_all(&repo).unwrap();
    write_template_repo(&repo, with_v2, min_ironflow);
    write_registry(&registry, &repo);
    write_project(&project, PROJECT_CARGO, installed_version, &repo);
    Fixture {
        registry_url: format!("file://{}", registry.display()),
        project,
        _tmp: tmp,
    }
}

fn run_update(project: &Path, registry_url: &str, check: bool, force: bool) -> anyhow::Result<()> {
    let previous = env::current_dir().unwrap();
    env::set_current_dir(project).unwrap();
    let result = execute(&TemplateArgs {
        command: TemplateCommands::Update {
            name: Some("hello".to_string()),
            check,
            force,
            registry_url: Some(registry_url.to_string()),
        },
    });
    env::set_current_dir(previous).unwrap();
    result
}

fn snapshot(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().display().to_string();
                out.insert(rel, fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn installed_version(project: &Path) -> String {
    LockFile::load(&project.join(LOCKFILE_NAME))
        .unwrap()
        .find_installed("hello")
        .unwrap()
        .version
        .clone()
}

fn v1_files() -> BTreeMap<String, String> {
    [
        ("mod.rs", "// v1\n"),
        ("plan.rs", "// plan\n"),
        ("old.rs", "// old\n"),
        ("local.rs", "// local only\n"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

#[test]
#[serial]
fn update_applies_new_version_in_place() {
    let fx = fixture(true, None, "0.1.0");

    run_update(&fx.project, &fx.registry_url, false, false).unwrap();

    let expected: BTreeMap<String, String> = [
        ("mod.rs", "// v2\n"),
        ("plan.rs", "// plan\n"),
        ("new.rs", "// new\n"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(snapshot(&fx.project.join(INSTALL_PATH)), expected);
    assert_eq!(installed_version(&fx.project), "0.2.0");
    let cargo = fs::read_to_string(fx.project.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("globset"));
}

#[test]
#[serial]
fn update_check_does_not_write() {
    let fx = fixture(true, None, "0.1.0");
    let lock_before = fs::read(fx.project.join(LOCKFILE_NAME)).unwrap();

    run_update(&fx.project, &fx.registry_url, true, false).unwrap();

    assert_eq!(snapshot(&fx.project.join(INSTALL_PATH)), v1_files());
    assert_eq!(
        fs::read_to_string(fx.project.join("Cargo.toml")).unwrap(),
        PROJECT_CARGO
    );
    assert_eq!(
        fs::read(fx.project.join(LOCKFILE_NAME)).unwrap(),
        lock_before
    );
}

#[test]
#[serial]
fn update_failure_leaves_old_copy_and_lockfile_intact() {
    let fx = fixture(true, Some("9.0.0"), "0.1.0");
    let lock_before = fs::read(fx.project.join(LOCKFILE_NAME)).unwrap();

    let err = run_update(&fx.project, &fx.registry_url, false, false).unwrap_err();

    assert!(err.to_string().contains("Use --force"));
    assert_eq!(snapshot(&fx.project.join(INSTALL_PATH)), v1_files());
    assert_eq!(
        fs::read(fx.project.join(LOCKFILE_NAME)).unwrap(),
        lock_before
    );
    assert_eq!(
        fs::read_to_string(fx.project.join("Cargo.toml")).unwrap(),
        PROJECT_CARGO
    );
}

#[test]
#[serial]
fn update_force_skips_min_version_check() {
    let fx = fixture(true, Some("9.0.0"), "0.1.0");

    run_update(&fx.project, &fx.registry_url, false, true).unwrap();

    assert_eq!(installed_version(&fx.project), "0.2.0");
    assert!(fx.project.join(INSTALL_PATH).join("new.rs").exists());
}

#[test]
#[serial]
fn update_ignores_older_tag_than_installed() {
    let fx = fixture(true, None, "0.3.0");
    let lock_before = fs::read(fx.project.join(LOCKFILE_NAME)).unwrap();

    run_update(&fx.project, &fx.registry_url, false, false).unwrap();

    assert_eq!(snapshot(&fx.project.join(INSTALL_PATH)), v1_files());
    assert_eq!(
        fs::read(fx.project.join(LOCKFILE_NAME)).unwrap(),
        lock_before
    );
}

#[test]
#[serial]
fn update_of_a_workspace_member_reads_version_from_workspace_root() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let registry = tmp.path().join("registry");
    let root = tmp.path().join("ws");
    let member = root.join("crates").join("app");
    fs::create_dir_all(&repo).unwrap();
    write_template_repo(&repo, true, Some("0.1.0"));
    write_registry(&registry, &repo);
    fs::create_dir_all(&member).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/app\"]\n\n[workspace.dependencies]\nironflow-engine = { version = \"0.1.0\" }\n",
    )
    .unwrap();
    write_project(
        &member,
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nironflow-engine = { workspace = true }\n",
        "0.1.0",
        &repo,
    );

    run_update(
        &member,
        &format!("file://{}", registry.display()),
        false,
        false,
    )
    .unwrap();

    assert_eq!(installed_version(&member), "0.2.0");
    assert!(member.join(INSTALL_PATH).join("new.rs").exists());
}

#[test]
#[serial]
fn add_on_existing_destination_still_refuses() {
    let fx = fixture(true, None, "0.1.0");
    let repo_url = LockFile::load(&fx.project.join(LOCKFILE_NAME))
        .unwrap()
        .find_installed("hello")
        .unwrap()
        .repo
        .clone();

    let previous = env::current_dir().unwrap();
    env::set_current_dir(&fx.project).unwrap();
    let result = execute(&TemplateArgs {
        command: TemplateCommands::Add {
            name: "hello".to_string(),
            from: Some(repo_url),
            registry: false,
            registry_url: None,
            output: Some(PathBuf::from(INSTALL_PATH)),
            force: true,
        },
    });
    env::set_current_dir(previous).unwrap();

    let err = result.unwrap_err();
    assert!(err.to_string().contains("already exists"));
}
