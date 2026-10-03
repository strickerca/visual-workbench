# T2.01 Android integration design

This is an implementation handoff, not a completed UI or acceptance result.
Authority is the T2.01 plan row and EDIT-005, EDIT-006, EDIT-007 and EXPORT-003 in
the v2 requirements, together with BUILD_SPECIFICATION section 4.3. The frozen
`vw-mask` crate supplies deterministic mask math; it does not supply transactions,
history, bindings, editor tools or vector-stroke splitting. The T1.12 dependency
and phase gates remain open while independent software work proceeds.

## Shared/native contract before UI controls

Add a project-owned selection facade that runs on the existing native project
worker and resolves `ProjectState.project()` so offline queued edits are visible.
Use the same `EditOptions`, authenticated local identity, operation counters,
idempotent transaction IDs and durable queue/ack semantics as other edits.
Expose typed selection commands rather than protobuf or mask math to Kotlin:

- Rectangle, lasso and painted paths, plus a combine mode (replace/add/subtract/
  intersect). Inputs are f64 oriented document coordinates, finite and bounded.
  Native code owns 1/256-pixel quantization, clipping and coverage generation.
- Invert, expand, shrink and feather; explicit radius in document pixels. The
  current algorithm uses an integer Euclidean disk and finite uniform-disk
  feather. Its provisional policy must remain visible in the versioned contract;
  Kotlin must not substitute a Gaussian or Android blur.
- A mask snapshot containing object ID, immutable version identity, content hash,
  document/grid dimensions, bounds, role and paged visible tile data. Each tile
  has grid coordinates, actual edge dimensions and straight 8-bit coverage.
  Cap a tile response to a fixed byte budget and reject stale snapshot tokens.
- Mask/cutout file export with integer crop, PNG depth/profile policy, cancellation,
  output and scratch budgets, and a visible-state revision receipt matching the
  existing streaming export boundary.

The first operation creates or updates a selection object on an unlocked mask
layer. Persist the strict `VWMASK01` encoded mask as an immutable content-addressed
blob and bind its metadata/dimensions to the model. Commit the reference change
as one transaction only after the complete blob is durable. Previous blobs stay
reachable from history. Undo/redo restores references through normal ops; it
does not mutate a mask in place. The mask library's numeric `version` is ancestry
depth, not a globally unique project version ID. Use project transaction/object
identity for history and sync; deduplicate identical content separately.

Before exposing this API, confirm the exact model selection-raster representation
and asset format validation. Any required schema/operation extension must be
explicit and migrated. Never tuck mutable state into unvalidated metadata or
store selection history only in Android preferences.

## Android interaction and rendering

Add rectangle, freehand lasso and mask paint tools once the facade exists. The
tool context has replace/add/subtract/intersect, brush radius/opacity where
applicable, and the current semantic role. A separate mask actions sheet contains
invert/expand/shrink/feather. Destructive changes require one normal undoable edit,
not a confirmation dialog for each brush movement.

Latch the existing core camera at pen-down, convert historical/coalesced samples
to document coordinates and send bounded batches. The library currently paints
constant-radius capsules; do not claim variable-pressure mask painting until its
algorithm is defined and tested. Keep previews ephemeral, cancelable and tagged
with the source mask version. Replace stale preview requests rather than letting
them accumulate. Pen-up commits one transaction; focus loss, palm cancellation,
back navigation or an exceeded queue budget discards only the unfinished preview.

Render the overlay from native coverage tiles using nearest document-pixel
sampling at 1:1. Camera/display transforms may scale its presentation; they must
not change saved coverage. Use a visible viewport tile cache with a byte cap and
clear it on document/version changes. The overlay indicates role and remains
distinct from image pixels. Both Android and desktop consume identical tile
bytes and hashes, rather than recreating masks with platform paths or filters.

Present three explicit eraser behaviors:

- Object: the existing precise swept-geometry hit test emits whole-object deletes.
- Mask: a paint gesture subtracts native mask coverage and creates a new version.
- Stroke: a deterministic `vw-ink` split operation must intersect the swept eraser
  with saved strokes, preserve source sample/brush semantics, and atomically
  replace the original with remaining vector fragments. Fragment IDs and inverse
  data must be retry-stable. That split API is currently missing; whole-object
  deletion cannot stand in for this requirement.

## Bounded export boundary

Mask export writes exact grayscale PNG8 values with no ICC color transform.
Cutout export uses the original oriented source pixels, retains RGB (including
RGB beneath zero alpha) and multiplies alpha by coverage with the native integer
rule. Preserve source ICC and 8/16-bit depth. Empty selections and out-of-grid
regions need explicit, tested behavior before enabling the export action.

The existing `cutout_rgba8/16` functions take a whole source image. A 50 MP RGBA16
buffer alone is 400 MB, so the app must not call those full-frame functions for
large files. Add a native strip adapter over `RasterSource` that reads mask tiles
for the same integer region and applies the exact alpha rule, then streams PNG
through the existing bounded writer. Account retained mask tiles, source scratch,
strip workspace and encoded output separately; no whole-frame Kotlin arrays or
silent downscale. Native owns scratch handles, while the settled Kotlin transfer
wrapper owns provider copying and cleanup.

## Required regression and acceptance evidence

- Native reference fixtures for every combine/filter operation within one 8-bit
  level, fractional/off-grid/self-crossing paths, edge tiles, inverse operations,
  invalid inputs and work/memory limits. Compare exact Android/Windows mask hashes.
- Real-core transaction, idempotent retry, cancel, undo/redo, reopen, concurrent
  ownership and offline sync/rebase tests retaining previous immutable versions.
- UI tests for all three selection tools, tool switching, mixed pointers, palm
  cancellation, rotation/recreation, mirrored rail, semantic role, accessible
  labels/touch targets and disabled actions while native work is pending.
- Separate object, mask and stroke eraser fixtures; canceled erasure changes no
  saved pixels/objects, and undo restores the exact prior document hash.
- PNG8/16 alpha and source-ICC cutouts, orientation, crop origin, original-byte
  preservation, tiny budgets, output failures and cleanup. Strip/full reference
  parity and a real bounded 50 MP export are required before closing the clause.

All device, performance, usability and phase-gate conclusions require the
parent's actual serialized runs and the owner's applicable acceptance work.
