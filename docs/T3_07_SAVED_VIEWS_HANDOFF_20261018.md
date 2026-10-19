# T3.07 saved views and document navigation leaves

Base: `83663107d864b21d22d2e746b9ca6eb80d47beac`, fetched and fast-forwarded
before editing. Recovery: `0d91b70558a995fa7654021de7465b1d3f615e3d`, fetched
by exact object ID. Worktree: `/workspace/vw-t3-07`. This checkpoint is a
bounded implementation slice, not a completed T3.07 or product gate.

## What the current implementation already covers

Current `CoreApi.kt` already supplies the complete document IDs in `ProjectInfo`,
document snapshots, layers' display properties, object transforms and ordinary
undo/redo. Both controllers already own project lifetime, serialized mutation,
revision checks, original image loading and export. Android already rotates the
camera through its existing viewport gesture. PDF/SVG page, text and export
controls are already composed in `EditableDocumentControls` and `EditorDocuments`.
Those paths get no changes or duplicate implementation here.

The public edit API still has no layer/group mutation commands. Neither root has
the recovered object rotation handle. An adjustment shape is still refused by
the interactive canvas. Source model support for layers, groups and adjustment
parameters is not a usable layer or tonal UI. EDIT-003, EDIT-009 and EDIT-012
remain open; their recovered candidates must be admitted through the coordinator
with the geometry, renderer and native schema owners. This slice does not expose
controls for unsupported edits or replay an old controller postimage.

CORE-009 lacks a chooser and saved views in the actual roots. This slice adds
the reusable leaves for those workflows. It borrows the current project and
catalog, and never creates or closes a project, engine or document manager.

## Owned source

- `core/crates/vw-store/src/local_views.rs`: recovered bounded per-installation,
  per-document camera/bookmark settings. Original camera bits survive storage;
  compare-and-set revision plus operation ID/digest make the latest exact retry
  idempotent. Limits are 128 documents per installation, 32 bookmarks per
  document, 256 UTF-8 bytes per name and 16 KiB per settings row.
- `core/crates/vw-store/src/lib.rs`: only the local-view module/export registration.
- `core/crates/vw-store/src/archive.rs`: only the recovered exact-namespace delete
  before the existing final VACUUM. Local names/cameras stay out of exported
  archives; the live database and unrelated settings are preserved.
- `core/crates/vw-store/tests/local_views.rs`: 11 recovered store regressions,
  including 24 documents/two installations, reopen, camera bits, cancellation,
  no canonical history/hash change, exact retry, limits and archive privacy.
- `apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/LocalViewsApi.kt`:
  dedicated local-view models and injected borrowed capability. No new common
  expect declaration, generated binding or change to existing shared schemas.
- `apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/LocalViewsInteraction.kt`:
  recovered controller with one conflated camera slot and one explicit action.
  Detach joins the actor; ambiguous results retain the identical request for
  retry. Dismissal does not cancel an accepted write. All bookmark/retry UI
  actions now carry the attachment epoch, adopted lists are owned, malformed
  revisions are refused and uncertain outcomes are described as unconfirmed.
- `apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/DocumentNavigator.kt`:
  chooser state over the existing complete catalog, a captured revision/epoch
  request, and `loadNavigationTarget` over the existing project API. No project
  reader, lifecycle owner or rendering path is installed. It rejects a catalog
  over 128 documents instead of truncating it. Unknown titles display the stable
  catalog position; the current document displays its actual title.
- Both platform `LocalViewsPanel.kt` and `DocumentNavigatorPanel.kt` leaves,
  under desktop's `desktop/` package and Android's `android/editor/` package.
  They receive callbacks and preserve the root's admission decisions.
- Shared JVM `LocalViewsInteractionTest.kt` and `DocumentNavigatorTest.kt`:
  19 and 12 focused regressions respectively. These use borrowed fake APIs and
  deliberately fail if navigation/local-view code writes canonical history.

Both EditorControllers, Desktop Main, Android WorkbenchApp, root build files,
FFI registration, existing shared schemas, geometry/precision, image/viewport/
stream and document rendering files are untouched.

## Minimal native composition for the coordinator

The storage leaf is registered and the archive privacy hook is included. Do not
register a writable native view facade without that archive hook.

Recover these existing sources from the recovery commit; do not invent another
native engine or settings store:

1. `recovery/source-candidates/ink-core/core/crates/vw-ffi/src/local_views.rs`
   to `core/crates/vw-ffi/src/local_views.rs`.
2. Its `core/crates/vw-ffi/tests/local_views.rs` to the matching active test path
   (four recovered facade cases; not executed by this lane).
3. `recovery/source-candidates/ink-core/apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeLocalViews.kt`
   to the matching active path. Change `public actual fun localViewWorkflows`
   to `public fun localViewWorkflows`: the common port is injected, with no new
   expect function. Existing `settledWorkflow` and `nativeProjectHandle` are
   present in current main and remain authoritative.

Only the coordinator adds to FFI `lib.rs`:

```rust
mod local_views;
pub use local_views::{LocalViewsBinding, LocalViewsMutation, LocalViewsSnapshot, RegionBookmark};
```

Generate bindings once with the other native schema changes. The recovered
methods are `ProjectSession.local_views_snapshot(binding, memory_budget_bytes,
cancellation)` and `change_local_views(binding, expected_revision, operation_id,
mutation, memory_budget_bytes, cancellation)`. They run on the existing project
worker, check membership and borrow the store; successful durable publication
is not relabeled as cancelled after the commit point. No dependency or lockfile
change is needed for this slice.

