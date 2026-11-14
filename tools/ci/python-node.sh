#!/usr/bin/env bash
# Tooling tests (Python) and the MCP server tests (Node, exact pinned version).
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

run_py() { step "python: $1"; "$PYTHON" -m unittest discover -s "$1" -p "${2:-test_*.py}"; }
run_py tools/ai-spike/tests
run_py tools/tests 'test*.py'
run_py tools/desktop-test
run_py tools/android-tunnel
run_py tools/assisted-selection/license-audit
run_py tools/assisted-selection/tests
run_py tools/release
run_py tools/remote-edit
run_py tools/pair-cli/tests
run_py tools/ffi-test
run_py tools/app-test
run_py mcp

step "node version"
[ "$(node --version)" = "v$NODE_VERSION" ] || { echo "Node $NODE_VERSION required, found $(node --version)" >&2; exit 1; }
step "mcp: install from lockfile"
(cd mcp && npm ci --ignore-scripts --no-audit --no-fund)
step "mcp: licences of installed packages"
"$PYTHON" tools/check_npm_licenses.py --installed mcp
step "mcp: tests"
(cd mcp && node --test --test-concurrency=2 test/*.test.mjs codex/test_contract.mjs)
