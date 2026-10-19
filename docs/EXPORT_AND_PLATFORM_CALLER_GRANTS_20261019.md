# Export and platform caller completion

This extends the existing file grants; the parent activates work in the named
existing lanes. Root retains native registration, shared contracts/adapters,
Cargo/Gradle manifests, lockfiles, generated bindings and integration. No worker
may overlay older platform roots. Start from this document's publication after
accepted composition134b421. E/G and P grants remain exclusive and unchanged.

## Root contract, implemented in this publication

```kotlin
public suspend fun imageOutputExportGeometry(
    project: WorkbenchProject, binding: WorkflowBinding,
    sourceRegion: SelectionRegion?,
    memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL,
): ImageOutputExportGeometry
```

The common DTO contains `binding`, `sourceAssetId`, oriented D
`sourceWidth/sourceHeight`, full O `outputWidth/outputHeight`, O `region`, and
actual regional/full `encodedWidth/encodedHeight`. A null source region means
full output and remains null. A positive integer D rectangle is intersected
with the enabled canonical crop, quarter-turned and resized with integer lower
floor/upper ceiling coverage. Empty intersections refuse. An absent/disabled
frame is identity, including ordinary source extents beyond32768. Malformed
frames still refuse. Pixel availability, color consent, depth/alpha/matte,
source format and export budgets remain independently checked by preflight.

The native query `ProjectSession.image_output_export_geometry` checks the exact
project/document/host_seq/state_hash on its existing worker, canonical workspace
budget, cancellation and close. It reads saved metadata without opening the
frame panel, decoding pixels, creating a child owner or changing the project.
The JVM factory holds the existing project lifetime borrow through joined
cancellation. Failures use the existing WorkflowFailure family; a stale binding
never silently adopts a new output frame. The sourceD→O math lives only in
`vw-geom::ImageOutputFrame::source_region`.

## F export helper extension

Existing owner01a14eeb-8086-76af-9670-dd5ffa5eb032 retains exclusive ownership:

- Desktop `DesktopTransfers.kt`, `DesktopTransferTest.kt` and NEW dedicated tests.
- Android `editor/ExportSelection.kt` and dedicated export-selection tests.

Implement `internal suspend fun projectExport(request: ExportRequest,
project: WorkbenchProject): ExportRequest` in DesktopTransfers. Preserve the
immutable original D intent separately from the resolved O geometry so retries
never project an O rectangle twice. Capture expected binding/source dimensions,
epoch/settings/selection/camera before the destination picker; resolve the saved
frame under that captured binding before preflight/export. Projected request
`options().region` and `width/height` must be O and actual encoded dimensions.
Match original UI intent against current state and result/cache against the
same projected request. Full output also uses O dimensions.

For Android expose `internal suspend fun projectExportOptions(project:
WorkbenchProject, binding: WorkflowBinding, sourceOptions: ExportOptions):
Pair<ExportOptions,ImageOutputExportGeometry>` from ExportSelection.kt. Only its
sourceOptions.region is interpreted as integer D input; validate all coordinates
as finite, nonnegative integral in UInt range before conversion. Return an
immutable options copy with the O region plus the revision-bound geometry. The
caller keeps original intent/options separate. No second transform in Kotlin.

Fix Desktop completeExport's generic regex search: framed receipts contain
nested `image_output.output_width/output_height` as well as top-level encoded
dimensions. Read bounded JSON top-level fields, rejecting duplicate top-level
keys, malformed/trailing data and wrong types without matching nested keys.
Preserve complete revision/source/digest/size validation. Include nested-frame
receipt success, duplicate-top-level refusal, changed-revision refusal and
asymmetric rotated selection/view/full dimensions in focused tests. Return this
helper interface to the parent/R and Android caller owners promptly.

## R Desktop controller extension

Parent activates the existing R owner01a15161-4bf5-71d5-bfe6-f25904e4b024 under
DESKTOP_OUTPUT_CALLER_HANDOFF_20261018.md. Its three Desktop editor roots remain
exclusive; F's transfer leaf remains separate. Wire F's projectExport under the
existing edits/current-binding checks before every preflight/export route,
including share/cached handoff. Carry the projected request into the actual
receipt and cache checks; keep picker capture and departure guards. Consume
F's suffix-intent/native format information and real RAW preview creation,
visible preview/flattened notices and unavailable-original refusal. Route
PDF-compatible AI through PDF, not ordinary raster. Complete the already granted
frame panel and full-D tonal viewport lifecycle. Report every source hook that
still needs root work; a visible panel alone does not complete actual exports.

