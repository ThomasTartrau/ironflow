use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Deletes the remote branches already merged into main.
pub struct CleanupBranches;

impl WorkflowHandler for CleanupBranches {
    fn name(&self) -> &str {
        "cleanup-branches"
    }

    fn description(&self) -> &str {
        "Delete the remote branches already merged into main."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let merged = ctx
                .shell(
                    "list-merged",
                    ShellConfig::new(
                        "git fetch --prune origin && git branch -r --merged origin/main \
                         | grep -v -e 'origin/main' -e 'HEAD' | sed 's|origin/||'",
                    )
                    .dir("/srv/repo"),
                )
                .await?;

            let branches: Vec<String> = merged
                .stdout()
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect();

            for (i, branch) in branches.iter().enumerate() {
                ctx.shell(
                    &format!("delete-{i}"),
                    ShellConfig::exec("git", &["push", "origin", "--delete", branch.as_str()])
                        .dir("/srv/repo"),
                )
                .await?;
            }

            Ok(())
        })
    }
}
