# B4: black canvas after eraser resize and Activity recreation, 2026-11-04

Branch `task/b4-canvas-present-20261104` from `main` 961cd80. Reproduction test
`adcf122`, fix `b562578`. Device evidence:
`docs/evidence/B4_CANVAS_PRESENT_20261104.json`.

## Symptom

On the S23 Ultra (SM-S918U, Android 17) the retained replay frame
`flow-013` (2026-10-20) showed the whole drawing region solid black while the
Stroke eraser controls were visible. Drawing frames from the first test never
went black. The post-recreation test only checked native state and a valid
Surface, never pixels.

## Root cause

The canvas exists only in the AndroidX `CanvasFrontBufferedRenderer` child
layers. The SurfaceView's own surface is never drawn. It is opaque RGB_565 and
has a black background layer, and the window cuts a permanent hole for it once
its first redraw handshake finishes. So whenever no renderer layer holds a
committed buffer, the region is solid black, without even the surround.

Two app paths created that state:

1. **Resize, break-before-make.** Eraser controls (and the busy progress bar,
   selection controls, Pen re-selection) change the height of the weighted
   canvas box. `surfaceChanged` released the presented renderer and built the
   successor only after the asynchronous AndroidX release callback, a Main
   post, construction and a first draw. The released renderer's
   `SurfaceHolder.Callback2` stayed in SurfaceView's per-update callback
   snapshot and dropped `drawingFinished`. Each occurrence logs
   `LowLatencyCanvas: Attempt to render to the multi buffered layer when
   CanvasFrontBufferedRenderer has been released`.
2. **New SurfaceView, hole before first frame.** The app callback was a plain
   `SurfaceHolder.Callback`. The renderer is created during the callback
   dispatch, so it is not in the snapshot. SurfaceView therefore finished its
   redraw immediately and the first window frame cut the hole over an empty
   layer (Activity recreation, editor open).

## Reproduction (before the fix)

`EditorSurfaceInstrumentedTest#eraserResizeAndRecreationPresentCanvasPixels`
runs these steps: draw, select the eraser (the canvas shrinks), recreate the
Activity, select Pen (the canvas grows). The post-recreation wait in
`textSelectionMoveResizeEraserUndoAndActivityRecreationUseRealCore` now uses
the same `assertPresented`.

`PixelCopy` cannot see child SurfaceControls, so `assertPresented` polls
`UiAutomation.takeScreenshot()` in memory and recycles each bitmap at once. It
requires all of these:

- the page area is white (±12 per channel);
- every surround probe matches the theme surround (±16);
- under 1% of the rail-free canvas band is near-black;
- no sample during the observation window is 90% or more black.

Results on the S23 through `build.ps1 hil-test app` (full suite, 226 cases),
receipt `app-hil-3503bf8c….json`:

| Phase | Samples | Fully black samples | First presented |
|---|---|---|---|
| baseline | 31 | 0 | 0 ms |
| after eraser resize | 30 | 1 (the first sample, t=0) | 28 ms |
| renderer after recreation (existing test) | 26 | 1 (t=0) | 114 ms |

224 of 226 cases passed. Both surface cases failed with `AssertionError`. The
logcat had 14 `LowLatencyCanvas` "released" warnings, one per resize.

## Fix (`CanvasSurface`, `CanvasGenerations`)

- **Make-before-break on size change.** The presented generation (renderer,
  buffer pools and scene slot) is kept, and a successor is created at the new
  size from the new viewport scene. The kept generation is released only when
  both of these hold:
  - the successor's first visible multi-buffered frame is *committed*
    (`SurfaceControlCompat.Transaction.addTransactionCommittedListener`;
    before API 31 the completion callback signals directly), and
  - the kept generation's own AndroidX resize redraw has completed. That
    redraw carries SurfaceView's `drawingFinished`; releasing earlier would
    quit the kept handler thread before AndroidX runs it. The redraw is
    detected as the first completion on a layer other than the pre-resize
    one.

  If that redraw never completes, a bounded fallback releases the kept
  generation 500 ms after the successor presented. Main waits at most 150 ms
  in `surfaceChanged` (the measured wait is logged as `VW_CANVAS
  resize successor_wait_ms=`), so AndroidX's own resize of the kept
  generation, which runs from its holder callback right after ours, finds the
  successor's frame already committed underneath.
- **Bounded overlap.**
  - Generations: at most two (kept and successor). No generation is
    allocated while any release settles.
  - Buffer pools: AndroidX rebuilds the kept generation in place at the new
    size and releases its pre-resize pools asynchronously. So up to three
    pool sets exist transiently (successor, kept resized, kept pre-resize),
    and two until the kept generation leaves. This is normally 1-3 display
    frames plus the release.
  - A second resize before the successor presents rebuilds both generations
    in place, transiently up to four pool sets.
