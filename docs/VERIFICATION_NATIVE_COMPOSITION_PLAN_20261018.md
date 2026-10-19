# Native verification root composition boundary

Await Astra source admission before applying8912cc39ff72329a71c7c4ece6a72aaf43fd7016
to active code. The four worker leaves/tests are fetched only. Root lineage
code88946bae68a265bf98a4e30f2f75b6066a1afc88 and receipt candidate
8bf738ad72c86ee3e7c752e98e6d8b54bb03eed8 have28passing package cases and strict
Clippy and remain separately pending review. Main is reviewedf464611.

The worker's exact private settings prefix is `vw.verification-review.v1.` and
its exported constant is `VERIFICATION_REVIEW_PREFIX`. Before registering the
store module, root must add the documented DELETE on the archive's private
SQLite copy, before the final VACUUM. Source database/notes and unrelated settings
remain intact. The worker already includes an archive export regression checking
no prefix rows/notes remain and source snapshot is preserved; root will also run
the existing archive suite and corruption/rollback regressions. No public store/
FFI/UI activation may precede this filter.

The two pending candidates define the same six-field VerificationOrigin shape
in different private modules. Root must compose ONE public record, never both:
use the reviewed worker leaf's record at crate::VerificationOrigin and point
Extensions.verification_origin to it, retaining None omission/default and the
already proposed schema/array bounds. Remove root's duplicate manifest definition
when composing. Public field names/types remain unchanged; the worker adds
Clone/PartialEq/Eq traits. Resolve importer validation through the accepted
source's validator while preserving the root contract invariants/tests. Record
this small root adaptation explicitly and review/test it on the combined SHA.

Root-only exports/helpers after admission: store lib module/re-export; package
lib module/re-export; private consuming Package::into_verification_parts and
checked Package::verification_allocation_bytes. Ownership moves encoded output
files without clones; original/after package owners remain borrowed and admitted.
The failure compiler requires a normally compiled prepared subset on the exact
after source and validates original text/roles/kinds and public identities. It
does not manufacture the subset, infer geometry alignment or grant publication.
Root's current review/pair/catalog/receipt/version fence must be real before UI
or native FFI activation; a constant true callback is a synthetic test witness.

The source/library hooks alone are a durable native foundation. No manual owner
assessment is exposed as an agent setter, no selector or note enters lineage,
and full T5.01/image alignment or hardware acceptance stays open. After admission,
compose a single store/package/lineage/privacy batch and run relevant package,
review/CAS, archive and strict native checks. Public FFI/shared/API registration
and Kotlin/native caller tests are a later coherent activation batch; no binding
regeneration is needed merely for private modules/serde metadata.
