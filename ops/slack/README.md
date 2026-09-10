# ironflow-ops-slack

Slack integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, powered by the [`slack_morphism`](https://crates.io/crates/slack-morphism) crate.

Provides typed Slack API operations (chat, conversations, files, users, reactions, pins, usergroups) as tracked workflow steps.

## Usage

```toml
[dependencies]
ironflow-ops-slack = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_slack::SlackClient;

// From a workflow step (reads slack_bot_token from the secret store)
let slack = SlackClient::from_context(&ctx).await?;

// Explicit token
let slack = SlackClient::new("xoxb-xxxx")?;
```

### Tracked operations

```rust,ignore
use ironflow_ops_slack::SlackClient;
use ironflow_ops_slack::chat::ChatPostMessage;
use slack_morphism::api::SlackApiChatPostMessageRequest;
use slack_morphism::{SlackChannelId, SlackMessageContent};

let slack = SlackClient::from_context(&ctx).await?;

let req = SlackApiChatPostMessageRequest::new(
    SlackChannelId::new("#deployments".to_string()),
    SlackMessageContent::new().with_text("Deploy v2.1.0 complete".to_string()),
);
let op = ChatPostMessage::new(&slack, req);
let output = ctx.operation("notify-deploy", &op).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `chat` | Post message, update, delete, schedule, unfurl |
| `conversations` | Create, archive, invite, kick, list, info, history, members, join, leave, set topic/purpose |
| `files` | Upload, delete, list, info |
| `reactions` | Add, remove, list, get |
| `pins` | Add, remove, list |
| `users` | Info, list, presence, profile |
| `usergroups` | Create, update, disable, enable, list, users |
| `team` | Info, access logs |
| `emoji` | List |
| `stars` | Add, remove, list |
| `auth` | Test |
| `bots` | Info |
| `apps` | Connections open |
| `assistant` | Thread set status, set title, set suggested prompts |

## Authentication

Register `slack_bot_token` in your workflow's secret store:

```yaml
secrets:
  - name: slack_bot_token
    env: SLACK_BOT_TOKEN   # xoxb-... bot token
```
