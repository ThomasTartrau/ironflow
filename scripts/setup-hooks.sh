#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)
HOOK_DIR="${REPO_ROOT}/.git/hooks"
SCRIPT_DIR="${REPO_ROOT}/scripts"

echo "Installing git hooks..."

cp "${SCRIPT_DIR}/pre-commit" "${HOOK_DIR}/pre-commit"
chmod +x "${HOOK_DIR}/pre-commit"

echo "Done. pre-commit hook installed."
