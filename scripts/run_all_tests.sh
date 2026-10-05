#!/usr/bin/env bash
# Builds everything and runs every test.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

./scripts/check_env.sh
cargo build --workspace --all-targets
cargo test --workspace
cargo run --bin tempo -- --smoke-test
echo "All checks passed"
