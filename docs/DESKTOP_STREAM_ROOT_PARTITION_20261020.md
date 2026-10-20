# Desktop receiver integration partition

Base: verified main90c0ee22d5e7d94a56c54068f85bf63ffa481b68. This is the next
implementation partition, not a claim that its callers or rendering are present.
Root owns every path below except the five reviewed leaf modules and their tests.
The original Desktop owner and its Astra reviewer retain presentation repair.

## Admission and exact file boundary

Latest fetched candidate: dc24312788e60ec7960df2a761d2de5995c5c4ef. Parent reports
27 focused cases and actual-JDK destination/memory witnesses; original Astra
acceptance is pending. Do not activate it, or older held candidates, before that
verdict. Promote only the accepted files under these two directories:

- `apps/desktop/src/main/kotlin/com/visualworkbench/desktop/stream/`:
  DesktopStreamCanvas, DesktopStreamDestination, DesktopStreamPixels,
  DesktopStreamSurface and DesktopStreamView.
- `apps/desktop/src/test/kotlin/com/visualworkbench/desktop/stream/`:
  DesktopStreamDestinationTest/JdkWitness, DesktopStreamPixelsTest,
  DesktopStreamSurfaceTest/JdkWitness and DesktopStreamViewTest.

Do not overlay the candidate's older Main, EditorController, shared APIs,
Gradle files, protocol, native registration or lockfiles. Root's current tunnel,
publication, capture/MCP and failed-retirement integration must survive.

Root's narrow caller partition is:

| Path | Required implementation |
| --- | --- |
| `apps/desktop/build.gradle.kts` | One shared JVM-option list for application and Test tasks, with both exports below. Verify packaged launcher config when Windows app-image/package execution becomes available |
| `desktop/DesktopStreamSession.kt` (new) | Own receiver annotation plus renderer/child-editor admission, exact contact/freeze/original/Save/resume sequence and joined close; retain late/failed results before cancellation checks |
| `desktop/StreamDraftEditorAdapter.kt` (new) | Borrow only the native temporary project and preserve its exact camera/document. Seal/join accepted edits; dispose editor jobs/render state without closing the borrowed native project |
| `desktop/EditorController.kt` | Retained receiver/session slots and acquisition job; exact project/document/page/generation/LiveAttachment/native-connection fence; temporary-editor install/settle/dispose methods; parent command/input exclusion and retirement before link/project replacement |
| `desktop/EditorCanvas.kt` | Explicit transparent draft-only rendering: allow a deliberately absent background, omit the ordinary white page fill, preserve existing annotation geometry/input. Never enable this for ordinary documents |
| `desktop/DesktopStreamPanel.kt` (new) | LIVE/source and FROZEN/source/age, lossless/degraded truth, Save/retry, explicit degraded consent, discard/resume, close/discard confirmation and cleanup retry |
| `desktop/Main.kt` | Explicit local receiver entry, panel mount and modal/shortcut/input routing; retain existing publisher, tunnel, capture picker and native-unload ordering |
| Focused new Desktop tests | Root acquisition/retirement and draft render/Save/lifecycle cases below; reuse accepted leaf tests rather than duplicate them |

`desktop/` in the table abbreviates
`apps/desktop/src/main/kotlin/com/visualworkbench/desktop/`.

Required runtime AND test options supplied by the Desktop owner:

```text
--add-exports=java.desktop/sun.java2d=ALL-UNNAMED
--add-exports=java.desktop/sun.java2d.d3d=ALL-UNNAMED
```

Root application.jvmArgs and Test.jvmArgs must receive the same values. The
packaged runtime already includes java.desktop, but that alone does not grant
these exports. A missing export must keep positive presentation unavailable;
never add guessed-success or reflective access bypasses. Actual packaged Windows
launcher configuration and D3D behavior remain separate verification.

## Existing interfaces and ownership sequence

No new public DTO, protocol field, FFI registration, generated binding, Cargo or
Android change is required by this partition. Use the current public
`ProjectLink.openCaptureStream()` and `captureAnnotationSession(...)`.

1. Snapshot the exact project, document/page, editor generation, LiveAttachment,
   preview transition and actual native connection epoch before acquisition.
   Seal ordinary input/commands; retain every receiver, annotation and renderer
   before cancellation or stale-attachment checks. Close late results through
   those retained slots. Existing LiveCaptureConnectionFence demonstrates the
   reconnect/conflated-status boundary; do not use status text alone.
