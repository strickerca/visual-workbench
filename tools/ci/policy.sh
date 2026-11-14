#!/usr/bin/env bash
# Repository policy checks: IDs, plan coverage, licences of pinned npm packages, secrets.
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "setup and requirement IDs"
"$PYTHON" tools/check_setup.py all
step "plan coverage"
"$PYTHON" tools/check_plan_coverage.py
step "npm licences (lockfile)"
"$PYTHON" tools/check_npm_licenses.py
step "document links"
"$PYTHON" tools/ci/check_doc_links.py
step "requirement-to-test links and README status table"
"$PYTHON" tools/ci/requirements_status.py

step "secret scan (tracked files only)"
gitleaks version | grep -q "$GITLEAKS_VERSION" || { echo "gitleaks $GITLEAKS_VERSION required" >&2; exit 1; }
# Scan exactly what Git tracks, so build output and node_modules never count.
scan="$(mktemp -d)"
trap 'rm -rf "$scan"' EXIT
git ls-files -z | while IFS= read -r -d '' file; do
  if [ -f "$file" ]; then printf '%s\0' "$file"; fi
done | xargs -0 cp --parents -t "$scan"
(cd "$scan" && gitleaks dir . --config "$VW_ROOT/.gitleaks.toml" --redact=100 --no-banner)
