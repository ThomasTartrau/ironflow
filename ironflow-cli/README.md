# ironflow-cli

Command-line interface for the **ironflow** workflow engine. Manage runs, workflows, secrets, API keys, users and audit logs, stream logs, and view statistics from the terminal.

## Installation

```bash
cargo install ironflow-cli
```

## Configuration

Create `~/.ironflow.toml`:

```toml
base_url = "https://ironflow.example.com"
api_key = "your-api-key"
```

Or use environment variables:

| Variable | Description |
|----------|-------------|
| `IRONFLOW_URL` | Base URL of the ironflow API |
| `IRONFLOW_API_KEY` | API key for authentication |

CLI flags (`--url`, `--api-key`) take the highest priority, then env vars, then the TOML file.

## Commands

| Command | Description |
|---------|-------------|
| `run create <workflow> [--payload '{}'] [--payload-file <path>]` | Create a new run |
| `run list [--status <s>] [--workflow <w>]` | List runs with optional filters |
| `run get <id>` | Get run details and steps |
| `run cancel <id>` | Cancel a run |
| `run approve <id>` | Approve a run waiting for approval |
| `run reject <id>` | Reject a run waiting for approval, failing it |
| `run retry <id>` | Retry a failed run |
| `workflow list` | List registered workflows |
| `workflow get <name>` | Get workflow details |
| `logs <run_id> [--follow]` | Stream run logs via SSE |
| `stats` | Show global statistics |
| `secret list` | List secret keys (values are never returned) |
| `secret set <key> [value]` | Create or replace a secret; reads the value on stdin when omitted |
| `secret update <key> [value]` | Replace an existing secret; fails if the key is unknown |
| `secret delete <key> [--yes]` | Delete a secret |
| `api-key list` | List API keys (prefix only, never the raw key) |
| `api-key create <name> --scope <s>... [--expires-at <rfc3339>]` | Create an API key; the raw key is printed once |
| `api-key scopes` | List the scopes an API key can be granted |
| `api-key delete <id> [--yes]` | Delete an API key |
| `user list` | List users |
| `user create <username> --email <e> [--password <p>] [--admin]` | Create a user; reads the password on stdin when omitted |
| `user delete <id> [--yes]` | Delete a user |
| `user set-role <id> --admin\|--member` | Promote or demote a user |
| `audit-log list [--run <id>] [--type <kind>] [--from <d>] [--to <d>]` | List audit log entries |

Secrets, users and audit logs are admin-only.

Destructive commands ask for confirmation. `--yes` skips it, and is **required**
when stdin is not a terminal (CI): without it the command fails rather than
prompting into the void or deleting silently.

Secret values and passwords can be piped instead of passed as arguments, which
keeps them out of the shell history and out of `ps` output:

```bash
echo -n "$TOKEN" | ironflow-cli secret set workflows/inbox/gmail_token
```

## Global flags

| Flag | Description |
|------|-------------|
| `--json` | Output raw JSON instead of formatted tables |
| `--verbose` | Show verbose output (e.g. full step details) |
| `--url <url>` | Override the API base URL |
| `--api-key <key>` | Override the API key |

## Usage

```bash
# List all runs
ironflow-cli run list

# Create a run with a JSON payload
ironflow-cli run create deploy --payload '{"env": "prod"}'

# Create a run with payload from file
ironflow-cli run create deploy --payload-file payload.json

# Get run details as JSON
ironflow-cli --json run get 01234567-89ab-cdef-0123-456789abcdef

# Stream logs in real-time
ironflow-cli logs 01234567-89ab-cdef-0123-456789abcdef --follow

# Global stats
ironflow-cli stats

# Set a secret without leaking it into the shell history
echo -n 'super-secret' | ironflow-cli secret set workflows/inbox/gmail_token

# Mint a scoped API key for CI (the raw key is shown once)
ironflow-cli api-key create ci-deploy --scope runs_read --scope runs_write

# Promote a user to admin
ironflow-cli user set-role 01234567-89ab-cdef-0123-456789abcdef --admin

# Audit what happened on a run
ironflow-cli audit-log list --run 01234567-89ab-cdef-0123-456789abcdef

# Delete without a prompt, e.g. from a CI job
ironflow-cli secret delete workflows/inbox/gmail_token --yes
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
