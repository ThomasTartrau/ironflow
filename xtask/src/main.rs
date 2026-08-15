//! ironflow developer task runner.
//!
//! ```sh
//! cargo xtask seed           # seed InMemoryStore (dry run)
//! cargo xtask seed --force   # reset and re-seed
//! ```

mod seed;

use std::process;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

/// ironflow developer task runner.
#[derive(Parser)]
#[command(name = "xtask", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Available tasks.
#[derive(Subcommand)]
enum Command {
    /// Seed the store with development data.
    Seed(seed::SeedArgs),
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".parse().expect("valid filter")),
        )
        .init();

    let cli = Cli::parse();

    let result = match cli.command {
        Command::Seed(args) => seed::run(args).await,
    };

    if let Err(err) = result {
        eprintln!("error: {err:#}");
        process::exit(1);
    }
}
