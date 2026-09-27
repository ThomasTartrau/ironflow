#!/usr/bin/env bash
# Run the PostgreSQL test suites: the `#[ignore]` tests that need a database.
# The in-memory store accepts whatever the Rust types allow, so migrations and
# the SQL state machines are only exercised here. Run by CI; usable locally.
#
#   scripts/test-postgres.sh                 # every PostgreSQL suite
#   scripts/test-postgres.sh <cargo test args>  # one run of your own, e.g.
#   scripts/test-postgres.sh -p ironflow-store --features store-postgres \
#     --test postgres_fsm -- --ignored
#
# Uses DATABASE_URL when it is set (CI). Otherwise starts a throwaway postgres
# container, removed on exit.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ -z "${DATABASE_URL:-}" ]; then
  container="ironflow-test-postgres-$$"
  docker run -d --rm --name "$container" \
    -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=ironflow \
    -p 127.0.0.1::5432 "${POSTGRES_IMAGE:-postgres:17-alpine}" >/dev/null
  trap 'docker rm -f "$container" >/dev/null 2>&1' EXIT

  # Wait on TCP: the image first runs a setup server on the unix socket only,
  # then restarts it, and a socket check would pass during that setup.
  for _ in $(seq 1 60); do
    docker exec "$container" pg_isready -h 127.0.0.1 -U postgres -q && break
    sleep 0.5
  done
  docker exec "$container" pg_isready -h 127.0.0.1 -U postgres

  mapping="$(docker port "$container" 5432/tcp | head -n1)"
  export DATABASE_URL="postgres://postgres:postgres@127.0.0.1:${mapping##*:}/ironflow"
fi

# With DATABASE_URL set, sqlx's query! macros would check every query against
# that database at compile time, before any migration ran. Use the committed
# offline cache instead.
export SQLX_OFFLINE=true

if [ "$#" -gt 0 ]; then
  exec cargo test "$@"
fi

cargo test -p ironflow-store --features store-postgres,secret-store -- --ignored
cargo test -p ironflow-api --features store-postgres --test postgres_schedule_sync \
  -- --ignored
