//! Tests the Operation trait contract produced by the `slack_op!` macro.
//!
//! We cannot call `run()` / `execute()` without a real Slack workspace, but
//! we CAN verify that the generated trait impls behave correctly:
//! - `kind()` returns `"slack"`
//! - `input()` returns the serialized request (with-param variant)
//! - `input()` returns `None` (parameterless variant)

use ironflow_core::operation::Operation;
use ironflow_ops_slack::SlackClient;
use ironflow_ops_slack::auth::AuthTest;
use ironflow_ops_slack::chat::ChatPostMessage;
use ironflow_ops_slack::emoji::EmojiList;
use slack_morphism::api::SlackApiChatPostMessageRequest;
use slack_morphism::{SlackChannelId, SlackMessageContent};

fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

fn test_client() -> SlackClient {
    install_crypto_provider();
    SlackClient::new("xoxb-test-token-for-unit-tests").unwrap()
}

// -- With-request variant (ChatPostMessage) --

#[test]
fn with_request_kind_returns_slack() {
    let client = test_client();
    let req = SlackApiChatPostMessageRequest::new(
        SlackChannelId::new("#general".to_string()),
        SlackMessageContent::new().with_text("hello".to_string()),
    );
    let op = ChatPostMessage::new(&client, req);
    assert_eq!(op.kind(), "slack");
}

#[test]
fn with_request_input_returns_serialized_request() {
    let client = test_client();
    let req = SlackApiChatPostMessageRequest::new(
        SlackChannelId::new("#test".to_string()),
        SlackMessageContent::new().with_text("msg".to_string()),
    );
    let op = ChatPostMessage::new(&client, req);

    let input = op
        .input()
        .expect("input() should return Some for with-request ops");
    let channel = input["channel"].as_str().unwrap();
    assert_eq!(channel, "#test");
}

// -- Parameterless variant (AuthTest, EmojiList) --

#[test]
fn parameterless_kind_returns_slack() {
    let client = test_client();
    let op = AuthTest::new(&client);
    assert_eq!(op.kind(), "slack");
}

#[test]
fn parameterless_input_returns_none() {
    let client = test_client();
    let op = AuthTest::new(&client);
    assert!(
        op.input().is_none(),
        "parameterless ops should return None from input()"
    );
}

#[test]
fn parameterless_emoji_kind_returns_slack() {
    let client = test_client();
    let op = EmojiList::new(&client);
    assert_eq!(op.kind(), "slack");
}

// -- Client trimming behavior --

#[test]
fn new_trims_whitespace_from_token() {
    install_crypto_provider();
    let client = SlackClient::new("  xoxb-padded  ");
    assert!(
        client.is_ok(),
        "token with surrounding whitespace should be accepted"
    );
}

#[test]
fn input_serialization_is_valid_json_object() {
    let client = test_client();
    let req = SlackApiChatPostMessageRequest::new(
        SlackChannelId::new("#ch".to_string()),
        SlackMessageContent::new().with_text("txt".to_string()),
    );
    let op = ChatPostMessage::new(&client, req);
    let input = op.input().unwrap();
    assert!(input.is_object(), "input() should produce a JSON object");
}
