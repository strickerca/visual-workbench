# Editing/output and assisted-proposal source handoff

Accepted implementation base: `6d505416963fb42009921bd3fb8639994e73752a`.
Grant: `1dc90e2546ab05cb5edf91bf5b969423c07468e4`, section E of
`ROADMAP_PARALLEL_IMPLEMENTATION_GRANTS_20261018.md`.
Branch: `feat/editing-output-assisted-20261018`.
Recovery reference: `0d91b70558a995fa7654021de7465b1d3f615e3d` under
`recovery/source-candidates/ink-core`. No recovered root overlay was applied.

This is a source candidate. Reserved roots, manifests/lock, generated bindings,
all giant paths, existing platform callers and both EditorControllers remain
untouched. Registering these sources alone does not complete the caller journeys.
Krita, actual model admission and all unperformed device/product gates remain open.

## Existing T3.07 workflows: no replacement needed

The accepted base already composes layer/group controls, canonical object
rotation, tonal controls, local cameras/bookmarks and multi-document navigation
through the existing project and mutation owners. Both platform
`EditorWorkflowControls.kt` files compose the navigator, saved-view, layer and
tone panels; both existing EditorControllers own canonical object rotation.
Their prior integration and
24-document headless regression evidence are recorded in
`EDITING_OFFLINE_INTEGRATION_HANDOFF_20261018.md`; those are historical receipts,
not fresh checks of this candidate. No replacement manager or duplicate controls
are added here. The remaining independent editing gap is the nondestructive
document output frame and canonical tonal editing composition. This candidate
supplies the bounded leaves; the reserved root hooks below remain necessary for
the actual canvas/export journeys.

## Exact additive interfaces

The new `ImageFramesApi.kt` defines `ImageFrameSettings` (integer D crop,
clockwise quarter turn, positive O dimensions), `ImageFrameEntry`,
`ImageFramesSnapshot`, Set/Remove `ImageFrameCommand`, `ImageFramePreview`,
`ImageEditingPreview` and `ImageEditingBackground`.

`WorkbenchImageFrames` borrows the existing project with:

- `snapshot(documentId, memoryBudgetBytes)`.
- `prepare(binding, metadata, command, memoryBudgetBytes)` returning the existing
  immutable `WorkbenchWorkflowPlan`.
- `preview(binding, command?, region, marked, assumeUntaggedSrgb, memoryBudgetBytes)`.
- `editingView(binding, region, assumeUntaggedSrgb, memoryBudgetBytes)`.
- `editingBackground(binding, assumeUntaggedSrgb, memoryBudgetBytes)`.

The plain JVM factory is `imageFrameWorkflows(project)`. Matching native methods
are `image_frames_snapshot`, `prepare_image_frame`, `image_frame_preview`,
`image_editing_preview` and `image_editing_background`. Each native method adds
its existing `Cancellation` parameter last. Preview regions are at most 512 by
512. The whole editing background is budgeted at its actual D dimensions, never
resized to fit. Display output is explicitly RGBA8 sRGB with source identity;
canonical originals retain their depth, profile and bytes.

Set/Remove build existing canonical operations, retaining the root's metadata,
Lamport, visible CAS, immutable-plan retry and undo authority. D object coordinates
never change. O crop/quarter-turn/resize has one explicit model convention;
ordinary Crop guides retain their old meaning. Clean output evaluates global
tones and omits annotations; marked output preserves canonical layer order.

## Required native registration and validation hooks

The following are exact coordinator-owned changes; none was applied here.

```toml
# core/crates/vw-raster/Cargo.toml; existing workspace crate, no new third party
vw-geom = { path = "../vw-geom", version = "=0.1.0" }
```

```rust
// vw-geom/src/lib.rs
mod image_output;
pub use image_output::{IMAGE_OUTPUT_MAX_SIDE, ImageOutputFrame};
// vw-model/src/lib.rs
mod image_output;
pub use image_output::{DocumentImageOutput, IMAGE_OUTPUT_CAPABILITY, IMAGE_OUTPUT_LAYER_KIND};
// vw-raster/src/lib.rs
mod image_output;
pub use image_output::{DocumentOutput, ImageOutputMetadata, ImageOutputResampling,
    document_editing_region_reservation, document_output_estimate,
    document_output_reservation, export_document_output, image_output_estimate,
    image_resident_bytes, prepare_document_output, render_document_editing_region,
    render_image_output, render_image_output_region, requires_document_output};
pub use export::prepare_export_pixels_with_output;
// vw-ffi/src/lib.rs
mod image_frames;
pub use image_frames::{ImageFrameSettings, ImageFrameEntry, ImageFramesSnapshot,
    ImageFrameCommand, ImageFramePreview, ImageEditingPreview, ImageEditingBackground};
```

