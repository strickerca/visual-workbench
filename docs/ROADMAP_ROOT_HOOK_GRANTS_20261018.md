# Remaining root-hook implementation grants

This continues the initial parallel grants. The publication containing this file
is the common working base: it composes accepted offline/Desktop/Air source
dd18c238, editing707a92c/portable9e41ed1, formatsf8df49e9, giant2f018f296
(GP1 repair9d7ffa1 accepted) and verification490de501 (VF1 accepted). Root will
report the exact publication SHA to the parent. Workers fetch that SHA and use
only the paths assigned below; no preserved recovery archive is an overlay.

Parent activates and assigns the R/O/F jobs to available existing workers. The
V/A/P entries extend their already named owners on parent delivery. These are
source implementation grants. One heavy build globally, at most two build
workers, allocated by the parent; broad builds, ABI generation, package assembly
and device/runtime journeys remain deferred to the coherent final tree. Cheap
focused checks must preserve actual failure receipts. Return candidate SHA,
changed paths, review binding, actual checks and proposed central-ledger rows.

Root retains Cargo/lock, protocol/schema, generated bindings, NativeCore,
workspace/module registration except the R roots explicitly listed, both main
EditorControllers, Desktop Main, WorkbenchApp, and native/Android URI staging.
The Air owner still owns its Activity/canvas/controller/manifest/remote paths;
the capture owner still owns Desktop Main/capture until explicit handback. Do
not infer their release from source acceptance. Root does not simultaneously
edit a delegated file. API changes outside these lists require an exact minimal
hook request. No paid calls, credentials/provisioning, signing/deployment,
driver/security/device identity changes or hardware execution. Krita stays
deferred. Compilation never passes physical or product gates.

## V: private review history, existing verification owner

Owner: **01a1507a-2f92-71e6-b312-c985eb631012**. Extend the existing V grant by
exactly these two store files:

- core/crates/vw-store/src/verification_review.rs
- core/crates/vw-store/tests/verification_review.rs

Implement the real restart/history retrieval route through the existing V-owned
native/shared/Desktop modules. Add the requested method on the existing locked
store, without a second database, index, catalog or selector persistence:

```text
verification_review_history(after: Option<String>, limit: u32)
  -> VerificationReviewResult<VerificationReviewHistoryPage>
VerificationReviewHistoryPage {
  snapshots: Vec<VerificationReviewSnapshot>, next_cursor: Option<String>
}
```

Limit at most8 records and2MiB aggregate admitted bytes. Validate the canonical
namespace/hash cursor, schema, row size, key/project/record binding before
allocation/return, using the existing connection/transaction. Fresh v0 reads do
not create history. Cover reopen, pagination, corruption, limits and unchanged
portable privacy. Existing archive-copy note removal and the six admitted
lineage fields remain unchanged.

Root supplies `withCurrentVerificationOriginal(body)` beside the existing
planned `withVerificationOriginal(expected, body)` under the editor's existing
edits mutex, through `McpVerificationOriginalAccess`. This borrows the currently
open original before the historical pair is known. No raw path or new source
lifetime; all later mutations/compiles keep exact currentness checks. History
selection never restores capture selectors or grants. Return exact additive
access-interface signatures promptly; the root controller/Main hooks remain
excluded.

## R: canonical output compositor and model registration

Parent assigns one idle owner. Exclusive files:

- core/crates/vw-geom/src/lib.rs
- core/crates/vw-model/src/lib.rs and src/validate.rs
- core/crates/vw-raster/src/lib.rs, src/render.rs, src/export.rs, src/stream.rs
- core/crates/vw-{geom,model,raster}/tests/image_output.rs and NEW dedicated
  output composition tests in those crates

Implement the exact geom/model/raster hooks in
EDITING_OUTPUT_ASSISTED_HANDOFF_20261018.md against the already composed leaves.
Use one `ImageOutputFrame` matrix and one compositor. Keep existing wrappers and
ordinary behavior; admit the image-output layer kind and validation, exclude its
metadata from drawing, and add annotation mode to the current renderer rather
than copying tone/result composition.

