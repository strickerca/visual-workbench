#!/usr/bin/env bash
set -euo pipefail
_vw_install_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export VW_TOOLS_ROOT="${VW_TOOLS_ROOT:-/workspace/.vw-tools}"
mkdir -p "$VW_TOOLS_ROOT"
# One installer/build owner per VM. Independent VMs do not share this lock.
flock -w 30 "$VW_TOOLS_ROOT/heavy-build.lock" \
  timeout --signal=TERM --kill-after=10s 20m \
  python3 -u "$_vw_install_dir/install.py" "$@"
