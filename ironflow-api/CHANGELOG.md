# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [2.30.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.30.0...ironflow-api-v2.30.1) - 2026-08-21
## [2.30.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.29.4...ironflow-api-v2.30.0) - 2026-08-20

### Added

- #41 identity-aware rate limiting with per-API-key overrides

## [2.29.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.28.0...ironflow-api-v2.29.0) - 2026-08-15

### Added

- #9 add OpenTelemetry tracing and enhanced Prometheus observability

## [2.28.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.27.0...ironflow-api-v2.28.0) - 2026-08-15

### Added

- #30 add polling trigger for external sources

## [2.27.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.26.0...ironflow-api-v2.27.0) - 2026-08-10

### Added

- #27 add conditions on EventTriggerRule (labels, expressions)

## [2.26.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.25.0...ironflow-api-v2.26.0) - 2026-08-10

### Added

- #26 add on_error handlers for step and scope error handling

## [2.25.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.24.0...ironflow-api-v2.25.0) - 2026-08-08

### Added

- #24 add Trigger trait with event and NATS trigger sources

## [2.24.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.23.2...ironflow-api-v2.24.0) - 2026-08-08

### Added

- #23 enforce handler version compatibility on retry


### Fixed

- mark retry force query param as optional in OpenAPI spec

## [2.23.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.23.0...ironflow-api-v2.23.1) - 2026-08-03
## [2.23.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.22.0...ironflow-api-v2.23.0) - 2026-07-29

### Added

- #21 version and rotate the secret encryption keys

## [2.22.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.21.1...ironflow-api-v2.22.0) - 2026-07-28

### Added

- #14 add worker lease and reaper to recover orphaned runs

## [2.21.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.20.8...ironflow-api-v2.21.0) - 2026-07-27

### Added

- #17 trace run authorship (created_by)

## [2.20.8](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.20.7...ironflow-api-v2.20.8) - 2026-07-27
## [2.20.7](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.20.6...ironflow-api-v2.20.7) - 2026-07-27
## [2.20.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.19.0...ironflow-api-v2.20.0) - 2026-06-02

### Added

- #12 add ironflow-sdk and ironflow-types crates


### Fixed

- #12 resolve unused import and clippy let_and_return warnings

## [2.19.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.18.13...ironflow-api-v2.19.0) - 2026-06-01

### Added

- add CronSchedule newtype and schedule() to WorkflowHandler

## [2.18.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.18.0...ironflow-api-v2.18.1) - 2026-04-28

### Fixed

- initialize MasterKey in get_secret test_state()

## [2.18.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.17.8...ironflow-api-v2.18.0) - 2026-04-26

### Added

- add LogSink trait for real-time log streaming from providers

## [2.17.8](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.17.7...ironflow-api-v2.17.8) - 2026-04-26

### Fixed

- restrict has_steps filter to terminal runs only

## [2.17.6](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.17.5...ironflow-api-v2.17.6) - 2026-04-26

### Documentation

- add README.md to each workspace crate

## [2.17.4](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.17.3...ironflow-api-v2.17.4) - 2026-04-25

### Fixed

- use --all-features for OpenAPI generation to include sign-up routes

- #11 widen api_keys.key_prefix column from VARCHAR(12) to VARCHAR(16)

## [2.17.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.17.2...ironflow-api-v2.17.3) - 2026-04-24

### Fixed

- remove duplicate HashMap import in push_logs tests

## [2.17.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.17.1...ironflow-api-v2.17.2) - 2026-04-22
## [2.17.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.17.0...ironflow-api-v2.17.1) - 2026-04-22
## [2.17.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.16.0...ironflow-api-v2.17.0) - 2026-04-22

### Added

- add real-time log streaming for workflow runs


### Fixed

- use broadcast channel instead of event publisher for SSE log streaming

## [2.16.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.15.0...ironflow-api-v2.16.0) - 2026-04-21

### Added

- add persistent audit log store for event compliance

## [2.15.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.14.0...ironflow-api-v2.15.0) - 2026-04-21

### Added

- add workflow handler versioning


### Fixed

- use Option<String> for handler version instead of default string

- add missing handler_version field to NewRun struct literals

## [2.14.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.13.0...ironflow-api-v2.14.0) - 2026-04-21

### Added

- add encrypted secret store with unified Store trait and CRUD API


### Fixed

- align test and struct fields with unified Store and secret-store feature gate

## [2.13.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.12.0...ironflow-api-v2.13.0) - 2026-04-19

### Added

- add workflow categories with tree view in dashboard

## [2.12.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.11.5...ironflow-api-v2.12.0) - 2026-04-19

### Added

- live dashboard with filters and real-time run timeline

## [2.11.4](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.11.3...ironflow-api-v2.11.4) - 2026-04-19

### Added

- agent conversation trace timeline in dashboard

## [2.11.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.10.0...ironflow-api-v2.11.0) - 2026-04-15

### Added

- publish lifecycle events from worker-facing API routes

- CI all-features testing, OpenAPI snapshot check, and pre-commit hook

- OpenAPI documentation and TypeScript type generation

## [2.10.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.9.0...ironflow-api-v2.10.0) - 2026-04-14

### Added

- add user management CRUD, admin guards, and scope-based API key permissions

## [2.9.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.8.1...ironflow-api-v2.9.0) - 2026-04-14

### Added

- add has_steps filter on list runs and guard sign-up route when disabled

## [2.8.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.7.3...ironflow-api-v2.8.0) - 2026-04-13

### Added

- add API keys management and MCP server

## [2.7.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.7.2...ironflow-api-v2.7.3) - 2026-04-12

### Fixed

- structured output deserialization, debug message persistence, and tools/schema typestate

## [2.7.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.6.3...ironflow-api-v2.7.0) - 2026-04-11

### Added

- add rate limiting, worker timeout, poison pill guard, and graceful drain

## [2.6.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.5.0...ironflow-api-v2.6.0) - 2026-04-04

### Added

- add PostgresStore pool config and startup config validation

## [2.5.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.4.0...ironflow-api-v2.5.0) - 2026-04-04

### Added

- add AwaitingApproval and Rejected step statuses

- #9 add Prometheus metrics across API, engine, and worker

## [2.4.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.3.1...ironflow-api-v2.4.0) - 2026-04-03

### Added

- #4 #3 approval gates and event-driven notifications


### Changed

- fix magic imports across all crates

## [2.3.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.2.1...ironflow-api-v2.3.0) - 2026-04-03

### Added

- #2 add internal step-dependencies route, ci-pipeline demo, and review fixes

- #2 expose step dependencies in API response

## [2.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.1.1...ironflow-api-v2.2.0) - 2026-04-02

### Added

- resolve dashboard path for crates.io and monorepo

## [2.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-api-v2.0.1...ironflow-api-v2.1.0) - 2026-04-02

### Added

- add sign-up feature flag to conditionally compile registration

