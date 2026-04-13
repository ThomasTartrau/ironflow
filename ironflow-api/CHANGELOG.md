# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
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

