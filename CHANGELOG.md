## [2.0.0](https://gitlab.com/ThomasTartrau/ironflow/compare/v1.3.1...v2.0.0) (2026-03-31)

### ⚠ BREAKING CHANGES

* Complete architecture overhaul from v1 to v2.

- Add workflow engine with FSM-based run execution
- Add worker with step executors (shell, agent, webhook, sub-workflow)
- Add REST API with axum (runs, workflows, steps, dashboard stats)
- Add React/TypeScript dashboard with filtering, retry, and theming
- Add store layer (SQLx/PostgreSQL) with migrations
- Add auth layer with JWT, Argon2id password hashing, and cookie management
- Refactor runtime into webhook + cron modules
- Add comprehensive test suite (unit + integration)
- Add example server, worker, and workflow definitions

### Features

* add workflow engine, worker, API, and dashboard ([9db68e8](https://gitlab.com/ThomasTartrau/ironflow/commit/9db68e8340482f3f709efffbe00943fe1edaf176))

## [1.3.1](https://gitlab.com/ThomasTartrau/ironflow/compare/v1.3.0...v1.3.1) (2026-03-25)

### Bug Fixes

* strip all CLAUDE* env vars from child process to prevent sub-agent mode ([e590a03](https://gitlab.com/ThomasTartrau/ironflow/commit/e590a034bea06756a65b347715d03d09b6b61ceb))

## [1.3.0](https://gitlab.com/ThomasTartrau/ironflow/compare/v1.2.0...v1.3.0) (2026-03-24)

### Features

* add remote transport providers (SSH, Docker, Kubernetes) ([717f815](https://gitlab.com/ThomasTartrau/ironflow/commit/717f815c20328faf6b84a021ee36472f6c061fe0))

### Bug Fixes

* address review feedback on remote transport providers ([0884ed3](https://gitlab.com/ThomasTartrau/ironflow/commit/0884ed392c75204bfb24fc43ac373482f2ba113e))
* unset CLAUDECODE and IRONFLOW_ALLOW_BYPASS in SSH remote commands ([d172e46](https://gitlab.com/ThomasTartrau/ironflow/commit/d172e463585864674115219fcfe5b0318a4c89db))

## [1.2.0](https://gitlab.com/ThomasTartrau/ironflow/compare/v1.1.0...v1.2.0) (2026-03-24)

### Features

* add Runtime::run_crons() to run cron jobs without HTTP server ([9655d56](https://gitlab.com/ThomasTartrau/ironflow/commit/9655d565be18f8e970e9846c872b317fcb2720ea))

## [1.1.0](https://gitlab.com/ThomasTartrau/ironflow/compare/v1.0.1...v1.1.0) (2026-03-22)

### Features

* add retry policy with exponential backoff for HTTP and Agent operations ([b3151da](https://gitlab.com/ThomasTartrau/ironflow/commit/b3151dad350c89e3f525a3143353b4aebb5fe122))

## [1.0.1](https://gitlab.com/ThomasTartrau/ironflow/compare/v1.0.0...v1.0.1) (2026-03-17)

### Bug Fixes

* validate prompt size before spawning claude process and improve error reporting ([cec2d77](https://gitlab.com/ThomasTartrau/ironflow/commit/cec2d771242ba6cfdd9b303c425ee5bd92c146c3))

## 1.0.0 (2026-03-17)

### Features

* ironflow v0.1.0 ([fe0df2d](https://gitlab.com/ThomasTartrau/ironflow/commit/fe0df2dad483289ebbd384b95fe6a4c6780b2727))
