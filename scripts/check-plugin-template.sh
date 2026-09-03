#!/usr/bin/env bash
# Scaffold the plugin's project template against this checkout and make sure
# it builds and its tests pass. Run by CI; usable locally.
#
#   scripts/check-plugin-template.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${PLUGIN_TEMPLATE_DIR:-$(mktemp -d)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

"$ROOT/plugins/ironflow/skills/setup/scripts/scaffold.sh" "$WORK/app" \
  --ironflow-path "$ROOT" --without-dashboard

cd "$WORK/app"
cargo build --workspace
cargo test --workspace
echo "plugin template OK ($WORK/app)"
