#!/usr/bin/env bash
# Rust formatting, lints with warnings denied, and the dependency licence gate.
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "format"
cargo fmt --all -- --check
step "clippy"
cargo clippy --workspace --all-targets --locked -- -D warnings
step "dependency licences, sources and bans"
cargo deny --version | grep -q "$CARGO_DENY_VERSION" || { echo "cargo-deny $CARGO_DENY_VERSION required" >&2; exit 1; }
cargo deny check licenses sources bans
