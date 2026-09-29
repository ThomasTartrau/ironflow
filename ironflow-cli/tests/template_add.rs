//! Functional test of `template add --from <local repository>` against a
//! project laid out like the one the ironflow plugin scaffolds.

use std::env;
use std::fs;
use std::path::Path;

use ironflow_cli::commands::template::{TemplateArgs, TemplateCommands, execute};
use serial_test::serial;
use tempfile::TempDir;

const SCAFFOLD_LIB: &str = "\
//! Workflow handlers for this project.

mod hello;

pub use hello::{Hello, HelloInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler of this project, boxed.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(Hello)]
}
";

fn write_project(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"workflows\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), SCAFFOLD_LIB).unwrap();
    fs::write(root.join("src/hello.rs"), "// hello\n").unwrap();
}

/// A repository holding one template at its root, as `gitlab-mr-review` does.
fn write_root_template(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("template.toml"),
        r#"[template]
name = "gitlab-mr-review"
description = "Review GitLab merge requests"
version = "0.1.0"

[dependencies]
globset = "0.4"

[requirements]
tools = ["git"]
"#,
    )
    .unwrap();
    fs::write(root.join("src/mod.rs"), "//! Template entry point.\n").unwrap();
    fs::write(root.join("src/plan.rs"), "// plan\n").unwrap();
}

#[test]
#[serial]
fn add_from_root_template_installs_and_registers_a_rust_module() {
    let project = TempDir::new().unwrap();
    let template = TempDir::new().unwrap();
    write_project(project.path());
    write_root_template(template.path());

    let previous = env::current_dir().unwrap();
    env::set_current_dir(project.path()).unwrap();
    let result = execute(&TemplateArgs {
        command: TemplateCommands::Add {
            name: "gitlab-mr-review".to_string(),
            from: Some(template.path().display().to_string()),
            registry: false,
            registry_url: None,
            output: None,
            force: false,
        },
    });
    env::set_current_dir(previous).unwrap();
    result.unwrap();

    let module = project.path().join("src/gitlab_mr_review");
    assert!(module.join("mod.rs").is_file(), "mod.rs not installed");
    assert!(module.join("plan.rs").is_file(), "plan.rs not installed");

    let lib = fs::read_to_string(project.path().join("src/lib.rs")).unwrap();
    assert!(
        lib.contains("mod hello;\npub mod gitlab_mr_review;\n"),
        "{lib}"
    );
    assert!(
        lib.contains("pub use gitlab_mr_review::GitlabMrReview;"),
        "{lib}"
    );
    assert!(
        lib.contains("vec![Box::new(Hello), Box::new(GitlabMrReview)]"),
        "{lib}"
    );

    let cargo = fs::read_to_string(project.path().join("Cargo.toml")).unwrap();
    assert!(cargo.contains("globset"), "{cargo}");
}
