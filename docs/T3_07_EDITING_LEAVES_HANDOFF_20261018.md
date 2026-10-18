# T3.07 editing leaves: layers, rotation and tonal controls

Continuation from `88cb9b374c52e8a751879797f8152771f11af6b8`; initial main
fetch was `83663107d864b21d22d2e746b9ca6eb80d47beac`. Recovery remains
`0d91b70558a995fa7654021de7465b1d3f615e3d`. These are source leaves for the
coordinator's phase composition. No root/controller/build/renderer/precision
file is edited, and the image owner retains the heavy build slot.

Published continuation source checkpoints are rotation
`70992320f67d53d9f7b7f187c211f759b700260d` and tonal controls
`96f7a60a7b7faffa2e07461664749e7107fc33ef` on
`feat/t3-07-saved-views-navigation`. The final fetch found newer image-owner main
`6f3e2f3b9ed7a760d4dd7f4c6f26fa44dc5ad88d`. A read-only `git merge-tree
--write-tree` reports no merge conflicts (combined tree
`3029acb1e3251d6f5d76311e8b2ccfbc8a2d1a35`). Its changes to the native project
wrapper/store are additive; the borrowed core/workflow/affine/raster ports used
here are unchanged. This is source reconciliation, not a combined build pass.
The coordinator retains the actual integration merge and reserved root edits.

## Local-view facade checkpoint

`f37ea514f6036ae236a4f2fe65e7de6dc5157920` adds the previously requested
`vw-ffi/src/local_views.rs`, its six native test cases and `NativeLocalViews.kt`.
They reuse the existing worker, store and `settledWorkflow`. Read-only review,
Rust formatting, Kotlin syntax and staged privacy checks passed. Native tests,
generated-binding compatibility and compilation are unrun. The coordinator
still owns the four-line FFI module/re-export hook and binding generation.
The common factory is injected; the JVM factory is a plain public function,
not an actual for a new common expect. Bookmarks remain image-document only.

## Layer/group source ownership and interfaces

Owned native leaves:

- `core/crates/vw-ffi/src/document_controls.rs`
- `core/crates/vw-ffi/src/document_controls/dto.rs`
- `core/crates/vw-ffi/src/document_controls/plan.rs`
- `core/crates/vw-ffi/src/document_controls/snapshot.rs`
- `core/crates/vw-ffi/tests/document_controls.rs`

Owned Kotlin leaves under `com/visualworkbench/shared/`:

- commonMain: `DocumentControlsApi.kt`, `DocumentControlsInteraction.kt`,
  `DocumentControlDrawing.kt`.
- jvmMain: `NativeDocumentControls.kt`.
- jvmTest: `DocumentControlsInteractionTest.kt`.

The two platform leaves are `LayersPanel.kt` under desktop's `desktop/` package
and Android's `android/editor/` package. They expose visibility, order, opacity,
blend, locks, empty-layer deletion, object targets, groups and moves. They do
not install another document chooser or project manager.

The recovered native adapter constructs the existing canonical operations and
returns the existing immutable `WorkflowPlan`. One explicit edit is one plan,
one transaction and one undo step; preparing does not change canonical state.
Cross-document targets, locked changes, exhausted order intervals and oversized
complete operations refuse. Hidden/locked objects remain listed for recovery.
The recovered create-from-source command shares the unchanged original asset;
it does not import or duplicate image bytes.

Exact native entrypoints:

```rust
pub async fn document_controls_snapshot(
    &self, document_id: String, memory_budget_bytes: u64,
    cancellation: Arc<Cancellation>,
) -> WorkflowResult<DocumentControlSnapshot>;

pub async fn prepare_document_control(
    self: Arc<Self>, binding: WorkflowBinding, metadata: WorkflowMetadata,
    command: DocumentControlCommand, memory_budget_bytes: u64,
    cancellation: Arc<Cancellation>,
) -> WorkflowResult<Arc<WorkflowPlan>>;
```

The common port is `WorkbenchDocumentControls`, with `snapshot(documentId,
memoryBudgetBytes)` and `prepare(binding, metadata, command, memoryBudgetBytes)`.
The injected JVM factory is
`documentControlWorkflows(project: WorkbenchProject): WorkbenchDocumentControls`.
No new common expect declaration or product dependency is introduced.

