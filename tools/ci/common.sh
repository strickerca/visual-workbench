# Shared setup for the cloud/CI check scripts. Sourced, not executed.
set -euo pipefail
VW_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$VW_ROOT"
# shellcheck disable=SC1091
. "$VW_ROOT/tools/ci/pins.env"
export PATH="${VW_CI_BIN:-$HOME/.local/bin}:$PATH"
export PYTHONDONTWRITEBYTECODE=1
# .cargo/config.toml caps jobs at 2 for the 16 GB laptop; runners and cloud
# sessions have more room. Debug info is not needed for checks.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
step() { printf '\n=== %s ===\n' "$*"; }
PYTHON="${PYTHON:-python3}"
