# T3.11 offline conflict review leaf — 2026-10-18

Source checkpoint only. The Rust regression cases are written but **unrun**.
This does not complete T3.11, SYNC-002/003, PERF-007, A14/A25 or G3.

## Baseline and ownership

Fetched main and fast-forwarded the saved checkout from `7be9da7` to
`83663107d864b21d22d2e746b9ca6eb80d47beac` before editing. Work is isolated on
`feat/t3-11-offline-review` in `/workspace/vw-offline-review`.

Exact owned files:

- `core/crates/vw-store/src/conflict_review.rs`
- `core/crates/vw-store/src/lib.rs` (two module/export lines only)
- `core/crates/vw-store/tests/conflict_review.rs`
- This handoff and `docs/evidence/T3.11-offline-review-source-20261018.json`

No schemas, dependency manifests/lockfiles, FFI registration, controllers, app
roots, document/precision/image/viewport code or live stream/carrier modules
were edited. Main integration remains with its single integration owner.

Reviewed `AGENTS.md`, the implementation plan, specification §4.4, existing
`vw-ops` host/replica/inverse semantics, `vw-store` journal/outbox/checkpoint
recovery, `vw-net` authenticated sync and carrier authorization, and FFI project
reopen/replication/history paths. The preserved recovery branch at
`473395197f6437586a8b0c05fa8cd35aa6800441` has 796 original manifest entries;
none name an offline/conflict/rebase/replica/sync/journal candidate. The existing
recovered `Replica::queue_recovered` and authenticated checkpoint installation
are reused. No replacement sync engine, carrier, identity or provider is added.

## Implemented source behavior

The leaf uses existing conflict rows, local `settings`, normal typed
transactions, the existing outbox and host sequencer. Review never edits or
removes a conflict's retained losing value, accepted journal or tombstone.

- `conflict_review_ids(after_row, limit)` returns a stable append-order cursor,
  at most 128 IDs. Fetch one private payload with `conflict_review(id)`; records
  and saved draft encodings are admitted at 16 MiB each. An oversized record is
  refused without deleting it. No full-conflict or full-outbox clone is needed
  to enumerate IDs or check whether pending edits prevent a review.
- `stage_conflict_review(id, expected_revision, choice, identity)` requires the
  complete current host revision (sequence and hash) and an empty durable
  outbox. It builds and validates a normal operation, then saves its exact
  transaction under `settings[offline-review/v1/<conflict UUID>]`. This is a
  local versioned intent, outside the replicated protocol and canonical hash.
  Existing saved intents cannot be overwritten. No edit is submitted yet.
- `submit_conflict_review(id, now_ms)` resumes only that saved transaction.
  On the host it uses `ProjectStore::commit`; on a mirror it uses
  `enqueue_pending`. A never-submitted stale draft is preserved and refused.
  An exact queued or accepted retry succeeds after the authority has advanced.
  An ID collision with different transaction bytes fails without substitution.
- Draft, Queued and Applied are derived from exact bytes in settings, outbox and
  op log. In particular, an interruption after the durable edit but before the
  `swapped`/`restored` metadata write still reads Applied. Retrying settles that
  metadata without creating another edit. A storage error after commit must
  therefore be reconciled by rereading this state, not reported as unwritten.
- `keep_conflict_current(id, expected_revision, now_ms)` writes existing local
  `kept` metadata only. It preserves both conflict values and changes no project
  revision. `cancel_conflict_review(id)` explicitly removes only a never-sent
  review intent. Queued/applied edits require the existing inverse workflow;
  this API cannot cancel accepted history or discard an outbox entry.
- `conflict_review_lamport_floor()` includes saved intent counters before they
  enter pending/accepted history. Staging also refuses counter/transaction-ID
  reuse among saved intents. It does not allocate or change device identity.

The first translator supports document title; layer name/order/visibility/lock/
opacity/blend; existing editable object transform/style/role/lock/visibility/
order/shape properties. Style width, stroke color and booleans, text fields
and marker number retain existing field-level operations when possible. Other
shape/style conflicts use the existing coherent aggregate operation. Color-only
fill swaps are refused because the current fill setter would also enable fill,
including after an independent host disable during delayed delivery. The UI must show the actual prepared operation before submission.

Unsupported collections/properties, removed destinations, invalid values and
locks are explicit refusals. Their records remain available for review/keep.
Whole layer/group/instruction/result restoration is not implemented by this
bounded leaf. No arbitrary journal address or raw patch becomes a wire command.

Delete-versus-edit uses `RestoreCopy { document, object }` with an explicitly
chosen fresh object ID. It copies the retained edited object into its still-valid
original layer and matching document, without changing the deleted ID or its
tombstone. The new copy is ungrouped and authored by the current local device
with the supplied creation time; retained content/style/geometry and the original
conflict remain. Missing/locked layers, cross-document destinations and reused
object IDs are refused by normal model/sequencer validation. Restoration never
unlocks or recreates a destination implicitly.