The coordinator registers only the dedicated module and DTO exports in FFI
`lib.rs`, then regenerates bindings with the phase batch:

```rust
mod document_controls;
pub use document_controls::{DocumentControlCommand, DocumentControlRow,
    DocumentControlSnapshot, DocumentControlTargets, GroupControlRow,
    LayerControlRow, LayerControlUpdate, ObjectControlRow};
```

## Minimal layer caller hooks

Instantiate one `DocumentControlsInteraction(scope, existingMutationMutex,
metadata, mayMutate, committed, ::documentControlWorkflows)`. Metadata uses the
existing installation identity, transaction ID and fresh Lamport allocation;
`committed` refreshes the current root through its normal snapshot admission.

- Call `bind(project, snapshot, epoch, changeSequence)` on every admitted current
  snapshot, including duplicate refreshes. Call `observe(sequence)` immediately
  on OPS notifications before render refresh. The controller owns one bounded
  metadata reader and one mutation task; it owns no project/service/rendering.
- Panel entry cancels current input and respects the root's existing modal,
  AI, instruction draft, semantic, export, transfer and gesture guards. Fence
  competing routes while `active || busy || retry != null`. A retained exact
  retry prevents project/document departure until it is resolved or explicitly
  discarded. Discarding a retry handle does not undo a durable edit.
- Compose `LayersPanel(controls, newId, selectedObjectIds, activate)`. Each
  callback captures `DocumentControlsToken(binding, attachmentEpoch, serial,
  page)` through `capture()`. Do not replace it with a fresh token when delivering
  an old event. This fences same-hash ABA, reload, reopen and page transitions.
  Local field/inspection state is remembered by the full token; local row
  selection also checks the captured token before redirecting later actions.
- Use `documentControlDrawingLayer(snapshot, controls.state.value.drawingLayerId)`
  for every pen, new-object preview and shape-create layer choice. A chosen
  layer hidden/locked/deleted by a peer yields null. Report that the user must
  choose an editable layer; never silently draw into another layer. The native
  layer kind is immutable and the controller chooses annotation layers only.
- Await `detach()` before changing document/page or closing a borrowed project;
  await `close()` before disposing the parent scope. Accepted durable receipts
  are recorded and their plans closed before any fallible root refresh callback.
  Cancellation retains producer ownership and an uncertain exact plan for retry.
- The full native command port also supplies `RenameDocument` and
  `CreateDocumentFromSource`; the coordinator may connect these to the existing
  project actions and refresh the existing navigator. There is no second
  document-selection/lifetime implementation in the panel.

Initially compose the controls for image/capture documents. The page field is
an additional transient stale-callback fence using the existing snapshot page;
it does not grant page-wide layer behavior or introduce a page bookmark schema.
Paged layer/canvas acceptance remains with the document/rendering owners.

## Rotation solver, contact and one-edit lifecycle

Continuation ownership adds isolated `vw-geom/src/object_rotation.rs` and its
native test, FFI `object_rotation.rs` and its native test, JVM
`NativeObjectRotation.kt`, and common/test `ObjectRotationInteraction.kt` /
`ObjectRotationInteractionTest.kt`. Existing `ObjectRotationContact.kt` and
Android `RotationSnapControl.kt` remain the reusable contact and context control.
The native solver/facade were recovered from source candidate
`0d91b70558a995fa7654021de7465b1d3f615e3d`; they compose the existing `Affine`
with pinned libm rather than introducing another transform engine.

Coordinator-only native registration:

```rust
// vw-geom/src/lib.rs
mod object_rotation;
pub use object_rotation::object_rotation;
// vw-ffi/src/lib.rs
mod object_rotation;
pub use object_rotation::rotate_object_transforms;
```

Regenerate the existing Kotlin bindings. The plain JVM function
`objectRotationTransforms(originals, pivot, first, current, snap15)` requires no
common expect/actual declaration. It preflights at most 256 invertible transforms,
left-composes a document-space pivot rotation, and returns absolute transforms.
The delta snaps to 15 degrees from the original pointer vector. Every contact
sample starts from immutable originals; no incremental matrix drift occurs.