2. Construct DesktopStreamView with the current core/stream/attached predicate.
   This view borrows the stream. Root owns the annotation/receiver chain and
   closes it only after its renderer and child editor actually retire.
3. For a valid pen DOWN, stop/settle actual live draws through
   `stopForContact(viewPoint)` before `annotation.freeze()`. Preserve the returned
   displayed frame and source point; verify the draft describes that exact frame.
   Use view-local coordinates and the view's mapping without a second DPI scale.
   Do not replay rejected palms, secondary contacts or stale/modal input.
4. Install a private borrowed editor at that exact frozen camera. Register its
   owner before install suspends. Its published background is deliberately null;
   the retained stream raster owns pixels. The current ordinary EditorCanvas
   early-returns without a background and paints an opaque white page, so simply
   reusing it unchanged would hide or omit the stream. Transparent draft mode
   and actual AWT/Compose composition/input ordering must be verified together.
5. On original arrival, call `view.adoptLossless(annotation)` with the same
   camera/draft and preserve provisional/committed annotations. Never reload a
   PNG into an uncharged second background or replace the child project.
6. Save seals new child mutations and joins all admitted ordinary edit work
   before `annotation.save(...)`. Reserve its admitted maximum4096-operation
   counter range before immutable plan preparation; keep the same metadata/plan
   on unknown-outcome retries. Android ProjectRepository.streamLamport is the
   current rule. Source/transport loss permits retained-plan receipt inspection,
   not new preparation against a different attachment. Degraded Save needs the
   explicit consent already required by the shared state machine.
7. Resume/discard retires the renderer and child borrow before
   `annotation.resumeAfterEditorDisposed()`. DesktopStreamView.stopLiveDraws
   permanently cancels its worker; it has no resume method. Create a fresh view
   over the same retained native stream after successful explicit resume. Failed
   close retains the old owner and blocks replacement/counter-credit reuse.
8. Root departure joins admitted acquisition/operation work, retires renderer,
   retires child borrow, then closes annotation/stream before link/project or
   native runtime unload. Use DesktopLiveLink.beforeRetire and controller
   departure paths. Do not wait for child Save/edit queues while holding their
   parent edit mutex. UI disposal is not retirement evidence.

## Focused acceptance after source admission

Run one combined Desktop batch after the integrated root candidate is ready;
reuse current generated ABI and native artifacts because no public contract is
changing. New bindings are needed only if implementation proves that assumption
wrong. No historical full-suite or per-small-commit regeneration is requested.

Required root cases: cancellation before/during each acquisition; late success;
failed renderer/child/annotation close and retained retry; project/page switch;
same-LiveAttachment reconnect before observation; valid contact exactly once;
no-frame/refused contact; lossless swap preserving stroke/camera; Save while edits
settle; unknown Save retry; explicit degraded consent; closed-source retained
Save; discard/resume creating a fresh renderer; modal shortcuts/commands blocked;
transparent annotation draw and input over the retained background.

Run the accepted stream module suite, new root suites and existing
DesktopStreamPublication/DesktopSessions/DesktopAndroidTunnel regressions in that
batch, with the required exports. Keep the Linux Windows-runtime packaging
exclusion explicit. Real Windows D3D/Swing/Compose composition, pen input,
retirement, DPI and packaged launcher checks remain unverified until executed.
No >=15fps, memory/latency, source-loss/Save/resume or owner gate is passed here.

## Other exact remaining boundaries

The independent missing root release-assembly entry is being implemented on
`cloud/release-assembly-hook-20261020`; its accepted assembler is otherwise
unchanged. The separate Gradle/MSI build hook, complete admitted Windows runtime
inputs and actual packages remain outstanding; MSVC lib.exe is absent here.
E transfer remains CONNECT403 before bytes and cannot use an alternate route.
Model/checkpoint/TinyViT/native aggregate/runtime/memory and configured scrcpy
DLL/replaceability/notices admission remain closed. Accepted provider/Air/
diagnostics leaves are not new source tasks merely because their live/provider/
Bluetooth/Windows journeys are unrun.

The explicit outstanding owner decision is INSTR-005: on-canvas handwriting near
a marker becoming an instruction. T5.08 requires either a dated next-release plan
or an owner-approved drop. Current requirements defer recognizer evaluation;
there is no instruction to implement handwriting now or evidence approving its
removal. Optional PSD/PSB/AI composite and RAW/DNG preview source already exists;
its OS/runtime journeys remain separate from a decision to drop those formats.
Krita and broader deferred Phase4 remain outside current scope.