## Minimal root composition

Instantiate one `DocumentNavigator()` and one
`LocalViewsInteraction(scope, newId, ::localViewWorkflows)` per current editor.
Use the installation's existing explicit device ID, never the original project's
author, when constructing `LocalViewsBinding(projectId, documentId, deviceId)`.
All calls stay on the root's UI dispatcher.

1. **Bind and observe.** Call `navigator.bind(snapshot, epoch)` only when the
   root admits a visible snapshot. Call `navigator.invalidate()` as soon as an
   OPS/project-change notification is observed, before its asynchronous refresh.
   Bind again even for the existing duplicate-snapshot refresh branch. On project
   close/replacement call `navigator.detach()` and cancel/join any root switch
   task before releasing the borrowed project.
2. **Saved camera.** For an image document, await `views.attach(project, binding,
   epoch, currentCamera)` on entry. A nonnull result supplies stored center,
   scale and rotation with the current viewport size; otherwise retain the root's
   current fit/session fallback. On camera publication call
   `views.remember(camera, epoch, documentId)`. Await `views.detach()` before
   closing/replacing the project, switching documents or cancelling the parent
   scope. A false result means an unconfirmed local save, not failed document
   persistence. Report the controller's issue without claiming it was unwritten.
3. **Open panels.** Gate entry with the same current busy, transfer, gesture,
   AI-review, instruction-draft, semantic and modal guards as the existing editor
   actions. Cancel active input before opening either panel. While open, fence
   competing input/edit/modal/follow-peer routes. Closing the saved-view panel
   hides it; accepted writes continue until actual settlement.
4. **Switch.** `DocumentNavigatorPanel` passes a `DocumentNavigationRequest`
   captured from the displayed binding/epoch. Run it in the root's existing
   serialized task/mutation mutex. Check `navigator.matches(request, current,
   epoch)`, project identity and all current admission predicates. Await
   `loadNavigationTarget(project, request)`; check cancellation, those predicates
   and `matches` again before retiring old owners. Fetch alone never adopts the
   target. Follow the root's current normal attachment sequence to retire input,
   live previews, exports, AI results, instruction/semantic/selection/eraser
   bindings and document frames; await saved views; fence and retire pending
   viewport/session work; advance the root epoch; clear old transient selection,
   peer and prepared output; then rebind the existing owners, prepare the
   background/frame and restore that document's saved view. Keep the same
   project/service/history. Do not call the current whole-project `attach`,
   which closes/replaces a project. Use `finally { navigator.finish(request) }`;
   `finish` ignores an older request after rebind. If task admission refuses,
   call `finish` immediately rather than leaving the chooser busy.
5. **Cancel.** Retain the root switch Job. The panel's cancel callback supplies
   its displayed binding/epoch; reject a stale cancel callback before touching
   a newer task. Invalidate the request with `navigator.cancel()`, cancel the
   matching root Job and await it before any close/adoption. There is no
   independent navigator producer to close. Once an attachment transition has
   begun, use the root's established noncancellable cleanup/adoption boundary;
   never strand a half-retired current document on a cancelled dispatcher hop.
6. **Bookmarks.** Panel callbacks carry `(binding, epoch, nameOrId)`. Recheck
   current project/document/epoch, panel state and all normal root guards.
   Save calls `views.addBookmark(binding, epoch, name, currentCamera)`.
   Restore calls `views.restoreBookmark(binding, id, currentCamera, epoch)`;
   cancel input, disable peer following, apply the returned camera through the
   existing camera/precision synchronization and hide the panel. This performs
   no canonical edit, creates no undo transaction and must not reset history.
   Rename/delete/retry are already wired inside the leaf panels with their
   captured epochs. **Do not expose these bookmarks for PDF/multipage documents:**
   this recovered schema has no page field. Page-aware bookmarks need the
   document owner's later interface; no page or geometry change is in this lane.

Root composition is intentionally remaining work for the integration owner.
Merely calling `loadNavigationTarget` and assigning its snapshot would bypass
the current frame/AI/gesture/precision retirement contract and is not sufficient.

## Verification and open work

See `evidence/T3.07-SAVED-VIEWS-LEAVES-20261018.json` for source hashes and exact
check scope. Focused Kotlin tests use the saved Gradle-bundled compiler, pinned
coroutines and JUnit with a test-only actual for the unused core factory. This
is independent leaf evidence, not the pinned product compiler, Compose, native
facade, installed app, hardware or gate acceptance. No heavy build, new product
dependency, paid call, credential or device operation was performed.

The coordinator still owns native registration/facade admission, both root
controllers/panels, the real 24-document on-disk switching/undo run and full
app checks when the shared slot is granted. The 11 native store and four
recovered FFI tests remain unrun here. Layer/group editing, object rotation,
nondestructive tonal output and page-aware bookmarks remain separate open
T3.07 work. Current requirement statuses and product gates are not promoted.

Next bounded task: compose and verify these CORE-009 callers (medium). The
standing AGENTS selection is `gpt-6.1-sol / xhigh`, superseding the plan's older
task recommendation. This session's exact executor model/effort is not exposed
by the tool environment; no session setting was changed. The delegated read-only
review explicitly requested that prescribed model and effort.