Instantiate `ObjectRotationInteraction(scope, existingMutationMutex, newId,
::objectRotationTransforms, current, mayMutate, commit)`. `current` returns one
`ObjectRotationAttachment(project, snapshot, epoch, camera, selectedObjectIds)`
read together from the existing UI owner. The guard must admit this rotation's
own commit while fencing competing gestures/modal/workflow routes. The existing
viewport owner retains handle painting, hit-testing, pointer-to-document mapping
and preview rendering. It calls `begin(pivot, first)`, `move(token, point,
snap15)`, and `finish(token, realUpPoint, snap15)`, displaying `state.preview`
through its current preview lease. Selection, camera, page, attachment, document,
project and full visible revision are fenced again under the mutation mutex.

`commit(ObjectRotationCommit)` uses the existing absolute-edit path with the
captured full binding/page and stable gesture ID, allocating metadata through the
root's existing allocator. Return false for a refused stale edit and true only
for an accepted edit. Finish seals synchronously and calls this callback at most
once for the entire selection; no-op/cancelled drags create no edit. The callback
must settle its native producer before returning or throwing. Cancellation keeps
`busy` until settlement; await `detach()` before changing/closing the borrowed
project and `close()` before disposing the UI scope. An uncertain callback result
blocks new rotation gestures. After admitting a fresh native document snapshot,
call `reconciled()`; this never generates a retry or another undo transaction.
Use the current desktop Shift value or Android snap preference on each sample.

Cheap verification passed: strict shared explicit-API/warnings compilation,
20 focused contact/lifecycle JVM tests, rustfmt on the four new native files,
and the commonMain platform-import guard. The JVM suite covers real up-point and
snap state, immutable originals, one callback, queued stale CAS admission,
cancellation settlement, pre-body parent cancellation, reentrant callbacks and
old-token refusal. Nine geometry and three FFI integration tests are written but
unrun. The FFI oracle compares all exported pixels against independently placed
rectangles, retries the same transaction, performs one undo/redo, reopens the
project and verifies original asset bytes. These sources are not an executed
native, Compose, viewport, export or one-undo acceptance claim.

## Tonal controls and exact composition hooks

Inspection confirmed that model validation already covers brightness, contrast
and five levels parameters, and the existing raster engine composites these
adjustments on normal layers. Those engines, undo operations, asset storage,
document lifetime, viewport and export implementations need no duplicate system.
The new tonal adapter recovers the tone-only portion of the existing source
candidate; output-frame/crop geometry is left with its owner.

Exact new owned paths under `core/crates/vw-ffi/` are `src/tone_controls.rs`,
`src/tone_controls/{dto,plan,snapshot,preview,editing_view}.rs` and
`tests/tone_controls.rs`. Kotlin ownership under shared is commonMain
`ToneControlsApi.kt`, `ToneControlForm.kt`, `ToneControlsInteraction.kt`, jvmMain
`NativeToneControls.kt`, and jvmTest `ToneControlFormTest.kt` /
`ToneControlsInteractionTest.kt`. Desktop and Android editor each add only
`ToneControlsPanel.kt` and `TonePreviewBitmap.kt`. No existing engine, controller,
query, rendering or root composition source was changed.

Coordinator-only FFI registration and editing-query hook:

```rust
// vw-ffi/src/lib.rs, followed by existing binding generation
mod tone_controls;
pub use tone_controls::{ImageToneSettings, ToneControlEntry, ToneControlSnapshot,
    ToneControlCommand, ToneControlPreview};
// queries.rs render_list_for_page, before draw_shape/local_bounds for each object
if crate::tone_controls::is_editing_metadata(project, &document, id) {
    continue;
}
```

This omission recognizes only a valid typed adjustment in its own normal-blend
layer, with identity transform and no group/hidden/role semantics. Unsupported
adjustments remain explicit errors. It keeps metadata out of the vector editing
path that currently refuses `Shape.Adjustment`; it does not bake tone pixels into
the original background. The dedicated panel preview and existing marked export
use the canonical adjustment renderer. Root/viewport composition must preserve
that distinction until its owner supplies any broader live editing view.

