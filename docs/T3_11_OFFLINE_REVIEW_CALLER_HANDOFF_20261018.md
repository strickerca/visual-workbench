# T3.11 offline review caller source — 2026-10-18

This continues the native store checkpoint `7f5ff6fe8df0c43643ec617b5f774fb6f028c077`
on `feat/t3-11-offline-review`. The initial fifteen store regressions are
preserved byte-for-byte. Five native caller and seven shared interaction tests
are added, all **unrun**. No product compilation or heavy build was started.
The image slot and integration admission remain with the coordinator.

## Exclusive source paths

- `core/crates/vw-store/src/conflict_review.rs`: adds read-only
  `conflict_review_submission(id, now_ms) -> Transaction`; the existing standalone
  submit delegates to those same original checks. A bounded projection-only
  `conflict_review_label(id)` labels list entries without loading value bodies.
- `core/crates/vw-ffi/src/offline_review.rs`
- `core/crates/vw-ffi/src/offline_review/dto.rs`
- `core/crates/vw-ffi/src/offline_review/tests.rs`
- `apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/OfflineReviewApi.kt`
- `apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/OfflineReviewInteraction.kt`
- `apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeOfflineReview.kt`
- `apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/OfflineReviewInteractionTest.kt`
- `apps/android/src/main/kotlin/com/visualworkbench/android/offline/OfflineReviewPanel.kt`
- `apps/desktop/src/main/kotlin/com/visualworkbench/desktop/offline/OfflineReviewPanel.kt`
- This document and `docs/evidence/T3.11-offline-review-callers-source-20261018.json`

All API names below are additive **proposed seams for coordinator admission**.
The Rust adapter is deliberately unregistered; it cannot enter generated bindings
until the owner admits it. No existing common schema, protocol, root manifest,
lockfile, FFI registration, EditorController, Main, WorkbenchApp or live transport
module was edited. New platform panels accept state and callbacks only.

## Proposed signatures

Rust exports on the existing `ProjectSession`, all returning `WorkflowResult`:

```rust
offline_review_page(after_row: i64, limit: u32, memory_budget_bytes: u64,
    cancellation: Arc<Cancellation>) -> OfflineReviewPage
offline_review_details(conflict_id: String, memory_budget_bytes: u64,
    cancellation: Arc<Cancellation>) -> OfflineReviewDetails
stage_offline_review(conflict_id: String, expected: OfflineReviewBinding,
    metadata: WorkflowMetadata, choice: OfflineReviewChoice,
    memory_budget_bytes: u64, cancellation: Arc<Cancellation>) -> OfflineReviewMutation
submit_offline_review(conflict_id: String, expected: OfflineReviewBinding,
    memory_budget_bytes: u64, cancellation: Arc<Cancellation>) -> OfflineReviewSubmission
keep_offline_review(conflict_id: String, expected: OfflineReviewBinding,
    memory_budget_bytes: u64, cancellation: Arc<Cancellation>) -> OfflineReviewMutation
cancel_offline_review_draft(conflict_id: String, expected: OfflineReviewBinding,
    memory_budget_bytes: u64, cancellation: Arc<Cancellation>) -> OfflineReviewMutation
```

`OfflineReviewBinding(project_id, host_seq, state_hash)` fences the complete
project visible state; it intentionally has no fabricated document ID. The
choice is `SwapOther` or `RestoreCopy { document_id, object_id }`. Existing
`WorkflowMetadata`, `WorkflowReceipt`, `ProjectInfo`, `WorkflowError` and
`Cancellation` are reused. The dedicated DTO leaf supplies page/index,
Unreviewed/Kept/PreviouslyResolved/Draft/Queued/Applied progress, private bounded
value previews, details, mutation result and submission result. There is no new
transaction schema, outbox, sync sender or auth policy.

The shared capability is `offlineReviews(project): WorkbenchOfflineReview`, with
`page`, `details`, `stage`, `submit`, `keep`, `cancelDraft` methods matching the
Rust boundary. All retain a caller memory budget, default 256 MiB. The JVM facade
uses the existing `nativeProjectHandle`, `settledWorkflow`, native error mapping,
workflow metadata/info conversion and cancellation ownership. It creates no new
global worker or lifecycle owner. ABI generation/names still require compilation.

