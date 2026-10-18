# T3.03 / T3.04 precision source candidate

Status: implementation source, not integrated, compiled, executed, or physically
verified. No image capture, device action, network request, paid attempt, key,
clipboard, or existing project mutation was performed to create this candidate.
No dependency or version changes. Parent is the only build/device executor.

## Behavior and authority

- `vw-ffi::PrecisionLens` wraps the existing canonical `vw-geom::LensTransform`.
  The saved base affine is never reconstructed on close. Math is bounded,
  synchronous, uses `try_lock`, admits at most 512 samples, and owns no worker,
  I/O, document, event subscription, or network handle.
- `PrecisionController` is owned by the UI dispatcher. Inspection uses a separate
  render-only native matrix, 4× contact/available hover, mirrored above-left/right
  placement, edge avoidance and a 16-physical-pixel nib exclusion. If no safe
  rectangle fits, it explicitly hides rather than covers the nib. Hover exit or
  unavailable hover clears the reticle. Pane size is at most 512 physical pixels.
- Editable lens 2/4/8× has its own D-to-viewport affine. Down chooses inset or base
  input once; crossing the inset boundary never changes that choice. Toggles,
  moves, resize and magnification changes queue with last-request-wins semantics
  until UP/cancel. The base Camera object and its values stay unchanged. Closing
  restores its exact view. Explicit base-view changes cancel contact and close
  the local lens; project replacement resets it; final controller disposal frees
  the native handle.
- Samples are converted to existing base-camera pixels **before** entering the
  existing Android stroke queue. Pressure, tilt, orientation, time and identity
  survive. Predictions use the same latched route, do not move the aim, and retain
  the existing non-broadcast disposable prediction path. Desktop Canvas uses
  distinct raw-input wrappers; existing pointerUp's internal move receives an
  already-mapped base point and is not mapped a second time.
- Each marker gesture captures the active D-to-physical matrix for both 4-pixel
  box classification and canonical 12-pixel semantic snapping. It survives queued
  lens close and asynchronous marker planning. This composes with semantic UI R2.
- The Android generic primary barrel button holds an inspection peek; a release
  within 250 ms requests a lens toggle. Physical hold time is retained across
  hover-to-contact transition. The existing optional semantic inversion modifier
  moves to the secondary barrel button, leaving its explicit UI toggle available
  when that button is absent. Samsung Air Actions and the broader unified
  single/double/custom button dispatcher remain a separate unfinished requirement.
- Both apps expose lens toggle, 2/4/8×, size, draggable headers, handedness, loupe,
  smooth/nearest mode, and a base-viewport/lens-footprint overview. Pane drawing
  clips and reuses existing immutable source bitmap and canonical saved/wet object
  geometry, including compound highlighter/marker passes. It allocates no second
  full-resolution bitmap. Pixel-grid calculation is render-only and bounded to
  at most 258 integer document-grid lines, omitted below 8 physical pixels/pixel.
  Lens preferences are currently session-local; persistence is not claimed.

## Integration

Copy only the new files listed in the candidate manifest. Apply `integration.patch`
after instructions UI R2, focus R2, focus-app R2 and semantic UI R2. `BASES.json`
binds actual ROOT inputs and the composed in-memory predecessors. `make_patch.py`
only reads them and emits these two owned artifacts. It refuses ambiguous/drifted
contexts; it never modifies canonical or predecessor source.

R2 repairs physical hit admission: Android reuses the actual `EditorDrawing`
selection/text/marker geometry with the raw contact, latched D-to-physical matrix,
and matching physical scale. Its Region clip remains the actual viewport, so
content visible in the lens beyond the base viewport remains selectable.
Both platforms test resize corners in physical pixels; canonical edit/ink points
still use the original single remapping boundary. The new `PrecisionHit.kt`
helpers are called by the real controller hooks and covered by 2/4/8x fixtures.

The eight literal targets are core `lib.rs`, Android `EditorController`,
`EditorDrawing`, `CanvasSurface`, `WorkbenchApp`, and desktop `EditorController`,
`EditorCanvas`, `Main`. Shared source discovery needs no Gradle dependency change.
The app/private coroutine ownership, camera geometry and existing mutation queues
remain authoritative.

Selection, vector eraser and AI app patches are independent siblings and must be
composed deliberately by the parent. Keep marker-specific painting ahead of the
eraser ordinary-object transform. Route any later raw eraser/mask/selection input
at the same single pre-queue boundary; retain its captured physical matrix for
screen-pixel tolerance. AI Result replacement-alpha painting must remain inside
the reused document renderer, so panes show the same canonical composite. Keep
all later modal/picker guards; these controls create no modal or permission flow.
Capture/MCP/HEIC patches are separate and not overwritten by this candidate.

## Written verification, not runtime results

34 regression cases: 8 Rust, 12 shared JVM (using actual native geometry),
9 Android instrumentation and 5 desktop rendering/physical-admission cases. They cover
exact restore over scales/rotations; 200 deterministic 60-sample mid-toggle traces;
outside-down crossing; malformed/bounded input; queued mapping; unchanged
inspection samples; prediction metadata; physical snap calibration; all-edge and
mirrored placement; grid work caps; hover-to-contact hold classification; actual
clipped source pixels, compound alpha, 2/4/8x physical selection/text/resize hit
radii, and lens-visible geometry beyond the base viewport. Synthetic bitmap fixtures are created
and disposed in memory, never saved as verification screenshots.

The 200 mathematical traces are not 200 completed app edit/undo/network gestures.
One-frame appearance, S23 hover rate, first-contact ≤1 physical pixel, physical
barrel dispatch, low-latency rendering cost, actual duplicate-commit/undo traces,
stylus hover across Compose draggable headers, and human placement/comfort remain
unverified. A03/A04/A05 and PEN-004/005/006/007/008/013/016 are not passed by this
uncompiled source candidate. Full-window/DPI/resizing and cancellation acceptance
must be exercised after central composition. No performance target was relaxed.

R1's exact 18-file source and manifest are preserved outside the worktree under
`.handoffs/t3-precision-r1-preserved-20261003`. R2's generator accepts an explicit
hexadecimal `--revision` and reads Git blobs without changing the checkout; this
handoff uses `0930c7b`, whose 17 canonical input hashes match the frozen R1 bases.
This prevents concurrent Phase 2 imports from changing the reviewed baseline.
The final literal patch still layers after the four stated predecessors. During
selection/eraser/AI composition, preserve the raw physical hit scene and Result
hit branch as well as the single edit-sample remapping boundary.
