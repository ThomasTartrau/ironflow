#!/usr/bin/env bash
# Start the API server and one worker, stop both on Ctrl+C.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build -p server -p worker

cargo run -p server &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT

# Give the server time to bind before the worker starts polling.
for _ in $(seq 1 30); do
  if curl -fsS "http://localhost:${PORT:-3000}/api/v1/health-check" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

cargo run -p worker
