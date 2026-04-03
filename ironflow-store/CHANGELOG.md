# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
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

