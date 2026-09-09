use ironflow_core::operation::Operation;
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::alerts::{
    DeleteAlertmanagerConfig, GetAlertmanagerConfig, GetAlertmanagerConfigs, GetAlertmanagerStatus,
    GetAlerts, SetAlertmanagerConfig,
};
use ironflow_ops_mimir::cardinality::{GetLabelNamesCardinality, GetLabelValuesCardinality};
use ironflow_ops_mimir::compactor::{FinishBlockUpload, StartBlockUpload, UploadBlockFile};
use ironflow_ops_mimir::distributor::{
    GetDistributorRing, GetDistributorUserStats, GetHaTrackerStatus,
};
use ironflow_ops_mimir::ingest::{InfluxWrite, OtlpMetricsWrite, RemoteWrite};
use ironflow_ops_mimir::ingester::{
    CancelPartitionDownscale, CancelShutdown, Flush, GetIngesterRing, GetIngesterTenants,
    PreparePartitionDownscale, PrepareShutdown, Shutdown,
};
use ironflow_ops_mimir::query::{FormatQuery, QueryExemplars, QueryInstant, QueryRange};
use ironflow_ops_mimir::rules::{
    CreateRuleGroup, DeleteRuleGroup, DeleteRuleNamespace, GetAllTenantRules, GetRuleGroup,
    GetRules, GetRulesByNamespace,
};
use ironflow_ops_mimir::series::{
    GetActiveSeries, GetLabelValues, GetLabels, GetMetadata, GetSeries,
};
use ironflow_ops_mimir::status::{
    GetBuildInfo, GetConfig, GetConfigDiff, GetMetrics, GetReady, GetServices, GetUserLimits,
};
use reqwest::Client;
use serde_json::json;

fn mimir() -> MimirClient {
    MimirClient::new("http://localhost:8080", Client::new())
}

