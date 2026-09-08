# Writing an Operation

Operations let you extend Ironflow with custom step types for integrating external services.

## 1. Implement the Operation trait

```rust,ignore
use std::env;
use std::future::Future;
use std::pin::Pin;

use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;
use serde_json::{Value, json};

pub struct SlackNotify {
    webhook_url: String,
    message: String,
}

impl SlackNotify {
    pub fn new(message: &str) -> Self {
        let webhook_url = env::var("SLACK_WEBHOOK_URL")
            .expect("SLACK_WEBHOOK_URL env var required");
        Self {
            webhook_url,
            message: message.to_string(),
        }
    }
}

impl Operation for SlackNotify {
    fn kind(&self) -> &str {
        "slack-notify"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({ "message": self.message }))
    }

    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
        Box::pin(async move {
            let client = reqwest::Client::new();
            let resp = client
                .post(&self.webhook_url)
                .json(&json!({ "text": self.message }))
                .send()
                .await
                .map_err(|e| EngineError::OperationFailed {
                    kind: "slack-notify".to_string(),
                    message: e.to_string(),
                })?;

            Ok(json!({ "status": resp.status().as_u16() }))
        })
    }
}
```

## 2. Use it in a workflow

```rust,ignore
let notifier = SlackNotify::new("Deploy complete!");
ctx.operation("notify-team", &notifier).await?;
```

The step is tracked in the database like any other step, with its input, output, and status.
