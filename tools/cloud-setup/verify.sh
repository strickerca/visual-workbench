#!/usr/bin/env bash
set -euo pipefail
_vw_verify_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$_vw_verify_dir/start.sh"
flock -w 30 "$VW_TOOLS_ROOT/heavy-build.lock" \
  timeout --signal=TERM --kill-after=10s 8m \
  python3 -u "$_vw_verify_dir/verify.py" "$@"