Each value is an explicit UTF-8 prefix of at most 4096 bytes with original byte
length, BLAKE3 and truncation/availability flags. No raw bytes cross this ABI.
The values and operation are private UI content and have no payload Debug output.
Details include historic kept/other, current value and the actual saved operation,
exact transaction ID, original reviewed revision and next counter. Truncated or
unavailable previews refuse a new submission; they never imply full review.
Existing queued/applied exact retries remain inspectable without new authority.
Unsupported or oversized content remains retained in the original journal.

The native adapter checks project identity, full visible revision, metadata,
active native gestures and existing workflow workspace budgets before mutation.
It reserves 160 MiB for the bounded record/draft/encoding representations alongside
existing canonical/transaction clone accounting. Large workspaces may therefore
receive Memory/Limit; there is no silent smaller-budget or partial-data fallback.

## Real counter, commit, replica and OPS path

Current source paths inspected:

1. `core/crates/vw-ffi/src/editor.rs::History::restore` rebuilds accepted and
   pending history and `next_lamport`. Its **required coordinator hook** before
   `Ok(result)` is:

   ```rust
   result.next_lamport = result.next_lamport.max(store.conflict_review_lamport_floor()?);
   ```

   This also covers `session/replication.rs::persist`, which restores history
   after authenticated checkpoint installation. It must land before this source
   is admitted for ordinary edits after reopen. The adapter also restores that
   floor immediately after staging, and before details/metadata admission.
   A saved review is not an undo entry until it is actually queued/accepted.
2. `project.rs::ProjectSession::from_store` already reconstructs the authenticated
   host snapshot, then `Replica::queue_recovered` for every durable pending txn.
   That path remains unchanged. New read-only submission validation lets the
   adapter reuse `ProjectState::commit` instead of directly enqueueing behind the
   replica's back.
3. `editor.rs::ProjectState::commit` stages replica + inverse-history owners
   before mirror SQLite enqueue, then adopts them, publishes `Committed`, and
   calls `state.changed.notify_one()`. Its host path uses normal commit and
   accepted history. The adapter calls this exact path once for a new saved
   review. Exact duplicates skip a second commit and keep the same identity.
4. `project.rs::from_store` aliases that notification Arc into
   `ProjectSession.network_changed`. `session/link.rs::exchange` selects on
   `project.network_changed.notified()`, calls existing `sync::pending`, and
   sends those transactions through the existing authenticated OPS sender.
   No extra sender/notification implementation is needed.
5. After normal commit, the adapter retries the existing local review marker
   settlement. A marker error returns a durable receipt with
   `metadata_pending=true`; it never changes an already saved edit into Cancelled.
   Exact history/outbox bytes continue to establish Applied/Queued if marker
   settlement was interrupted.

The existing Android metadata path already uses
`repository.instructionLamport(project.info().nextLamport)`; reuse it for review
metadata. Desktop instruction/ordinary edit metadata already reads
`project.info().nextLamport`; reuse that path. Neither path needs a new counter
subsystem or device ID. The native History restore floor above is still required.

## UI/lifecycle hooks for the coordinator

`OfflineReviewInteraction` borrows an injected scope, the **existing** edit mutex,
a project/info/epoch attachment, mutation-admission predicate, existing metadata
allocator, existing fresh object-ID allocator, and two callbacks:

```kotlin
updated: suspend (OfflineReviewAttachment, ProjectInfo) -> Unit
committed: suspend (OfflineReviewAttachment, WorkflowReceipt) -> Unit
```

Callbacks run while the interaction owns the edit mutex. They must not reacquire
that mutex. They must be idempotent and check captured project identity + epoch
before publishing into the current editor. The existing project-refresh functions
already supply the needed behavior:

- Desktop: reuse `edits`, `project`, `generation`, the shown document's
  `render.revision`, existing metadata/new-ID lambdas, and `refresh()` after
  checking `project === attachment.project && generation == attachment.epoch`
  and `!closing`. Invoke review `refresh()` after accepted editor/OPS refresh.
- Android: reuse `mutations`, `project`, `epoch`, the shown revision, `newId()`,
  `repository.instructionLamport`, and
  `refresh(current, suppliedInfoOrReceipt.revision, attachment.epoch)` after
  matching project/epoch and `!disposing`. Invoke review `refresh()` after the
  normal refresh path adopts an OPS/editor revision.