Native methods are `tone_controls_snapshot(documentId, budget, cancellation)`,
`prepare_tone_control(binding, metadata, command, budget, cancellation)` returning
the existing `WorkflowPlan`, and `preview_tone_control(binding, command?, region,
assumeUntaggedSrgb, budget, cancellation)`. Shared `WorkbenchToneControls` has
`snapshot`, `prepare` and `preview` with the same fields. The plain JVM factory is
`toneControlWorkflows(project)`; there is no new common expect declaration.

Instantiate `ToneControlsInteraction(scope, existingMutationMutex, newId,
metadata, mayMutate, committed, ::toneControlWorkflows)` and compose
`ToneControlsPanel(controls, activate)`. Offer it only for image/capture documents.
The root retains installation identity, Lamport allocation, project switching,
modal admission and its normal admitted-snapshot refresh callback.

- Feed `bind(project, snapshot, epoch, changeSequence)` on each admitted current
  document snapshot and `observe(sequence)` immediately on OPS before refresh.
  Use the panel's captured full token when delivering callbacks; never substitute
  a fresh capture for an old event. Root guards cancel current input before entry
  and fence competing actions while `state.blocksInput` is true.
- Honor `state.mayLeave` for document/project/page departure and panel closing.
  Dirty private values require Apply or explicit discard. New drafts retain one
  identity pair across previews and Apply. Whole forms validate together; locked
  targets refuse. Rapid field callbacks transform the latest current form.
- Apply/Remove each prepare one immutable existing workflow plan and commit once
  under the root's mutex. A failed or cancelled commit retains that exact plan
  for explicit retry or discard; it never allocates replacement transaction IDs.
  A received durable receipt is recorded and its plan released before any fallible
  refresh. Discarding a retry handle does not reverse saved history.
- Await `detach()` before switching or closing the borrowed project and `close()`
  before disposing its UI scope. Actual reader and mutation producers are joined,
  including cancellation and repeated concurrent close callers. Rebinding cannot
  republish old snapshot/pixel results or adopt IDs allocated for an old owner.

Preview is explicit and read-only, with private or saved settings, region origin
and per-preview untagged-sRGB consent. A single conflated reader admits only the
latest full binding/source/input serial and copies its at-most-512-by-512 RGBA8
tile. Native draft evaluation applies the same transaction through a temporary
canonical sequencer over the visible project; it writes no store, history, event,
blob or offline pending transaction. The verified original decoder and current
whole-document raster engine remain the pixel authorities, followed by exact
cropping and display conversion. It never resizes a source to hide a budget
failure. A complete render that exceeds 256MiB returns typed Memory; the bounded
tile is not a promise that every large original can be previewed. Tagged-source
ICC and result-asset work remain charged; untagged sources do not reserve a
nonexistent ICC parser. Noninterruptible rendering retains producer ownership
until it returns; cancelled pixels are discarded before publication.

## Verification limits and next composition

For this continuation, 20 rotation contact/lifecycle and 34 tonal form/controller
JVM tests pass with the saved Kotlin compiler, warnings denied and strict shared
explicit API. Read-only review findings were repaired and regression cases added.
Kotlin syntax checks cover the isolated platform/native adapters; rustfmt,
commonMain platform-import and staged privacy/whitespace checks pass. These cheap
checks do not compile generated bindings or Compose and do not establish product
Kotlin/ABI compatibility.

Nine geometry, three rotation FFI and seven tonal FFI integration tests are
written but unrun pending the coordinator's shared native build slot. The tonal
cases cover preview/prepare immutability, commit/retry/undo/redo/remove/reopen,
original bytes, stale revisions/locks, explicit color consent, cancellation,
16-bit input, corrupt blobs, predecode memory refusal and offline visible drafts.
Earlier checkpoint receipts retain their own exact check scopes; native layer,
local-view and archive regressions also remain subject to coordinated execution.
No device, paid provider, new credential, binary publication or main merge
occurred. Krita stays deferred, bookmarks stay image-only, and no requirement,
phase gate or end-to-end workflow acceptance is promoted.

Next: coordinator registers/composes these leaves, then runs native and platform
validation using the shared slot (medium). The standing AGENTS executor
recommendation remains `gpt-6.1-sol / xhigh`; delegated leaf executors used those
settings. The root's exact runtime settings are not exposed.
