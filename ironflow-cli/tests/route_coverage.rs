//! Anti-drift test: every API route must be reachable from the CLI.
//!
//! The CLI once lagged four whole domains behind the API (secrets, API keys,
//! users, audit logs). This test makes that impossible to repeat silently:
//! adding a route to `openapi.json` without either a CLI command or a written
//! exemption fails the build.
//!
//! The mapping is not taken at face value. Every `Coverage::Command` entry is
//! fed to the real clap parser, so a route cannot be declared covered by a
//! command that does not exist.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use clap::Parser;
use serde_json::{Value, from_str};

use ironflow_cli::cli::Cli;

/// The spec the SDK is generated from, and therefore the CLI's ground truth.
const OPENAPI: &str = include_str!("../../ironflow-sdk/openapi.json");

/// How a route is reachable -- or why it deliberately is not.
#[derive(Debug, Clone, Copy)]
enum Coverage {
    /// A full argument vector that must parse, proving the command exists.
    Command(&'static [&'static str]),
    /// A route with no CLI counterpart, and the reason why.
    Exempt(&'static str),
}

const UUID: &str = "01234567-89ab-cdef-0123-456789abcdef";

/// Every route of the spec, mapped to the command that drives it.
///
/// Adding a route to the API without touching this table fails
/// [`every_api_route_is_reachable_from_the_cli`].
const COVERAGE: &[(&str, &str, Coverage)] = &[
    // ── Runs ──
    ("GET", "/api/v1/runs", Coverage::Command(&["run", "list"])),
    (
        "POST",
        "/api/v1/runs",
        Coverage::Command(&["run", "create", "deploy"]),
    ),
    (
        "GET",
        "/api/v1/runs/{id}",
        Coverage::Command(&["run", "get", UUID]),
    ),
    (
        "POST",
        "/api/v1/runs/{id}/cancel",
        Coverage::Command(&["run", "cancel", UUID]),
    ),
    (
        "POST",
        "/api/v1/runs/{id}/approve",
        Coverage::Command(&["run", "approve", UUID]),
    ),
    (
        "POST",
        "/api/v1/runs/{id}/reject",
        Coverage::Command(&["run", "reject", UUID]),
    ),
    (
        "POST",
        "/api/v1/runs/{id}/retry",
        Coverage::Command(&["run", "retry", UUID]),
    ),
    // ── Workflows ──
    (
        "GET",
        "/api/v1/workflows",
        Coverage::Command(&["workflow", "list"]),
    ),
    (
        "GET",
        "/api/v1/workflows/{name}",
        Coverage::Command(&["workflow", "get", "deploy"]),
    ),
    // ── Stats ──
    ("GET", "/api/v1/stats", Coverage::Command(&["stats"])),
    // ── Secrets ──
    (
        "GET",
        "/api/v1/secrets",
        Coverage::Command(&["secret", "list"]),
    ),
    (
        "POST",
        "/api/v1/secrets",
        Coverage::Command(&["secret", "set", "db/password", "value"]),
    ),
    (
        "PUT",
        "/api/v1/secrets/{key}",
        Coverage::Command(&["secret", "update", "db/password", "value"]),
    ),
    (
        "DELETE",
        "/api/v1/secrets/{key}",
        Coverage::Command(&["secret", "delete", "db/password", "--yes"]),
    ),
    (
        "POST",
        "/api/v1/secrets/rotate",
        Coverage::Command(&["secret", "rotate"]),
    ),
    (
        "GET",
        "/api/v1/secrets/key-versions",
        Coverage::Command(&["secret", "key-status"]),
    ),
    // ── API keys ──
    (
        "GET",
        "/api/v1/api-keys",
        Coverage::Command(&["api-key", "list"]),
    ),
    (
        "POST",
        "/api/v1/api-keys",
        Coverage::Command(&["api-key", "create", "ci", "--scope", "runs_read"]),
    ),
    (
        "GET",
        "/api/v1/api-keys/scopes",
        Coverage::Command(&["api-key", "scopes"]),
    ),
    (
        "DELETE",
        "/api/v1/api-keys/{id}",
        Coverage::Command(&["api-key", "delete", UUID, "--yes"]),
    ),
    // ── Users ──
    ("GET", "/api/v1/users", Coverage::Command(&["user", "list"])),
    (
        "POST",
        "/api/v1/users",
        Coverage::Command(&[
            "user",
            "create",
            "alice",
            "--email",
            "alice@example.com",
            "--password",
            "hunter2hunter2",
        ]),
    ),
    (
        "DELETE",
        "/api/v1/users/{id}",
        Coverage::Command(&["user", "delete", UUID, "--yes"]),
    ),
    (
        "PATCH",
        "/api/v1/users/{id}/role",
        Coverage::Command(&["user", "set-role", UUID, "--admin"]),
    ),
    // ── Audit logs ──
    (
        "GET",
        "/api/v1/audit-logs",
        Coverage::Command(&["audit-log", "list"]),
    ),
    // ── Deliberately out of the CLI's reach ──
    (
        "GET",
        "/api/v1/health-check",
        Coverage::Exempt("infrastructure probe, not an administration action"),
    ),
    (
        "POST",
        "/api/v1/auth/sign-in",
        Coverage::Exempt("the CLI authenticates with an API key, not a browser session"),
    ),
    (
        "POST",
        "/api/v1/auth/sign-out",
        Coverage::Exempt("session-only; there is no CLI session to end"),
    ),
    (
        "POST",
        "/api/v1/auth/refresh",
        Coverage::Exempt("session-only; API keys do not expire mid-command"),
    ),
    (
        "POST",
        "/api/v1/auth/sign-up",
        Coverage::Exempt(
            "public self-registration belongs to the dashboard; `user create` is the CLI path",
        ),
    ),
    (
        "GET",
        "/api/v1/auth/me",
        Coverage::Exempt("introspects the caller's session, which the CLI does not hold"),
    ),
    (
        "GET",
        "/api/v1/runs/{id}/steps/{step_id}/artifacts/{name}",
        Coverage::Exempt(
            "streams a raw file body, which the JSON-shaped SDK client cannot surface; \
             `run get` already lists the artifacts and their sizes",
        ),
    ),
];

