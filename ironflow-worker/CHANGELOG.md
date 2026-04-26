# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
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

