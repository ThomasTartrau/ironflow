# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
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