- The admission predicate must cover existing in-memory ink/shape/text/slider,
  selection/eraser, instruction/semantic and dialog/export ownership. Core's
  active-gesture check cannot see every app-local draft. Include review busy in
  the existing shared input/mutation admission so an operation cannot race a
  newly started gesture. Preserve dirty instruction/AI decision rules.
- Before close/replacement, cancel/join `interaction.detach()` before the old
  project is closed, alongside existing instruction/semantic detach. Resume only
  after publishing the new attachment. Reconciliation runs only while the child
  already owns the edit gate; cancelling a child still waiting for that gate does
  not reacquire it. Existing close-under-gate ownership can therefore be retained.
  The operation slot remains held through actual native retirement.
- Compose roots may pass the interaction's state and callbacks into each new
  `offline/OfflineReviewPanel.kt`. Collect state in the root or a small wrapper.
  Supply the explicitly shown restore document ID/label; no hidden destination.
  Both panels use human-readable list/property labels and show historic/current
  values, an expandable exact prepared operation, stale or incomplete status, and separate Preview/Submit/Keep/Cancel actions. Queued
  work stays with the normal link; ordinary Undo reverses it.

Refresh/page/select perform reads only. Staging saves one exact intent without
submitting. Reopen/refresh never auto-submits a Draft. Submit uses that exact ID;
stale never-sent drafts require refresh and explicit cancellation/new review.
Cancellation is not evidence of an unwritten edit: the existing settled wrapper
may discard a completed DTO if cancellation wins delivery. The interaction now
reconciles the exact conflict through details + project.info under NonCancellable
while still holding the edit gate. It restores the parent counter/view, and only
for a durably Queued/Applied exact identity invokes a duplicate receipt callback.
That also covers ordinary errors after persistence. Detached/replaced attachments
cannot adopt those results. A failed reconciliation keeps refresh required and
leaves all durable content untouched.

## Admission, checks and remaining work

Required native registration (owner edit only): `mod offline_review;` and
`pub use offline_review::*;` in `vw-ffi/src/lib.rs`. Admit the additive shared DTOs,
then regenerate bindings once within the coordinator's coherent batch. The
native tests live under the module and become discoverable after registration.
No manifest/dependency/lock change is required by this slice.

Cheap checks completed: Rust syntax/rustfmt checks, whitespace, byte-for-byte
preservation of the fifteen existing store tests, and Kotlin PSI syntax parse
of all six new Kotlin files using the cached Kotlin 2.4.0 parser. The temporary
256 MiB parser utility was compiled and executed under `/tmp`; it performs no
Kotlin type checking, product compilation or runtime tests. Staged redacted
secret scanning and exact source hashes are bound in the accompanying receipt.

The prior complete offline Cargo dependency-policy attempt remains blocked by
missing cached crates.io index metadata for existing `pdfium-render`; no locks,
pins or policy were weakened. The earlier source setup/npm policy checks remain
scoped to their unchanged inputs. No heavy build slot was granted. No Rust or
Kotlin product compilation, binding generation, Clippy, new Rust/JVM test,
Android/Windows app test or device/carrier run was executed here.

Written native caller cases: normal host receipt/history + exact duplicate;
mirror optimistic replica/history + existing OPS notification; cancellation and
live gesture refusal before stage; full-hash stale refusal + exact draft reopen;
private UTF-8 preview bound/hash. Shared cases: reads never submit; cancellation
after durable stage; ambiguous submit completion for cancellation/storage error;
same-sequence changed-hash draft preservation; incomplete-preview refusal;
cancel-before-dispatch; late attachment result and joined detach. Source review
found and repaired the ambiguous-completion reconciliation gap. These are source
reviews/written regressions, not executed acceptance.

After image-slot release and the coordinator's license/admission gate, run the
focused `offline_review::tests` native cases, shared interaction cases, preserved
store regressions and affected recovery checks through the admitted build
harness. Then wire and exercise actual desktop/phone review, undo, restart,
reconnect and duplicate/stale-delivery flows. Counter-floor startup, binding
names/type checks and platform callbacks are explicit remaining integration
hooks. Carrier switching/time measurements, scale rebase/performance, A14/A25,
SYNC-003 and G3 remain open. Phone AI/provider execution, external model calls,
credentials, paid services, binaries, Krita and device actions are excluded.
