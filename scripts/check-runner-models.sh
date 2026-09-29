#!/usr/bin/env bash
# Check that the Claude Code version baked into the runner image knows every
# full model id of `Model` (ironflow-core/src/operations/agent.rs). The API
# refuses a model to a CLI release that predates it (400
# claude_code_version_too_old), and a CLI release carries the ids of the models
# it supports: 2.1.274 has no "claude-opus-5-5", 2.1.284 has it. Run by CI;
# usable locally.
#
#   scripts/check-runner-models.sh            # version of docker/claude-runner/IMAGE_TAG
#   scripts/check-runner-models.sh 2.1.274    # any other Claude Code version
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAG="$(cat "$ROOT/docker/claude-runner/IMAGE_TAG")"
VERSION="${1:-${TAG%-*}}"
MODELS_RS="$ROOT/ironflow-core/src/operations/agent.rs"

# Full ids of `impl Model` only: the aliases (sonnet, opus, haiku) are resolved
# by the CLI itself, and `[1m]` is a context-window suffix, not part of the id.
MODELS="$(awk '/^impl Model \{/,/^\}/' "$MODELS_RS" \
  | sed -n 's/^ *pub const [A-Z0-9_]*: &str = "\(claude-[^"[]*\)\(\[1m\]\)\{0,1\}";$/\1/p' \
  | sort -u)"
if [ -z "$MODELS" ]; then
  echo "no claude-* model id found in impl Model of $MODELS_RS"
  exit 1
fi

# The linux-x64 package holds the same binary the runner image installs.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
curl -sSfL "https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-${VERSION}.tgz" \
  | tar -xz -C "$WORK"
BIN="$WORK/package/claude"
if [ ! -f "$BIN" ]; then
  echo "no claude binary in @anthropic-ai/claude-code-linux-x64@${VERSION}"
  exit 1
fi

missing=0
for id in $MODELS; do
  if grep -aqF "\"$id\"" "$BIN"; then
    echo "ok       $id"
  else
    echo "MISSING  $id"
    missing=1
  fi
done

if [ "$missing" -ne 0 ]; then
  echo "Claude Code ${VERSION} does not know every Model constant: bump docker/claude-runner/IMAGE_TAG."
  exit 1
fi
echo "Claude Code ${VERSION} knows every Model constant"
