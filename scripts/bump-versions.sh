#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:?Usage: bump-versions.sh <version>}"

# All workspace crates to bump
CRATES=(
  ironflow-core
  ironflow-runtime
  ironflow-store
  ironflow-engine
  ironflow-api
  ironflow-auth
  ironflow-worker
)

# Bump each crate's own version
for crate in "${CRATES[@]}"; do
  sed -i "s/^version = \".*\"/version = \"${VERSION}\"/" "${crate}/Cargo.toml"
done

# Update inter-crate dependency versions
declare -A DEPENDENTS
DEPENDENTS[ironflow-core]="ironflow-runtime ironflow-engine ironflow-api ironflow-worker"
DEPENDENTS[ironflow-store]="ironflow-engine ironflow-api ironflow-worker"
DEPENDENTS[ironflow-engine]="ironflow-api ironflow-worker"
DEPENDENTS[ironflow-auth]="ironflow-api"

for dep in "${!DEPENDENTS[@]}"; do
  for consumer in ${DEPENDENTS[$dep]}; do
    sed -i "s/${dep} = { version = \"[^\"]*\"/${dep} = { version = \"${VERSION}\"/" "${consumer}/Cargo.toml"
  done
done

cargo generate-lockfile
