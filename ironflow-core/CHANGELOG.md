# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [2.7.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.6.0...ironflow-core-v2.7.0) - 2026-04-17

### Added

- add disallowed_tools field compatible with structured output

- add bare flag to isolate agents from auto-memory and CLAUDE.md


### Documentation

- note that --bare requires ANTHROPIC_API_KEY, not OAuth

## [2.6.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.5.1...ironflow-core-v2.6.0) - 2026-04-16

### Added

- add strict_mcp_config flag to isolate agents from global MCP servers


### Fixed

- preserve cost, duration, and tokens on failed agent steps

## [2.5.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.5.0...ironflow-core-v2.5.1) - 2026-04-16

### Fixed

- add random suffix to generate_pod_name to prevent collision on parallel calls

## [2.5.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.4.1...ironflow-core-v2.5.0) - 2026-04-15

### Added

- add image_pull_secret support to K8s providers


### Fixed

- clippy bool_assert_comparison and unused variable in tests

## [2.4.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.4.0...ironflow-core-v2.4.1) - 2026-04-12

### Documentation

- document Claude CLI structured output known limitations and workarounds


### Fixed

- panic on schema serialization failure to preserve typestate integrity

- replace map_or with is_none_or to satisfy clippy

- structured output deserialization, debug message persistence, and tools/schema typestate

## [2.4.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.3.0...ironflow-core-v2.4.0) - 2026-04-11

### Added

- add SSH HostKeyPolicy and RunStore::update_run_returning

## [2.3.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.2.0...ironflow-core-v2.3.0) - 2026-04-07

### Added

- add verbose debug mode for agent conversation tracing


### Changed

- unify AgentStepConfig and AgentConfig into single type


### Fixed

- add --verbose flag required by stream-json output format

## [2.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.1.1...ironflow-core-v2.2.0) - 2026-04-04

### Added

- #9 add Prometheus metrics across API, engine, and worker

## [2.1.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.1.0...ironflow-core-v2.1.1) - 2026-04-03

### Changed

- fix magic imports across all crates

## [2.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.0.1...ironflow-core-v2.1.0) - 2026-04-03

### Added

- propagate output_schema through AgentStepConfig and AgentExecutor

## [2.0.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.0.0...ironflow-core-v2.0.1) - 2026-04-01

### Changed

- #1 make AgentConfig model field provider-agnostic

