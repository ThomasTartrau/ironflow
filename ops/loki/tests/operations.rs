use ironflow_core::operation::Operation;
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::delete::{CancelDeleteRequest, CreateDeleteRequest, ListDeleteRequests};
use ironflow_ops_loki::format::FormatQuery;
use ironflow_ops_loki::index::{GetIndexStats, GetIndexVolume, GetIndexVolumeRange};
use ironflow_ops_loki::ingest::{PushLogs, PushLogsOtlp};
use ironflow_ops_loki::ingester::{CancelShutdown, Flush, PrepareShutdown, Shutdown};
use ironflow_ops_loki::labels::{GetLabelValues, GetLabels, GetSeries};
use ironflow_ops_loki::patterns::{DetectFields, DetectPatterns, GetDetectedFieldValues};
use ironflow_ops_loki::query::{QueryInstant, QueryRange};
use ironflow_ops_loki::rings::{
    GetCompactorRing, GetDistributorRing, GetIndexGatewayRing, GetRulerRing,
};
use ironflow_ops_loki::rules::{
    CreateRuleGroup, DeleteRuleGroup, DeleteRuleNamespace, GetAlerts, GetRuleGroup, GetRules,
    GetRulesByNamespace,
};
use ironflow_ops_loki::status::{GetConfig, GetLogLevel, GetMetrics, GetReady, SetLogLevel};
use reqwest::Client;
use serde_json::json;

fn loki() -> LokiClient {
    LokiClient::new("http://localhost:3100", Client::new())
}

#[test]
fn all_operations_return_kind_loki() {
    let c = loki();
    let ops: Vec<Box<dyn Operation>> = vec![
        Box::new(QueryInstant::new(c.clone(), "q")),
        Box::new(QueryRange::new(c.clone(), "q", "s", "e")),
        Box::new(GetLabels::new(c.clone())),
        Box::new(GetLabelValues::new(c.clone(), "n")),
        Box::new(GetSeries::new(c.clone(), vec!["m".into()])),
        Box::new(PushLogs::new(c.clone(), json!({}))),
        Box::new(PushLogsOtlp::new(c.clone(), json!({}))),
        Box::new(GetIndexStats::new(c.clone())),
        Box::new(GetIndexVolume::new(c.clone(), "q")),
        Box::new(GetIndexVolumeRange::new(c.clone(), "q", "s", "e")),
        Box::new(DetectPatterns::new(c.clone(), "q")),
        Box::new(DetectFields::new(c.clone(), "q")),
        Box::new(GetDetectedFieldValues::new(c.clone(), "f", "q")),
        Box::new(GetRules::new(c.clone())),
        Box::new(GetRulesByNamespace::new(c.clone(), "ns")),
        Box::new(GetRuleGroup::new(c.clone(), "ns", "g")),
        Box::new(CreateRuleGroup::new(c.clone(), "ns", "body")),
        Box::new(DeleteRuleGroup::new(c.clone(), "ns", "g")),
        Box::new(DeleteRuleNamespace::new(c.clone(), "ns")),
        Box::new(GetAlerts::new(c.clone())),
        Box::new(CreateDeleteRequest::new(c.clone(), "q", "s", "e")),
        Box::new(ListDeleteRequests::new(c.clone())),
        Box::new(CancelDeleteRequest::new(c.clone(), "id")),
        Box::new(FormatQuery::new(c.clone(), "q")),
        Box::new(GetReady::new(c.clone())),
        Box::new(GetLogLevel::new(c.clone())),
        Box::new(SetLogLevel::new(c.clone(), "debug")),
        Box::new(GetMetrics::new(c.clone())),
        Box::new(GetConfig::new(c.clone())),
        Box::new(GetDistributorRing::new(c.clone())),
        Box::new(GetIndexGatewayRing::new(c.clone())),
        Box::new(GetRulerRing::new(c.clone())),
        Box::new(GetCompactorRing::new(c.clone())),
        Box::new(Flush::new(c.clone())),
        Box::new(PrepareShutdown::new(c.clone())),
        Box::new(CancelShutdown::new(c.clone())),
        Box::new(Shutdown::new(c.clone())),
    ];

    for op in &ops {
        assert_eq!(
            op.kind(),
            "loki",
            "operation {:?} returned wrong kind",
            std::any::type_name_of_val(&**op)
        );
    }
}

#[test]
fn operations_return_structured_input() {
    let c = loki();

    let instant = QueryInstant::new(c.clone(), r#"{job="test"}"#);
    let input = instant.input().unwrap();
    assert_eq!(input["operation"], "query_instant");
    assert_eq!(input["query"], r#"{job="test"}"#);

    let range = QueryRange::new(c.clone(), r#"{job="test"}"#, "s", "e");
    let input = range.input().unwrap();
    assert_eq!(input["operation"], "query_range");
    assert_eq!(input["start"], "s");
    assert_eq!(input["end"], "e");

    let push = PushLogs::new(c.clone(), json!({}));
    let input = push.input().unwrap();
    assert_eq!(input["operation"], "push_logs");

    let labels = GetLabels::new(c.clone());
    let input = labels.input().unwrap();
    assert_eq!(input["operation"], "get_labels");

    let rules = GetRules::new(c.clone());
    let input = rules.input().unwrap();
    assert_eq!(input["operation"], "get_rules");

    let format = FormatQuery::new(c.clone(), "q");
    let input = format.input().unwrap();
    assert_eq!(input["operation"], "format_query");
    assert_eq!(input["query"], "q");
}
