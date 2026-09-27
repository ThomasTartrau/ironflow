---
paths:
  - "ironflow-store/migrations/**"
  - "ironflow-store/src/entities/run_status.rs"
  - "ironflow-store/src/entities/step_status.rs"
  - "ironflow-store/src/postgres/**"
  - "ironflow-store/src/memory/run_store.rs"
  - "ironflow-store/tests/postgres_*.rs"
---

# PostgreSQL state machines and migrations

Run and step statuses live twice. In Rust: `RunStatus::can_transition_to`,
`PostgresStore::run_status_to_event` / `step_status_to_event`, and the step matrix in
`InMemoryStore::update_step`. In SQL: the `run_lifecycle` and `step_lifecycle` machines of
`lib_fsm`, written by migrations. Nothing ties them at compile time, and the in-memory
store accepts whatever Rust allows: a transition missing from SQL passes every
`cargo test` and only fails on a real database. That is how approval gates and delay steps
shipped broken on PostgreSQL, and how a migration that failed on every blank database
reached `main` (#113).

## When you touch a status or a transition

- A new variant or a new allowed transition needs, in the same MR, a migration that
  creates the SQL state and every transition to and from it. Model it on
  `20260914163438_add_run_warning_state` (up and down).
- `tests/postgres_fsm.rs` compares every `from -> to` pair across the in-memory store and
  PostgreSQL. A new variant breaks its compilation in `run_path` / `step_path` until you
  say how the variant is reached: add the arm, never a `_` wildcard.
- A down migration moves instances out of the state it removes (the precedent sends them
  to a terminal state), deletes their `state_machine_event` rows, then the transitions,
  then the state.

## When you write a migration

- Create it with `sqlx migrate add -r <name> --source ironflow-store/migrations`.
- Never `SELECT ... INTO STRICT` a state that no earlier migration creates. Grep the
  migrations for its `abstract_state_create` call first.
- Never edit a migration that has run anywhere: sqlx checks its checksum. The one
  exception is a migration that never succeeded on any database, which leaves no
  checksum behind (the case of #113).
- `ironflow-store/build.rs` makes cargo rebuild when a migration is added. Without it,
  `sqlx::migrate!` keeps the list it embedded at the last build and silently skips the
  new file.

## Proof before calling it done

The in-memory tests prove nothing about SQL. Run:

```bash
scripts/test-postgres.sh
```

It uses `DATABASE_URL` when set, otherwise starts a throwaway `postgres:17-alpine`
container, and runs the same suites as the `postgres-tests` CI job. It needs Docker when
`DATABASE_URL` is not set; if neither is available, say so instead of reporting green.
