//! Integration tests for ironflow-ops-helm.

use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_helm::HelmClient;
use ironflow_ops_helm::chart::{
    DependencyBuild, DependencyList, DependencyUpdate, Lint, Package, Pull, Push, Show,
    ShowSubcommand, Template,
};
use ironflow_ops_helm::plugin::{PluginInstall, PluginList, PluginUninstall, PluginUpdate};
use ironflow_ops_helm::registry::{RegistryLogin, RegistryLogout};
use ironflow_ops_helm::release::{
    Get, GetSubcommand, History, Install, List, Rollback, Status, Test, Uninstall, Upgrade,
};
use ironflow_ops_helm::repo::{
    RepoAdd, RepoIndex, RepoList, RepoRemove, RepoUpdate, SearchHub, SearchRepo,
};
use ironflow_ops_helm::util::{EnvInfo, Verify, Version};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

fn client() -> HelmClient {
    HelmClient::default()
}

#[tokio::test]
async fn all_ops_kind_helm() {
    let c = client();
    let ops: Vec<Box<dyn Operation>> = vec![
        Box::new(Install::new(c.clone(), "r", "c")),
        Box::new(Upgrade::new(c.clone(), "r", "c")),
        Box::new(Uninstall::new(c.clone(), "r")),
        Box::new(Rollback::new(c.clone(), "r", 1)),
        Box::new(List::new(c.clone())),
        Box::new(Status::new(c.clone(), "r")),
        Box::new(History::new(c.clone(), "r")),
        Box::new(Get::new(c.clone(), GetSubcommand::Values, "r")),
        Box::new(Test::new(c.clone(), "r")),
        Box::new(Template::new(c.clone(), "c")),
        Box::new(Lint::new(c.clone(), "p")),
        Box::new(Package::new(c.clone(), "p")),
        Box::new(Show::new(c.clone(), ShowSubcommand::Chart, "c")),
        Box::new(Pull::new(c.clone(), "c")),
        Box::new(Push::new(c.clone(), "c", "r")),
        Box::new(DependencyUpdate::new(c.clone(), "c")),
        Box::new(DependencyBuild::new(c.clone(), "c")),
        Box::new(DependencyList::new(c.clone(), "c")),
        Box::new(RepoAdd::new(c.clone(), "n", "u")),
        Box::new(RepoRemove::new(c.clone(), "n")),
        Box::new(RepoUpdate::new(c.clone())),
        Box::new(RepoList::new(c.clone())),
        Box::new(RepoIndex::new(c.clone(), "d")),
        Box::new(SearchRepo::new(c.clone(), "k")),
        Box::new(SearchHub::new(c.clone(), "k")),
        Box::new(RegistryLogin::new(c.clone(), "h", "u", "p")),
        Box::new(RegistryLogout::new(c.clone(), "h")),
        Box::new(PluginInstall::new(c.clone(), "s")),
        Box::new(PluginUninstall::new(c.clone(), "n")),
        Box::new(PluginList::new(c.clone())),
        Box::new(PluginUpdate::new(c.clone(), "n")),
        Box::new(Version::new(c.clone())),
        Box::new(EnvInfo::new(c.clone())),
        Box::new(Verify::new(c.clone(), "p")),
    ];

    for op in &ops {
        assert_eq!(op.kind(), "helm", "all ops must return kind=helm");
    }
}

#[tokio::test]
async fn registry_login_input_does_not_contain_password() {
    let op = RegistryLogin::new(client(), "host.io", "admin", "super-secret-password");
    let input = op.input().unwrap();
    let serialized = serde_json::to_string(&input).unwrap();
    assert!(
        !serialized.contains("super-secret-password"),
        "input() must not leak password, got: {serialized}"
    );
    assert!(serialized.contains("admin"), "username should be present");
}

#[tokio::test]
async fn registry_login_debug_does_not_leak_password() {
    let op = RegistryLogin::new(client(), "host.io", "admin", "super-secret-password");
    let debug = format!("{op:?}");
    assert!(
        !debug.contains("super-secret-password"),
        "Debug must redact password, got: {debug}"
    );
}

#[tokio::test]
#[ignore = "requires helm binary in PATH"]
async fn version_returns_output() {
    let op = Version::new(client());
    let result = op.run(&ctx()).await.unwrap();
    assert!(!result.version.is_empty(), "version should not be empty");
}

#[tokio::test]
async fn ops_provide_input() {
    let install = Install::new(client(), "my-release", "bitnami/nginx");
    let input = install.input().unwrap();
    assert_eq!(input["name"], "my-release");
    assert_eq!(input["chart"], "bitnami/nginx");
    assert_eq!(input["command"], "install");

    let rollback = Rollback::new(client(), "rel", 5);
    let input = rollback.input().unwrap();
    assert_eq!(input["revision"], 5);

    let get = Get::new(client(), GetSubcommand::Manifest, "rel");
    let input = get.input().unwrap();
    assert_eq!(input["subcommand"], "manifest");
}