- **Rapid successive resizes.** A resize while a replacement is still
  settling is deferred. The presented generations resize in place through
  AndroidX, whose own layer swap can leave a gap of one render. One coalesced
  make-before-break replacement follows once the release settles. That
  replacement is not redundant: an in-place AndroidX rebuild can lose a
  completion callback and wedge the generation's dry-frame accounting.
  `CanvasReplacement` holds this state machine and has JVM unit tests.
- **Giant mode** keeps break-before-make: its single surface reservation is
  never shared. Its redraw completion stays immediate (giant frames are
  hidden until their tiles are current). Pre-existing: a giant resize still
  releases inside the SurfaceView dispatch, so that renderer's redraw
  completion is dropped as before.
- **Redraw handshake.** The holder callback is now `Callback2`. SurfaceView's
  `drawingFinished` is held until this canvas has committed a frame, bounded
  by one 750 ms fallback that later requests do not restart
  (`CanvasRedrawHandshake`). The canvas host box paints the surround
  (`EditorDrawing.surroundArgb`), so a SurfaceView without its hole shows the
  surround, never black.
- **Releases join as a batch.** `CanvasGenerations.joined()` completes only
  after every overlapping release (for example a kept release and a window
  detach) has settled. It then fails with the first failure. With nothing
  outstanding it reports the last batch.
- **Document frames.** The kept generation only follows a newer scene with
  the same document frame. A restored generation whose document frame changed
  is retired, using the same rule as `publish`.
- **Constructor failure.** If the AndroidX renderer constructor throws, the
  session closes and its reader lease is released.
- **Retirement confirmation.** A window-detached retirement now confirms its
  `CanvasRetirementOwner` hook even when it completed exceptionally. That is
  only possible after AndroidX released the pipeline: the document reader
  close failed, and that frame keeps its own quarantine. Before, the failure
  gated every successor canvas forever. Project retirement (`retireAll`)
  failure semantics are unchanged.

## After the fix

Same harness and device, receipt `app-hil-af2c5415….json`: **226/226 passed,
0 skipped**, cleanup all true.

| Phase | Samples | Fully black samples | First presented |
|---|---|---|---|
| baseline | 28 | 0 | 5 ms |
| after eraser resize | 24 | 0 | 0 ms |
| after recreation | 34 | 0 | 30 ms |
| after Pen resize | 27 | 0 | 0 ms |
| renderer after recreation (existing test) | 20 | 0 | 117 ms |

There were 0 `LowLatencyCanvas` "released" warnings, against 14 before.

A repeat run (receipt `app-hil-37b1e2b9….json`, identical main APK
`61551d3d…`) also passed 226/226 with 0 fully black samples in all five
phases (25-32 samples each) and 0 warnings. In the before run the eraser test
stopped at its first failing phase, so its recreation and Pen phases were not
reached.

Unit tests: `:android:testDebugUnitTest` had 43 suites and 240 tests, all
passing. That includes the new `CanvasGenerationsTest` (5) and
`CanvasRedrawHandshakeTest` (3), and the unchanged `CanvasRetirementOwnerTest`
and `CanvasDryFramesTest`. `tools/app-test` Python: 53 tests OK.

## Evidence handling and limits

- No screenshots or video were retained. Screenshots exist only in memory
  inside the test. Filtered logcat text was used for the per-phase counts,
  then deleted; only counts and hashes are kept.
- The `.hil`/`.hil.test` packages were uninstalled by the harness after every
  run (`pm list packages --user 0` checked). The normal app was untouched.
- Native libraries were reused from the main checkout (built after the last
  core commit `9108b0f`) by copying them into the worktree's `target/`. No
  Rust compile ran; the license gate's `cargo deny check` ran as part of the
  harness.
- **Review follow-up (code only, not yet built or run).** The independent
  review of `b562578` found nothing blocking. Its findings are addressed in
  the follow-up commit, but builds were paused when it was written: it is
  **not compiled, not unit-tested and not device-tested**. The device results
  above belong to `b562578`. The follow-up also tightens the pixel assertion:
  it now fails on any contiguous near-black strip of at least 2% of the
  canvas rows, not only on samples that are 90% or more black.
- **Still break-before-make:** a document-frame change (`publish`), a giant
  resize, and a restored generation whose document frame changed. These
  release inside the dispatch, as before.
- `CanvasRetirementProgress` reports only the most recent release, so with
  overlapping releases the timeout diagnostic shows the latest one.
- If the kept generation never completed a frame before the resize, its
  first later completion is taken as the resize redraw. In that rare case the
  redraw completion can still be dropped.
- **Not verified:**
  - Giant-mode and document-page resizes on device (no fixture exercises
    them).
  - Rotation, split-screen and fold window-manager-synced resizes.
  - Behaviour when the successor misses the 150 ms bound (a brief gap is then
    possible).
  - The follow-up commit's behaviour on device (see above).
  - Physical S Pen use.
  - Owner visual acceptance.
  - Sampling cadence was about 25-40 ms. A gap shorter than one sample
    interval could escape detection; a persistent or ≥1-sample gap cannot.