#[test]
fn all_operations_return_kind_mimir() {
    let c = mimir();
    let ops: Vec<Box<dyn Operation>> = vec![
        // query (4)
        Box::new(QueryInstant::new(c.clone(), "up")),
        Box::new(QueryRange::new(c.clone(), "up", "s", "e")),
        Box::new(QueryExemplars::new(c.clone(), "up", "s", "e")),
        Box::new(FormatQuery::new(c.clone(), "up")),
        // series (5)
        Box::new(GetSeries::new(c.clone(), vec!["up".into()])),
        Box::new(GetLabels::new(c.clone())),
        Box::new(GetLabelValues::new(c.clone(), "job")),
        Box::new(GetMetadata::new(c.clone())),
        Box::new(GetActiveSeries::new(c.clone(), "up")),
        // cardinality (2)
        Box::new(GetLabelNamesCardinality::new(c.clone())),
        Box::new(GetLabelValuesCardinality::new(c.clone(), "__name__")),
        // ingest (3)
        Box::new(RemoteWrite::new(c.clone(), vec![])),
        Box::new(OtlpMetricsWrite::new(c.clone(), json!({}))),
        Box::new(InfluxWrite::new(c.clone(), "cpu v=0.5")),
        // rules (7)
        Box::new(GetRules::new(c.clone())),
        Box::new(GetRulesByNamespace::new(c.clone(), "ns")),
        Box::new(GetRuleGroup::new(c.clone(), "ns", "g")),
        Box::new(CreateRuleGroup::new(c.clone(), "ns", "body")),
        Box::new(DeleteRuleGroup::new(c.clone(), "ns", "g")),
        Box::new(DeleteRuleNamespace::new(c.clone(), "ns")),
        Box::new(GetAllTenantRules::new(c.clone())),
        // alerts (6)
        Box::new(GetAlerts::new(c.clone())),
        Box::new(GetAlertmanagerConfig::new(c.clone())),
        Box::new(SetAlertmanagerConfig::new(c.clone(), "cfg")),
        Box::new(DeleteAlertmanagerConfig::new(c.clone())),
        Box::new(GetAlertmanagerStatus::new(c.clone())),
        Box::new(GetAlertmanagerConfigs::new(c.clone())),
        // distributor (3)
        Box::new(GetDistributorRing::new(c.clone())),
        Box::new(GetDistributorUserStats::new(c.clone())),
        Box::new(GetHaTrackerStatus::new(c.clone())),
        // ingester (8)
        Box::new(Flush::new(c.clone())),
        Box::new(PrepareShutdown::new(c.clone())),
        Box::new(CancelShutdown::new(c.clone())),
        Box::new(Shutdown::new(c.clone())),
        Box::new(PreparePartitionDownscale::new(c.clone())),
        Box::new(CancelPartitionDownscale::new(c.clone())),
        Box::new(GetIngesterRing::new(c.clone())),
        Box::new(GetIngesterTenants::new(c.clone())),
        // store_gateway (4)
        Box::new(ironflow_ops_mimir::store_gateway::GetRing::new(c.clone())),
        Box::new(ironflow_ops_mimir::store_gateway::GetTenants::new(
            c.clone(),
        )),
        Box::new(ironflow_ops_mimir::store_gateway::GetTenantBlocks::new(
            c.clone(),
            "tenant-1",
        )),
        Box::new(ironflow_ops_mimir::store_gateway::PrepareShutdown::new(
            c.clone(),
        )),
        // compactor (5)
        Box::new(ironflow_ops_mimir::compactor::GetRing::new(c.clone())),
        Box::new(StartBlockUpload::new(c.clone(), "block1")),
        Box::new(UploadBlockFile::new(c.clone(), "block1", "idx", vec![])),
        Box::new(FinishBlockUpload::new(c.clone(), "block1")),
        Box::new(ironflow_ops_mimir::compactor::GetTenants::new(c.clone())),
        // status (7)
        Box::new(GetReady::new(c.clone())),
        Box::new(GetMetrics::new(c.clone())),
        Box::new(GetConfig::new(c.clone())),
        Box::new(GetConfigDiff::new(c.clone())),
        Box::new(GetServices::new(c.clone())),
        Box::new(GetBuildInfo::new(c.clone())),
        Box::new(GetUserLimits::new(c.clone())),
    ];

    assert_eq!(ops.len(), 54, "expected 54 operations, got {}", ops.len());

    for op in &ops {
        assert_eq!(
            op.kind(),
            "mimir",
            "operation {:?} returned wrong kind",
            std::any::type_name_of_val(&**op)
        );
    }
}

#[test]
fn operations_return_structured_input() {
    let c = mimir();

    let instant = QueryInstant::new(c.clone(), "up == 1");
    let input = instant.input().unwrap();
    assert_eq!(input["operation"], "query_instant");
    assert_eq!(input["query"], "up == 1");

    let range = QueryRange::new(c.clone(), "up", "s", "e");
    let input = range.input().unwrap();
    assert_eq!(input["operation"], "query_range");
    assert_eq!(input["start"], "s");
    assert_eq!(input["end"], "e");

    let exemplars = QueryExemplars::new(c.clone(), "http_requests_total", "s", "e");
    let input = exemplars.input().unwrap();
    assert_eq!(input["operation"], "query_exemplars");

    let format = FormatQuery::new(c.clone(), "q");
    let input = format.input().unwrap();
    assert_eq!(input["operation"], "format_query");
    assert_eq!(input["query"], "q");

    let labels = GetLabels::new(c.clone());
    let input = labels.input().unwrap();
    assert_eq!(input["operation"], "get_labels");

    let rules = GetRules::new(c.clone());
    let input = rules.input().unwrap();
    assert_eq!(input["operation"], "get_rules");

    let remote_write = RemoteWrite::new(c.clone(), vec![1, 2, 3]);
    let input = remote_write.input().unwrap();
    assert_eq!(input["operation"], "remote_write");
    assert_eq!(input["payload_size"], 3);

    let create_rule = CreateRuleGroup::new(c.clone(), "ns", "body");
    let input = create_rule.input().unwrap();
    assert_eq!(input["operation"], "create_rule_group");
    assert_eq!(input["namespace"], "ns");
}
