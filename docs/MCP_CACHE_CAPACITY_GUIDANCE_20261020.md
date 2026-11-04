# Explain bounded MCP cache capacity

The new Windows image (reported hash prefix `82ae4aa`, published source
`a89df0041dd17a2a85bbd679217ffec6b6681007`) refused `first_start` in
`bundle_extract` after 6 ms. Local inspection found exactly eight ordinary,
non-reparse cache children and no child for the requested manifest SHA256
`bbd54f8afd89296922856700503388ebe1a66741e5ba6e231e308dee772ede80`.
The manifest reports 1,514 files / 119,499,486 bytes. These are parent-reported
measurements; cloud has not inspected the raw receipt.

This proves the original intentional capacity check explains this refusal. The
hash counters measure later verification, not extraction amount. Persistent app
cache entries survive the smoke harness's private log/process cleanup. The older
image's second-start timeout remains a separate unresolved observation.

The app previously suggested rebuilding for every bundle refusal, which cannot
free a cache slot. This change preserves the original count and strict smoke
failure while carrying an internal `cache_capacity` detail only from the actual
capacity check. The existing app issue now explains the limit and asks the build
owner to review cache ownership and use before anything is removed. An existing
verified manifest key still reuses its bytes at capacity. No eviction, deletion,
limit increase, alternate runtime, new UI or verification bypass is implemented.

Eight new synthetic-resource cases verify the capacity boundary, unchanged prior
bytes, no payload open/new key, active leases, existing-key reuse, incomplete-key
marker failure, forged-detail rejection and existing app/smoke behavior. Fresh
focused compilation includes all 25 non-Compose MCP production sources so default
constructor changes do not rely on stale callers. JUnit runs 33 cases: 30 execute,
three existing actual-Windows methods stop at their assumptions, and none fail.
The collector's 41 tests pass. One delicate-API warning matches unchanged base
source. Dependencies are cached; this is not a fresh package or Windows run.
Independent Astra source review is clear. Exact source, test and log bindings are
in the [receipt](evidence/MCP_CACHE_CAPACITY_GUIDANCE_20261020.json).

Local owns cache disposition and disk headroom restoration. Close relevant
Workbench/agent owners and review exact cache ownership/use before removing any
entry. This patch frees no slot. Restore task-owned disk headroom without lowering
reserves, then rebuild and run the unchanged two-cycle smoke after a reviewed slot
is available. Neither the older timeout nor Windows acceptance is passed by the
guidance change. Cloud publishes a branch only; main promotion remains local.

## 2026-11-04 follow-up: B8 preflight root cause and read-only plan

**Root cause of `canonical_path_changed`.** The approved action's helper pinned
`%LOCALAPPDATA%\Visual Workbench\...` and required each handle's final path to
equal the requested string. The retained read-only diagnosis
(`WINDOWS_MCP_CANONICAL_READONLY_DIAGNOSIS_20261020.json`) shows the first
difference at the `Visual Workbench` child: 6 requested components, 10
observed, no reparse tag, and the same volume serial and 128-bit file id on
both opens. That is MSIX app-data virtualization. The build and app were
launched from inside the Codex desktop app's package container, so Windows
redirected the child to
`%LOCALAPPDATA%\Packages\<family>\LocalCache\Local\Visual Workbench`. A
read-only listing on 2026-11-04 confirmed that the cache exists only there
(8 entries) and that `%LOCALAPPDATA%\Visual Workbench` is absent. It was not
8.3 names, case, a `\\?\` prefix, a trailing separator, a junction or OneDrive.
The desktop app already admits this one-child redirection (`NativeCacheAnchor`)
and uses the canonical path for the MCP cache. Only the ad-hoc helper did not,
and that helper was never committed.

**Consequence.** The cache depends on the launcher's container. A run started
from another packaged agent app, or from a plain PowerShell window, uses a
different `Visual Workbench` directory that has its own capacity count.

**`tools/mcp_cache_preflight.py`** (read-only; tests in
`tools/tests/test_mcp_cache_preflight.py`):

- Path identity accepts syntax-only, case-only and the single MSIX redirection
  of the `Visual Workbench` child, and only when a reopen of the final path is
  the same file object. Reparse points, a different object, other packages,
  redirection anywhere else, a descendant that leaves the redirection, and
  unexpanded 8.3 names still refuse.
- The consumer guard refuses when a process image lies under the cache base
  (requested or canonical form) or under an explicit `--packaged-root`. It
  also refuses when a runtime-named process from a task-owned tree is still
  alive, or when such a process's image is unreadable. Unrelated Node tools
  are counted and ignored. A PID that exits after the snapshot is dropped.
- The dry-run plan lists entries and proposes the fewest task-owned entries to
  retire: incomplete entries first, then the oldest. Entries without a valid
  owner marker, the requested key and entries in use are always kept. Nothing
  is moved or deleted; the owner approves and moves entries by hand.

A 2026-11-04 dry run against the Codex-container cache (manifest key
`3ec32279…`) proposed one slot: `5ca6cb7d…`, which has an owner marker and no
`ready` marker. The guard was clear: 9 unrelated Node processes were ignored
and no image was under the cache. Neither the cache nor any process was
changed. The packaged two-cycle smoke has still not run.
