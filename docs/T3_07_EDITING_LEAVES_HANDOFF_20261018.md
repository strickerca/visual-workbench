# T3.07 editing leaves: layer controls and rotation contacts

Continuation from `88cb9b374c52e8a751879797f8152771f11af6b8`; main was fetched
again and remains `83663107d864b21d22d2e746b9ca6eb80d47beac`. Recovery remains
`0d91b70558a995fa7654021de7465b1d3f615e3d`. These are source leaves for the
coordinator's phase composition. No root/controller/build/renderer/precision
file is edited, and the image owner retains the heavy build slot.

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

## Rotation contact model and remaining owner hooks

Owned leaves: commonMain `ObjectRotationContact.kt` and jvmTest
`ObjectRotationContactTest.kt`, plus Android editor `RotationSnapControl.kt`.
This recovers only the contact/frame/target
model, not the original platform handle painters or affine implementation.
It requires an injected solver:

```kotlin
ObjectRotationContact.begin(frame, items, pivot, first,
    solve: (List<Transform>, Point, Point, Point, Boolean) -> List<Transform>)
```

The geometry owner supplies the canonical solver over immutable originals.
No native solver registration, transform math, handle hit geometry, renderer or
precision file is added here. Each real preview solves again from pen-down
originals; stale project/document/hash/attachment/page/camera seals the contact.
Nonfinite samples/results refuse. Finish emits `EditCommand.SetTransform` once.

The root needs only to create `snapshot.objectRotationFrame(epoch, camera)` at
contact start, feed real samples/current frame/current snap state, display the
returned preview through its existing preview owner, then submit the one final
command list through its existing guarded edit/gesture transaction. Cancel on
input/lifecycle/selection changes, and clear root preview state after retirement.
Use current Shift state on desktop; compose `RotationSnapControl(checked,
enabled, changed)` in the existing Android context bar with a root-owned snap
preference read on each sample. The geometry/renderer owners retain handle hit-testing and
painting, native pinned-math solving, export parity and goldens. The model tests
use injected test solvers and do not verify those outstanding capabilities.

## Tonal inspection: no duplicate output or renderer

The actual model already validates brightness, contrast and five levels
parameters, and `vw-raster/src/render.rs` already composites an adjustment on its
own normal-blend layer. Both app canvases still refuse `Shape.Adjustment`.
Consequently a new live tonal action would leave the interactive document
unavailable until its owner composes the existing editing-view path.

Recovery has `ImageOutputApi`, form/controller/panels and the native
`image_outputs` adapter, but that composition also changes model/geometry,
queries, raster output, preview pixels, transfer and rendering paths already
owned elsewhere. None are copied over or forked here. Preserve that recovered
system for the image/renderer owner, including explicit color consent and the
separate original/editing/export representations. This is an unresolved tonal
UI integration dependency, not tonal acceptance or an engine rewrite request.

## Verification limits and next composition

Per-checkpoint JSON receipts bind the exact source and test scope. Focused shared
checks use the saved Kotlin 2.4.0 compiler with warnings denied, strict explicit
API and injected test capabilities; product Kotlin 2.4.20/Compose/native checks
are not implied. Seven native layer/engine cases are written but unrun while
the image owner retains the shared heavy slot. No device, paid provider, new
credential, binary publication or main merge occurred. Requirement and gate
statuses are unchanged.

Next: coordinator phase composition of these roots and native DTOs, then focused
native/Compose validation when the shared slot is available (medium). The
standing AGENTS executor recommendation remains `gpt-6.1-sol / xhigh`; session
settings were not changed and the root's exact runtime settings are not exposed.