Once queued, a review is an ordinary offline transaction with its original base
and bytes. Existing host wall-clock conflict rules, host tie break, delete win,
losing-value records, authenticated receipt/checkpoint fences and per-device
inverse ownership continue to apply. A later opposing write may itself create a
new retained conflict; it does not silently retarget this review to a new base.

## Exact caller hooks for the integration owner

The exported native types are `ConflictReviewChoice`, `ConflictReviewIdentity`,
`ConflictReviewStatus` and `ConflictReview`. No shared Kotlin/FFI schema has been
admitted in this lane.

1. Expose the seven public `ProjectStore` methods above through the existing
   project worker, with the existing project/lifecycle/cancellation and workspace
   memory admission. Never call them from a parallel writer or carrier thread.
   Keep payloads private to the chosen project review surface, out of diagnostics.
2. On project open and after staging, merge
   `store.conflict_review_lamport_floor()` into `History.next_lamport` by `max`.
   Allocate fresh transaction/object IDs and advance the existing counter owner.
   Saved intents reserve counters but are **not** undo entries until queued or
   accepted. This hook is required before admitting ordinary edits on reopen.
3. Refuse staging while an in-memory gesture or other unpersisted edit is live;
   the leaf can see durable pending work only. Capture the exact attachment and
   current revision for the review surface. Display the historic kept/other
   values alongside current project value, and preview the prepared operation.
   Fence callback adoption to that same project/attachment. A stale choice needs
   refreshed user review, then explicit cancellation and a new identity.
4. After submit/keep/cancel, reread the review state. On host acceptance, rebuild
   existing history from the store (preserving inverse ownership), refresh the
   normal project info and publish through existing change notifications. On
   mirror enqueue, rebuild or stage the existing `Replica` from the durable
   checkpoint plus **all** pending transactions via `queue_recovered`, and restore
   pending history, before publishing optimistic state. Signal the existing OPS
   sender. Do not introduce a second outbox, send path or speculative base.
5. After authenticated reconnect/checkpoint installation, refresh statuses from
   `conflict_review`; the existing atomic installation removes exact acknowledged
   outbox entries and preserves settings. Do not gate Applied on `resolved_at`
   alone: its final metadata write may have been interrupted. Do not auto-submit
   Draft entries. Only exact previously Queued entries use ordinary retry.
6. Review UI actions: Keep current; preview/stage Swap other; preview/stage Restore
   copy with a fresh ID; explicit Submit/Resume; explicit Cancel for Draft only;
   use ordinary Undo for Queued/Applied. Show typed refusal/stale/pending status
   and preserve the displayed draft on errors. Root owns shared DTOs, FFI
   registration, both EditorControllers, Desktop Main and Android WorkbenchApp.

Carrier authentication and fresh control grants remain with their existing
owners. This leaf grants no remote input authority, weakens no pairing/pin and
changes no identity. Actual USB/Wi-Fi switching and the ≤2-second measurement
remain open. Phone AI/provider sends are excluded and require separate authority.

## Validation and limits

Completed cheap checks: Rust parser/format checks on the two new Rust files and
module registry; `git diff --check`; locked offline Cargo workspace metadata;
`tools/check_setup.py all`; reviewed npm lock/license policy. The npm result
explicitly has `installed_verified=false` and `node_verified=false`.

`cargo deny --offline check licenses sources bans` was attempted and blocked by
missing cached crates.io index metadata for the existing `vw-ffi` dependency
`pdfium-render`. The first invocation put `--offline` after `check` and failed
argument parsing; the corrected invocation reached this metadata blocker. No
license gate is claimed passed and no dependency pin changed.

Fifteen deterministic Rust regression cases are written and **unrun**. They
cover draft-only interruption/reopen, post-commit/pre-metadata interruption,
stale full-revision refusal, pending-edit preservation, mirror reconnect and
exact duplicates, foreign/stale checkpoint refusal, local keep, collisions and
changed intent content, normal object undo, fresh-copy restoration, unsupported
entity refusal, lock preservation, reserved counter recovery, and preservation
of an independent stroke color during a delayed width review, and refusal of a
color-only fill swap whose setter could re-enable an independently disabled fill.

No parent shared build slot was granted, and the fresh worktree has no compiled
product target. No compilation, Rust test binary, Clippy, complete license gate,
Windows/Android app test, device input, carrier timing, scale/performance run or
product acceptance was executed. After slot allocation, run the focused
`vw-store` conflict-review target and affected store/recovery checks through the
coordinator's admitted build harness, preceded by its license gate; then wire and
verify actual app flows. A source checkpoint is not runtime validation.

No screenshot, private source image, credential, binary, external model call,
paid service or device action was created. The exact session model/effort is not
exposed here, so the historical Sol 6.1/xhigh setting cannot be attested; settings
were not changed.

Next task: T3.11 caller integration and focused validation (bounded medium
slice of the plan's L task), then remaining carrier switching and separately
authorized phone-send work. The plan recommends gpt-6-astra/max for Phase 3; its
later owner override requests gpt-6.1-sol/xhigh. Keep the owner's selected session
settings and report their actual verification; do not silently change them.
