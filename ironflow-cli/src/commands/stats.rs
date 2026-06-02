//! Stats command.

use anyhow::Result;
use ironflow_sdk::IronflowClient;

use crate::output;

/// Execute the `stats` command.
///
/// # Errors
///
/// Returns an error on API failure.
pub async fn execute(client: &IronflowClient, json_mode: bool) -> Result<()> {
    let response = client.get_stats().await?;
    output::print_output(json_mode, &response, || output::stats_table(&response.data))?;
    Ok(())
}
