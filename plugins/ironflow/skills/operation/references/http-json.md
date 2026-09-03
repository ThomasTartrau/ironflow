# Example: generic JSON API call with a typed response

Wraps any JSON endpoint. The handler picks the response type; the operation persists a
compact output.

```rust,no_run
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use ironflow_core::operations::http::Http;
use ironflow_core::error::OperationError;
use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;
use serde::Deserialize;
use serde_json::{Value, json};

/// `GET <base_url>/<path>` with a bearer token, response parsed as JSON.
pub struct JsonGet {
    pub base_url: String,
    pub path: String,
    pub bearer: String,
}

/// Shape the operation expects back. Adapt per endpoint.
#[derive(Debug, Deserialize)]
pub struct Page {
    pub id: u64,
    pub title: String,
}

impl Operation for JsonGet {
    fn kind(&self) -> &str {
        "json_get"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"url": format!("{}/{}", self.base_url, self.path)}))
    }

    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
        Box::pin(async move {
            let url = format!("{}/{}", self.base_url, self.path);
            let resp = Http::get(&url)
                .header("Authorization", &format!("Bearer {}", self.bearer))
                .header("Accept", "application/json")
                .timeout(Duration::from_secs(30))
                .await?;

            if !resp.is_success() {
                return Err(EngineError::Operation(OperationError::Http {
                    status: Some(resp.status()),
                    message: format!("{url} answered {}", resp.status()),
                }));
            }

            // `json()` maps a parse failure to OperationError::Deserialize.
            let page: Page = resp.json()?;
            Ok(json!({"id": page.id, "title": page.title}))
        })
    }
}
```

Handler side:

```rust,no_run
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

struct JsonGet {
    base_url: String,
    path: String,
    bearer: String,
}

impl Operation for JsonGet {
    fn kind(&self) -> &str {
        "json_get"
    }
    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
        Box::pin(async move { Ok(json!({"id": 1, "title": "stub"})) })
    }
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let bearer = ctx
        .secrets()
        .get("api_token")
        .await
        .map_err(EngineError::Store)?
        .ok_or_else(|| EngineError::StepConfig("secret api_token missing".to_string()))?
        .value;

    let op = JsonGet {
        base_url: "https://api.example.com".to_string(),
        path: "pages/1".to_string(),
        bearer,
    };
    let out = ctx.operation("fetch-page", &op).await?;
    let _title = out.output["title"].as_str().unwrap_or_default();
    Ok(())
}
```
