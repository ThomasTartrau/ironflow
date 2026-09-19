# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [0.1.10](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-ops-k8s-v0.1.9...ironflow-ops-k8s-v0.1.10) - 2026-09-19

### Added

- #90 add activeDeadlineSeconds to k8s pod/job runs and ephemeral provider

## [0.1.8](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-ops-k8s-v0.1.7...ironflow-ops-k8s-v0.1.8) - 2026-09-18

### Added

- #89 mount multiple PVCs via additive .pvc() on PodRun and JobRun

## [0.1.7](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-ops-k8s-v0.1.6...ironflow-ops-k8s-v0.1.7) - 2026-09-17

### Added

- #86 add pod/job security hardening builders (ops-k8s)

## [0.1.6](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-ops-k8s-v0.1.5...ironflow-ops-k8s-v0.1.6) - 2026-09-16

### Added

- #85 add k8s apply, job_run and pod_run operations

## [0.1.5](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-ops-k8s-v0.1.4...ironflow-ops-k8s-v0.1.5) - 2026-09-16

### Fixed

- make k8s client incluster test deterministic in CI

## [0.1.3](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-ops-k8s-v0.1.2...ironflow-ops-k8s-v0.1.3) - 2026-09-10

### Documentation

- #69 add mdBook documentation and READMEs for ops/ crates

## [0.1.0](https://gitlab.com/ThomasTartrau/ironflow/releases/tag/ironflow-ops-k8s-v0.1.0) - 2026-09-09

### Added

- #57 add ironflow-ops-k8s crate powered by the kube crate

