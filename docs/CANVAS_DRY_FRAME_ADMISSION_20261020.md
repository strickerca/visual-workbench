# Bound dry frames through buffer completion

The S23 replay at `397a8852bef495c2f723147b28cd06571fd570fa` reported document
attempt 2 at `SurfaceRetirement/Running`, canvas generation 2 at
`SurfaceChanged/ReleaseRequested`, and close attempt 2 at `DocumentOwner/Running`.
No failure milestone was set; cleanup passed and the normal app was unchanged.
These are parent-reported observations. Cloud has not inspected the raw receipt
or a renderer-thread stack, and has not confirmed the device's active API path.

Independent source inspection found a concrete application admission defect.
`dryQueued` reopened at the beginning of `onDrawMultiBufferedLayer`, before
hardware completion. In pinned graphics-core 1.0.4's API34 implementation,
multi-buffer acquisition and hardware completion share one HandlerThreadExecutor.
The default pool contains three buffers. Four admitted draws can leave acquisition
waiting for a buffer while the completion work that would publish earlier buffers
is queued behind it. A renderer release queued on that executor also cannot run.
This schedule is demonstrated; attribution to the measured S23 wait is pending.

The repair keeps one application dry request pending through actual buffer
completion, coalescing newer scenes in the existing latest-scene slot. Each
request has object identity. Constructor/Surface redraws are counted too: an
anonymous completion cannot acknowledge an application request that has not yet
been drawn, and overlapping draws drain conservatively in either completion order.
The existing completion-to-Main replay draws the newest scene after admission
opens. Session retirement seals the gate and ignores later accounting callbacks.

All accounting remains under the session lock. Gate counts grant no retirement
authority. The actual library release callback, reader close and deferred
completion body are unchanged. The 12-field timeout contract and existing bounds
are unchanged. Arbitrary framework-only redraw storms are outside this app gate.

Focused Kotlin compilation includes the actual changed Surface and helper with
cached Android/shared/bindings dependencies. All 95 focused JVM tests pass,
including nine new gate/scheduling cases and existing document, draw-lease,
retained-Surface, close, workflow and LocalViews suites. The finite old/new pool
schedule is synthetic; it does not execute AndroidX or a GPU. Four controller
warnings match unchanged base source. Independent Astra source review is clear.
The source-bound [receipt](evidence/CANVAS_DRY_FRAME_ADMISSION_20261020.json)
records exact library/source and validation hashes.

Local should rebuild the exact source and replay the same focused case with its
existing assertions and time bound. Keep all 12 timeout fields. If release remains
pending, capture the active CanvasRenderThread stack before cleanup and retain
only sanitized finite evidence. BufferPool acquisition versus a fence wait gives
a specific next investigation. No runtime acceptance or physical gate is passed
by this source repair. Local owns main promotion; cloud publishes branches only.
