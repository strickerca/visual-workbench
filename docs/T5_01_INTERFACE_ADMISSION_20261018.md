# Verification interface admission, 2026-10-18

Root inspected both unapplied proposals and dedicated leaves at 6591961d.
This admits a bounded implementation contract, not the unreviewed leaf batch,
compiled caller behavior, image alignment or an owner verdict. Await Astra's
leaf verdict before composing active code; root will adapt the patches deliberately.

## Private recapture owner

Admit `rememberCapture(info: PackageInfo, selector: String)` only after the exact
owned capture settles, under the current grant. The actual picker/native owner
retains the selector-to-target identity. Keep selectors/OS targets session-private:
never add them to package, review persistence, lineage, diagnostics or public DTOs.
Retire associations before existing unpublish/drain; join actual captures before
registry close. A cancellation, indicator, grant or target failure grants nothing.

Admit `recaptureOriginal(session, original: PackageInfo)` as the coordinator's
private owner operation. Resolve and recheck the exact original package/target/
manifest hash through the current checked catalog, with retiring checks, before
begin and again at completion. Reuse the existing exact-selector grant, drawn
indicator, capture gate and joined cancellation. Only the actual joined capture's
published PackageInfo may complete its ticket. This records a route association;
it does not prove image alignment, exterior pixels or semantic correspondence.

Do not apply the proposal mechanically: its post-publication `rememberPackage`
call must not turn an already durable ordinary package publication into a failed
result when the optional verification registry is full/unavailable. Preserve the
published result, expose verification unavailability explicitly and fail closed
for recapture. Never silently overwrite a route, evict a live ticket, mint a new
grant, or release admission before actual capture completion.

## Public failure-package lineage

Admit the proposed optional `Extensions.verification_origin` fields unchanged:
original_package_id, original_target, original_manifest_sha256,
comparison_receipt_id, comparison_receipt_blake3, original_marker_numbers.
They contain public immutable identities only. Entry n maps follow-up marker n+1
to the original marker number. Preserve ordinary contiguous marker numbering and
initialize None for ordinary compilation. Root retains manifest/compiler/admission/
verify registration and will compose the bounded optional-field changes separately.
No protocol/schema version, signing, dependency or generated-binding change is
admitted by this document.

Final construction must derive original identity from a verified Package and
receipt identity/hash from the exact checked review pair, with a current review
version. Refuse pending markers, duplicates, out-of-range/unknown original numbers,
changed package/hash/receipt, self-reference, target mismatch or mapping-count
mismatch. An all-pass review produces no failure package. Verify the final bytes
through existing Package::from_files and its normal bounds/proof checks; a
lineage field is descriptive and cannot authorize pixels or replace proof.

Owner assessments are explicit manual input. No agent verdict setter or automatic
pass inference is admitted. Semantic translation remains unresolved unless the
exact existing alignment leaf proves a unique stable-ID correspondence and root
checks overview mappings/pixel bounds. Image registration remains open.

## Exact continuation file grant

The parent may grant the verification owner these new dedicated leaves/tests:

- core/crates/vw-store/src/verification_review.rs
- core/crates/vw-store/tests/verification_review.rs
- core/crates/vw-package/src/verification.rs
- core/crates/vw-package/tests/verification.rs

Use the existing ProjectStore transaction/settings owner for bounded durable
review records keyed by the exact immutable package/receipt pair, with versioned
CAS and strict checked decoding. Preserve unrelated data and prior records on a
failed write. Keep manual notes private and bounded. Export/archive privacy needs
an explicit prefix-removal handoff before this store is wired; do not copy notes
or selectors into failure lineage. No new database, filesystem authority or wire
resolution engine is needed. Root retains module exports/registration, archive
composition, shared/FFI APIs and host/controller wiring. Existing three dedicated
MCP verification leaves and their focused probes remain with that lane.

The package leaf may attach the checked lineage to normal bounded compiler output
and verify final inventory/manifest bytes; it must preserve original marker
references in the follow-up instructions. Return exact proposed root exports and
input types rather than editing shared roots or handing root an unfinished whole
workflow. Worker source may proceed while root owns the heavy slot; full tests
wait for the parent's slot handoff. Do not absorb picker/native target ownership,
new UI roots, credentials, live calls, device work or other lane implementations.
