# Format root prerequisites and remaining caller handoff

Root implements F's c936c27ad requested model/raster prerequisites in this
publication. The reviewed R compositor has returned; its original raster/model
roots are now integrated under root ownership. The Desktop-only R caller grant
in DESKTOP_OUTPUT_CALLER_HANDOFF_20261018.md does not include these Rust files.

`vw-raster::export_with_output` is now public and re-exported. It retains R's
checked D-to-O metadata preparation before encoding; this is an additive API,
not a new compositor or a post-hoc provenance patch. `display_preview_rgba8`
uses the exact existing display conversion, admission, cancellation and color
consent code, admitting only `original_available=false` inputs. The ordinary
entry retains its existing original requirement/error/check order. No flag is
temporarily set true. The result is the existing display derivative type and
grants no full-original export or RasterSource authority. Preview depth describes
preview samples, never RAW sensor bit depth.

The model admits psd/psb/dng/cr2/raw-preview with typed, truthful import notices:
flattened_composite exactly for supported8/16-bit PSD/PSB; raw_preview exactly for
8-bit preview-labelled containers; first_tiff_page_only only for TIFF. Unknown,
nested, wrongly typed or inconsistent metadata remains refused. RAW source
container hashes/bytes remain immutable and original availability remains false.

Four focused Rust test methods are authored, UNRUN: format/notice/depth admission
and refusal; canonical preview conversion and persistent original/export refusal;
consent/memory/cancel/shape checks. Final commands after source composition are
`cargo test --locked -p vw-model --test format_notice_admission` and
`cargo test --locked -p vw-raster --test display --test preview_display` with the
parent heavy slot. Source formatting/whitespace checks passed; this does not
claim native compilation, regenerated ABI or actual OS/RAW runtime execution.

F c936c27ad is still under its own narrow review and has not been composed here.
It cannot be accepted as an actual caller path from synthetic ABI scaffolding.
Its native caller cases and final shared/app assembly remain deferred, while
the excluded picker/controller routes below are actual implementation work.

## F export-chooser handback

Parent delivers to existing F owner01a14eeb-8086-76af-9670-dd5ffa5eb032; root
will not edit these paths concurrently after this publication:

- apps/android/src/main/kotlin/com/visualworkbench/android/editor/ExportSelection.kt
- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/DesktopTransfers.kt
- apps/desktop/src/test/kotlin/com/visualworkbench/desktop/DesktopTransferTest.kt
- NEW dedicated Android export-selection tests

Remove the BMP-only availability refusal/messages only in the coherent caller
composition using the accepted in-file BMP provenance encoder. Preserve all
alpha/matte/depth/color/output-region/no-clobber/budget policies and ordinary
transfer behavior. No fake encoder or sidecar-only provenance. Avoid overlaying
older export/transfer files. This is narrow chooser/preflight enablement; return
any actual D-to-O region callsite changes needed by the R controller owner before
changing its state/API. The active frame's output region must never be silently
treated as a D rectangle. Other platform roots remain excluded.

R's Desktop controller owner must consume F's published suffix-intent adapters
and native RAW-preview creation/notice APIs after F review/composition; route
PDF-compatible AI through the existing PDF importer, not ordinary image decode.
Keep explicit preview labeling and unavailable-original export refusal. The
Android controller/Activity/WorkbenchApp owner must do the corresponding picker,
intent and native-preadmitted URI route after explicit handback or parent
assignment. No Android root ownership is changed by this document. For the
separate giant-URI intent, use root's preadmitted native staging; foreign
pre-spooling cannot substitute for it. Ordinary bounded image/RAW-preview intake
remains separate. Do not send RAW containers to the fixed giant JPEG producer.

## O verification/prompt hooks and V coordinate contract

O2d633a87 is held for review/composition. Root will apply its truthful O prompt
labels in package/prompt.rs and frame-preservation check in
package/verification.rs only with the positive O candidate. Those fields do not
yet exist in the current package base, so no uncompilable forward field access
is added early. Ordinary package prompt/golden bytes must remain unchanged.

Existing V owner01a1507a-2f92-71e6-b312-c985eb631012 already exclusively owns
FFI verification/checked.rs. On parent delivery, apply O's exact two refusal
hunks there before composing transformed packages (catalog pair original/after
or checked current source with Some image_output returns Unsupported). Root
does not overlap V's currently active P2 repairs. Preserve all approved history,
CAS, selector/grant and original-owner checks.

Then implement actual bounded D/O-aware verification alignment in V's existing
native/shared/Desktop registration/workflow files. The checked metadata is
descriptive geometry: D source dimensions/hash/marker coordinates remain D;
package image maps are O followed by target scaling. Use the single canonical
frame for crop/quarter-turn/resize projection, actual source/PNG owners and
quality-tested comparison. A square quarter-turn is not pixel identity. Never
infer alignment from equal dimensions or convert an O displacement directly to
a D translation. Correctly inverse-map supported measured displacement/geometry
to the existing native D-space translation contract. Refuse ambiguous,
nonrecoverable or unimplemented mappings explicitly rather than asserting a
translation. Keep the interim refusal until the real reviewed route replaces it.

Return exact additional shared/package API hooks before editing reserved files;
no second source owner, alignment authority, compiler or captured-pixel factory.
This is real implementation work, separate from the final Windows/A30/physical
journeys. Synthetic alignment fixtures prove algorithms only. Root's central
ledger retains unsupported cases and unperformed tests explicitly.