Stable shared hooks for F/O are the handoff's current `DocumentOutput`,
`ImageOutputMetadata`, `requires_document_output`,
`prepare_document_output`/`render_document_editing_region`,
`prepare_export_pixels_with_output(source, request, output, cancel)` and
`export_with_output` contracts. Add optional defaulted `ExportMetadata.image_output`
and initialize existing literals with None. Embed truthful original D extent and
D-to-O provenance before encoding. The legacy stream PNG API must refuse active
output frames unless actually routed through the bounded compositor. Preserve
ICC/depth, region bounds, current budgets and cancellation. Root has admitted
the existing vw-raster -> vw-geom Cargo edge; no dependency changes are delegated.
No FFI roots, export callers, package/compiler or platform files in this job.

## F: actual format callers and bounded output export

Existing formats owner: **01a14eeb-8086-76af-9670-dd5ffa5eb032**, activated by
parent. Extend exclusive ownership to:

- core/crates/vw-ffi/src/format_images.rs, src/streaming_ffi.rs, src/queries.rs
  and NEW dedicated format/output caller tests
- apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/OriginalFormatsApi.kt
- apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeOriginalFormats.kt
- apps/shared/src/commonTest/kotlin/com/visualworkbench/shared/OriginalFormatsTest.kt
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/OrdinaryFormatFiles.kt
  and its OrdinaryFormatSelectionTest.kt
- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/OrdinaryFormatFiles.kt
  and its OrdinaryFormatSelectionTest.kt

Wire accepted bounded PSD/PSB decode/inspect and visible flattened-composite
notice, in-file BMP provenance, existing TIFF metadata and explicit RAW embedded
preview intake. Retain original source bytes/hash. RAW previews remain visibly
labelled, `original_available=false`; never present them as full original pixels,
feed them into `BorrowedSource::new`, or sniff arbitrary RAW as TIFF by extension.
Keep supported variants and refusal behavior exactly bounded by accepted leaves.

Also implement the handoff's metadata-aware `encode_private_with_output` and
`encode_buffered_with_output` wrappers with ordinary calls passing None. Use R's
stable `prepare_export_pixels_with_output` before codecs. Route legacy queries
through `image_frames::admit_legacy_export/legacy_export` when required, filter
image-frame editing metadata, and route seekable preflight/export through the
existing `image_frames::prepare_output`. O dimensions/regions must be truthful;
preserve existing source/working charges, scratch caps, no-clobber FileOwner and
joined retirement. Remove the BMP chooser gate only with coherent provenance
routing. No new export/cache/budget owner. Controllers/Main/NativeCore/module
registration remain excluded; return their exact tiny hooks if needed.

## O: package pixels and instruction geometry through the output frame

Suggested existing idle offline owner: **01a1500a-13fe-77e4-835e-201034a9901c**;
parent activates. Exclusive:

- core/crates/vw-package/src/compiler.rs, src/geometry.rs, src/pixels.rs,
  src/image_receipt.rs, src/verify.rs
- NEW core/crates/vw-package/tests/image_output.rs and dedicated output fixtures

The original recovered-outbox work is already composed; do not rewrite it.
Implement actual image-output package mapping using the accepted model frame and
R's canonical raster exports. D remains immutable oriented source authority.
Apply the same `ImageOutputFrame` D-to-O matrix to pixels, marker points/bounds,
contexts and package image provenance, then ordinary target mapping. Refuse a
crop that omits any exported instruction rather than dropping or clipping its
meaning. Keep text/role/kind/original marker identity, exact revisions/source
hashes, existing package validation, bounded compiler and cancellation. No second
compiler or display-thumbnail source. Verify identity/crop/quarter-turn/resize
and out-of-crop refusal with synthetic inputs; return exact metadata/schema/root
hooks before needing changes beyond this list.

Existing manifest/schema/six verification lineage fields, verification modules,
private reviews, catalog and FFI packages/compile.rs remain root-owned. Do not
silently add a portable schema extension. If current admitted mapping cannot
truthfully express a transform, return the precise required additive contract;
root coordinates it before promotion. Root separately adds V's same-owner
constructor/original resolver hooks in FFI packages/compile.rs.

## A: complete provider credential, dispatch and settlement code

