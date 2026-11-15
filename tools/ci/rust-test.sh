#!/usr/bin/env bash
# Rust tests. Mirrors build.ps1 test-all: the workspace in debug, and the
# fault-injection simulator in release (its 10,000-seed test is slow in debug).
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "workspace tests"
cargo test --workspace --exclude vw-sim --locked
step "simulator tests (release)"
cargo test --release --locked -p vw-sim
# Speed budget: saving one stroke into a 2,000-stroke document must stay under
# 10 ms (it measures about 0.7 ms), and no single save may take 250 ms (the
# slowest measures 12-20 ms; the recovery checkpoint that used to run inside a
# save took 450 ms at this size). Fails on any return to work that grows with
# the size of the document. The second limit is wide on purpose: a busy disk
# on a shared machine has produced a 100 ms save.
step "stroke-save speed budget"
VW_BENCH_STROKES=2000 VW_BENCH_BUDGET_MS=10 VW_BENCH_SLOWEST_BUDGET_MS=250 cargo bench -p vw-ffi --bench commit --locked
