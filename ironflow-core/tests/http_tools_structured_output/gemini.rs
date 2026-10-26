//! Gemini `generateContent` API: registry functions next to `structured_output`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use ironflow_core::provider::{AgentConfig, AgentProvider, ToolProfile};
use ironflow_core::providers::http::HttpAgentProvider;
use ironflow_core::providers::http::gemini::GeminiModel;
use ironflow_core::providers::http::gemini::adapter::GeminiAdapter;

use crate::server::{CountingTool, Reply, registry_with, spawn_llm, within_timeout};

const PROFILE: ToolProfile = ToolProfile::new("lookup");
const SCHEMA: &str = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;

fn provider(url: &str, tool: CountingTool) -> (HttpAgentProvider<GeminiAdapter>, Arc<AtomicUsize>) {
    let (registry, calls) = registry_with(tool);
    let provider =
        HttpAgentProvider::new(GeminiAdapter::new("k".to_string()).with_base_url(url.to_string()))
            .with_tool_profile(PROFILE, registry);
    (provider, calls)
}

fn config() -> AgentConfig {
    AgentConfig::new("find x")
        .model(GeminiModel::FLASH_3_5)
        .tool_profile(PROFILE)
        .output_schema_raw(SCHEMA)
        .into()
}

fn function_call_reply(part: Value) -> Reply {
    Reply::Json(json!({
        "candidates": [{"content": {"role": "model", "parts": [part]}, "finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1},
        "modelVersion": "gemini-3.5-flash"
    }))
}

#[tokio::test]
async fn gemini_tool_profile_with_output_sends_function_declarations() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![function_call_reply(
            json!({"functionCall": {"name": "structured_output", "args": {"x": 1}}}),
        )])
        .await;
        let (provider, _) = provider(&url, CountingTool::new("lookup"));

        provider.invoke(&config()).await.expect("invoke");

        let requests = requests.lock().expect("lock");
        let declarations: Vec<&str> = requests[0]["tools"][0]["functionDeclarations"]
            .as_array()
            .expect("declarations")
            .iter()
            .map(|d| d["name"].as_str().expect("name"))
            .collect();
        assert_eq!(declarations, vec!["structured_output", "lookup"]);
        assert!(requests[0].get("generationConfig").is_none());
        assert_eq!(
            requests[0]["toolConfig"]["functionCallingConfig"]["mode"],
            "ANY"
        );
        assert!(requests[0].get("messages").is_none());
    })
    .await;
}

#[tokio::test]
async fn gemini_model_calls_registry_function_then_returns_structured_output() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![
            function_call_reply(json!({
                "functionCall": {"name": "lookup", "args": {}},
                "thoughtSignature": "sig-1"
            })),
            function_call_reply(
                json!({"functionCall": {"name": "structured_output", "args": {"x": 42}}}),
            ),
        ])
        .await;
        let (provider, calls) = provider(&url, CountingTool::new("lookup"));

        let output = provider.invoke(&config()).await.expect("invoke");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.value, json!({"x": 42}));
        let requests = requests.lock().expect("lock");
        assert_eq!(requests.len(), 2);
        let contents = requests[1]["contents"].as_array().expect("contents");
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["functionCall"]["name"], "lookup");
        assert_eq!(contents[1]["parts"][0]["thoughtSignature"], "sig-1");
        assert_eq!(contents[2]["role"], "user");
        let response = &contents[2]["parts"][0]["functionResponse"];
        assert_eq!(response["name"], "lookup");
        assert_eq!(response["response"]["output"], "lookup ran");
    })
    .await;
}
