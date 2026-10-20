# Nested document and renderer retirement probe

This candidate is based on the exact published S23 replay source
`cb29511566e7eb316b7e1fbb1bc401db12724299`, tree
`344125cbd0a10acbae1a58699b3ddeca2578469b`. The parent reported receipt
`app-hil-fresh-diagnostic-64ee1c9ac0544a16b0ca803e5e065fcb.json`, SHA256
`2534728860f691768adfa64256fb0526a36a873b285b73af813573f2375c1422`.
Cloud verified that source tree but did not inspect the raw device receipt.

The reported timeout snapshot was close attempt 2, `DocumentOwner/Running`, no
failed stage, busy true, no pending edits and no displayed message. Cleanup
subsequently passed. This narrows the wait to `EditorDocuments.detach()` but
does not distinguish its controller jobs, document gate, renderer callback or
retained frame readers. Independent source reviews found no deterministic
deadlock or lost callback to repair from that snapshot alone.

This candidate adds observations at those existing boundaries. It preserves
all waits, final writes, mutation admission, task cancellation, draw leases,
renderer release and retained failed-owner behavior. It does not establish a
causal repair or pass the original Surface test.

## Exact callback ownership

The document snapshot distinguishes task join, document gate acquisition,
unpublication, surface retirement, frame-reader retirement and cache cleanup.
Each unfinished detach retains only its latest immutable snapshot. Completed
attempts leave the registry; a suspended successor becomes visible when the
active attempt finishes.

Each actual renderer release receives its own identity token and bounded
generation label. Its immutable atomic snapshot records release request,
library callback entry, posted main callback entry, reader close and completion
publication. The library callback marker is visible before the main callback
runs. Old tokens cannot overwrite a newer generation, including counter wrap.

A project close may encounter a renderer release already started by a Surface
resize. It keeps that release's original generation and reason. Showing the
busy bar can change layout and trigger `surfaceChanged`; source supports that
mechanism, but it has not been shown to cause the S23 timeout.

The aggregate getter observes the exact hook currently being awaited, including
detached Surfaces still owned for retirement. A failed aggregate retains the
same observation. Observer exceptions cannot change retirement results. The
exact pinned graphics-core artifact's queue cancels pending requests but still
awaits an in-flight frame before invoking its release callback; the probe never
substitutes a marker for that callback or releases a reader early.

## Instrumentation handoff

The harness does not read logcat. The local timeout reporter must run on main
and replace its existing manual close formatter with:

```kotlin
val closeDetail = editor.retirementTimeoutSuffix
```

That getter reads `documentRetirementProgress`, `canvasRetirementProgress` and
`closeProgress` once each, then returns these exact ordered groups on one line:

```text
doc_attempt=1 doc_stage=SurfaceRetirement doc_status=Running doc_failed_stage=None canvas_generation=2 canvas_reason=SurfaceChanged canvas_milestone=ReleaseRequested canvas_failed_milestone=None close_attempt=2 close_stage=DocumentOwner close_status=Running close_failed_stage=None
```

A null snapshot group uses counter `0` and `None` for its remaining fields.
Live counters are 1 through 2147483647 and wrap; each belongs to its component,
not a shared project or device identity. The original four `close_*` fields
remain last and retain their published vocabulary. Fixed enums contain no IDs,
paths, drawing data or exception text.

The full getter/field/enum contract is
`evidence/DOCUMENT_RENDERER_RETIREMENT_SCHEMA_20261020.json`. The local lane owns
the instrumentation reporter and parser changes. This production branch does
not itself add those fields to the device's instrumentation output.

## Focused validation and next evidence

The source-bound receipt is
`evidence/DOCUMENT_RENDERER_RETIREMENT_PROBE_20261020.json`. Focused checks compile
the actual modified controller, document owner, Surface and aggregate owner;
they exercise document wait/failure/cancellation observation, callback visibility
before main delivery, stale generations, an existing pending release, exact
detached-hook selection and formatter literals. Existing project workflow,
LocalViews, document-controller, tile and draw-lease ownership tests are included.
All 86 focused tests passed, including 18 new cases. Compilation emitted four
warnings whose source lines are unchanged from the exact replay base. The three
setup policy checks passed. Independent Astra source review found no actionable
ownership or ordering regression. Cached dependencies are not a fresh APK or a
GPU/JNI/S23 execution result.

After local reporter/parser integration, replay the same focused S23 case with
its existing assertions and 45-second bound. Retain all three snapshot groups.
If document retirement is awaiting a Surface and the library callback remains
absent, capture the corresponding renderer-thread stacks while the wait is
active. If the library callback arrived but main did not, investigate delivery;
if reader close began, inspect that retained lease. Select a minimal behavior
repair only from the demonstrated wait, then add its deterministic regression.
Windows retains final integration and main promotion. Cloud makes no main write.
