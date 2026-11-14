#!/usr/bin/env bash
# Rust tests. Mirrors build.ps1 test-all: the workspace in debug, and the
# fault-injection simulator in release (its 10,000-seed test is slow in debug).
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "workspace tests"
cargo test --workspace --exclude vw-sim --locked
step "simulator tests (release)"
cargo test --release --locked -p vw-sim
