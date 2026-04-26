# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [2.14.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.13.4...ironflow-store-v2.14.0) - 2026-04-26

### Added

- add LogSink trait for real-time log streaming from providers

## [2.13.4](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.13.3...ironflow-store-v2.13.4) - 2026-04-26

### Fixed

- restrict has_steps filter to terminal runs only

## [2.13.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.13.2...ironflow-store-v2.13.3) - 2026-04-26

### Documentation

- add README.md to each workspace crate

## [2.13.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.13.1...ironflow-store-v2.13.2) - 2026-04-25

### Fixed

- #11 widen api_keys.key_prefix column from VARCHAR(12) to VARCHAR(16)

## [2.13.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.13.0...ironflow-store-v2.13.1) - 2026-04-22
## [2.13.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.12.0...ironflow-store-v2.13.0) - 2026-04-21

### Added

- add persistent audit log store for event compliance

## [2.12.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.11.0...ironflow-store-v2.12.0) - 2026-04-21

### Added

- add workflow handler versioning


### Fixed

- add missing handler_version field to NewRun struct literals

## [2.11.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.10.0...ironflow-store-v2.11.0) - 2026-04-21

### Added

- add encrypted secret store with unified Store trait and CRUD API


### Fixed

- align test and struct fields with unified Store and secret-store feature gate

## [2.10.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.9.0...ironflow-store-v2.10.0) - 2026-04-19

### Added

- live dashboard with filters and real-time run timeline

## [2.9.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.8.0...ironflow-store-v2.9.0) - 2026-04-15

### Added

- publish lifecycle events from worker-facing API routes

- OpenAPI documentation and TypeScript type generation


### Fixed

- clippy bool_assert_comparison and unused variable in tests

## [2.8.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.7.0...ironflow-store-v2.8.0) - 2026-04-14

### Added

- add user management CRUD, admin guards, and scope-based API key permissions

## [2.7.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.6.1...ironflow-store-v2.7.0) - 2026-04-14

### Added

- add has_steps filter on list runs and guard sign-up route when disabled

## [2.6.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.6.0...ironflow-store-v2.6.1) - 2026-04-13

### Fixed

- migrate sqlx compile-time macros to runtime queries in ironflow-store

## [2.6.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.5.1...ironflow-store-v2.6.0) - 2026-04-13

### Added

- add API keys management and MCP server

## [2.5.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.5.0...ironflow-store-v2.5.1) - 2026-04-12

### Fixed

- structured output deserialization, debug message persistence, and tools/schema typestate

## [2.5.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.4.1...ironflow-store-v2.5.0) - 2026-04-11

### Added

- add SSH HostKeyPolicy and RunStore::update_run_returning

## [2.4.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.4.0...ironflow-store-v2.4.1) - 2026-04-09

### Fixed

- remove unused connect_timeout from PgConnectOptions

- correct lib_fsm table and column names in step_awaiting_approval migration

- replace PgPoolOptions::connect_timeout with PgConnectOptions::connect_timeout for SQLx 0.8

## [2.4.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.3.0...ironflow-store-v2.4.0) - 2026-04-04

### Added

- add PostgresStore pool config and startup config validation

## [2.3.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.2.0...ironflow-store-v2.3.0) - 2026-04-04

### Added

- add AwaitingApproval and Rejected step statuses

## [2.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.1.3...ironflow-store-v2.2.0) - 2026-04-03

### Added

- #4 #3 approval gates and event-driven notifications


### Changed

- fix magic imports across all crates

## [2.1.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.1.2...ironflow-store-v2.1.3) - 2026-04-03
## [2.1.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.1.1...ironflow-store-v2.1.2) - 2026-04-02

### Fixed

- #10 serialize StepKind::Custom as plain string instead of object

## [2.1.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.1.0...ironflow-store-v2.1.1) - 2026-04-02

### Fixed

- bind cost_usd as Decimal and read total_cost as Decimal in queries

- decode cost_usd as Decimal directly instead of String in row helpers

- add rust_decimal feature to sqlx for NUMERIC type support

- cast SUM(duration_ms) to BIGINT in get_stats query

## [2.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-store-v2.0.0...ironflow-store-v2.1.0) - 2026-04-02

### Added

- add Operation trait for user-defined custom step types

