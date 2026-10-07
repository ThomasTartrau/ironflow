# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [0.1.59](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.58...ironflow-cli-v0.1.59) - 2026-10-07

### Added

- #172 pause and resume a run or a whole workflow


### Fixed

- #172 drop redundant paused field and regenerate OpenAPI snapshots

## [0.1.58](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.57...ironflow-cli-v0.1.58) - 2026-10-07

### Added

- #174 add catchup, overlap and timezone policies to cron schedules

## [0.1.57](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.56...ironflow-cli-v0.1.57) - 2026-10-06
## [0.1.56](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.55...ironflow-cli-v0.1.56) - 2026-10-06
## [0.1.55](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.54...ironflow-cli-v0.1.55) - 2026-10-06

### Added

- #170 route runs to workers that support their workflow and tags

## [0.1.54](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.53...ironflow-cli-v0.1.54) - 2026-10-06

### Added

- #184 wait or fast fail when Claude accounts are rate limited (max_capacity_wait)

## [0.1.53](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.52...ironflow-cli-v0.1.53) - 2026-10-06
## [0.1.52](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.51...ironflow-cli-v0.1.52) - 2026-10-06

### Fixed

- #169 cascade run cancellation to active child runs

## [0.1.51](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.50...ironflow-cli-v0.1.51) - 2026-10-05
## [0.1.49](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.48...ironflow-cli-v0.1.49) - 2026-10-05
## [0.1.47](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.46...ironflow-cli-v0.1.47) - 2026-10-04

### Added

- #162 add a run concurrency key enforced by the store, sub-workflows included

## [0.1.46](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.45...ironflow-cli-v0.1.46) - 2026-10-03

### Fixed

- #152 harden auth endpoints against abuse

## [0.1.45](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.44...ironflow-cli-v0.1.45) - 2026-10-03

### Fixed

- address review on MR !445

- #156 template update applies the update in place, version read from workspace root

## [0.1.44](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.43...ironflow-cli-v0.1.44) - 2026-10-02

### Added

- #146 add ctx.wait_for_signal, signal delivery and wake-up of Sleeping runs

## [0.1.42](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.41...ironflow-cli-v0.1.42) - 2026-09-30

### Added

- #135 add Provider Accounts for live-managed Claude subscriptions with usage windows

- #135 add Provider Accounts for live-managed Claude subscriptions with usage windows

## [0.1.41](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.40...ironflow-cli-v0.1.41) - 2026-09-29

### Added

- support root templates and report registry not found or unreachable

## [0.1.39](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.38...ironflow-cli-v0.1.39) - 2026-09-27
## [0.1.38](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.37...ironflow-cli-v0.1.38) - 2026-09-27

### Added

- #108 add label/labels builder to PodRun pods

## [0.1.37](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.36...ironflow-cli-v0.1.37) - 2026-09-25

### Added

- #105 type the workflow author API end to end

## [0.1.36](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.35...ironflow-cli-v0.1.36) - 2026-09-25
## [0.1.35](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.34...ironflow-cli-v0.1.35) - 2026-09-25

### Fixed

- #102 complete and align dashboard stats across stores, API and charts

## [0.1.34](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.33...ironflow-cli-v0.1.34) - 2026-09-23

### Added

- #98 add execution plan (dry-run planner) with CLI, API and dashboard views

## [0.1.33](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.32...ironflow-cli-v0.1.33) - 2026-09-22

### Added

- #92 add pagination to approval delegations list

- #92 add approval delegation for absent approvers

## [0.1.31](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.30...ironflow-cli-v0.1.31) - 2026-09-21

### Added

- #91 add SLA deadlines and escalation policies to approval gates

## [0.1.27](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.26...ironflow-cli-v0.1.27) - 2026-09-11

### Added

- #79 add stats history endpoint with time-series dashboard charts

## [0.1.26](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.25...ironflow-cli-v0.1.26) - 2026-09-11

### Added

- #78 add template registry versioning and dependency management

## [0.1.24](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.23...ironflow-cli-v0.1.24) - 2026-09-10

### Added

- #74 add init, dashboard, run watch, run diff and template create CLI commands

## [0.1.23](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.22...ironflow-cli-v0.1.23) - 2026-09-10

### Added

- #73 add schedule source tracking and handler-declared schedule reconciliation

- #73 add cron schedules with full API, CLI, SDK, MCP and dashboard support

## [0.1.22](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.21...ironflow-cli-v0.1.22) - 2026-09-10

### Added

- #71 add delay step for timed workflow pauses

## [0.1.20](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.19...ironflow-cli-v0.1.20) - 2026-08-27

### Added

- #49 add per-run SSE route for WorkflowEventBus

## [0.1.18](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.17...ironflow-cli-v0.1.18) - 2026-08-21
## [0.1.17](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.16...ironflow-cli-v0.1.17) - 2026-08-20

### Added

- #39 add shell completions and man page generation

## [0.1.14](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.13...ironflow-cli-v0.1.14) - 2026-08-14

### Added

- #8 add shadcn-style template system with CLI commands

## [0.1.13](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.12...ironflow-cli-v0.1.13) - 2026-08-10

### Added

- #25 allow_failure on steps to continue run with Warning status

## [0.1.11](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.10...ironflow-cli-v0.1.11) - 2026-08-08

### Added

- #23 enforce handler version compatibility on retry

## [0.1.9](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.8...ironflow-cli-v0.1.9) - 2026-08-03
## [0.1.8](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.7...ironflow-cli-v0.1.8) - 2026-07-29

### Added

- #20 add secret, api-key, user and audit-log CLI commands

## [0.1.5](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.4...ironflow-cli-v0.1.5) - 2026-07-27

### Added

- #17 trace run authorship (created_by)

## [0.1.4](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.3...ironflow-cli-v0.1.4) - 2026-07-27
## [0.1.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-cli-v0.1.2...ironflow-cli-v0.1.3) - 2026-07-27

### Added

- #18 add per-run and per-workflow cost caps

## [0.1.0](https://gitlab.com/ThomasTartrau/ironflow/releases/tag/ironflow-cli-v0.1.0) - 2026-06-02

### Added

- #13 add ironflow-cli crate

