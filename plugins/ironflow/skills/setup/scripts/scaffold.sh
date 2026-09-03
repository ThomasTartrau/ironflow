#!/usr/bin/env bash
# Scaffold an ironflow project: copy the template, then resolve the ironflow
# crates with `cargo add` so the project always starts on the latest release.
#
# Usage:
#   scaffold.sh <target-dir> [--ironflow-path <ironflow-workspace>] [--without-dashboard]
#
# --ironflow-path   use path dependencies into a local ironflow checkout
#                   instead of crates.io (used by ironflow's own CI).
# --without-dashboard
#                   do not enable the `dashboard` feature of ironflow-api.
set -euo pipefail

TARGET=""
IRONFLOW_PATH=""
DASHBOARD=1
while [ $# -gt 0 ]; do
  case "$1" in
    --ironflow-path) IRONFLOW_PATH="$2"; shift 2 ;;
    --without-dashboard) DASHBOARD=0; shift ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) TARGET="$1"; shift ;;
  esac
done
[ -n "$TARGET" ] || { echo "usage: scaffold.sh <target-dir> [--ironflow-path <dir>] [--without-dashboard]" >&2; exit 2; }

ASSETS="$(cd "$(dirname "$0")/../assets" && pwd)"

if [ -e "$TARGET" ] && [ -n "$(ls -A "$TARGET" 2>/dev/null)" ]; then
  echo "refusing to scaffold into non-empty directory: $TARGET" >&2
  exit 1
fi
mkdir -p "$TARGET"
cp -R "$ASSETS"/. "$TARGET"/
cd "$TARGET"

# Resolve a crate spec, from crates.io or from a local checkout.
dep() {
  local pkg="$1" crate="$2"; shift 2
  if [ -n "$IRONFLOW_PATH" ]; then
    cargo add -p "$pkg" "$crate" --path "$IRONFLOW_PATH/$crate" "$@"
  else
    cargo add -p "$pkg" "$crate" "$@"
  fi
}

# workflows: handlers only need the engine; tests run a real engine in memory.
dep workflows ironflow-engine
dep workflows ironflow-core --dev
dep workflows ironflow-store --dev

# server: API, store, engine, auth, artifacts.
API_FEATURES="sign-up,openapi"
[ "$DASHBOARD" = 1 ] && API_FEATURES="dashboard,$API_FEATURES"
dep server ironflow-api --features "$API_FEATURES"
dep server ironflow-store --features "store-memory,secret-store"
dep server ironflow-engine
dep server ironflow-core
dep server ironflow-auth
dep server ironflow-artifacts
cargo add -p server dotenvy

# worker: provider plus the worker runtime.
dep worker ironflow-worker
dep worker ironflow-core
cargo add -p worker dotenvy

cp .env.example .env
echo
echo "Scaffolded ironflow project in $TARGET"
echo "  next: cargo build && scripts/dev.sh"
