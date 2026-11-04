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
both opens. The cause is **inferred, not proven by a receipt**: MSIX app-data
virtualization. The likely sequence is that the build and app were launched
from inside the Codex desktop app's package container, and Windows redirected
the child to `%LOCALAPPDATA%\Packages\<family>\LocalCache\Local\Visual Workbench`.
Supporting observations:

- The 10-component shape matches that redirect exactly.
- A read-only listing on 2026-11-04 found the cache only under the Codex
  package's `LocalCache\Local` (8 entries), with no `%LOCALAPPDATA%\Visual Workbench`.
  That listing was not retained.
- The retained receipt below records `environment_anchor_visible=false` from
  this (Claude) session and the cache's location under the Codex package.

No receipt was taken from inside the Codex container, which is what would show
the redirect directly. The observed shape rules out 8.3 names, case, a `\\?\`
prefix, a trailing separator and a junction (reparse tag 0); OneDrive does not
redirect `AppData\Local`. The desktop app already admits a one-child
redirection (`NativeCacheAnchor`) and uses the canonical path for the MCP
cache. Only the ad-hoc helper did not, and that helper was never committed.
The new preflight is stricter than `NativeCacheAnchor`, which only requires the
canonical anchor to start with LOCALAPPDATA and end in `Visual Workbench`. The
preflight also requires the exact
`Packages\<family>\LocalCache\Local` shape, a valid family name and file-object
identity.

**Consequence.** The cache very likely depends on the launcher's container. A
run started from another packaged agent app, or from a plain PowerShell window,
would use a different `Visual Workbench` directory with its own capacity count.

**`tools/mcp_cache_preflight.py`** (read-only; tests in
`tools/tests/test_mcp_cache_preflight.py`):

- Path identity accepts syntax-only, case-only and the single MSIX redirection
  of the `Visual Workbench` child, and only when a reopen of the final path is
  the same file object. Reparse points, a different object, other packages,
  redirection anywhere else, a descendant that leaves the redirection, and
  unexpanded 8.3 names still refuse.
- The consumer guard refuses on:
  - any `VisualWorkbench.exe` / `VisualWorkbenchDev.exe` process, wherever it
    is installed;
  - any `vw-mcp-host.exe`, `vw-mcp-package.exe`, `vw-codex-host.exe` or
    `vw-mcp.exe`, wherever it is;
  - any image under the cache base (requested or canonical form) or under an
    explicit `--packaged-root`;
  - a `node.exe` descended from a Workbench app or a task-owned root
    (`--owned-root-pid`);
  - a runtime-named process whose image is unreadable or not a drive path.

  Only `--exclude-pid` exempts a process, after owner review. Other `node.exe`
  processes are counted and ignored. **Limitation:** a `node.exe` installed
  outside the cache that runs a script from the cache is not detectable by
  image path. It is reported only if it descends from a Workbench or
  task-owned process. Snapshot walks that end with any error other than
  ERROR_NO_MORE_FILES refuse. A PID that exits after the snapshot is dropped.
- The dry-run plan lists entries and proposes the fewest task-owned entries to
  retire: incomplete entries first, then the oldest. Entries without a valid
  owner marker, the requested key and entries in use are always kept. While
  any Workbench app runs, incomplete entries are kept, because they may be a
  live extraction. A requested key that exists but is incomplete is reported
  as needing owner action. Nothing is moved or deleted; the owner approves
  and moves entries by hand.
- Every run writes a JSON receipt under `.local/mcp-cache-preflight/` with
  redacted paths. Every failure, including snapshot, scan and OS errors, is
  recorded as a structured refusal.

Retained dry run: `.local/mcp-cache-preflight/preflight-20261104T091138Z-66515db6.json`
(sha256 `b4a02514…f2c6`, local only and gitignored). It used
`--local-app-data` pointed at the Codex package's `LocalCache\Local`, run from
this Claude session, with manifest key `3ec32279…` from the 2026-10-20 receipt.
Results:

- Status `ready_for_owner_review`, `environment_anchor_visible=false`.
- Guard clear: no Workbench app running, 4 unrelated Node processes ignored.
- 8 entries, 1 slot needed. Proposed: `5ca6cb7d…`, which has an owner marker
  and no `ready` marker.

The earlier receipt `preflight-20261104T091111Z-d3c49eb1.json` is superseded:
it showed the cache base relative to the override root. Neither the cache nor
any process was changed. The packaged two-cycle smoke has still not run.

## 2026-11-04 B8: MCP start verification cost

**Measured cause.** The packaged smoke spent `bundle_verify_ms=59226` on the
first start and timed out in `bundle_verify` on the second. A read-only replica
of `McpRuntime.verify` was run against the existing extracted bundle on this
laptop (1,514 files, 119,499,486 bytes, Defender real-time protection on). It
took 64.2 s:

- 32.7 s in the per-file `noRedirect` check;
- 30.6 s in the same check during the final inventory walk;
- 0.96 s opening, reading and SHA-256 hashing.

`noRedirect` calls `toRealPath` on every ancestor of every path. On Windows,
each `toRealPath` does one directory lookup per path component, so each start
cost O(files x depth^2) lookups. Defender, per-file opens and buffer size were
not the cause. Sequential 64 KiB hashing of every file took 1.4 s. Holding
read-only, no-write/no-delete handles on every file did not change the replica
time (54.7 s).

**Change.** Every start still opens and hashes every admitted byte against the
application-resource manifest, in manifest order. The redirect check now runs
`toRealPath` on the bundle root once, then walks the tree once. The walk takes
each entry's name as stored and reads that entry's own attributes again
(links are never followed). It refuses any symlink, junction or other reparse
point, any special file, any unlisted file, and any directory at the depth
limit. The stored relative names must equal the manifest keys exactly. A
single case-variant or 8.3 manifest name was already refused by the old final
walk. The exact comparison also refuses what the old code accepted: two
manifest names that alias one stored file. `DesktopMcpBundle.parse` already
refuses such case-insensitive duplicates, so the bundle path was not exposed.
`verify` now also applies the manifest path grammar itself (charset, reserved
DOS names, trailing dot, depth). The same inventory is checked again after
hashing.

Files are then opened by path. Between the walk and each open, `verify`
relies on the caller's inventory pins: `DesktopMcpBundle` holds
no-write/no-delete handles on every runtime file and directory, checked against
their final paths, from before verification until the bundle closes. The old
per-ancestor checks were also only point-in-time checks under the same pins.

The replica of the new procedure took 0.74 s on the same bundle (walk 64 ms,
hash 617 ms, re-walk 45 ms). The follow-up that re-reads each entry's
attributes was measured at 1.9 s total (walks 226 ms and 169 ms) while a
distribution build was loading the laptop. Nothing reuses an earlier verification. There is
no cross-process trust cache, no metadata-based trust and no security setting
change. Bundle pins, inventory and markers are unchanged.

**Expected packaged timing (not yet measured in the app).** Each start should
take about 3.9 s inventory + 1 s verify + 1.1 s service open. Both starts of the
two-cycle smoke should then finish well inside the 90 s budget. Only a packaged
`--mcp-runtime-smoke` run can confirm this.
