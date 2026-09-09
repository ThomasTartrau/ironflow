use ironflow_core::operation::Operation;
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::admin::AdminGetHealth;
use ironflow_ops_grafana::alerting::AlertRuleList;
use ironflow_ops_grafana::annotations::AnnotationGetTags;
use ironflow_ops_grafana::dashboards::DashboardSearch;
use ironflow_ops_grafana::data_sources::DataSourceList;
use ironflow_ops_grafana::folders::FolderGet;
use ironflow_ops_grafana::organizations::OrgGetCurrent;
use ironflow_ops_grafana::other::GetFrontendSettings;
use ironflow_ops_grafana::playlists::PlaylistList;
use ironflow_ops_grafana::rbac::RbacGetRoles;
use ironflow_ops_grafana::service_accounts::ServiceAccountList;
use ironflow_ops_grafana::snapshots::SnapshotList;
use ironflow_ops_grafana::teams::TeamList;
use ironflow_ops_grafana::users::UserList;
use serde_json::json;

#[test]
fn all_operations_return_grafana_kind() {
    let client = GrafanaClient::new("tok", "http://localhost:3000").unwrap();

    assert_eq!(AdminGetHealth::new(&client).kind(), "grafana");
    assert_eq!(AlertRuleList::new(&client).kind(), "grafana");
    assert_eq!(AnnotationGetTags::new(&client).kind(), "grafana");
    assert_eq!(DashboardSearch::new(&client, None, None).kind(), "grafana");
    assert_eq!(DataSourceList::new(&client).kind(), "grafana");
    assert_eq!(FolderGet::new(&client, "uid").kind(), "grafana");
    assert_eq!(OrgGetCurrent::new(&client).kind(), "grafana");
    assert_eq!(GetFrontendSettings::new(&client).kind(), "grafana");
    assert_eq!(PlaylistList::new(&client).kind(), "grafana");
    assert_eq!(RbacGetRoles::new(&client).kind(), "grafana");
    assert_eq!(ServiceAccountList::new(&client).kind(), "grafana");
    assert_eq!(SnapshotList::new(&client).kind(), "grafana");
    assert_eq!(TeamList::new(&client).kind(), "grafana");
    assert_eq!(UserList::new(&client).kind(), "grafana");

    let body = json!({"dashboard": {}});
    assert_eq!(
        ironflow_ops_grafana::dashboards::DashboardCreate::new(&client, body).kind(),
        "grafana"
    );
}
