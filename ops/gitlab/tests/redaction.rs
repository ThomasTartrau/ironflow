use std::borrow::Cow;
use std::sync::Arc;

use gitlab::GitlabBuilder;
use gitlab::api::common::NameOrId;
use gitlab::api::endpoint_prelude::{BodyError, Endpoint, JsonParams, Method};
use gitlab::api::projects::Project;
use gitlab::api::projects::merge_requests::EditMergeRequest;
use gitlab::api::projects::variables::CreateProjectVariable;
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_gitlab::GitLab;
use ironflow_ops_gitlab::endpoints::merge_requests::CreateMergeRequestDiscussionNote;
use serde_json::json;
use wiremock::matchers::{body_json, body_string, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CALLER_SECRET: &str = "caller-secret+value/with=reserved";
const GLPAT: &str = "glpat-abcdefghij0123456789";

async fn gitlab(server: &MockServer) -> GitLab {
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1})))
        .mount(server)
        .await;

    let client = GitlabBuilder::new(server.address().to_string(), "token")
        .insecure()
        .build_async()
        .await
        .unwrap();
    GitLab::from(client).with_secrets([CALLER_SECRET])
}

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

fn note(body: &str) -> CreateMergeRequestDiscussionNote {
    CreateMergeRequestDiscussionNote {
        project: NameOrId::from(42),
        merge_request: 7,
        discussion_id: "abcd1234".to_string(),
        body: body.to_string(),
    }
}

const NOTE_PATH: &str = "/api/v4/projects/42/merge_requests/7/discussions/abcd1234/notes";

/// An endpoint sending a JSON body, like the `gitlab` crate's JsonParams endpoints.
struct JsonNote {
    body: String,
}

impl Endpoint for JsonNote {
    fn method(&self) -> Method {
        Method::POST
    }

    fn endpoint(&self) -> Cow<'static, str> {
        "projects/42/json-notes".into()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, BodyError> {
        JsonParams::into_body(&json!({"note": {"body": self.body, "lines": [self.body]}}))
    }
}

/// An endpoint sending a plain text body (a raw file upload).
struct TextUpload {
    content: String,
}

impl Endpoint for TextUpload {
    fn method(&self) -> Method {
        Method::PUT
    }

    fn endpoint(&self) -> Cow<'static, str> {
        "projects/42/uploads/notes.txt".into()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, BodyError> {
        Ok(Some(("text/plain", self.content.clone().into_bytes())))
    }
}

#[tokio::test]
async fn note_with_a_caller_secret_reaches_gitlab_masked() {
    let server = MockServer::start().await;
    // The secret holds `+`, `/` and `=`, which the form encoding escapes: a
    // search on the encoded bytes would miss it.
    Mock::given(method("POST"))
        .and(path(NOTE_PATH))
        .and(body_string("body=leak%3A+%5BREDACTED%5D+end"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 1})))
        .expect(1)
        .mount(&server)
        .await;

    let gitlab = gitlab(&server).await;
    let op = gitlab.op(note(&format!("leak: {CALLER_SECRET} end")));

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["id"], 1);
}

#[tokio::test]
async fn merge_request_title_and_description_with_token_shapes_reach_gitlab_masked() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v4/projects/42/merge_requests/7"))
        .and(body_string(
            "title=fix+%5BREDACTED%5D&description=agent+printed+%5BREDACTED%5D",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"iid": 7})))
        .expect(1)
        .mount(&server)
        .await;

    let gitlab = gitlab(&server).await;
    let endpoint = EditMergeRequest::builder()
        .project(42)
        .merge_request(7)
        .title(format!("fix {GLPAT}"))
        .description("agent printed sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUv")
        .build()
        .unwrap();

    let result = gitlab.op(endpoint).execute(&ctx()).await.unwrap();
    assert_eq!(result["iid"], 7);
}

#[tokio::test]
async fn json_body_reaches_gitlab_masked_in_every_string() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/42/json-notes"))
        .and(body_json(json!({
            "note": {"body": "see [REDACTED]", "lines": ["see [REDACTED]"]}
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 2})))
        .expect(1)
        .mount(&server)
        .await;

    let gitlab = gitlab(&server).await;
    let op = gitlab.op(JsonNote {
        body: format!("see {GLPAT}"),
    });

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["id"], 2);
}

#[tokio::test]
async fn text_body_reaches_gitlab_masked() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v4/projects/42/uploads/notes.txt"))
        .and(body_string("line one\nkey [REDACTED]\n"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let gitlab = gitlab(&server).await;
    let op = gitlab.op(TextUpload {
        content: format!("line one\nkey {CALLER_SECRET}\n"),
    });

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["ok"], true);
}

#[tokio::test]
async fn clean_note_reaches_gitlab_unchanged() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(NOTE_PATH))
        .and(body_string("body=looks+good+%C3%A9t%C3%A9+%2B+50%25"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 3})))
        .expect(1)
        .mount(&server)
        .await;

    let gitlab = gitlab(&server).await;
    let result = gitlab
        .op(note("looks good été + 50%"))
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["id"], 3);
}

#[tokio::test]
async fn masks_secrets_tells_whether_the_body_carries_a_secret() {
    let server = MockServer::start().await;
    let gitlab = gitlab(&server).await;

    assert!(gitlab.op(note(GLPAT)).masks_secrets());
    assert!(gitlab.op(note(CALLER_SECRET)).masks_secrets());
    assert!(
        gitlab
            .op(JsonNote {
                body: GLPAT.to_string()
            })
            .masks_secrets()
    );
    assert!(!gitlab.op(note("nothing to hide")).masks_secrets());
}

#[tokio::test]
async fn ci_variable_written_without_masking_reaches_gitlab_as_is() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/42/variables"))
        .and(body_string(format!("key=DEPLOY_TOKEN&value={GLPAT}")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"key": "DEPLOY_TOKEN"})))
        .expect(1)
        .mount(&server)
        .await;

    let gitlab = gitlab(&server).await;
    let endpoint = CreateProjectVariable::builder()
        .project(42)
        .key("DEPLOY_TOKEN")
        .value(GLPAT)
        .build()
        .unwrap();
    let op = gitlab.op(endpoint).without_masking();

    assert!(!op.masks_secrets());
    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["key"], "DEPLOY_TOKEN");
}

#[tokio::test]
async fn step_input_records_that_a_secret_was_masked() {
    let server = MockServer::start().await;
    let gitlab = gitlab(&server).await;

    let masked = gitlab.op(note(GLPAT)).input().unwrap();
    assert_eq!(
        masked,
        json!({
            "endpoint": "projects/42/merge_requests/7/discussions/abcd1234/notes",
            "secrets_masked": true
        })
    );

    let clean = gitlab.op(note("nothing to hide")).input().unwrap();
    assert_eq!(
        clean,
        json!({"endpoint": "projects/42/merge_requests/7/discussions/abcd1234/notes"})
    );
}

#[tokio::test]
async fn masks_secrets_is_false_for_an_endpoint_without_body() {
    let server = MockServer::start().await;
    let gitlab = gitlab(&server).await;
    let endpoint = Project::builder().project(42).build().unwrap();

    assert!(!gitlab.op(endpoint).masks_secrets());
}
