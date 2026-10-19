# T5.01 verification leaves and admission, 2026-10-18

Integration owner: `01a14eea-d204-762c-94b8-f07105b5001e`.
Baseline queried/fetched before implementation: remote main and checkout both
`6f3e2f3b9ed7a760d4dd7f4c6f26fa44dc5ad88d`. Compared recovery
`0d91b70558a995fa7654021de7465b1d3f615e3d` and tip
`473395197f6437586a8b0c05fa8cd35aa6800441`, including stream-finish package/inbox
postimages. No T5.01 implementation was found there; saved-frame provenance is
not a package-to-retained-target relation. Read AGENTS, the v2 plan/requirements,
latest checkpoint/ledger and lane admission. `/workspace/.agents` is empty.
Existing `submit_result`, immutable inbox and `McpComparePanel` are preserved.

## Exact ownership and delivered code

Only these additive leaves/artifacts and this handoff are owned:

- `apps/desktop/src/main/kotlin/com/visualworkbench/desktop/mcp/`:
  `McpVerificationTargets.kt`, `McpMarkerAssessments.kt`,
  `McpSemanticMarkerAlignment.kt`.
- `tools/tests/T501VerificationLeafProbe.kt`,
  `tools/tests/T501RecapturePublicationProbe.kt` and
  `tools/tests/test_t501_verification_leaves.py`.
- `integration/t5-01/private-recapture-owner.patch` and
  `integration/t5-01/verification-origin-admission.patch`, **unapplied**.

No existing product source, manifests/locks, controllers, app roots, schemas,
registration, generated bindings, or other workers' leaves were edited.

## Three concrete links

**1. Original package to retained target.** `McpVerificationTargets` consumes the
existing `PackageInfo` / `WorkflowBinding`; it stores only an opaque selector.
The native/picker owner remains the sole owner of its exact target. Picker thread
`01a15080-f8b9-7495-9194-b33660abf005` needs only this private hook:
`rememberCapture(info: PackageInfo, selector: String)` after the exact capture
settles. No shared target model, OS handle or public selector field is needed.
`rememberPackage(info)` binds later marked revisions of the same project,
document and source asset. `begin(info)` yields a session-only ticket; root must
use its selector through the existing grant/indicator/joined-cancellation path.
`complete(ticket, afterInfo)` associates that exact completed route, **not image
alignment**. Changed full bindings, ambiguous source routes, retirement, foreign
or consumed tickets, close and capacity limits refuse. Retiring an original
invalidates pending tickets without prematurely releasing their admission.

The repaired, unapplied `private-recapture-owner.patch` adds those calls plus
`recaptureOriginal(session: String, original: PackageInfo)` to the existing
coordinator. It factors the current capture body into `captureInside`, preserving
its ownership/grant checks, and invalidates before unpublish. Its optional private
`checkOriginal` callback combines with the grant check after captureGate, indicator
wait, native compilation and **inside catalogGate before catalog.publish**.
The same guard still runs before exposure, and final ticket validation, abandon
and joined cleanup remain. Both patches pass `git apply --check` against the
baseline. The coordinator proposal now has focused executable race coverage;
the native lineage postimage remains uncompiled.

**2. Aligned marker correspondence and owner assessment.** Read `semantic.json`
with existing `WorkbenchPackageCatalog.readFile(id, target, manifestSha256,
"semantic.json", 4 MiB)`. `McpAlignmentSnapshot.fromCheckedPackage(info, bytes)`
matches native admission: 4 MiB JSON, 4,096 elements, 2 MiB UTF-8 text. A bounded
structural framer separates the flat element records; unchanged `McpJson` validates
the header and each record, avoiding the IPC codec's 32,768-node whole-tree limit.
The decoder retains existing `SemanticElement` records without inventing the
collection timing omitted from exports. No snapshot returns unresolved.
`alignMcpMarker(recapture, originalNumber, markerBounds: Rect,
anchor: SemanticReference, before, after)` returns paired canonical Rects only
for a unique stable-ID translation across the exact bindings/platform. Source
and destination duplicate IDs, absent IDs, clipping, resized elements or stale
bindings refuse/unresolve. Names/text and raw snapshot-local EIDs are never
matching evidence. This supplies semantic translation; image registration is
still pending. Root must apply the verified package overview mappings and check
pixel bounds before using existing inbox `pixels` (maximum 512 by 512).