In `vw-model/src/validate.rs`, add `crate::IMAGE_OUTPUT_LAYER_KIND` to the existing
layer-kind allowlist and call `self.validate_image_outputs()?` before successful
project validation. The helper validates every dedicated output layer, including
disabled ones, while imposing no new source-size restriction on legacy projects.

Advertise `vw_model::IMAGE_OUTPUT_CAPABILITY` in `session/link.rs` local hello,
and require it alongside the current mandatory application capabilities before
normal sync admission and bootstrap. Preserve old-peer `upgrade_required`
behavior. Never sync the new canonical layer convention to an unnegotiated peer.
Add the following OR clause to `queries.rs` before draw_shape/local_bounds:

```rust
if crate::tone_controls::is_editing_metadata(project, &document, id)
    || crate::image_frames::is_editing_metadata(project, &document, id)
{ continue; }
```

## Canonical raster mode and provenance hooks

In `vw-raster/src/render.rs`, preserve `render_buffer` as a wrapper passing true,
and move its existing body unchanged into this function with one added argument:

```rust
pub(crate) fn render_buffer_mode(source: &DecodedImage, project: &Project,
    document: &Id, assets: &impl AssetResolver, options: RenderOptions,
    region: RegionRender<'_>, annotations: bool) -> Result<DecodedImage, RasterError>
```

Immediately after each layer cancellation check, skip only
`layer.definition.kind == vw_model::IMAGE_OUTPUT_LAYER_KIND`. Immediately after
the existing adjustment branch, add `if !annotations { continue; }`. Thus the
existing compositor remains the only tone, geometry and result implementation.
The original render_buffer wrapper continues ordinary marked behavior.