/// Routes of the spec, parsed once for the whole test binary.
static SPEC_ROUTES: LazyLock<BTreeSet<(String, String)>> = LazyLock::new(spec_routes);

/// Routes declared by [`COVERAGE`], collected once for the whole test binary.
static DECLARED_ROUTES: LazyLock<BTreeSet<(String, String)>> = LazyLock::new(declared_routes);

/// Collect `(METHOD, path)` for every operation in the spec.
fn spec_routes() -> BTreeSet<(String, String)> {
    let spec: Value = from_str(OPENAPI).expect("openapi.json must be valid JSON");
    let paths = spec["paths"]
        .as_object()
        .expect("openapi.json must have a paths object");

    let mut routes = BTreeSet::new();
    for (path, operations) in paths {
        let operations = operations
            .as_object()
            .expect("each path must map to an object of operations");
        for method in operations.keys() {
            let upper = method.to_ascii_uppercase();
            if matches!(upper.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
                routes.insert((upper, path.clone()));
            }
        }
    }
    routes
}

/// Collect `(METHOD, path)` for every entry of [`COVERAGE`].
fn declared_routes() -> BTreeSet<(String, String)> {
    COVERAGE
        .iter()
        .map(|(method, path, _)| ((*method).to_string(), (*path).to_string()))
        .collect()
}

#[test]
fn every_api_route_is_reachable_from_the_cli() {
    let missing: Vec<_> = SPEC_ROUTES.difference(&DECLARED_ROUTES).cloned().collect();

    assert!(
        missing.is_empty(),
        "these API routes have no CLI command and no exemption: {missing:?}\n\
         add a command in ironflow-cli/src/commands/, then declare it in \
         tests/route_coverage.rs (or add a Coverage::Exempt with a reason)"
    );
}

#[test]
fn the_coverage_table_has_no_dead_entries() {
    let stale: Vec<_> = DECLARED_ROUTES.difference(&SPEC_ROUTES).cloned().collect();

    assert!(
        stale.is_empty(),
        "these entries of COVERAGE no longer match any API route: {stale:?}\n\
         remove them from tests/route_coverage.rs"
    );
}

#[test]
fn every_declared_command_really_parses() {
    for (method, path, coverage) in COVERAGE {
        let Coverage::Command(args) = coverage else {
            continue;
        };

        let mut argv = vec!["ironflow-cli"];
        argv.extend_from_slice(args);

        let parsed = Cli::try_parse_from(&argv);
        assert!(
            parsed.is_ok(),
            "{method} {path} claims to be covered by `{}`, which does not parse: {}",
            args.join(" "),
            parsed.unwrap_err()
        );
    }
}

#[test]
fn every_exemption_states_a_reason() {
    for (method, path, coverage) in COVERAGE {
        if let Coverage::Exempt(reason) = coverage {
            assert!(
                reason.len() > 20,
                "{method} {path} is exempt with a reason too vague to review: {reason:?}"
            );
        }
    }
}

#[test]
fn the_coverage_table_has_no_duplicate_routes() {
    assert_eq!(
        DECLARED_ROUTES.len(),
        COVERAGE.len(),
        "COVERAGE lists the same route twice"
    );
}
