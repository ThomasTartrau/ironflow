# ironflow-mcp

MCP (Model Context Protocol) server for **ironflow**. Exposes workflow orchestration as MCP tools that Claude (or any MCP client) can call directly.

## Available tools

| Tool | Description |
|------|-------------|
| `list_workflows` | List all registered workflows |
| `get_workflow` | Get details of a specific workflow |
| `create_run` | Create a new workflow run |
| `get_run` | Get run status and details |
| `list_runs` | List runs with filtering |
| `approve_run` | Approve a run waiting for approval |
| `reject_run` | Reject a pending run |
| `cancel_run` | Cancel a running workflow |
| `retry_run` | Retry a failed run |
| `get_stats` | Get aggregate statistics |

## Configuration

Set the following environment variables:

| Variable | Description |
|----------|-------------|
| `IRONFLOW_API_URL` | Base URL of the ironflow API server |
| `IRONFLOW_API_KEY` | API key for authentication |

## Usage with Claude Code

Add to your MCP configuration:

```json
{
  "mcpServers": {
    "ironflow": {
      "command": "ironflow-mcp",
      "env": {
        "IRONFLOW_API_URL": "http://localhost:3000",
        "IRONFLOW_API_KEY": "your-api-key"
      }
    }
  }
}
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
