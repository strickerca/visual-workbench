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
Signal beginDeparture before waiting for the root mutation gate; await detach
before project/document replacement and close before parent-scope disposal.

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

To be finalized after focused checks. No model/runtime artifacts are downloaded
or selected by this lane. Existing manual selection remains authoritative.

## Checks and proposed deferred-ledger rows

Focused results and final candidate identity are filled at source freeze. Native
registration/bindings, broad compilation/Clippy/licensing, platform/device and
end-to-end acceptance are deferred to the coordinator's aggregate batch.
