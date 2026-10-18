# Remaining roadmap implementation grants

All workers start from accepted main **6d505416963fb42009921bd3fb8639994e73752a**.
Fetch that exact commit. Recovery0d91b705 is reference source, not an active-code
overlay. The held caller branch70475b8 is NOT a required worker base. The root
public giant-input branch is independent and retains its own pending changes.
The parent's latest implementation-first instruction supersedes repeated broad
per-slice builds: use cheap focused checks needed to avoid propagating defects,
return source/evidence for Astra review, and record aggregate checks as deferred.
Krita implementation/reactivation remains deferred. No product gate is passed.

## Global exclusions and stable boundaries

Root retains Cargo manifests/lock, protocol/schema files, workspace/root module registration,
generated bindings, NativeCore.kt, all giant-input/producer/provider/renderer files,
both EditorControllers, Desktop Main and WorkbenchApp. Existing Air owner keeps
its full Android Activity/canvas/manifest/preferences/remote-panel grant until
explicit handback; existing capture owner keeps Desktop Main/capture/region files.
The existing grants remain authoritative. New grants below delegate only their
listed shared/FFI leaves, superseding the earlier root reservation for those
specific files. Do not edit another lane's paths, tools/cloud-setup, build entry
points or this central ledger. Return an exact minimal integration patch for a
reserved root; do not apply a stale controller/app overlay.

Keep existing public declarations and error/cancellation behavior compatible.
Add optional capability interfaces or new modules when required, with one owner
per API. No duplicate transaction, budget, selection/mask, package/catalog,
credential, transport-grant, dispatcher or lifetime system. Existing joined
retirement and failed-owner retention are authoritative. No live provider call,
key/credential/signing/deployment/device identity change, driver/emulator install
or hardware action. Required external model/runtime/license decisions stay open.
One heavy build globally, at most two build workers; parent allocates any slot.
No broad app/native rebuild, packaging or binding generation in these lanes.

## E — Editing/output now; assisted selection isolated

Owner **01a15009-f394-70eb-9e10-e873087f1003**. Immediate exclusive output grant:

- NEW core/crates/vw-geom/src/image_output.rs,
  core/crates/vw-model/src/image_output.rs,
  core/crates/vw-raster/src/image_output.rs and their dedicated tests.
- NEW core/crates/vw-ffi/src/image_frames.rs and image_frames/{dto,plan,pixels,snapshot}.rs
  plus dedicated image-frame tests. Existing module roots remain excluded.
- NEW shared commonMain ImageFramesApi.kt, ImageFramesForm.kt,
  ImageFramesInteraction.kt, jvmMain NativeImageFrames.kt and dedicated tests.
- NEW Android editor/ImageFramesPanel.kt and Desktop ImageFramesPanel.kt with
  their dedicated tests. No EditorController, Main, WorkbenchApp or viewport edit.

Deliberately recover the output-frame leaves against current canonical document
transactions, undo/source coordinates and admitted output/color contracts. Root
keeps module registration, validation/capability hooks, export/package routing,
viewport composition and all platform roots. Return minimal exact hunks for those
hooks. Existing APIs stay compatible; this is one output-frame system, not a
parallel document/transform/export pipeline. The worker owns the new ImageFrames
API files and should publish exact additive signatures with its first batch.

