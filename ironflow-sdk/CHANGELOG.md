# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [0.1.16](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.15...ironflow-sdk-v0.1.16) - 2026-08-27

### Added

- #49 add per-run SSE route for WorkflowEventBus

## [0.1.15](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.14...ironflow-sdk-v0.1.15) - 2026-08-27

### Added

- #45 granular step tracking with deterministic trace IDs

## [0.1.14](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.13...ironflow-sdk-v0.1.14) - 2026-08-21
## [0.1.13](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.12...ironflow-sdk-v0.1.13) - 2026-08-20

### Added

- #41 identity-aware rate limiting with per-API-key overrides

## [0.1.12](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.11...ironflow-sdk-v0.1.12) - 2026-08-15

### Added

- #30 add polling trigger for external sources

## [0.1.11](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.10...ironflow-sdk-v0.1.11) - 2026-08-10

### Added

- #25 allow_failure on steps to continue run with Warning status


### Fixed

- #27 regenerate OpenAPI snapshots with sign-up feature

- #25 regenerate openapi snapshots with full CI features

- #25 regenerate openapi snapshots and dashboard TS types

- #25 cargo fmt and openapi snapshot sync for Warning variant

## [0.1.10](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.9...ironflow-sdk-v0.1.10) - 2026-08-08

### Added

- #24 add Trigger trait with event and NATS trigger sources

## [0.1.9](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.8...ironflow-sdk-v0.1.9) - 2026-08-08

### Added

- #23 enforce handler version compatibility on retry

## [0.1.8](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.7...ironflow-sdk-v0.1.8) - 2026-08-03
## [0.1.7](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.6...ironflow-sdk-v0.1.7) - 2026-07-29

### Added

- #20 add secret, api-key, user and audit-log CLI commands

## [0.1.6](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.5...ironflow-sdk-v0.1.6) - 2026-07-28

### Added

- #14 add worker lease and reaper to recover orphaned runs

## [0.1.4](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.3...ironflow-sdk-v0.1.4) - 2026-07-27

### Added

- #17 trace run authorship (created_by)

## [0.1.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.2...ironflow-sdk-v0.1.3) - 2026-07-27
## [0.1.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-sdk-v0.1.1...ironflow-sdk-v0.1.2) - 2026-07-27

### Added

- #18 add per-run and per-workflow cost caps

## [0.1.0](https://gitlab.com/ThomasTartrau/ironflow/releases/tag/ironflow-sdk-v0.1.0) - 2026-06-02

### Added

- #12 add ironflow-sdk and ironflow-types crates

