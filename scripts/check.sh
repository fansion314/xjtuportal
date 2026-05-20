#!/usr/bin/env bash
set -euo pipefail

cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
