# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [0.1.27](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.26...ironflow-mcp-v0.1.27) - 2026-10-04

### Added

- #162 add a run concurrency key enforced by the store, sub-workflows included

## [0.1.26](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.25...ironflow-mcp-v0.1.26) - 2026-10-03

### Fixed

- #152 harden auth endpoints against abuse

## [0.1.25](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.24...ironflow-mcp-v0.1.25) - 2026-10-02

### Added

- #146 add ctx.wait_for_signal, signal delivery and wake-up of Sleeping runs

## [0.1.24](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.23...ironflow-mcp-v0.1.24) - 2026-09-30

### Added

- #135 add Provider Accounts for live-managed Claude subscriptions with usage windows

## [0.1.23](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.22...ironflow-mcp-v0.1.23) - 2026-09-27

### Added

- #109 add HumanInput step to suspend a run and resume it with a typed payload

## [0.1.22](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.21...ironflow-mcp-v0.1.22) - 2026-09-25

### Added

- #96 add replay endpoint to re-run a finished run on the current handler version

## [0.1.21](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.20...ironflow-mcp-v0.1.21) - 2026-09-25

### Fixed

- #102 complete and align dashboard stats across stores, API and charts

## [0.1.20](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.19...ironflow-mcp-v0.1.20) - 2026-09-23

### Added

- #98 add execution plan (dry-run planner) with CLI, API and dashboard views


### Fixed

- #98 fix MCP plan_workflow payload JsonSchema derive and dashboard formatting

## [0.1.19](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.18...ironflow-mcp-v0.1.19) - 2026-09-22

### Added

- #92 add pagination to approval delegations list

- #92 add approval delegation for absent approvers

## [0.1.17](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.16...ironflow-mcp-v0.1.17) - 2026-09-11

### Added

- #79 add stats history endpoint with time-series dashboard charts

## [0.1.16](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.15...ironflow-mcp-v0.1.16) - 2026-09-10

### Added

- #73 add cron schedules with full API, CLI, SDK, MCP and dashboard support

## [0.1.15](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.14...ironflow-mcp-v0.1.15) - 2026-09-10

### Added

- #75 add MCP tools for api-keys, secrets, users, artifacts, audit-logs and run search

## [0.1.13](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.12...ironflow-mcp-v0.1.13) - 2026-08-21

### Added

- #37 add run logs with store, API, SDK, CLI and MCP support

## [0.1.11](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.10...ironflow-mcp-v0.1.11) - 2026-08-08

### Added

- #23 enforce handler version compatibility on retry

## [0.1.9](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.8...ironflow-mcp-v0.1.9) - 2026-07-27

### Added

- #17 trace run authorship (created_by)

## [0.1.8](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.7...ironflow-mcp-v0.1.8) - 2026-07-27
## [0.1.7](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.6...ironflow-mcp-v0.1.7) - 2026-07-27

### Added

- #18 add per-run and per-workflow cost caps

## [0.1.5](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.4...ironflow-mcp-v0.1.5) - 2026-04-28

### Fixed

- initialize MasterKey in get_secret test_state()

## [0.1.4](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.3...ironflow-mcp-v0.1.4) - 2026-04-26

### Documentation

- add README.md to each workspace crate

## [0.1.2](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-mcp-v0.1.1...ironflow-mcp-v0.1.2) - 2026-04-19

### Added

- add workflow categories with tree view in dashboard

