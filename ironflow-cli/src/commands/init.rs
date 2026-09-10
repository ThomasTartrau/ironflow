//! `ironflow-cli init` -- scaffold a new Ironflow project.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Args;
use dialoguer::{Input, Select};

/// Arguments for the `init` command.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Skip interactive prompts and use defaults.
    #[arg(long)]
    pub non_interactive: bool,
    /// Project name (defaults to current directory name).
    #[arg(long)]
    pub name: Option<String>,
    /// Force creation even if the directory is not empty.
    #[arg(long)]
    pub force: bool,
}

/// Backend store choices.
const STORE_OPTIONS: &[&str] = &["in-memory", "postgres"];

/// Execute the `init` command.
///
/// # Errors
///
/// Returns an error if the directory is not empty (without `--force`),
/// or if any file cannot be written.
pub fn execute(args: &InitArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;

    let (project_name, store_index) = if args.non_interactive {
        let name = args.name.clone().unwrap_or_else(|| {
            cwd.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("my-ironflow-project")
                .to_string()
        });
        (name, 0)
    } else {
        let default_name = args.name.clone().unwrap_or_else(|| {
            cwd.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("my-ironflow-project")
                .to_string()
        });
        let name: String = Input::new()
            .with_prompt("Project name")
            .default(default_name)
            .interact_text()?;
        let store = Select::new()
            .with_prompt("Backend store")
            .items(STORE_OPTIONS)
            .default(0)
            .interact()?;
        (name, store)
    };

    let project_dir = cwd.join(&project_name);

    if project_dir.exists() {
        let is_empty = project_dir.read_dir()?.next().is_none();
        if !is_empty && !args.force {
            bail!(
                "directory '{}' is not empty; use --force to overwrite",
                project_dir.display()
            );
        }
    }

    fs::create_dir_all(&project_dir)?;

    let store_name = STORE_OPTIONS[store_index];
    let use_postgres = store_name == "postgres";

    write_cargo_toml(&project_dir, &project_name, use_postgres)?;
    write_server_main(&project_dir, &project_name)?;
    write_worker_main(&project_dir, &project_name)?;
    write_hello_workflow(&project_dir)?;

    if use_postgres {
        write_docker_compose(&project_dir)?;
    }

    println!(
        "Project '{project_name}' created at {}",
        project_dir.display()
    );
    println!();
    println!("Next steps:");
    println!("  cd {project_name}");
    println!("  cargo build");
    if use_postgres {
        println!("  docker compose up -d");
    }

    Ok(())
}

fn write_cargo_toml(dir: &Path, name: &str, use_postgres: bool) -> Result<()> {
    let store_dep = if use_postgres {
        r#"ironflow-store = { version = "0.1", features = ["store-postgres", "secret-store"] }"#
    } else {
        r#"ironflow-store = { version = "0.1", features = ["store-memory", "secret-store"] }"#
    };

    let content = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
ironflow-core = "0.1"
ironflow-engine = "0.1"
ironflow-api = "0.1"
ironflow-worker = "0.1"
{store_dep}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
anyhow = "1"
tracing = "0.1"
tracing-subscriber = {{ version = "0.3", features = ["env-filter"] }}
serde_json = "1"
"#
    );

    fs::write(dir.join("Cargo.toml"), content)?;
    Ok(())
}

fn write_server_main(dir: &Path, _name: &str) -> Result<()> {
    let src = dir.join("src");
    fs::create_dir_all(&src)?;

    let content = r#"use std::sync::Arc;

use anyhow::Result;
use ironflow_api::state::AppState;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_store::memory::InMemoryStore;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

mod workflows;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let store = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);

    engine.register(workflows::hello::HelloWorkflow)?;

    let jwt_cfg = Arc::new(ironflow_auth::jwt::JwtConfig {
        secret: "change-me-in-production".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604_800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_tx, _) = broadcast::channel(128);

    let state = AppState::new(
        store,
        Arc::new(engine),
        jwt_cfg,
        "worker-token".to_string(),
        event_tx,
    );

    let router = create_router(state, RouterConfig::default());
    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("listening on {}", listener.local_addr()?);

    axum::serve(listener, router).await?;
    Ok(())
}
"#;

    fs::write(src.join("main.rs"), content)?;
    Ok(())
}

fn write_worker_main(dir: &Path, _name: &str) -> Result<()> {
    let src = dir.join("src");
    fs::create_dir_all(dir.join("src/bin"))?;

    let content = r#"use std::sync::Arc;

use anyhow::Result;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_store::memory::InMemoryStore;
use ironflow_worker::Worker;
use tracing_subscriber::EnvFilter;

#[path = "../workflows/mod.rs"]
mod workflows;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let store = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);

    engine.register(workflows::hello::HelloWorkflow)?;

    let worker = Worker::new(
        store,
        Arc::new(engine),
        "http://localhost:3000",
        "worker-token",
    );

    worker.run().await?;
    Ok(())
}
"#;

    fs::write(src.join("bin/worker.rs"), content)?;
    Ok(())
}

fn write_hello_workflow(dir: &Path) -> Result<()> {
    let workflows_dir = dir.join("src/workflows");
    fs::create_dir_all(&workflows_dir)?;

    let mod_content = "pub mod hello;\n";
    fs::write(workflows_dir.join("mod.rs"), mod_content)?;

    let content = r#"use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

pub struct HelloWorkflow;

impl WorkflowHandler for HelloWorkflow {
    fn name(&self) -> &str {
        "hello"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("greet", "echo 'Hello from Ironflow!'").await?;
            Ok(())
        })
    }
}
"#;

    fs::write(workflows_dir.join("hello.rs"), content)?;
    Ok(())
}

fn write_docker_compose(dir: &Path) -> Result<()> {
    let content = r#"services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: ironflow
      POSTGRES_USER: ironflow
      POSTGRES_PASSWORD: ironflow
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data

volumes:
  pgdata:
"#;

    fs::write(dir.join("docker-compose.yml"), content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn init_non_interactive_creates_project_structure() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("test-project");
        fs::create_dir_all(&project_dir).unwrap();

        std::env::set_current_dir(tmp.path()).unwrap();

        let args = InitArgs {
            non_interactive: true,
            name: Some("test-project".to_string()),
            force: false,
        };

        execute(&args).unwrap();

        assert!(project_dir.join("Cargo.toml").exists());
        assert!(project_dir.join("src/main.rs").exists());
        assert!(project_dir.join("src/bin/worker.rs").exists());
        assert!(project_dir.join("src/workflows/hello.rs").exists());
        assert!(project_dir.join("src/workflows/mod.rs").exists());
        assert!(!project_dir.join("docker-compose.yml").exists());

        let cargo_toml = fs::read_to_string(project_dir.join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("store-memory"));
        assert!(cargo_toml.contains(r#"name = "test-project""#));
    }

    #[test]
    fn init_refuses_non_empty_dir_without_force() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("existing");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("something.txt"), "data").unwrap();

        std::env::set_current_dir(tmp.path()).unwrap();

        let args = InitArgs {
            non_interactive: true,
            name: Some("existing".to_string()),
            force: false,
        };

        let result = execute(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not empty"));
    }
}
