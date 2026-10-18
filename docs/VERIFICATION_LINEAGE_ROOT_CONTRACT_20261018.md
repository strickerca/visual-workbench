# Optional verification lineage — root source contract

Candidate `88946bae68a265bf98a4e30f2f75b6066a1afc88` on cloud/verification-lineage-contract-20261018 implements the
six-field contract admitted in T5_01_INTERFACE_ADMISSION_20261018.md. It is based
on exact reviewed/published main 30b77615. Main has been advanced normally through
accepted G1 receipt02f5766 and private recapture30b77615; both remote SHAs were
verified. This new lineage candidate remains source-only and awaits review/tests.

`vw_package::VerificationOrigin` is exported by the existing manifest wildcard.
Its public fields are unchanged: original_package_id: Id, original_target: String,
original_manifest_sha256: String, comparison_receipt_id: Id,
comparison_receipt_blake3: String, original_marker_numbers: Vec<u32>.
Use `manifest.extensions.verification_origin: Option<VerificationOrigin>`; normal
compilation initializes None and omits it. No schema-version, dependency, FFI DTO
or binding-generation change is required. Known optional JSON null means None.

The mapping is positional: entry n maps new contiguous marker n+1 to its original
number. Native admission checks original ID differs from output package ID,
original_target equals the output compiled_for.target (one of the four existing
target names), both hashes are canonical lowercase64, mapping is nonempty and
matches marker count, numbers are unique1..512. The array preflight applies the
configured marker limit before serde allocation; fixed stack uniqueness census
adds no second growing inventory. Existing package payload/proof checks still run.
The schema covers local shape/bounds; native checks cover cross-field constraints.

Imported descriptive lineage cannot authenticate a review or prove an original
marker exists. The granted failure compiler must derive exact original package
and checked receipt identities/current review from its actual owners, validate
original marker membership and generate no package for an all-pass review. Private
notes, selectors, paths and OS identities never enter lineage. The original target
must match the follow-up target; do not silently mint a new destination. Module
exports, shared/FFI APIs and archive privacy filtering remain separate root work.

Seven native contract tests are written and NOT RUN. Rust formatting, whitespace
and source secret scans pass. The schema structure and15 lightweight valid/invalid
fixtures pass; that is not native compilation/verification. Image retains the
heavy build slot for PI1; no compiler or per-commit bindings were launched here.
See `evidence/VERIFICATION_LINEAGE_CONTRACT_20261018.json` for exact paths/hashes.
No four-file verification leaf or other granted platform/privacy source was edited.
After parent slot handoff, run focused package tests/strict Clippy and send exact
source/results to Astra before coherent native review/privacy registration.

## Native validation after slot handoff

The image owner released the slot. On unchanged six-file source, all28 package
unit/integration tests pass across six result blocks, including the seven new
contract tests. Strict all-target package Clippy passes with warnings denied.
Both first attempts succeed. Cargo manifests/lock remain identical to accepted
main; no extra native app build or bindings were needed. Exact results are in
`evidence/VERIFICATION_LINEAGE_VALIDATION_20261018.json` and its excerpts.
New lineage source still requires Astra review before main promotion. The current
verification owner can use this exact typed field/type contract while retaining
its four-file grant; return tiny root exports/privacy hunks separately. PI1
ba338ad is fetched only and remains uncomposed pending its independent verdict.