Add a defaulted optional field to `ExportMetadata` and initialize existing
metadata literals in export.rs/stream.rs with None:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub image_output: Option<crate::ImageOutputMetadata>,
```

Add this public metadata-aware preparer without changing existing callers:

```rust
pub fn prepare_export_pixels_with_output(source: &DecodedImage, r: &ExportRequest,
    output: Option<crate::ImageOutputMetadata>, cancel: &dyn Fn() -> bool)
    -> Result<PreparedRaster, RasterError>
{
    crate::source::check_cancel(cancel)?;
    if let Some(metadata) = output
        && metadata.frame()?.output_size() != [source.width, source.height]
    { return Err(RasterError::Invalid("image output metadata binding")); }
    let mut prepared = prepare_export_pixels(source, r, cancel)?;
    if let Some(metadata) = output {
        prepared.metadata.source_width = metadata.source_width;
        prepared.metadata.source_height = metadata.source_height;
    }
    prepared.metadata.image_output = output;
    Ok(prepared)
}
```

Keep public `export` as `export_with_output(source, r, None)`. Move its current
body to `pub(crate) fn export_with_output(source, r, output)` and replace just its
prepare call with `prepare_export_pixels_with_output(source, r, output, &|| false)`.
This embeds the truthful D extent and D-to-O mapping before codec encoding.
Post-hoc result JSON does not repair embedded PNG/JPEG/WebP/TIFF provenance.

The existing seekable format owner needs additive
`format_images::encode_buffered_with_output(source, request, output, cancel)`;
the normal `encode_buffered` delegates with None. Thread output through its
existing private encode helper and call the metadata-aware preparer before
embedding provenance. Preserve current BMP refusal until the format owner's
separate provenance admission is composed. No new file-export owner is supplied.

## Required existing export/package routing

Before legacy query-export uses the old D-only route, test
`vw_raster::requires_document_output(project, document, marked)`. When true,
call `image_frames::admit_legacy_export` / `image_frames::legacy_export`; they
retain the current original decoder, compositor and codec policy. The new
`image_frames::prepare_output(state, document, marked, assume, budget, cancel)`
returns the current binding, source identity/extent and one `DocumentOutput` plus
remaining working budget for the existing seekable/no-clobber file owner.

In streaming_ffi preflight and export, O dimensions and O-region checks must come
from the active frame; route required composition through prepare_output and the
existing destination/staging owner. Pass output.frame to the metadata-aware
encoder, subtract retained allocations as required by the existing adapter, and
keep actual encoded/scratch limits and final no-clobber publication. The old
`export_document_png_to` must explicitly refuse an active output frame unless
that call is routed through the output compositor; add this after document
validation to prevent a silent untransformed export:

```rust
if document.project.active_image_output(document.document)?.is_some() {
    return Err(RasterError::Unsupported("image output requires the bounded output compositor"));
}
```

Package compilation remains root-owned. Compose the recovered package mapping
hunks against current lineage/compiler source: use the same ImageOutputFrame
matrix for marker points/bounds and context pixels, refuse a crop omitting an
instruction, and retain original D source authority and checked O mapping. Do
not blindly apply recovered compiler/verification files over current lineage.
Until those package hooks are composed, explicitly refuse active output frames
in that export route rather than silently publishing D packages as O output.

## Exact caller ownership and full-canvas behavior

Construct `ImageFramesInteraction(scope, existingMutationMutex, newId, metadata,
mayMutate, committed, ::imageFrameWorkflows)`. Compose the new platform
`ImageFramesPanel(controls, activate)` for image/capture documents. Feed bind on
all admitted current snapshots and observe immediately on ordered OPS. Preserve
captured full tokens, dirty-form/retry departure guards and state.blocksInput.
Check departure permission before signaling beginDeparture (which intentionally
makes the control unready); signal it before waiting for the root mutation gate.
Do not re-admit UI callbacks during that wait. Await detach
before project/document replacement and close before parent-scope disposal.
The committed callback runs under the shared mutation gate; refresh without
reacquiring that gate, and defer this owner's retirement until the callback
returns. A failed plan close retains its exact retirement owner and blocks
further edits and departure. Keep that owner and borrowed project; retry
detach/close to settle cleanup without recommitting an already saved edit.

Full-canvas tones use the EXISTING background/viewport owner, not the frame-panel
reader. Request editingBackground with the captured complete binding, convert its
returned already-sRGB pixels through the existing bitmap path, and admit only
against the same project reference, epoch, document/page and visible binding.
Invalidate it on OPS before async refresh; refresh it after each accepted change
while tones are active, and restore the ordinary original background after the
last tone is removed. Draw canonical content once: a composed background already
contains annotations/results. Selection geometry and transient UI overlays retain
their current owners. A moved vector must never be painted over its old baked
position; retire the composed display during existing transient geometry edits,
or supply a correctly composed private preview before retaining that route.
The tile editingView port supplies the same D-space composition for the existing
viewport owner; O-frame crop never hides source editing pixels. Giant ownership,
renderer admission and charges remain with their current owner and require
explicit integration; no new tile/provider/cache manager was created here.

## Assisted proposal import and correction

Only four existing source paths receive bounded additive changes: common
`MaskApi.kt`, JVM `NativeMasks.kt`, native `masks/dto.rs` and `masks/edit.rs`.
The new `SelectionOperation.ImportCoverage(width, height, coverage, combine)`
checks full oriented-source dimensions/byte count and reserves input/canonical
working copies before copying or hashing. It uses the ordinary selection target,
version, native mask math, immutable blob, transaction identity and undo history.
No protocol schema or second mutation/selection engine is introduced. Regenerate
bindings once in the phase batch for this additional enum variant.

New common `AssistedSelectionApi.kt` and `AssistedSelectionInteraction.kt` own
one exact `AssistedSelectionSource(SelectionDocument, page, attachmentEpoch)`.
The controller constructor receives the existing WorkbenchSelections, shared
mutation mutex, metadata allocator, currentness predicate, afterMutation callback,
mandatory existing process-pool factory, and optional admitted pins/backend.
`prompt` owns at most 64 real document-space points and one latest queued prompt;
one completed encoder session reuses its actual embeddings. Public proposal
previews copy at most 256-square coverage regions. Explicit Accept imports once,
returns the actual SelectionReceipt and never fabricates a workflow receipt.

Required Android caller composition:

1. Construct from the actual current SelectionDocument; `isCurrent` must compare
   the real project reference, complete binding/source identity, page and epoch.
   Do not use a constant predicate or derive validity from dimensions alone.
2. Map real pen taps through the existing captured-camera mapping, then
   `appendAssistedPenPrompt` and `prompt`. Compose the dedicated
   `assistedselection/AssistedSelectionPanel.kt` without replacing its canvas,
   manual SelectionInteraction, project or command dispatcher. For the main
   canvas proposal overlay, request bounded current viewport regions through
   proposal.tile under its source/sequence fence and reuse the existing overlay
   painter; do not invent a committed SelectionVersion for an unsaved proposal.
3. Invalidate before ordered refresh; await successful close before replacing or
   closing the borrowed project. Preserve the owner and its exact charges after
   failed retirement. Cancellation never credits an unjoined worker as retired.
4. After any attempted import, refresh through afterMutation even if receipt is
   null: the edit may have crossed its durable point. Once the callback returns,
   schedule controller retirement; never synchronously call this same owner's
   close from inside afterMutation (it would join its own operation). Route the
   saved object ID to existing SelectionInteraction.useObject for Add/Subtract,
   invert/expand/shrink/feather and ordinary mask editing.

There is deliberately no selected production backend, model, artifact download
or invented output. Missing pins/backend reports Unavailable and keeps manual
selection usable. Runtime admission must hash real local artifacts and check
closed model/runtime/license admissions before returning inspected execution and
working-set bounds. It may not load weights/embeddings/source buffers before the
existing session-pool reservation. CPU capability is mandatory; only a settled
Unsupported preferred initialization may try that admitted CPU initialization
once. Cancellation/storage/inference failures never loop or choose an unadmitted
runtime. Actual model selection, hash/license/runtime pins, Android implementation
and S23 NPU/GPU/CPU inference/timing remain external dependencies; this source
candidate does not complete SEL-001.

## Checks and proposed deferred-ledger rows

Output checkpoint: `f8a5bc8201873a7688a1b72e3d17c45801a52b76`.
Assisted checkpoint: `6e7c017b6eb5feb21ec7ed2b495afc271fd54fe2`.
Thirty focused ImageFrames form/API/lifecycle JVM cases pass, with warnings
denied and strict explicit API common compilation. The focused harness uses the
saved Kotlin compiler and native expect stubs only; product generated bindings
and Compose are not compiled by this check. Twenty-one focused assisted-source
cases also pass with warnings denied and strict common compilation. Their test
fixtures inject native capabilities/resource charges; no actual model inference
runs. The earlier test-only unnecessary-nonnull warning failure and one failed
retirement-status assertion were repaired, then the full focused suite reran
successfully. Fifty-one focused JVM cases pass in total. All owned Kotlin syntax and commonMain import checks, native rustfmt
and scoped whitespace checks pass. Review findings were repaired with cases for
departure, source-side bounds, reentrant ownership, terminal cancellation and
failed retirement. No expensive native/platform/device run was performed.

The requested independent Astra source review found one further frame-plan
retirement defect: a throwing close could lose its owner and permit reuse.
The final repair retains separate retirement ownership, preserves committed
receipts without recommitting, fences callbacks and joins concurrent cleanup
attempts. Five injected-close regressions pass in the 30-case frame suite,
including discard, commit/refresh, detach, concurrent close and describe failure.
Astra confirmed that finding resolved and found no additional blocker in the
reviewed output/assisted source scope. It inspected source only; this disposition
does not claim native compilation, generated-ABI or caller/device acceptance.

| Scope | Exact deferred command/journey | Prerequisites | Expected result | Status |
|---|---|---|---|---|
| Output engines: 30 written cases | `cargo test --locked -p vw-geom -p vw-model -p vw-raster --test image_output -j2 -- --test-threads=1` | Module, model validation, renderer and metadata hooks above | Exact independent quarter-turn pixels, premultiplied resize/alpha/ICC/depth, legacy guides, D-coordinate preservation, O metadata and cancellation pass | Unrun |
| Frame/import FFI: 10 written cases | `cargo test --locked -p vw-ffi --test image_frames --test selection_import -j2 -- --test-threads=1` | Registered enum/FFI and metadata-aware external codec adapter | Read-only drafts, one undo/redo/retry, source bytes, ordinary correction, stale/lock/cancel/memory/corruption and embedded D/O provenance pass | Unrun |
| Shared and platform callers | Final `:shared:desktopTest`, both-app unit tests, instrumentation source compile and lint | One fresh binding pass, exact root composition, frozen candidate | Current ImageFrames/Assisted tests and existing regressions pass; three Android pen-boundary cases execute; no stale rendered publication/double-paint | Deferred aggregate |
| Full-canvas/export journey | Original + annotations + layered tones -> full D canvas -> private O crop/quarter-turn/resize -> Apply -> PNG/TIFF export -> Undo/Redo -> reopen | Both real platform roots and admitted codec path | Original unchanged; camera/hit tests remain in D; O pixels/provenance agree; guides/transient geometry are explicitly handled and painted once | Unrun |
| Mixed-peer/package behavior | New/old peer capability handshake; package containing marker outside crop; output-frame context/lineage tests | Mandatory frame capability and current package mapper integration | Old peer refuses upgrade before sync; no silent omitted instruction or untransformed export; existing lineage/private-review boundaries retained | Unrun |
| Assisted actual execution | Admitted artifact hashes/licenses -> phone encoder once -> positive/negative pen prompts -> GPU/NPU and admitted CPU fallback -> manual correction -> Undo | Real admitted model/runtime and authorized S23/device slot | Accurate editable mask; measured <=100ms prompts after encoding; cancellation/retirement/source fences and license gate pass | Unavailable external prerequisite; not a feature pass |
| Final policy/runtime | Affected strict Clippy, root license/dependency checks, final ABI/platform build and device/performance journeys | Coherent reviewed integrated tree, one heavy slot/max two workers | Current-source/artifact receipts; no stale ABI or unsupported runtime claim | Deferred to central ledger |

## Exact unapplied seekable-codec hook

This delta is against the accepted base and belongs to the coordinator/format
owner. It is included as reviewable text; protected format_images.rs is unchanged.

```diff
--- a/core/crates/vw-ffi/src/format_images.rs
+++ b/core/crates/vw-ffi/src/format_images.rs
@@ -161,6 +161,17 @@
     file: &mut File,
     cancel: &Cancellation,
 ) -> Result<(u64, vw_raster::ExportMetadata), RasterError> {
+    encode_private_with_output(source, request, None, cap, file, cancel)
+}
+
+pub(crate) fn encode_private_with_output(
+    source: &DecodedImage,
+    request: &ExportRequest,
+    image_output: Option<vw_raster::ImageOutputMetadata>,
+    cap: u64,
+    file: &mut File,
+    cancel: &Cancellation,
+) -> Result<(u64, vw_raster::ExportMetadata), RasterError> {
     if request.format == ExportFormat::Bmp8 {
         return Err(RasterError::Unsupported(
             "BMP standalone provenance is not supported; use TIFF or PNG",
@@ -176,8 +187,12 @@
     let mut admitted = request.clone();
     admitted.memory_budget_bytes =
         subtract_retained(request.memory_budget_bytes, retained_source_bytes(source)?)?;
-    let mut prepared =
-        vw_raster::prepare_export_pixels(source, &admitted, &|| cancel.is_cancelled())?;
+    let mut prepared = vw_raster::prepare_export_pixels_with_output(
+        source,
+        &admitted,
+        image_output,
+        &|| cancel.is_cancelled(),
+    )?;
     prepared.metadata.settings = request.clone();
     let limits = vw_formats::Limits {
         output_bytes: cap,
@@ -206,6 +221,15 @@
     request: &ExportRequest,
     cancel: &Cancellation,
 ) -> Result<ExportedImage, RasterError> {
+    encode_buffered_with_output(source, request, None, cancel)
+}
+
+pub(crate) fn encode_buffered_with_output(
+    source: &DecodedImage,
+    request: &ExportRequest,
+    image_output: Option<vw_raster::ImageOutputMetadata>,
+    cancel: &Cancellation,
+) -> Result<ExportedImage, RasterError> {
     let bound = output_bound(source, request)?;
     if bound > BUFFERED_LIMIT {
         return Err(RasterError::EncodedLimit {
@@ -235,7 +259,8 @@
     let mut file = tempfile::tempfile().map_err(|_| RasterError::Io)?;
     let mut admitted = request.clone();
     admitted.memory_budget_bytes = subtract_retained(request.memory_budget_bytes, copies)?;
-    let (length, mut metadata) = encode_private(source, &admitted, bound, &mut file, cancel)?;
+    let (length, mut metadata) =
+        encode_private_with_output(source, &admitted, image_output, bound, &mut file, cancel)?;
     metadata.settings = request.clone();
     file.seek(SeekFrom::Start(0)).map_err(|_| RasterError::Io)?;
     let mut bytes = Vec::new();
```
