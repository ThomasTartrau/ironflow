# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [2.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-runtime-v2.1.37...ironflow-runtime-v2.2.0) - 2026-08-08

### Added

- #24 add Trigger trait with event and NATS trigger sources


### Fixed

- #24 regenerate dashboard types and upgrade async-nats to 0.50

## [2.1.34](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-runtime-v2.1.33...ironflow-runtime-v2.1.34) - 2026-07-27
## [2.1.17](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-runtime-v2.1.16...ironflow-runtime-v2.1.17) - 2026-04-28

### Fixed

- initialize MasterKey in get_secret test_state()

## [2.1.14](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-runtime-v2.1.13...ironflow-runtime-v2.1.14) - 2026-04-26

### Documentation

- add README.md to each workspace crate

## [2.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-runtime-v2.0.0...ironflow-runtime-v2.1.0) - 2026-04-01

### Added

- migrate from semantic-release to release-plz for per-crate versioning


### Changed

- #1 make AgentConfig model field provider-agnostic