Owner: **01a15145-4045-72b2-830e-fe6043d55011**. Candidate135747df is held for
Astra's retryable-close ownership/prompt-rewriting findings; finish those repairs,
truthful provider result provenance and retained FLUX cost receipt first. The
two platform `AiServiceController.kt` files were already in the A grant and stay
exclusively with A, including late-delivery cleanup. Root performs no edits there.

Extend the existing A grant by these exact files and dedicated provider tests:

- core/crates/vw-ai-provider/src/secret.rs
- core/crates/vw-ai-platform/src/lib.rs, src/android.rs, src/windows.rs
- core/crates/vw-ai/src/budget.rs, tests/budget.rs, tests/ledger_integrity.rs
- apps/shared/src/androidMain/kotlin/com/visualworkbench/shared/AndroidProviderRuntime.kt
  and AndroidProviderKeyStore.kt
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/ProviderKeyDialog.kt
  and ProviderIo.kt, plus ProviderIoInstrumentedTest.kt
- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/AiKeyDialog.kt

Complete actual provider-scoped protected-secret capability, sensitive raw-key
headers, adapter dispatch in the already owned ai_edit leaves and fixed-charge
settlement in the existing BudgetLedger/attempt identity. Closed provider IDs map
to separate protected namespaces; preserve the existing OpenAI alias/API and
behavior. No cross-provider/environment/chat fallback, secret/body logs, retries,
second ledger or configured-quote-as-actual-charge. Carry authenticated actual
provider cost evidence from PollResult to settlement, or leave the consumed
attempt unresolved. Confirm actual receipt semantics from primary specifications;
do not manufacture billing authority from caller JSON. Add offline synthetic
scope/header/settlement/cancellation/pending tests. This grants code changes,
never reading/provisioning real credentials or issuing a live/paid request.
Root retains manifests/lock, NativeCore/main controllers and generated bindings;
return exact tiny factory/dependency/registration hooks promptly.

## P: production compatibility receipt admission/import

Owner: **01a15145-22a1-76b1-ba3e-d2bbe0326006**. The original P work through
source73e06f0/portable1daa7a65 has a supplied exact Astra review binding. Its
T4.03/04/08 normal source exists; actual composed runtime/physical journeys remain
deferred. T4.07's production receipt importer is real missing implementation.

Extend P exclusively to NEW shared commonMain `RemoteCompatibilityImport.kt`,
NEW shared jvmMain `NativeRemoteCompatibility.kt`, their dedicated tests, and
NEW Desktop remote `RemoteCompatibilityImport.kt`/dedicated tests. Existing
NativeRemoteEdit.kt is also delegated for only the minimal checked observation
capability/factory hook; preserve command/transport/guard and current lifetime
contracts. P's existing palette/panel/profile-evidence leaves remain its own.

Implement a bounded production importer callable from the actual owned Desktop
palette panel, admitting real receipt bytes/source image, exact current editor
identity/tool/settings, method/input API, device and evidence digest before the
existing `palette.compatibility(tool, settings, records)` call. Current target
borrowing must stay within its existing owner and joined lifetime. A digest,
native Injected/ACK/query result, profile metadata or synthetic fixture cannot
assert observed editor effect/physical support. Historical evidence never grants
commands; source/identity changes refuse stale adoption. Preserve honest Not
tested and all existing source-derived action eligibility.

Receipt authenticity must follow the existing actual observed-receipt owner,
not a new caller-asserted trust bit or arbitrary signed catalog. Inspect that
source first and return any additional exact native receipt-reader file proposal
before editing it. Catalog/profile selection policy, NativeCore, FFI/session
roots, transport, existing DesktopRemoteCatalog and all platform main/controller
files remain excluded. No actual editor observations, GUI injection, device
actions, invented receipts or Krita activation in this lane. The implementation
can admit no records when real evidence is unavailable, but must include the
production call path and test real refusal/adoption against owned synthetic
receipt fixtures without claiming those fixtures as product evidence.

## Root closure and review

Root next implements module/capability registration, V's checked package/source
and editor hooks, active-frame viewport/platform callbacks and preadmitted URI
staging. Platform hooks wait for explicit Air/capture handback or a separate
nonoverlapping grant. All worker returns receive narrow source review before
promotion. One final policy/ABI/native/shared/app/package validation sequence
binds the coherent tree; unperformed runtime/device/physical gates stay open.
