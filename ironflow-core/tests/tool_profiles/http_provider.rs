//! Profile selection on `HttpAgentProvider`: what each request exposes, what
//! each tool call may run, and what the run trace shows.

use std::sync::atomic::Ordering;

use serde_json::json;

use ironflow_core::error::AgentError;
use ironflow_core::provider::{AgentConfig, AgentProvider};
use ironflow_core::providers::http::tools::ToolRegistry;

use crate::harness::{
    BUG, CountingTool, INCIDENT, SUGGESTION, VecSink, final_answer, names, provider, registry,
    spawn_llm, tool_call_answer, tool_names, within_timeout,
};

#[tokio::test]
async fn tool_profile_each_step_sees_only_its_tools() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![final_answer("ok"), final_answer("ok")]).await;
        let provider = provider(&url)
            .with_tool_profile(SUGGESTION, registry(&["code_search", "gitlab_mr"]))
            .with_tool_profile(
                BUG,
                registry(&["code_search", "gitlab_mr", "grafana_query", "sentry_issue"]),
            );

        let suggestion: AgentConfig = AgentConfig::new("suggest").tool_profile(SUGGESTION).into();
        let bug: AgentConfig = AgentConfig::new("triage").tool_profile(BUG).into();
        provider.invoke(&suggestion).await.expect("suggestion step");
        provider.invoke(&bug).await.expect("bug step");

        let requests = requests.lock().expect("lock");
        assert_eq!(requests.len(), 2);
        assert_eq!(
            tool_names(&requests[0]),
            names(&["code_search", "gitlab_mr"])
        );
        assert_eq!(
            tool_names(&requests[1]),
            names(&["code_search", "gitlab_mr", "grafana_query", "sentry_issue"])
        );
    })
    .await;
}

#[tokio::test]
async fn tool_profile_unknown_fails_without_calling_the_model() {
    within_timeout(async {
        let (url, requests) = spawn_llm(Vec::new()).await;
        let provider = provider(&url)
            .with_tools(registry(&["code_search"]))
            .with_tool_profile(SUGGESTION, registry(&["gitlab_mr"]))
            .with_tool_profile(BUG, registry(&["grafana_query"]));

        let config: AgentConfig = AgentConfig::new("triage").tool_profile(INCIDENT).into();
        let err = provider
            .invoke(&config)
            .await
            .expect_err("an unknown profile must fail the step");

        match err {
            AgentError::UnknownToolProfile {
                ref profile,
                ref available,
            } => {
                assert_eq!(profile, "incident");
                assert_eq!(
                    available,
                    &vec!["bug".to_string(), "suggestion".to_string()]
                );
            }
            ref other => panic!("expected UnknownToolProfile, got {other:?}"),
        }
        assert_eq!(
            err.to_string(),
            "unknown tool profile 'incident' (registered profiles: bug, suggestion)"
        );
        assert!(
            requests.lock().expect("lock").is_empty(),
            "no request may reach the model with an unknown profile"
        );
    })
    .await;
}

#[tokio::test]
async fn tool_profile_unknown_on_provider_without_profiles_lists_none() {
    within_timeout(async {
        let (url, requests) = spawn_llm(Vec::new()).await;
        let config: AgentConfig = AgentConfig::new("triage").tool_profile(BUG).into();
        let err = provider(&url)
            .with_tools(registry(&["code_search"]))
            .invoke(&config)
            .await
            .expect_err("the default profile is never a fallback for a named one");

        assert_eq!(
            err.to_string(),
            "unknown tool profile 'bug' (registered profiles: none)"
        );
        assert!(requests.lock().expect("lock").is_empty());
    })
    .await;
}

#[tokio::test]
async fn tool_profile_absent_exposes_no_tools() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![final_answer("ok")]).await;
        let provider = provider(&url).with_tool_profile(BUG, registry(&["grafana_query"]));

        provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .expect("step without profile");

        assert_eq!(tool_names(&requests.lock().expect("lock")[0]), None);
    })
    .await;
}

#[tokio::test]
async fn tool_profile_absent_uses_with_tools_as_default() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![final_answer("ok")]).await;
        let provider = provider(&url)
            .with_tools(registry(&["code_search"]))
            .with_tool_profile(BUG, registry(&["grafana_query"]));

        provider
            .invoke(&AgentConfig::new("hello"))
            .await
            .expect("step without profile");

        assert_eq!(
            tool_names(&requests.lock().expect("lock")[0]),
            names(&["code_search"])
        );
    })
    .await;
}

