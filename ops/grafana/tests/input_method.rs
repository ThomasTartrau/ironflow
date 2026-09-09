use ironflow_core::operation::Operation;
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::annotations::{AnnotationCreate, AnnotationGetById, AnnotationUpdate};
use ironflow_ops_grafana::dashboards::{
    DashboardCreate, DashboardDelete, DashboardGet, DashboardSearch,
};
use ironflow_ops_grafana::folders::{FolderCreate, FolderDelete, FolderGet, FolderUpdate};
use ironflow_ops_grafana::playlists::{PlaylistCreate, PlaylistGet};
use ironflow_ops_grafana::service_accounts::{ServiceAccountCreate, ServiceAccountGet};
use ironflow_ops_grafana::users::{UserGetById, UserSearch, UserUpdate};
use serde_json::json;

fn client() -> GrafanaClient {
    GrafanaClient::new("tok", "http://localhost:3000").unwrap()
}

#[test]
fn dashboard_get_input_contains_uid() {
    let c = client();
    let op = DashboardGet::new(&c, "my-uid");
    let input = op.input().unwrap();
    assert_eq!(input["uid"], "my-uid");
}

#[test]
fn dashboard_create_input_contains_body() {
    let c = client();
    let body = json!({"dashboard": {"title": "Test"}, "overwrite": false});
    let op = DashboardCreate::new(&c, body.clone());
    assert_eq!(op.input().unwrap(), body);
}

#[test]
fn dashboard_delete_input_contains_uid() {
    let c = client();
    let op = DashboardDelete::new(&c, "del-uid");
    let input = op.input().unwrap();
    assert_eq!(input["uid"], "del-uid");
}

#[test]
fn dashboard_search_input_reflects_params() {
    let c = client();
    let op = DashboardSearch::new(&c, Some("prod"), Some("deploy"));
    let input = op.input().unwrap();
    assert_eq!(input["query"], "prod");
    assert_eq!(input["tag"], "deploy");
}

#[test]
fn folder_create_input_contains_title_and_uid() {
    let c = client();
    let op = FolderCreate::new(&c, "My Folder", Some("f-uid"));
    let input = op.input().unwrap();
    assert_eq!(input["title"], "My Folder");
    assert_eq!(input["uid"], "f-uid");
}

#[test]
fn folder_create_input_uid_is_null_when_none() {
    let c = client();
    let op = FolderCreate::new(&c, "No UID", None);
    let input = op.input().unwrap();
    assert_eq!(input["title"], "No UID");
    assert!(input["uid"].is_null());
}

#[test]
fn folder_get_input_contains_uid() {
    let c = client();
    let op = FolderGet::new(&c, "folder-uid");
    let input = op.input().unwrap();
    assert_eq!(input["uid"], "folder-uid");
}

#[test]
fn folder_update_input_contains_uid_and_title() {
    let c = client();
    let op = FolderUpdate::new(&c, "uid-1", "New Title", 3);
    let input = op.input().unwrap();
    assert_eq!(input["uid"], "uid-1");
    assert_eq!(input["title"], "New Title");
}

#[test]
fn folder_delete_input_contains_uid() {
    let c = client();
    let op = FolderDelete::new(&c, "del-uid");
    let input = op.input().unwrap();
    assert_eq!(input["uid"], "del-uid");
}

#[test]
fn annotation_create_input_is_body() {
    let c = client();
    let body = json!({"text": "deploy", "dashboardId": 1});
    let op = AnnotationCreate::new(&c, body.clone());
    assert_eq!(op.input().unwrap(), body);
}

#[test]
fn annotation_get_by_id_input_contains_id() {
    let c = client();
    let op = AnnotationGetById::new(&c, 42);
    let input = op.input().unwrap();
    assert_eq!(input["id"], 42);
}

#[test]
fn annotation_update_input_contains_id_and_body() {
    let c = client();
    let body = json!({"text": "updated"});
    let op = AnnotationUpdate::new(&c, 5, body.clone());
    let input = op.input().unwrap();
    assert_eq!(input["id"], 5);
    assert_eq!(input["body"], body);
}

#[test]
fn user_get_by_id_input_contains_id() {
    let c = client();
    let op = UserGetById::new(&c, 99);
    let input = op.input().unwrap();
    assert_eq!(input["id"], 99);
}

#[test]
fn user_search_input_contains_query() {
    let c = client();
    let op = UserSearch::new(&c, "alice");
    let input = op.input().unwrap();
    assert_eq!(input["query"], "alice");
}

#[test]
fn user_update_input_contains_id_and_body() {
    let c = client();
    let body = json!({"name": "Updated"});
    let op = UserUpdate::new(&c, 42, body.clone());
    let input = op.input().unwrap();
    assert_eq!(input["id"], 42);
    assert_eq!(input["body"], body);
}

#[test]
fn playlist_get_input_contains_uid() {
    let c = client();
    let op = PlaylistGet::new(&c, "pl-uid");
    let input = op.input().unwrap();
    assert_eq!(input["uid"], "pl-uid");
}

#[test]
fn playlist_create_input_is_body() {
    let c = client();
    let body = json!({"name": "New", "interval": "5m"});
    let op = PlaylistCreate::new(&c, body.clone());
    assert_eq!(op.input().unwrap(), body);
}

#[test]
fn service_account_get_input_contains_id() {
    let c = client();
    let op = ServiceAccountGet::new(&c, 10);
    let input = op.input().unwrap();
    assert_eq!(input["id"], 10);
}

#[test]
fn service_account_create_input_is_body() {
    let c = client();
    let body = json!({"name": "sa", "role": "Viewer"});
    let op = ServiceAccountCreate::new(&c, body.clone());
    assert_eq!(op.input().unwrap(), body);
}
