//! `ironflow-cli dashboard` -- open the web dashboard.

use anyhow::Result;
use clap::Args;
use ironflow_sdk::IronflowClient;

/// Arguments for the `dashboard` command.
#[derive(Debug, Args)]
pub struct DashboardArgs {
    /// Print the URL instead of opening it in a browser.
    #[arg(long)]
    pub print: bool,
}

/// Execute the `dashboard` command.
///
/// Constructs the dashboard URL from the client's base URL and either opens
/// it in the default browser or prints it to stdout.
///
/// # Errors
///
/// Returns an error if the browser cannot be opened.
pub fn execute(client: &IronflowClient, args: &DashboardArgs) -> Result<()> {
    let url = client.base_url().to_string();

    if args.print {
        println!("{url}");
    } else {
        println!("Opening {url}");
        open::that(&url)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_print_returns_ok() {
        let client = IronflowClient::new("https://ironflow.example.com", "key");
        let args = DashboardArgs { print: true };
        execute(&client, &args).unwrap();
    }
}
