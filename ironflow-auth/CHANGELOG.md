# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [2.3.8](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-auth-v2.3.7...ironflow-auth-v2.3.8) - 2026-04-28

### Fixed

- initialize MasterKey in get_secret test_state()

## [2.3.5](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-auth-v2.3.4...ironflow-auth-v2.3.5) - 2026-04-26

### Documentation

- add README.md to each workspace crate

## [2.3.4](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-auth-v2.3.3...ironflow-auth-v2.3.4) - 2026-04-25

### Fixed

- #11 widen api_keys.key_prefix column from VARCHAR(12) to VARCHAR(16)

## [2.3.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-auth-v2.2.2...ironflow-auth-v2.3.0) - 2026-04-21

### Added

- add encrypted secret store with unified Store trait and CRUD API

## [2.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-auth-v2.1.2...ironflow-auth-v2.2.0) - 2026-04-14

### Added

- add user management CRUD, admin guards, and scope-based API key permissions

## [2.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/ironflow-auth-v2.0.0...ironflow-auth-v2.1.0) - 2026-04-13

### Added

- add API keys management and MCP server