Separate assisted-selection grant, usable without making output wait: NEW shared
AssistedSelectionApi.kt/AssistedSelectionInteraction.kt and tests; NEW Android
assistedselection/** and dedicated tests. Existing shared MaskApi.kt, native
masks/dto.rs, masks/edit.rs and JVM NativeMasks.kt may receive only bounded
proposal-import additions; no existing declaration/transaction behavior rewrite.
Keep exact SelectionBinding/Version, WorkflowBinding/Metadata/Receipt, current
source identity, revision/page/epoch and the existing mask operation authority.
Model proposals must be bounded, cancellable, manually correctable and imported
through ordinary selection operations. No selected model/runtime exists on main.
Model artifact/hash/license/runtime pins require real admission before inference;
no download, invented output or backend placeholder counts as completion. Finish
independent output/correction code and report the model-bound dependency clearly.

## O — Bounded atomic recovered-outbox batch

Owner **01a1500a-13fe-77e4-835e-201034a9901c**. Exclusive EXACT paths:

- core/crates/vw-ops/src/replica.rs
- core/crates/vw-ops/tests/recovered_outbox.rs
- NEW core/crates/vw-sim/tests/offline_rebase.rs
- NEW core/crates/vw-net/tests/phone_result_sync.rs
- NEW core/crates/vw-ffi/src/session/offline_recovery_tests.rs

Existing conflict review, draft/counter floors, transaction-bound phone AI asset
synchronization, sender and outbox remain authoritative. Do not create another.
The real gap is4096 pending versus SYNC-002's5000, plus growing queue clone/replay
on each startup restoration. Stable additive method:
`Replica::queue_recovered_batch(&mut self, transactions: Vec<v1::Transaction>) -> Result<(), OpsError>`.
Keep `queue_recovered` source-compatible and preserve its existing semantics.
Support at least5000 pending with a finite explicit cap; preserve existing
per-transaction operation and identity/byte limits. Validate/admit before growing
allocations. Apply one atomic bounded batch, preserving exact bytes/order,
blocked operations, inverse ownership, idempotency and counters. Invalid/collision
input leaves the complete prior state unchanged; no per-item growing replay.
Root keeps the from_store_owned restoration-loop hook and test-module
registration. Return those two tiny hunks; no other native/session/store/network
or shared/UI files are granted. Scale/kill/restart/timing tests go in the central
ledger; the focused implementation must not wait for another review UI rewrite.

## V — Full verification caller/FFI/UI/agent slice

Owner **01a1507a-2f92-71e6-b312-c985eb631012**. Exclusive:

- Desktop desktop/mcp/** and its corresponding test directory, including
  DesktopMcpCoordinator, McpCaptureEngine, compare/settings panels and the three
  admitted verification leaves. This hands those former root paths to this lane;
  root performs no simultaneous MCP edits. Desktop Main remains excluded.
- NEW shared VerificationApi.kt, VerificationInteraction.kt, NativeVerification.kt
  and their dedicated tests; do not change PackagesApi/NativeCore or inbox APIs.
- NEW vw-ffi/src/verification.rs, verification/** and dedicated verification FFI
  tests. Existing store/package verification modules are read-only admitted
  dependencies unless the parent explicitly extends this grant.
- NEW mcp/src/verification.mjs and mcp/test/verification.test.mjs only; return the tiny root
  handler/tool registration/schema patch for integration. Existing server roots
  and protocol/schema files remain root-owned.

Implement the actual workflow using admitted store CAS reviews and package
failure compiler, existing checked native package catalog, exact retained capture
selector/ticket/grant/indicator and joined recapture owners. Stable identity is
original+after package IDs/targets/manifest hashes, actual comparison receipt
ID/hash/project/marker count, current review version and original marker numbers.
Derive the receipt from real checked original/recapture owners; never accept an
agent-supplied success assertion or use constant-true currentness fences. Imported
lineage is descriptive, not authorization/alignment. Agent-returned PNGs are
untrusted inbox content, not actual after-capture proof. Verdicts/notes are manual
owner-only, installation-local and excluded from portable output. No agent
verdict setter, arbitrary OS selector, implicit recapture permission or note log.

Keep exactly the six admitted lineage fields/schema version; all-pass returns no
failure package, pending/stale review refuses, and failed subsets must be normally
compiled against the exact after source with original text/role/kind preserved.
Implement semantic alignment through admitted stable IDs; image alignment must
have real bounded implementation/quality refusal, never inferred from dimensions.
Public native/shared API ownership is delegated to this lane for these NEW files;
return additive signatures/registration patches promptly, without waiting on root
giant intake. Root composes module exports and one final ABI generation later.

## F — Provenance, bounded composites/previews and release tooling

Owner **01a14eeb-8086-76af-9670-dd5ffa5eb032**. Exclusive EXACT paths:

- core/crates/vw-formats/src/bmp.rs, src/lib.rs and NEW tests/bmp_provenance.rs.
- NEW vw-formats/src/psd.rs, src/raw_preview.rs and tests/composites.rs,
  tests/raw_preview.rs. No Cargo changes or existing TIFF/AVIF/OS decoder rewrite.
- NEW tools/release/assemble.py, test_assemble.py, package-windows.ps1, README.md.

Reuse admitted bounded BMP/TIFF, restricted AVIF OS adapters, R164 conversion,
originals and package verification. Root keeps decoder/caller registration,
raster/export roots, Gradle/root MSI wiring and pinned runtime staging. Return
minimal hooks for those roots; no giant/input/runtime/URI or platform-root edits.

BMP CORE-006 requires a valid bounded IN-FILE provenance carrier; a sidecar alone
is insufficient. Preserve real ICC fields as ICC; never masquerade metadata as a
profile or silently strip it. Publish exact proposed byte layout/parser/exporter
compatibility evidence before activation. Keep BMP refusal if that design cannot
be established; it must not block independent composite/preview/tooling work.
PSD/PSB scope is labelled embedded flattened composite only, with bounded section/
dimension/compression/decoded allocation limits and originals retained byte-for-
byte; no layered round trip. RAW scope is labelled bounded embedded previews and
existing available OS decoders, with format/orientation/provenance maintained;
no raw-development engine or unreviewed bundled decoder. PDF-compatible AI
continues through existing PDF handling, not another parser.

Release tools assemble only an explicit bounded manifest of already built,
source-bound artifacts, verify hashes/paths/non-symlink identities and produce
checksums/notices/inventory in a task-owned destination. No implicit runtime
search/download, live signing/deployment, installation/driver action or execution
of MSI/package builds during implementation. package-windows.ps1 is orchestration
source for later reviewed root/MSI admission. Record unavailable licensed decoder,
composite or package inputs as external dependencies, not success. Packaging is a
separable commit/batch and can be reassigned by parent if it delays format work.

## A — AI/provider/editor slice, reserved for parent-assigned owner

No worker is assigned by this document; parent can activate an idle existing lane.
Exclusive once activated: vw-ai-provider/src/** and tests; vw-ai/src/config.rs,
request.rs and dedicated adapter tests; vw-ffi/src/ai_edit.rs and ai_edit/**;
shared AiInteraction.kt, AiEditsApi.kt, NativeAiEdits.kt and AI-specific tests;
platform editor Ai*.kt / Desktop Ai*.kt and matching tests. Excludes both main
EditorControllers, Main/WorkbenchApp, selection files and protected-key platform
implementation files. Return root hooks/credential-capability needs explicitly.

Implement Gemini/FLUX.1 Fill and candidate-grid/editor paths by extending existing
authorized request/proof/budget/configuration/candidate owners, not a second AI
service. Preserve explicit Send, admitted cost/profile confirmation, no retries,
unresolved attempted-charge retention, protected credential lookup, exact source/
mask/outside-pixel proof and normal edit application. Use primary official specs
when adapter details need verification. No live provider calls or fabricated
pricing/capability proof. A missing key blocks external execution, not offline
implementation and synthetic contract tests. Krita integration stays deferred.

## Returns and final integration

Each lane returns exact base/candidate SHA, changed paths, implemented behavior,
focused checks actually run, unresolved external inputs and rows for
DEFERRED_VALIDATION_LEDGER_20261018.md. Root alone reconciles that central ledger.
Source-only review is not compilation or acceptance. Parent requests Astra review
per coherent source batch. Preserve older candidates, receipts, archives and
worktrees until explicit disposition. Final aggregate builds/bindings/licensing,
packages and authorized hardware journeys run once against the integrated tree.
