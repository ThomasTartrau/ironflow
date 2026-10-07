---
paths:
  - "ironflow-store/migrations/**"
---

# Creating a migration

Create a migration only with the sqlx CLI, which timestamps it to the second:

```bash
cargo sqlx migrate add -r <name> --source ironflow-store/migrations
```

Never write or edit a version by hand (`20261008100000`, `...110000`): round versions
picked by hand are the ones two branches pick at once. sqlx keys `_sqlx_migrations` by
version alone, so two migrations sharing one compile, pass every in-memory test, and
fail on PostgreSQL with `duplicate key ... _sqlx_migrations_pkey` on a blank database,
or `migration <version> was previously applied but has been modified` on an existing
one. That is how ironflow-store 2.46.0 shipped broken (#190).

`tests/migration_versions.rs` (`migrations_have_unique_versions`) fails on a shared
version without a database, and every MR job merges `main` first
(`ci/merge-target-branch.sh`), so a collision between two open MRs shows up in the
second pipeline that runs. Rerun the pipeline of an MR that is behind `main` before
merging it.

A version already published cannot be renamed: existing databases recorded it. To
fix a collision, give the new version to the migration that was published last, make
its up migration replayable (`IF EXISTS` / `IF NOT EXISTS` on every statement), and
document how to repair a database that recorded the wrong one.

The PostgreSQL rules for the migrations themselves (state machines, checksums, proof
with `scripts/test-postgres.sh`) live in `postgres-fsm.md`.
