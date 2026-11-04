# B8 packaged MCP two-cycle smoke — 2026-11-04

Stage 1 blocker B8 of the 2026-11 completion roadmap, closed on this Windows
laptop at main `3354a8a`. The run used a fresh `build.ps1
build-desktop-distribution`, followed by the unchanged
`tools/desktop-test/run.ps1 -Execute -Mode mcp -TimeoutSeconds 180`.

## Result

- `run.ps1` receipt `6b520be533a4488e940b10e4d657eb99.json`: **status=passed**.
  Source check, pinned check, startup (37.5 s) and collect all exit 0.
- Diagnostic rerun of the same packaged executable, `--mcp-runtime-smoke`, for
  timing:

  | Step | step_ms | bundle_inventory_ms | bundle_verify_ms | service_open_ms | hashed files / bytes |
  | --- | --- | --- | --- | --- | --- |
  | first_start | 6,718 | 4,496 | 753 | 1,295 | 1,514 / 119,499,486 |
  | second_start | 6,522 | 4,863 | 629 | 969 | 1,514 / 119,499,486 |

  `VW_DESKTOP_MCP cycles=2 duplicate_owner=refused grants=0 sends=0
  cleanup=complete`

## What changed

1. **Cache preflight** (`tools/mcp_cache_preflight.py`, merge `ff819d8`). The
   2026-10-20 `canonical_path_changed` abort is attributed to MSIX app-data
   redirection: those builds ran inside the Codex app's package container.
   That attribution is inferred, not proven by a receipt. From this session
   the redirected 8-entry cache is not visible
   (`cache_absent_in_this_namespace`), so the app started from a clean cache.
2. **Verification speed** (merge `3354a8a`). The earlier run after the
   preflight fix failed `mcp_unknown_first_start_timeout`:
   - first start took 64 s, with `bundle_verify_ms=59226`;
   - the second start then timed out inside the 90 s lifecycle budget.

   The cause was a per-file, per-ancestor `toRealPath()` redirect check,
   which is O(files × depth²). Hashing itself took under 1 s. Verification
   is now one no-follow walk plus a full SHA-256 of every file on every start.
   The independent review found no regression; the inventory pins still hold
   every entry for the runtime's lifetime.

## Limits

- No grants, capture, package publication, external client or Send was
  exercised. Those remain separate acceptance checks (T5.01/T2.x journeys).
- The redirected Codex-container cache (8 entries) was not modified. It
  matters only if the app is launched from inside that container.
