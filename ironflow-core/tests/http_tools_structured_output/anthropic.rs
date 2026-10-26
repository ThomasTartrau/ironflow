//! Anthropic Messages API: registry tools next to `structured_output`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use ironflow_core::provider::{AgentConfig, AgentProvider, ToolProfile};
use ironflow_core::providers::http::HttpAgentProvider;
use ironflow_core::providers::http::anthropic::AnthropicModel;
use ironflow_core::providers::http::anthropic::adapter::AnthropicApiAdapter;

use crate::server::{CountingTool, Reply, registry_with, spawn_llm, within_timeout};

const PROFILE: ToolProfile = ToolProfile::new("lookup");
const SCHEMA: &str = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;

fn provider(
    url: &str,
    tool: CountingTool,
) -> (HttpAgentProvider<AnthropicApiAdapter>, Arc<AtomicUsize>) {
    let (registry, calls) = registry_with(tool);
    let provider = HttpAgentProvider::new(
        AnthropicApiAdapter::new("k".to_string()).with_base_url(url.to_string()),
    )
    .with_tool_profile(PROFILE, registry);
    (provider, calls)
}

fn config(verbose: bool) -> AgentConfig {
    AgentConfig::new("find x")
        .model(AnthropicModel::HAIKU_5_5)
        .verbose(verbose)
        .tool_profile(PROFILE)
        .output_schema_raw(SCHEMA)
        .into()
}

fn tool_use_reply(id: &str, name: &str, input: Value) -> Reply {
    Reply::Json(json!({
        "model": "claude-haiku-5-5",
        "content": [{"type": "tool_use", "id": id, "name": name, "input": input}],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))
}

fn names(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("tool name").to_string())
        .collect()
}

fn sse(events: &[Value]) -> String {
    events.iter().map(|e| format!("data: {e}\n\n")).collect()
}

#[tokio::test]
async fn anthropic_tool_profile_with_output_sends_registry_tools_and_structured_output() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![tool_use_reply(
            "tu_1",
            "structured_output",
            json!({"x": 1}),
        )])
        .await;
        let (provider, _) = provider(&url, CountingTool::new("lookup"));

        provider.invoke(&config(false)).await.expect("invoke");

        let requests = requests.lock().expect("lock");
        let tools = names(&requests[0]);
        assert_eq!(tools, vec!["structured_output", "lookup"]);
        if let Some(name) = requests[0]["tool_choice"]["name"].as_str() {
            assert!(tools.iter().any(|t| t == name));
        }
    })
    .await;
}

#[tokio::test]
async fn anthropic_model_calls_registry_tool_then_returns_structured_output() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![
            tool_use_reply("tu_1", "lookup", json!({})),
            tool_use_reply("tu_2", "structured_output", json!({"x": 42})),
        ])
        .await;
        let (provider, calls) = provider(&url, CountingTool::new("lookup"));

        let output = provider.invoke(&config(false)).await.expect("invoke");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.value, json!({"x": 42}));
        let requests = requests.lock().expect("lock");
        assert_eq!(requests.len(), 2);
        let messages = requests[1]["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["id"], "tu_1");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "tu_1");
        assert_eq!(messages[2]["content"][0]["content"], "lookup ran");
        assert_eq!(messages[2]["content"][0]["is_error"], false);
    })
    .await;
}

#[tokio::test]
async fn anthropic_unknown_tool_call_is_reported_as_error_tool_result() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![
            tool_use_reply("tu_1", "missing", json!({})),
            tool_use_reply("tu_2", "structured_output", json!({"x": 3})),
        ])
        .await;
        let (provider, calls) = provider(&url, CountingTool::new("lookup"));

        let output = provider.invoke(&config(false)).await.expect("invoke");

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(output.value, json!({"x": 3}));
        let requests = requests.lock().expect("lock");
        let result = &requests[1]["messages"][2]["content"][0];
        assert_eq!(result["tool_use_id"], "tu_1");
        assert_eq!(result["is_error"], true);
        assert!(
            result["content"]
                .as_str()
                .expect("content")
                .starts_with("Unknown tool: missing")
        );
    })
    .await;
}

#[tokio::test]
async fn anthropic_streaming_registry_tool_then_structured_output() {
    within_timeout(async {
        let first = sse(&[
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 1, "output_tokens": 0}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "tu_1", "name": "lookup", "input": {}}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{}"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 1}}),
            json!({"type": "message_stop"}),
        ]);
        let second = sse(&[
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 1, "output_tokens": 0}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "done"}}),
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "tu_2", "name": "structured_output", "input": {}}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"x\": 9}"}}),
            json!({"type": "message_stop"}),
        ]);
        let (url, requests) = spawn_llm(vec![Reply::Sse(first), Reply::Sse(second)]).await;
        let (provider, calls) = provider(&url, CountingTool::new("lookup"));

        let output = provider.invoke(&config(true)).await.expect("invoke");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.value, json!({"x": 9}));
        let requests = requests.lock().expect("lock");
        assert_eq!(requests[0]["stream"], true);
        let messages = requests[1]["messages"].as_array().expect("messages");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["name"], "lookup");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "tu_1");
        // An empty-name call would have been reported as an unknown tool.
        assert_eq!(messages[2]["content"][0]["is_error"], false);
        assert_eq!(messages[2]["content"].as_array().map(Vec::len), Some(1));
    })
    .await;
}
