# ironflow-cli

Command-line interface for the **ironflow** workflow engine. Manage runs, workflows, stream logs, and view statistics from the terminal.

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
| `run retry <id>` | Retry a failed run |
| `workflow list` | List registered workflows |
| `workflow get <name>` | Get workflow details |
| `logs <run_id> [--follow]` | Stream run logs via SSE |
| `stats` | Show global statistics |

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
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
