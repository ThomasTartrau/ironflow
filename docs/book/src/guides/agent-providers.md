# Agent Providers

A provider is what an agent step talks to: the Claude Code CLI, a remote Claude Code, or an
HTTP API such as OpenAI. Every provider implements `AgentProvider`, so a workflow written
against one runs against any other. Only `ClaudeCodeProvider` is always available; the others
are behind a [feature flag](../reference/feature-flags.md).

| Provider | Feature flag | Use case |
|----------|-------------|----------|
| `ClaudeCodeProvider` | *(always available)* | Claude Code CLI installed locally |
| `SshProvider` | `transport-ssh` | Claude Code on a remote build server |
| `DockerProvider` | `transport-docker` | Claude Code inside a running container |
| `K8sEphemeralProvider` | `transport-k8s` | One pod per invocation, full isolation |
| `K8sPersistentProvider` | `transport-k8s` | Reuses a worker pod, lower latency |
| `AnthropicApiProvider` | `provider-anthropic-api` | Anthropic Messages API, no CLI needed |
| `OpenAiProvider` | `provider-openai` | OpenAI Chat Completions |
| `GeminiProvider` | `provider-gemini` | Google Gemini |
| `MistralProvider` | `provider-mistral` | Mistral |
| `NvidiaProvider` | `provider-nvidia` | NVIDIA NIM, 100+ models behind one API |

The SSH, Docker and Kubernetes providers run Claude Code somewhere else; they are described
in [Transports](transports.md).

## HTTP providers

HTTP providers are used exactly like the local one:

```rust,no_run
use ironflow_core::prelude::*;
use ironflow_core::providers::http::{NvidiaModel, NvidiaProvider};

# async fn example() -> Result<(), OperationError> {
let provider = NvidiaProvider::from_env(); // reads NVIDIA_API_KEY

let result = Agent::new()
    .prompt("Summarize the changelog")
    .model(NvidiaModel::DEEPSEEK_V4_FLASH)
    .max_budget_usd(0.10)
    .run(&provider)
    .await?;
# Ok(())
# }
```

## Tools for HTTP providers

The Claude Code CLI brings its own tools. HTTP providers do not, so tools are opt-in, one
feature each: `tool-bash`, `tool-read-file`, `tool-grep`, `tool-glob`, `tool-web-fetch`,
`tool-web-search`, and `tool-mcp` to bring any MCP server into the agent's toolset.
`GrepTool` and `GlobTool` search a codebase without handing the agent a shell: they are
confined to the directories you give them.

```rust,no_run
use std::path::PathBuf;

use ironflow_core::providers::http::tools::glob::GlobTool;
use ironflow_core::providers::http::tools::grep::GrepTool;
use ironflow_core::providers::http::tools::{ToolError, ToolRegistry};

# fn example() -> Result<(), ToolError> {
let repo = vec![PathBuf::from("/srv/repo")];
let tools = ToolRegistry::new()
    .register(GrepTool::with_allowed_paths(repo.clone())?)
    .register(GlobTool::with_allowed_paths(repo)?);
# Ok(())
# }
```

### Tool profiles

A step picks a named tool profile and sees only its tools; without one it gets the
`with_tools` registry, or none. Declare each `ToolProfile` once as a constant shared by the
provider and the steps, so a misspelled profile does not compile. An MCP server shared between
profiles is opened once:

```rust,no_run
use std::sync::Arc;
use ironflow_core::prelude::*;
use ironflow_core::provider::{AgentConfig, ToolProfile};
use ironflow_core::providers::http::OpenAiProvider;
use ironflow_core::providers::http::tools::ToolRegistry;
use ironflow_core::providers::http::tools::mcp::{McpConnection, register_shared_mcp_tools};

const SUGGESTION: ToolProfile = ToolProfile::new("suggestion");
const BUG: ToolProfile = ToolProfile::new("bug");

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let mut gitlab = McpConnection::stdio("mcp-gitlab", &[], &[]).await?;
gitlab.initialize().await?;
let gitlab = Arc::new(gitlab);
let provider = OpenAiProvider::from_env()
    .with_tool_profile(SUGGESTION, register_shared_mcp_tools(ToolRegistry::new(), &gitlab, "gitlab").await?)
    .with_tool_profile(BUG, register_shared_mcp_tools(ToolRegistry::new(), &gitlab, "gitlab").await?);

let config = AgentConfig::new("Find the root cause").tool_profile(BUG);
let result = Agent::from_config(config).model("gpt-5.5").run(&provider).await?;
# Ok(())
# }
```

In a workflow, an agent step selects the profile with `.tool_profile(BUG)`; see
[Agent steps](../concepts/steps.md#agent-steps).

## Routing between providers

`ProviderRouter` picks the provider from the model name, so a single workflow can mix
vendors:

```rust,no_run
use std::sync::Arc;
use ironflow_core::prelude::*;
use ironflow_core::providers::http::NvidiaProvider;

# async fn example() -> Result<(), OperationError> {
let claude = Arc::new(ClaudeCodeProvider::new());
let nvidia = Arc::new(NvidiaProvider::from_env());

let router = ProviderRouter::new(claude)
    .route(ProviderMatcher::ModelPrefix("nvidia/".into()), nvidia);

// Goes to Claude Code
let a = Agent::new().prompt("Review").model(Model::SONNET).run(&router).await?;

// Goes to NVIDIA
let b = Agent::new().prompt("Review").model("nvidia/deepseek-v4-flash").run(&router).await?;
# Ok(())
# }
```

To spread agent steps over several Claude subscriptions or API keys, see
[Provider Accounts](../concepts/provider-accounts.md).
