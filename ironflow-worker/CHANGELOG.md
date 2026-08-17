# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [2.13.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.12.1...ironflow-worker-v2.13.0) - 2026-08-17

### Added

- expose ApiRunStore publicly for external consumers

## [2.12.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.11.7...ironflow-worker-v2.12.0) - 2026-08-15

### Added

- #9 add OpenTelemetry tracing and enhanced Prometheus observability

## [2.11.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.11.0...ironflow-worker-v2.11.1) - 2026-08-03
## [2.11.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.10.0...ironflow-worker-v2.11.0) - 2026-07-29

### Added

- #21 version and rotate the secret encryption keys

## [2.10.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.9.1...ironflow-worker-v2.10.0) - 2026-07-28

### Added

- #14 add worker lease and reaper to recover orphaned runs

## [2.9.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.8.29...ironflow-worker-v2.9.0) - 2026-07-27

### Added

- #17 trace run authorship (created_by)

## [2.8.29](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.8.28...ironflow-worker-v2.8.29) - 2026-07-27
## [2.8.28](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.8.27...ironflow-worker-v2.8.28) - 2026-07-27
## [2.8.12](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.8.11...ironflow-worker-v2.8.12) - 2026-05-01

### Fixed

- handle pod pre-Running failures, JoinError step tracking, and error preservation

## [2.8.10](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.8.9...ironflow-worker-v2.8.10) - 2026-04-28

### Fixed

- initialize MasterKey in get_secret test_state()

## [2.8.9](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.8.8...ironflow-worker-v2.8.9) - 2026-04-26
## [2.8.6](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.8.5...ironflow-worker-v2.8.6) - 2026-04-26

### Documentation

- add README.md to each workspace crate

## [2.8.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.8.1...ironflow-worker-v2.8.2) - 2026-04-22
## [2.8.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.7.0...ironflow-worker-v2.8.0) - 2026-04-22

### Added

- add run labels, scheduled execution, and label filtering

## [2.7.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.6.1...ironflow-worker-v2.7.0) - 2026-04-21

### Added

- add persistent audit log store for event compliance

## [2.6.1](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.6.0...ironflow-worker-v2.6.1) - 2026-04-21

### Fixed

- add missing handler_version field to NewRun struct literals

## [2.6.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.5.1...ironflow-worker-v2.6.0) - 2026-04-21

### Added

- add encrypted secret store with unified Store trait and CRUD API

## [2.5.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.4.4...ironflow-worker-v2.5.0) - 2026-04-19

### Added

- live dashboard with filters and real-time run timeline

## [2.4.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.3.7...ironflow-worker-v2.4.0) - 2026-04-15

### Added

- publish lifecycle events from worker-facing API routes

## [2.3.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.2.4...ironflow-worker-v2.3.0) - 2026-04-11

### Added

- add rate limiting, worker timeout, poison pill guard, and graceful drain

## [2.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.1.2...ironflow-worker-v2.2.0) - 2026-04-04

### Added

- #9 add Prometheus metrics across API, engine, and worker

## [2.1.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.1.1...ironflow-worker-v2.1.2) - 2026-04-03

### Changed

- fix magic imports across all crates

## [2.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-worker-v2.0.4...ironflow-worker-v2.1.0) - 2026-04-03

### Added

- #2 add internal step-dependencies route, ci-pipeline demo, and review fixes

- #2 add step_dependencies table, entity, and store implementations

