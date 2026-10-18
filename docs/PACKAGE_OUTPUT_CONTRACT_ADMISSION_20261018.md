# Output-frame package contract admission

Coordinator decision for O request37fd4817c5532f28864c1f71ad514b848086593c.
This admits the following source contract and delegates its implementation to
the already activated O owner01a1500a-13fe-77e4-835e-201034a9901c. The refusal
checkpoint is a safety repair, not completed transformed package support.

Extend O's exact file grant to:

- core/crates/vw-package/src/manifest.rs
- contracts/package.schema.json

Add `Extensions.image_output: Option<vw_raster::ImageOutputMetadata>` with
serde default and omission when None, using precisely the existing canonical
metadata/frame validator. Publish the worker's fixed bounded imageOutput schema:
version1; source/output dimensions1..32768; contained nonempty crop; clockwise
degrees0/90/180/270; PremultipliedBilinearSourceSamplesV1. No arbitrary affine,
executable/resolver, pixel replacement or alignment authority is admitted.
Native admission additionally checks crop containment, source extent/hash,
output dimensions, fixed resampling, every mapping/marker/context and PNG receipt
before accepting images. The JSON schema alone cannot prove those relations.

Version/compatibility rule: retain vip-1 and the established field meanings.
This is an additive bounded geometry extension. Ordinary None packages retain
exact existing serialized bytes and `extensions.compiler` value
`vw-package-1-area-integer`. Every Some package MUST use a distinct exact compiler
marker `vw-package-1-image-output-v1`. The verifier accepts that marker only with
valid Some metadata and accepts the ordinary marker only with None. Older native
verifiers then refuse the unsupported compiler/extension rather than interpreting
transformed pixels through the ordinary mapping. No permissive fallback or
post-hoc provenance patch. Keep all six verification lineage fields unchanged.

Source width/height/hash and marker point_document/bbox_document stay in oriented
immutable D. Apply the one admitted frame to package image pixels and all
target/crop point/bounds calculations; ImageMapping rectangles are in O before
ordinary target scaling. Target coordinates must match actual encoded pixels.
PNG receipts include and authenticate the same output metadata when present;
ordinary receipts remain byte-compatible. Refuse any omitted/partial exported
instruction, including hidden markers selected by the instruction exporter.
Do not change role/kind/text/original number, claim imported metadata as current
source authority, or substitute a preview for the original.

O completes its existing compiler/geometry/pixels/receipt/verifier work plus
these two paths. Preserve existing package tests/goldens and add transformed
round-trip, schema/metadata/marker mismatch, legacy-compiler mismatch and omitted
instruction refusal coverage. A transform remains unsupported until all its
pixel/geometry/receipt/verification paths are composed and reviewed. Return exact
source and changed-path binding for Astra. Broad native/runtime checks remain in
the central deferred ledger; root performs no simultaneous edits in these two
newly delegated files.

Verification callers must not infer same-pixel alignment from this descriptive
extension. If the current alignment workflow cannot handle an output frame,
return an explicit unsupported/refusal hook for V/root instead of inventing a
translation or silently comparing D and O. No review/catalog/schema-version or
capture-grant change is included in this admission.
