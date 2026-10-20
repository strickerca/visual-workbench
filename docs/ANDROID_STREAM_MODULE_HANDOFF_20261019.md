# Android stream modules — 2026-10-19

Base/contract: `e32c046e5670645a93920ffcd1c62291782b50a1`,
`docs/T3_STREAM_ROOT_CONTRACT_20261019.md`. Branch `codex/android-stream-modules`
uses an isolated checkout; `codex/android-remaining-callers` and its evidence are
preserved. Current main and recovered `47339519`/`0d91b705` have no Android stream
canvas modules to reuse. `.agents` is absent here and `/workspace/.agents` is empty.
Krita remains deferred. This is module implementation, not completed T3.02/A07.

## Exclusive files and behavior

New Android `stream/**` contains seven production leaves:

- `StreamCanvasLifecycle`: serializes platform draw/post/native receipt with
  accepted pen freeze. Pending never-shown work receives false; unknown posting
  seals the view. Exact frozen-frame/lossless identities are checked. Failed
  native/pixel/child retirement remains owned; surface retirement precedes stream
  close. Resume retires only the annotation child; the paired project stays open.
- `AndroidStreamPixels`: reserves bitmap/stride/batch memory before allocation
  and <=256 KiB RGBA row reads; copies exact opaque pixels. Partial/canceled
  allocations join cleanup; failed charge release refuses further allocation.
- `AndroidStreamCanvas`: actual software SurfaceView draw and successful
  `unlockCanvasAndPost` precede positive presentation. Its separate 96 MiB surface
  admission bounds buffers to <=5 Mi pixels. Retirement waits for surfaceDestroyed.
- `StreamInputFence`: uses the existing CanvasGestureRouter/StylusInput. Only
  root-admitted real pen DOWN freezes; hover/palm/finger do not. Interrupted input
  is dropped, first-gesture buffering is bounded and replay batches stay <=512.
  The held first identity survives subsequent physical contacts (including reused
  pointer IDs) until consumed; ignored contacts cannot send orphan MOVE/UP to the
  newly attached editor. Lifecycle/viewport interruption explicitly discards it.
- `StreamSignalObservation`: maps only upstream native SessionFailure.Closed to
  terminal UI state using the last signal identity; no closed-handle query occurs.
  Explicit Closed stops collection. Cancellation, other native failures and
  downstream callback exceptions retain their original meaning. Native reads/new
  input stop; shown/frozen pixels and Save-retry owners await explicit retirement.
- `AndroidStreamSession`: owns joined signal/render/freeze/child/save/resume work;
  verified lossless pixels replace only the latest child's background. Existing
  camera, provisional objects and gesture state survive adoption. Live fit camera
  is passed explicitly to the child so first contact cannot shift by its usual
  fit margin. Window/focus departure revokes admission and cancels contact.
- `AndroidStreamPanel`: LIVE/source or FROZEN/estimated age, independent source-loss
  banner, readiness/retry, durable-save result, explicit degraded confirmation and
  unsaved-draft discard/close confirmation. Composable disposal only pauses.

## Root hooks (unapplied)

`docs/handoffs/android-stream-root-hooks-20261019.patch` applies cleanly to this
base and supplies controller retention/close fences and WorkbenchApp mounting.
It is intentionally **not applied**. Root owns native acquisition, entry action,
all app/controller edits and the borrowed-editor adapter. Prerequisites:

1. One existing root attachment/acquisition job captures exact project reference,
   document/page, editor epoch, link reference and link epoch. An explicit local
   action opens `link.openCaptureStream()`, then `link.captureAnnotationSession`.
   Pass ordinary root ID/metadata allocators and that attachment predicate.
   Network connectivity alone must not invalidate an already retained Save plan.
2. Set `captureStreamOpening` before acquisition; stop parent input and await its
   actual canvas renderer retirement before mounting the new Surface. Retained
   giant/native resources stay charged. Join canceled/late acquisition results.
   Call `AndroidStreamSession.create(context, scope, annotation, stillAttached,
   admitPenDown, openEditor)`: annotation ownership transfers immediately. Store
   `AndroidStreamCreationFailure.owner` for cleanup retry. Register every produced
   owner before cancellation/stale-result cleanup. Clear the acquisition flag
   only after that job settles; republish the parent scene on failure/close.
3. `openEditor(project, CaptureDraftInfo, StreamEditorViewport)` must return an
   `AndroidStreamDraftEditor` over the **existing** editor implementation, ready
   at `viewport.initialCamera` (no second fit). Skip ordinary/giant/Result background
   loaders and publish null-background scenes; the stream module owns those pixels.
   Borrow the temporary project;
   child close retires jobs/scenes/input and never closes the paired project or
   calls project.close. `sealEdits()` is idempotent and joins accepted edits before
   Save. Restrict tools to native draft Save's supported annotations: no Results,
   raster masks, tone/image-frame changes, semantic references or vector eraser.
4. Reuse existing real-input/modal/session admission for `admitPenDown`; exclude
   only the stream's own modal marker. Keep parent Air Actions/root input blocked.
   Back invokes `owner.requestClose()` so the panel confirms unsaved discard.
   Close/join the stream and any acquisition job **outside** the root mutation
   mutex before link/project/page replacement, revoke, disconnect or app teardown.
   A failed close keeps the owner and blocks replacement. No new FFI/registration,
   manifest, permission, source enumeration or publisher approval belongs here.

## Checks and open gates

Run `python3 apps/android/src/test/kotlin/com/visualworkbench/android/stream/check_stream_modules.py`.
The offline probe compiles the non-Compose modules against Android SDK signatures,
actual common stream/draft/state APIs and the existing input code; unrelated
expect factories and the editor renderer boundary are substituted. It runs the
original 18 lifecycle/input cases plus 13 router/signal/closed-lifecycle boundary
regressions (31 total). Logs and source hashes stay in its
printed `/tmp/vw-android-stream-probe-*` evidence directory. This is not a JNI/app
build. Final passing evidence is `/tmp/vw-android-stream-probe-jbbtw1ac`;
source manifest SHA-256 `91013490a4d0d13467f4a502b53b3d5d085fe2ef21522074104c2a6625920327`,
run log SHA-256 `7043ccdbd7c1ddb2591c9d6b48f91e1c2b91d950f8cc3a604990192ff09c7e8b`.
Compiled platform/test source hashes match current files. Exclusive staged paths,
secret-pattern scan, root patch applicability and `git diff --check` pass. Four real Bitmap
instrumentation cases are authored but unrun. Compose/JNI/app/device/provider
execution, actual Surface presentation/retirement, lossless visual swap with wet
ink, source-close/reconnect/Save journeys, S23 stylus/Bluetooth, 15 fps, eight-frame
memory, <=300 ms, physical/owner acceptance all remain unverified. No heavy build,
install/device action, runtime admission, release or main merge occurred.