#[tokio::test]
async fn tool_profile_confines_tool_execution_to_the_profile() {
    within_timeout(async {
        let (url, requests) = spawn_llm(vec![
            tool_call_answer("grafana_query"),
            final_answer("done"),
        ])
        .await;
        let grafana = CountingTool::new("grafana_query");
        let grafana_calls = grafana.calls.clone();
        let provider = provider(&url)
            .with_tool_profile(SUGGESTION, registry(&["code_search"]))
            .with_tool_profile(BUG, ToolRegistry::new().register(grafana));

        let config: AgentConfig = AgentConfig::new("suggest").tool_profile(SUGGESTION).into();
        provider.invoke(&config).await.expect("suggestion step");

        assert_eq!(
            grafana_calls.load(Ordering::SeqCst),
            0,
            "a tool of another profile must never run"
        );
        let requests = requests.lock().expect("lock");
        let tool_message = requests[1]["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .find(|m| m["role"] == "tool")
            .cloned()
            .expect("the tool result is sent back to the model");
        assert_eq!(tool_message["content"], "Unknown tool: grafana_query");
    })
    .await;
}

#[tokio::test]
async fn tool_profile_runs_tools_of_the_selected_profile() {
    within_timeout(async {
        let (url, _requests) = spawn_llm(vec![
            tool_call_answer("grafana_query"),
            final_answer("done"),
        ])
        .await;
        let grafana = CountingTool::new("grafana_query");
        let grafana_calls = grafana.calls.clone();
        let provider = provider(&url)
            .with_tools(registry(&["code_search"]))
            .with_tool_profile(BUG, ToolRegistry::new().register(grafana));

        let config: AgentConfig = AgentConfig::new("triage").tool_profile(BUG).into();
        let output = provider.invoke(&config).await.expect("bug step");

        assert_eq!(grafana_calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.value, json!("done"));
    })
    .await;
}

#[tokio::test]
async fn tool_profile_is_logged_with_the_exposed_tools() {
    within_timeout(async {
        let (url, _requests) = spawn_llm(vec![final_answer("ok"), final_answer("ok")]).await;
        let provider = provider(&url)
            .with_tools(registry(&["code_search"]))
            .with_tool_profile(BUG, registry(&["code_search", "grafana_query"]));
        let sink = VecSink::new();

        let bug: AgentConfig = AgentConfig::new("triage").tool_profile(BUG).into();
        provider
            .invoke_with_logs(&bug, sink.clone())
            .await
            .expect("bug step");
        provider
            .invoke_with_logs(&AgentConfig::new("hello"), sink.clone())
            .await
            .expect("default step");

        assert_eq!(
            sink.lines(),
            vec![
                (
                    "system".to_string(),
                    "tool profile 'bug': 2 tools exposed (code_search, grafana_query)".to_string()
                ),
                (
                    "system".to_string(),
                    "default tool profile: 1 tool exposed (code_search)".to_string()
                ),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn tool_profile_absent_is_logged_as_no_tools() {
    within_timeout(async {
        let (url, _requests) = spawn_llm(vec![final_answer("ok")]).await;
        let provider = provider(&url).with_tool_profile(BUG, registry(&["grafana_query"]));
        let sink = VecSink::new();

        provider
            .invoke_with_logs(&AgentConfig::new("hello"), sink.clone())
            .await
            .expect("step without profile");

        assert_eq!(
            sink.lines(),
            vec![(
                "system".to_string(),
                "no tool profile: no tools exposed".to_string()
            )]
        );
    })
    .await;
}

#[tokio::test]
async fn tool_profile_unknown_is_not_logged_as_exposed() {
    within_timeout(async {
        let (url, _requests) = spawn_llm(Vec::new()).await;
        let provider = provider(&url).with_tool_profile(BUG, registry(&["grafana_query"]));
        let sink = VecSink::new();

        let config: AgentConfig = AgentConfig::new("triage").tool_profile(INCIDENT).into();
        let _ = provider
            .invoke_with_logs(&config, sink.clone())
            .await
            .expect_err("unknown profile");

        assert!(sink.lines().is_empty());
    })
    .await;
}

#[test]
#[should_panic(expected = "tool profile 'bug' already registered")]
fn tool_profile_registered_twice_panics() {
    let _ = provider("http://127.0.0.1:1")
        .with_tool_profile(BUG, ToolRegistry::new())
        .with_tool_profile(BUG, ToolRegistry::new());
}