## Desktop Main/capture extension

Existing owner01a15080-f8b9-7495-9194-b33660abf005 retains Desktop Main.kt,
desktop/capture/** and their dedicated tests; shared CaptureRegionEditor remains
unchanged unless its exact original task needs a correction. V's mcp/** remains
V-owned and read-only. Implement the real callsites in verification/INTEGRATION.md:

- At each MCP owner replacement attach McpVerificationOriginalAccess backed by
  the existing controller ownsVerificationOriginal/withVerificationOriginal/
  withCurrentVerificationOriginal methods. Keep exact source/CAS/lifetime checks.
- At successful capture destination delivery, pass the actual target, receipt,
  opened project's info and document through NativeMcpCaptureEngine.editorOrigin
  and the already-started MCP owner's rememberEditorCapture. Add the narrow
  callback seam if needed. Never infer target/grant identity or start MCP merely
  to remember a capture. Remember delivery even when later semantic collection
  fails. Optional callback failure reports verification unavailable, preserving
  the transferred project; it must not close successful destination ownership.
- Join MCP shutdown before controller/native release. Add meaningful callback,
  delivery-error and owner-replacement tests. Do not capture or input to live apps.

Root's sticky ordinary close error repair is distinct from retrying failed
pre-shutdown child drains. Do not blindly retry closeSession/destroy after a
worker's completed shutdown. Preserve V's exact retained owners and new sticky
failure result. D/O verification is still refused by V until its actual alignment
implementation exists; these callbacks do not grant transformed verification.

## Android caller extension

Parent sends to the existing Air owner01a1507a-7956-7594-8687-9e7a9a18900b, which
already owns MainActivity, EditorController, CanvasSurface, WorkbenchApp,
preferences/manifest, Air and remote roots. Extend narrowly to
`editor/EditorWorkflowControls.kt` and NEW dedicated caller tests. Native JNI,
NativeCore/shared APIs, Gradle dependencies/notices, E's backend and F's
ExportSelection remain root/E/F-owned. Preserve the accepted Window key callback.

Finish actual frame panel/full-D tonal lifecycle and use F's
projectExportOptions for preflight/export/share/ticket flows with immutable
binding/settings and correct O dimensions; never use a cached frame panel as
export authority. Gate frame/original controls truthfully for RAW previews.
Consume supported suffix-intent/source-format/preview notices as on Desktop.

Wire the genuine giant content-URI intent into `openGiantContentUri` through
AndroidGiantImports and the existing prepare/build/publish flow, preserving
explicit untagged-sRGB consent and returned original ownership. Root's URI
runtime admits before resolver.open, holds the strong reader before the attempt,
joins read/close and quarantines failed retirement. Do not pre-spool the URI in
Java or route RAW into the fixed JPEG giant producer. Ordinary bounded image,
RAW-preview and PDF-compatible AI import remain separate. Return precise
Activity/intent/MIME and ownership checks, including cancellation/replacement.

Assisted selection remains a real delivery gap: consume E's admitted backend
factory/currentSource interaction hooks only after root supplies exact converted
model artifacts/catalog admission/dependencies/notices. Empty catalog means
unavailable; do not claim inference, fabricate hashes or substitute synthetic
production models. Android owner may wire the honest capability/lifecycle now
and return the precise remaining root/model artifact requirements.

## Verification and publication

Keep cheap phase-level focused tests and prior failures. No broad native/Gradle,
binding regeneration, packaging or device execution per small commit. Return
reviewable logical commits, pushed branch/SHA, exact changed files, executed test
receipts and all unimplemented versus unrun paths. Parent/root integrate reviewed
returns. No credentials/provider calls, paid Actions, signing, deployment,
security/driver/device identity changes or real capture/input. Krita and physical
acceptance stay deferred. No old worktree cleanup before source disposition.

Root executed three independent region test methods and a locked Linux vw-ffi
library check for this contract. Native workflow integration and new generated
JVM ABI remain authored/unrun until the final coherent validation batch.
