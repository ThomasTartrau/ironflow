#!/usr/bin/env bash
# Copies a fixture crate into the current (empty) workspace and gives it a manifest.
# Usage: seed.sh <fixture dir>
set -euo pipefail
cp -R "$1/." .
cat > Cargo.toml <<'EOF'
[package]
name = "workflows"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
chrono = "0.4"
ironflow-core = "2"
ironflow-engine = { version = "2", features = ["secret-store"] }
reqwest = { version = "0.12", features = ["json"] }
schemars = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
EOF
