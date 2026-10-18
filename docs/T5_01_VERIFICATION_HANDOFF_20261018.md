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
- `tools/tests/T501VerificationLeafProbe.kt` and
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

The unapplied `private-recapture-owner.patch` adds those calls plus
`recaptureOriginal(session: String, original: PackageInfo)` to the existing
coordinator. It factors the current capture body into `captureInside`, preserving
its ownership/grant checks, and invalidates before unpublish. Both patches pass
`git apply --check` against the baseline; neither postimage is compiled.

**2. Aligned marker correspondence and owner assessment.** Read `semantic.json`
with existing `WorkbenchPackageCatalog.readFile(id, target, manifestSha256,
"semantic.json", 16 MiB)`. `McpAlignmentSnapshot.fromCheckedPackage(info, bytes)`
uses existing `McpJson` and `SemanticElement` records without inventing the
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

`python3 tools/tests/test_t501_verification_leaves.py`: **20 cases pass**.
It compiles the three leaves and existing McpJson with the cached Kotlin 2.4.0
compiler, warnings denied, against exact unmodified production DTO declaration
slices. This is a bounded leaf test, not a Gradle build or pinned app-toolchain
receipt. Temporary classes/type slices are disposed. The first run exposed an
unnormalized malformed-UTF-8 note exception; fixed refusal then passed all cases.
Previously run pinned Node 24.21.0 MCP bounds/packages/protocol suites: **24 pass**.

No heavy slot was granted/used. Both root patches are unapplied and uncompiled;
full Kotlin/native builds, durable review storage, pixel crop/UI composition,
MCP registration, final failure-package publication, Windows/phone recapture,
image-registration quality and A30 hardware/owner acceptance are unrun. No new
user-visible flow is wired yet. Requirement targets/statuses and gates remain
unchanged. No verification images were created. Existing drafts/evidence remain.

Next decision: admit the two narrow patches and assign the follow-up native
lineage leaf; coordinator/picker composition can proceed independently. T5.01 is
L (plan: gpt-6-astra/max); inherited session settings were not changed. Krita
remains deferred.
