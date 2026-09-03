# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [3.12.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.11.0...ironflow-core-v3.12.0) - 2026-09-03

### Added

- add Claude Fable 5.1 model support

## [3.11.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.10.0...ironflow-core-v3.11.0) - 2026-08-27

### Added

- #46 propagate W3C traceparent across all providers and workflow steps

## [3.10.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.9.0...ironflow-core-v3.10.0) - 2026-08-27

### Added

- #48 MCP tool routing by connector prefix with explicit rejection

## [3.9.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.8.1...ironflow-core-v3.9.0) - 2026-08-27

### Added

- #44 unified LLM cost attribution per step and workflow

## [3.8.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.8.0...ironflow-core-v3.8.1) - 2026-08-21

### Fixed

- eliminate ETXTBSY race in MCP bridge tests

- ETXTBSY race in MCP bridge test script creation

## [3.8.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.7.0...ironflow-core-v3.8.0) - 2026-08-20

### Added

- #38 step-level retry with exponential backoff

## [3.7.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.6.0...ironflow-core-v3.7.0) - 2026-08-20

### Added

- #36 finalize SSH, Docker, and K8s transports


### Fixed

- gate transport-only helpers behind feature flags

## [3.6.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.5.0...ironflow-core-v3.6.0) - 2026-08-15

### Added

- #9 add OpenTelemetry tracing and enhanced Prometheus observability

## [3.5.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.4.0...ironflow-core-v3.5.0) - 2026-08-10

### Added

- #25 allow_failure on steps to continue run with Warning status

## [3.4.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.3.0...ironflow-core-v3.4.0) - 2026-08-08

### Added

- #23 enforce handler version compatibility on retry

## [3.3.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.2.0...ironflow-core-v3.3.0) - 2026-07-28

### Added

- #14 add worker lease and reaper to recover orphaned runs

## [3.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.1.1...ironflow-core-v3.2.0) - 2026-07-27

### Added

- #15 automatic run retry with exponential backoff

## [3.1.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.1.0...ironflow-core-v3.1.1) - 2026-07-27
## [3.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v3.0.0...ironflow-core-v3.1.0) - 2026-07-27

### Added

- #18 add per-run and per-workflow cost caps

## [3.0.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.20.3...ironflow-core-v3.0.0) - 2026-07-25

### Added

- **BREAKING** add Claude 5 model family (Fable, Mythos, Opus, Sonnet)

## [2.20.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.20.2...ironflow-core-v2.20.3) - 2026-07-06

### Fixed

- resolve clippy warnings and missing PodConfig fields

- pipe prompt via stdin when it exceeds ARG_MAX threshold

## [2.20.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.20.1...ironflow-core-v2.20.2) - 2026-06-01

### Fixed

- remove unnecessary let binding to satisfy clippy let_and_return

## [2.20.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.20.0...ironflow-core-v2.20.1) - 2026-05-31

### Fixed

- prevent log stream hang when K8s pod completes

## [2.20.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.19.0...ironflow-core-v2.20.0) - 2026-05-31

### Added

- add PVC volume support to K8sEphemeralProvider

## [2.19.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.18.1...ironflow-core-v2.19.0) - 2026-05-29

### Added

- add Claude Opus 4.8 model support

## [2.18.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.18.0...ironflow-core-v2.18.1) - 2026-05-26

### Fixed

- strip markdown code fences from structured output in OpenAI-compat adapter

## [2.18.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.17.0...ironflow-core-v2.18.0) - 2026-05-25

### Added

- add MCP bridge tool for HTTP provider tool registry

## [2.17.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.16.0...ironflow-core-v2.17.0) - 2026-05-25

### Added

- add agentic HTTP loop with tool registry and provider router

## [2.16.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.15.0...ironflow-core-v2.16.0) - 2026-05-25

### Added

- add NVIDIA NIM provider with 25+ model constants

## [2.15.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.14.0...ironflow-core-v2.15.0) - 2026-05-25

### Added

- add HTTP providers for OpenAI, Mistral, Gemini, and Anthropic API


### Fixed

- *(ci)* remove http-providers example from workspace, fix fmt and model IDs

## [2.14.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.13.2...ironflow-core-v2.14.0) - 2026-05-13

### Added

- add schema transformation and auto-retry for structured output

## [2.13.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.13.1...ironflow-core-v2.13.2) - 2026-05-01

### Fixed

- truncate error_detail in handle_nonzero_exit to prevent oversized payloads

## [2.13.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.13.0...ironflow-core-v2.13.1) - 2026-05-01

### Fixed

- remove unused `conditions` import in ephemeral.rs

- handle pod pre-Running failures, JoinError step tracking, and error preservation

## [2.13.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.12.0...ironflow-core-v2.13.0) - 2026-04-30

### Added

- add AgentInput for declarative external file fetching

## [2.12.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.11.0...ironflow-core-v2.12.0) - 2026-04-26

### Added

- add LogSink trait for real-time log streaming from providers

## [2.11.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.10.1...ironflow-core-v2.11.0) - 2026-04-26

### Added

- capture raw response text in SchemaValidation errors

## [2.10.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.10.0...ironflow-core-v2.10.1) - 2026-04-26

### Documentation

- add README.md to each workspace crate

## [2.10.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.9.0...ironflow-core-v2.10.0) - 2026-04-26

### Added

- add hostPath volume support to K8sEphemeralProvider

## [2.9.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.8.0...ironflow-core-v2.9.0) - 2026-04-22

### Added

- support custom pod labels for K8s providers

## [2.8.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-core-v2.7.0...ironflow-core-v2.8.0) - 2026-04-19

### Added

- add Claude Opus 4.7 model constants

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

