#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
bash scripts/ci-node-harness.sh
bash scripts/verify-readme-value.sh
bash scripts/verify-installation-surface.sh