`McpMarkerAssessments.open(original: PackageInfo, receipt: McpInboxReceipt)` uses
an existing exact image-return receipt. `snapshot` / `record(snapshot, number,
verdict, note)` fence the immutable pair and review version. `failures(snapshot)`
returns original failed numbers; pending markers refuse, all-pass returns null.
`checkCurrent(selection)` rejects an older selection after any owner change.
Notes are literal, byte/total bounded and excluded from routing/lineage. These
are session-only manual decisions; durable review persistence and UI wiring
remain unimplemented. The leaf never infers an owner verdict or alignment.

**3. Failures selection to existing compiler.** `McpFailureSelection` is a private
input selection, not a package format. The minimal admission patch proposes
optional `Extensions.verification_origin` with these exact fields:
`original_package_id: Id`, `original_target: String`,
`original_manifest_sha256: String`, `comparison_receipt_id: Id`,
`comparison_receipt_blake3: String`, `original_marker_numbers: Vec<u32>`.
Entry n maps follow-up marker n+1 to the original number. The patch initializes
None for unchanged compilation, bounds preflight allocation and validates hashes,
target, nonempty/unique numbers and the mapping count. It does not loosen current
contiguous marker numbering. Root admission is needed before the next native
leaf can attach checked lineage to normal compiler output and reuse
`Package::from_files` validation. Final failure-package compilation is not done.
The existing schema's `extensions` object is already an extension point; native
Extensions currently rejects unknown fields. No schema file was changed.

Minimal MCP owner callback admission: `verification_recapture(session,
package_id, target, manifest_sha256)` resolves the private retained route and
returns original/recaptured package identities. A comparison callback accepts
those original identity fields plus `receipt_id`, `receipt_blake3`, `n`; it must
recheck the same existing selector grant and return the checked correspondence /
bounded crops or unresolved. No agent selector, grant minting or verdict setter
is needed. Root retains host dispatch, tool registration and public DTO decisions.

## Results and remaining acceptance

`python3 tools/tests/test_t501_verification_leaves.py`: **23 leaf cases and five
coordinator proposal race cases pass**. Valid 2,000/4,096-element snapshots retain
all records; count/byte/text oversize, duplicate keys/arrays, broken collection
grammar, Unicode, excessive nesting and nonfinite bounds refuse. The unchanged
IPC codec still rejects these large whole snapshots at its existing node ceiling.

The Kotlin 2.4.0 focused compiler uses warnings denied, cached pinned coroutines
1.9.0 and exact production DTO/interface declarations. The proposal is applied
only to a temporary coordinator copy; its scope dispatcher alone is replaced
with Unconfined to establish exact mutex waiter order without sleeps/polling.
All guard, grant, publication, retirement and cleanup function bodies are the
proposal's code. Tests cover retirement queued at catalogGate (no durable publish
or exposure), during compile, queued at captureGate, delivered cancellation with
joined cleanup, and an already admitted publication winning before retirement.
The IO fixture closes untransferred results and waits for delivered producer
cancellation rather than treating the outer caller's request as a commit barrier.

`--negative-publication-control` removes only the invocation inside catalogGate
in that temporary copy. It compiles and exits 1 at the exact assertion
`Retirement-winning recapture crossed catalog publication`, proving the race
regression detects this finding even with the later exposure guard retained.
Temporary candidates/classes/type slices are disposed. These are focused tests,
not a Gradle build or pinned app-toolchain receipt.
Previously run pinned Node 24.21.0 MCP bounds/packages/protocol suites: **24 pass**.

No heavy slot was granted/used. Both root patches remain unapplied;
full Kotlin/native builds, durable review storage, pixel crop/UI composition,
MCP registration, final failure-package publication, Windows/phone recapture,
image-registration quality and A30 hardware/owner acceptance are unrun. No new
user-visible flow is wired yet. Requirement targets/statuses and gates remain
unchanged. No verification images were created. Existing drafts/evidence remain.

Next decision: admit the two narrow patches and assign the follow-up native
lineage leaf; coordinator/picker composition can proceed independently. T5.01 is
L (plan: gpt-6-astra/max); inherited session settings were not changed. Krita
remains deferred.
