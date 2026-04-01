//! Parallel step execution utilities.
//!
//! Run independent workflow steps concurrently to reduce total wall-clock time.
//! Two patterns are supported:
//!
//! # Static parallelism (known number of steps)
//!
//! Use [`tokio::try_join!`] when you know at compile time how many steps
//! to run in parallel:
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let (files, status) = tokio::try_join!(
//!     Shell::new("ls -la"),
//!     Shell::new("git status"),
//! )?;
//!
//! println!("files:\n{}", files.stdout());
//! println!("status:\n{}", status.stdout());
//! # Ok(())
//! # }
//! ```
//!
//! # Dynamic parallelism (runtime-determined number of steps)
//!
//! Use [`try_join_all`] when the number of steps is determined at runtime:
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let commands = vec!["ls -la", "git status", "df -h"];
//! let results = try_join_all(
//!     commands.iter().map(|cmd| Shell::new(cmd).run())
//! ).await?;
//!
//! for (cmd, output) in commands.iter().zip(&results) {
//!     println!("{cmd}: {}", output.stdout());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Concurrency-limited parallelism
//!
//! Use [`try_join_all_limited`] to cap the number of steps running
//! simultaneously (useful when launching many agent calls):
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let provider = ClaudeCodeProvider::new();
//! let prompts = vec!["Summarize file A", "Summarize file B", "Summarize file C"];
//!
//! let results = try_join_all_limited(
//!     prompts.iter().map(|p| {
//!         Agent::new()
//!             .prompt(p)
//!             .model(Model::HAIKU)
//!             .max_budget_usd(0.10)
//!             .run(&provider)
//!     }),
//!     2, // at most 2 agent calls at a time
//! ).await?;
//! # Ok(())
//! # }
//! ```

use std::future::Future;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::error::OperationError;

/// Run a collection of futures concurrently and collect their results.
///
/// All futures start executing immediately. Returns a [`Vec<T>`] in the same
/// order as the input iterator, or the first [`OperationError`] encountered
/// (remaining futures are dropped on error).
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
///
/// # async fn example() -> Result<(), OperationError> {
/// let outputs = try_join_all(vec![
///     Shell::new("echo one").run(),
///     Shell::new("echo two").run(),
///     Shell::new("echo three").run(),
/// ]).await?;
///
/// assert_eq!(outputs.len(), 3);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the first [`OperationError`] produced by any future. When an error
/// occurs, all other in-flight futures are cancelled.
pub async fn try_join_all<I, F, T>(futures: I) -> Result<Vec<T>, OperationError>
where
    I: IntoIterator<Item = F>,
    F: Future<Output = Result<T, OperationError>>,
{
    futures_util::future::try_join_all(futures).await
}

/// Run a collection of futures with a concurrency limit.
///
/// At most `limit` futures execute simultaneously. Results are returned in
/// the same order as the input iterator. Useful when running many agent
/// calls to avoid overwhelming the system or exceeding rate limits.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
///
/// # async fn example() -> Result<(), OperationError> {
/// let commands: Vec<&str> = (0..20)
///     .map(|_| "echo hello")
///     .collect();
///
/// let outputs = try_join_all_limited(
///     commands.iter().map(|cmd| Shell::new(cmd).run()),
///     5, // run at most 5 in parallel
/// ).await?;
///
/// assert_eq!(outputs.len(), 20);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the first [`OperationError`] produced by any future. When an error
/// occurs, all other in-flight futures are cancelled.
///
/// # Panics
///
/// Panics if `limit` is `0`.
pub async fn try_join_all_limited<I, F, T>(
    futures: I,
    limit: usize,
) -> Result<Vec<T>, OperationError>
where
    I: IntoIterator<Item = F>,
    F: Future<Output = Result<T, OperationError>>,
{
    assert!(limit > 0, "concurrency limit must be greater than 0");

    let sem = Arc::new(Semaphore::new(limit));
    let guarded = futures.into_iter().map(|f| {
        let sem = sem.clone();
        async move {
            let _permit = sem.acquire().await.expect("semaphore closed unexpectedly");
            f.await
        }
    });

    futures_util::future::try_join_all(guarded).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::operations::shell::Shell;

    #[tokio::test]
    async fn try_join_all_empty_returns_empty_vec() {
        let result: Result<Vec<()>, OperationError> = try_join_all(Vec::<
            std::pin::Pin<Box<dyn Future<Output = Result<(), OperationError>> + Send>>,
        >::new())
        .await;
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn try_join_all_single_future() {
        let results = try_join_all(vec![Shell::new("echo hello").dry_run(false).run()])
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].stdout().trim(), "hello");
    }

    #[tokio::test]
    async fn try_join_all_multiple_futures_preserves_order() {
        let results = try_join_all(vec![
            Shell::new("echo one").dry_run(false).run(),
            Shell::new("echo two").dry_run(false).run(),
            Shell::new("echo three").dry_run(false).run(),
        ])
        .await
        .unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].stdout().trim(), "one");
        assert_eq!(results[1].stdout().trim(), "two");
        assert_eq!(results[2].stdout().trim(), "three");
    }

    #[tokio::test]
    async fn try_join_all_runs_concurrently() {
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let futs: Vec<_> = (0..3)
            .map(|i| {
                let concurrent = concurrent.clone();
                let max_concurrent = max_concurrent.clone();
                async move {
                    let current = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                    max_concurrent.fetch_max(current, Ordering::SeqCst);
                    let result = Shell::new(&format!("sleep 0.05 && echo {i}"))
                        .dry_run(false)
                        .run()
                        .await;
                    concurrent.fetch_sub(1, Ordering::SeqCst);
                    result
                }
            })
            .collect();

        let results = try_join_all(futs).await.unwrap();
        assert_eq!(results.len(), 3);
        // All 3 should run concurrently (no limit)
        assert!(
            max_concurrent.load(Ordering::SeqCst) >= 2,
            "expected concurrent execution, max concurrency was {}",
            max_concurrent.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn try_join_all_returns_first_error() {
        let result = try_join_all(vec![
            Shell::new("echo ok").dry_run(false).run(),
            Shell::new("exit 1").dry_run(false).run(),
            Shell::new("echo also ok").dry_run(false).run(),
        ])
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, OperationError::Shell { exit_code: 1, .. }));
    }

    #[tokio::test]
    async fn try_join_all_from_iterator() {
        let commands = ["echo alpha", "echo beta"];
        let results = try_join_all(commands.iter().map(|c| Shell::new(c).dry_run(false).run()))
            .await
            .unwrap();

        assert_eq!(results[0].stdout().trim(), "alpha");
        assert_eq!(results[1].stdout().trim(), "beta");
    }

    // --- try_join_all_limited ---

    #[tokio::test]
    async fn limited_empty_returns_empty_vec() {
        let result: Result<Vec<()>, OperationError> = try_join_all_limited(
            Vec::<std::pin::Pin<Box<dyn Future<Output = Result<(), OperationError>> + Send>>>::new(
            ),
            3,
        )
        .await;
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn limited_preserves_order() {
        let results = try_join_all_limited(
            vec![
                Shell::new("echo one").dry_run(false).run(),
                Shell::new("echo two").dry_run(false).run(),
                Shell::new("echo three").dry_run(false).run(),
            ],
            2,
        )
        .await
        .unwrap();

        assert_eq!(results[0].stdout().trim(), "one");
        assert_eq!(results[1].stdout().trim(), "two");
        assert_eq!(results[2].stdout().trim(), "three");
    }

    #[tokio::test]
    async fn limited_respects_concurrency_limit() {
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let futs: Vec<_> = (0..6)
            .map(|i| {
                let concurrent = concurrent.clone();
                let max_concurrent = max_concurrent.clone();
                async move {
                    let current = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                    max_concurrent.fetch_max(current, Ordering::SeqCst);
                    let result = Shell::new(&format!("sleep 0.05 && echo {i}"))
                        .dry_run(false)
                        .run()
                        .await;
                    concurrent.fetch_sub(1, Ordering::SeqCst);
                    result
                }
            })
            .collect();

        let results = try_join_all_limited(futs, 2).await.unwrap();
        assert_eq!(results.len(), 6);
        assert!(
            max_concurrent.load(Ordering::SeqCst) <= 2,
            "max concurrency was {}, expected <= 2",
            max_concurrent.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn limited_returns_first_error() {
        let result = try_join_all_limited(
            vec![
                Shell::new("echo ok").dry_run(false).run(),
                Shell::new("exit 42").dry_run(false).run(),
                Shell::new("echo also ok").dry_run(false).run(),
            ],
            2,
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    #[should_panic(expected = "concurrency limit must be greater than 0")]
    async fn limited_zero_limit_panics() {
        let _: Result<Vec<()>, _> = try_join_all_limited(
            Vec::<std::pin::Pin<Box<dyn Future<Output = Result<(), OperationError>> + Send>>>::new(
            ),
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn limited_with_limit_one_runs_sequentially() {
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let futs: Vec<_> = (0..3)
            .map(|i| {
                let concurrent = concurrent.clone();
                let max_concurrent = max_concurrent.clone();
                async move {
                    let current = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                    max_concurrent.fetch_max(current, Ordering::SeqCst);
                    let result = Shell::new(&format!("sleep 0.05 && echo {i}"))
                        .dry_run(false)
                        .run()
                        .await;
                    concurrent.fetch_sub(1, Ordering::SeqCst);
                    result
                }
            })
            .collect();

        let results = try_join_all_limited(futs, 1).await.unwrap();
        assert_eq!(results.len(), 3);
        // With limit=1, only 1 should run at a time
        assert_eq!(
            max_concurrent.load(Ordering::SeqCst),
            1,
            "expected max concurrency of 1, got {}",
            max_concurrent.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn limited_with_limit_greater_than_count() {
        let results = try_join_all_limited(
            vec![
                Shell::new("echo x").dry_run(false).run(),
                Shell::new("echo y").dry_run(false).run(),
            ],
            100,
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].stdout().trim(), "x");
        assert_eq!(results[1].stdout().trim(), "y");
    }
}
