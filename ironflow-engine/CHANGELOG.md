# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [2.24.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.23.2...ironflow-engine-v2.24.0) - 2026-08-20

### Added

- #38 step-level retry with exponential backoff

## [2.23.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.22.0...ironflow-engine-v2.23.0) - 2026-08-17

### Added

- add WorkflowContext::input() for typed payload deserialization

## [2.22.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.21.0...ironflow-engine-v2.22.0) - 2026-08-17

### Added

- #33 add RunCreator trait and CreateRunOpts builder

## [2.21.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.20.1...ironflow-engine-v2.21.0) - 2026-08-15

### Added

- #9 add OpenTelemetry tracing and enhanced Prometheus observability

## [2.20.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.19.0...ironflow-engine-v2.20.0) - 2026-08-10

### Added

- #27 add conditions on EventTriggerRule (labels, expressions)


### Fixed

- #27 allow too_many_arguments on publish_run_status_changed

## [2.19.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.18.1...ironflow-engine-v2.19.0) - 2026-08-10

### Added

- #26 add on_error handlers for step and scope error handling


### Fixed

- #26 remove unused imports and clippy len_zero warning

- #26 fix CI pipeline (missing field in postgres test, rustfmt diffs)

## [2.18.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.17.5...ironflow-engine-v2.18.0) - 2026-08-08

### Added

- #23 enforce handler version compatibility on retry

## [2.17.4](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.17.3...ironflow-engine-v2.17.4) - 2026-08-03
## [2.17.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.17.1...ironflow-engine-v2.17.2) - 2026-07-28
## [2.17.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.16.6...ironflow-engine-v2.17.0) - 2026-07-27

### Added

- #17 trace run authorship (created_by)

## [2.16.6](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.16.5...ironflow-engine-v2.16.6) - 2026-07-27
## [2.16.5](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.16.4...ironflow-engine-v2.16.5) - 2026-07-27
## [2.16.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.15.13...ironflow-engine-v2.16.0) - 2026-06-01

### Added

- add CronSchedule newtype and schedule() to WorkflowHandler

## [2.15.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.15.2...ironflow-engine-v2.15.3) - 2026-05-01

### Fixed

- handle pod pre-Running failures, JoinError step tracking, and error preservation

## [2.15.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.15.0...ironflow-engine-v2.15.1) - 2026-04-28

### Fixed

- move OperationError import into test module to remove doc warning

- initialize MasterKey in get_secret test_state()

## [2.15.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.14.1...ironflow-engine-v2.15.0) - 2026-04-26

### Added

- add LogSink trait for real-time log streaming from providers

## [2.14.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.13.6...ironflow-engine-v2.14.0) - 2026-04-26

### Added

- capture raw response text in SchemaValidation errors

## [2.13.6](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.13.5...ironflow-engine-v2.13.6) - 2026-04-26

### Documentation

- add README.md to each workspace crate

## [2.13.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.13.2...ironflow-engine-v2.13.3) - 2026-04-24

### Fixed

- remove trailing blank line in agent executor tests

- make agent output consistent between text and structured modes

## [2.13.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.13.1...ironflow-engine-v2.13.2) - 2026-04-22
## [2.13.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.12.1...ironflow-engine-v2.13.0) - 2026-04-22

### Added

- add real-time log streaming for workflow runs

## [2.12.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.12.0...ironflow-engine-v2.12.1) - 2026-04-21
## [2.12.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.11.0...ironflow-engine-v2.12.0) - 2026-04-21

### Added

- add HMAC-SHA256 signature on outbound webhooks

## [2.11.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.10.0...ironflow-engine-v2.11.0) - 2026-04-21

### Added

- add encrypted secret store with unified Store trait and CRUD API

## [2.10.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.9.3...ironflow-engine-v2.10.0) - 2026-04-19

### Added

- add workflow categories with tree view in dashboard

## [2.9.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.8.1...ironflow-engine-v2.9.0) - 2026-04-16

### Added

- add strict_mcp_config flag to isolate agents from global MCP servers


### Fixed

- preserve cost, duration, and tokens on failed agent steps

## [2.8.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.7.6...ironflow-engine-v2.8.0) - 2026-04-15

### Added

- publish lifecycle events from worker-facing API routes

- CI all-features testing, OpenAPI snapshot check, and pre-commit hook

## [2.7.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.7.1...ironflow-engine-v2.7.2) - 2026-04-12

### Documentation

- document Claude CLI structured output known limitations and workarounds


### Fixed

- collapse nested if to satisfy clippy collapsible_if

- structured output deserialization, debug message persistence, and tools/schema typestate

## [2.7.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.7.0...ironflow-engine-v2.7.1) - 2026-04-12

### Changed

- extract shared retry/backoff utility and add MessageFormatter trait

## [2.7.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.6.0...ironflow-engine-v2.7.0) - 2026-04-12

### Added

- add explicit Event::RUN_FAILED for self-documenting failure subscriptions

## [2.6.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.5.3...ironflow-engine-v2.6.0) - 2026-04-11

### Added

- add SSH HostKeyPolicy and RunStore::update_run_returning

## [2.5.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.5.1...ironflow-engine-v2.5.2) - 2026-04-07

### Changed

- unify AgentStepConfig and AgentConfig into single type

## [2.5.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.4.0...ironflow-engine-v2.5.0) - 2026-04-04

### Added

- add AwaitingApproval and Rejected step statuses

- #9 add Prometheus metrics across API, engine, and worker

## [2.4.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.3.0...ironflow-engine-v2.4.0) - 2026-04-03

### Added

- #4 #3 approval gates and event-driven notifications


### Changed

- fix magic imports across all crates

## [2.3.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.2.0...ironflow-engine-v2.3.0) - 2026-04-03

### Added

- propagate output_schema through AgentStepConfig and AgentExecutor

## [2.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.1.2...ironflow-engine-v2.2.0) - 2026-04-03

### Added

- #2 add internal step-dependencies route, ci-pipeline demo, and review fixes

- #2 add ctx.parallel() and DAG dependency tracking


### Changed

- #2 remove static workflow execution


### Documentation

- #2 fix rustdoc warnings and update position field docs


### Fixed

- #2 remove unused imports in dag_execution tests

## [2.1.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.1.0...ironflow-engine-v2.1.1) - 2026-04-02

### Fixed

- remove tautological assertion on u64 duration_ms

- relax duration_ms assertion to >= 0 in http step test

## [2.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.0.1...ironflow-engine-v2.1.0) - 2026-04-02

### Added

- add Operation trait for user-defined custom step types

## [2.0.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-engine-v2.0.0...ironflow-engine-v2.0.1) - 2026-04-01

### Changed

- #1 make AgentConfig model field provider-agnostic


### Fixed

- replace httpbin.org with local axum server in HTTP executor tests

