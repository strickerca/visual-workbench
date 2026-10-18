# Admitted caller boundaries and exclusive slices

This is the next caller implementation batch, based on integration `091e668`
plus the shared Android Build Tools pin and diagnostics cancellation repair.
Recovery reference remains `0d91b705`; no stale controller postimage is admitted.
These assignments admit source interfaces, not native/runtime acceptance.

## Shared interfaces admitted in source

Integration owns these files under
`apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/`:

- `OriginalFormatsApi.kt`: recovered `OriginalFormatInfo`,
  `WorkbenchOriginalFormats.originalFormatInfo(WorkflowBinding)`, receipt and
  revision validators, and the explicit first-TIFF-page notice. Validation never
  grants depth reduction, color conversion or preview-as-original permission.
- `DocumentsApi.kt`: recovered names for source/page/tile/operation/reply/export,
  `WorkbenchDocumentsCore`, `WorkbenchDocumentsProject` and `DocumentJobs`.
  `DocumentFailureKind` retains the proposed closed list. `DocumentFailure` takes
  only that kind, not a raw Throwable. Password/source-bearing records have safe
  debug formatting; use their properties, not data-class copy/destructuring.
- `DocumentJobsOwner.kt`: one app-owned injected `DocumentJobs` owner. Use
  `install(jobs)` once, pass the owner as `DocumentJobs` to interactions, and
  await `close()`. Failed close keeps the same owner for retry. New runs and
  replacement installation are refused after close begins. Do not add recovered
  global `installDocumentJobs` / `installedDocumentJobs` singletons.
- `DocumentFrames.kt`: accepted `DocumentFrameBinding(sourceAssetId, documentId,
  page, revision, attachmentEpoch, viewportGeneration)`, `DocumentDisplayTile`,
  `DocumentDisplayFrame` and `validateDocumentDisplayFrame(frame, current)`.
  Validate again before bitmap preparation and canvas adoption. Frame byte arrays
  stay unchanged and owned until all readers retire. Platform allocations need
  a separate reservation; this validator is not a GPU budget or hash verifier.

## Image owner — accepted ordinary-format/AVIF adapter slice

Exclusive implementation files:

- `core/crates/vw-ffi/src/format_images.rs`
- `core/crates/vw-ffi/src/avif_images.rs`
- `core/crates/vw-ffi/src/format_seek_tests.rs`
- `core/crates/vw-ffi/tests/ordinary_formats.rs`
- `core/crates/vw-raster/tests/ordinary_format_preparation.rs`
- `apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeOriginalFormats.kt`
- `apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeAvifImages.kt`
- `apps/shared/src/androidMain/kotlin/com/visualworkbench/shared/AndroidAvifImages.kt`
- `apps/shared/src/androidDeviceTest/kotlin/com/visualworkbench/shared/AvifAdmissionInstrumentedTest.kt`
- New `OrdinaryFormatFiles.kt` and dedicated `OrdinaryFormatSelectionTest.kt`
  under each platform's editor package (desktop's package is `desktop`).

Keep `decode(bytes: &[u8], limits: DecodeLimits, cancelled: &dyn Fn() -> bool)
-> Result<DecodedImage, RasterError>`, `is_format`, `label`, bounded
`import_metadata`, and the recovered seekable/private encoding adapter signatures.
Dispatch BMP/TIFF to `vw-formats`, then admitted AVIF to `vw-avif-os`, then the
existing `os_images::decode` HEIC/raster route. Check cancellation before and
after non-interruptible decode. Never retry refused metadata with a generic codec.

The leaf may declare the recovered UniFFI `OriginalFormatInfo` record and
`ProjectSession::original_format_info(&self, binding: WorkflowBinding,
cancellation: Arc<Cancellation>) -> Result<OriginalFormatInfo, WorkflowError>`.
Its read executes on the existing project worker, verifies the full revision and
document membership, and reads canonical preserved-asset metadata. Root registers
the module and forwards the existing project facade to `originalFormatInfo`.

Integration owns the following agreed prerequisites, composed in the next native
caller batch rather than as half-connected changes in the diagnostics build:

- `vw-raster::ExportFormat::{Bmp8,Tiff8,Tiff16}`;
  `PreparedRaster { image: DecodedImage, metadata: ExportMetadata }`;
  `prepare_export_pixels(&DecodedImage, &ExportRequest, &dyn Fn() -> bool)
  -> Result<PreparedRaster, RasterError>`. Extract existing crop/profile/matte/
  depth preparation once. The external adapter must not repeat color conversion.
- Shared `ImageFormat::{Bmp8,Tiff8,Tiff16}`, native DTO/native conversions and
  exhaustive enum mappings; Android `ExportSelection.kt`, desktop
  `DesktopTransfers.kt` enum/mapping hooks remain integration-owned.
- `RasterError::CodecUnavailable` maps only admitted
  `vw_avif_os::Error::CodecUnavailable` to `CoreError::ImageCodecUnavailable`
  and shared `CoreFailureKind.ImageCodecUnavailable`. Preserve that distinction
  in import, background and transfer mappings; no generic Raster/color-consent
  fallback. Other refusals retain their existing domain-specific classification.
- Common AVIF header DTOs follow `AvifImagesApi.kt` and its closed failure kinds;
  native callback registration and `NativeLibrary.kt` changes remain integration-owned.

Suffix helpers must recognize `.tif` and `.tiff` as TIFF and return the actual
selected format MIME/suffix. Keep PNG-only clipboard/drag rules and explicit
color/depth/alpha policy unchanged. Wider-gamut display work is outside this
slice; preserve R164. The Android AVIF callback is restricted to the recovered
opaque eight-bit identity-oriented, profile-free declared-sRGB path; unavailable
OS support is typed, never downloaded or substituted.

The owner may provide precise outside-slice integration hunks, but must not edit
root manifests/lock, registration, raster preparation, shared common APIs,
generated bindings, controllers, Main or WorkbenchApp. Compile/test dependencies
on the above root seams are pending until that single coherent native batch.

## Document owner — admitted platform worker/IO slice

Accept the proposal at `6c3566262d92a94fffc1bec5126d0c9da1bceb70` with the
source APIs above and these exact exclusive roots:

- `apps/android/src/main/kotlin/com/visualworkbench/android/documents/`
- `apps/desktop/src/main/kotlin/com/visualworkbench/desktop/documents/`
- Dedicated platform `documents/` tests and the lane's `RECOVERY_INTERFACE.json`.

First batch: Android jobs/service/wire/IO and tile-cache modules; Desktop owned
runtime/jobs and tile-cache modules; deterministic lifecycle, wire, frame and
provider-rollback tests. Stage UI panels/controls until root admits
`DocumentController` / `DocumentState`; controls must take callbacks rather than
depend directly on the current EditorController.

Retain proposed entrypoints:

- `DesktopDocumentRuntimeCache.prepare(local: Path): DocumentJobs`
- `prepareAndroidDocumentJobs(context: Context): DocumentJobs` returning a fresh
  owned instance, never a permanent global closed singleton.
- `AndroidDocumentIO(context, core, jobs).import(uri, kind, path, device, title,
  password, progress): WorkbenchProject`; use current `ProviderIo` for owned
  bounded provider operations. Root supplies/adopts `DocumentInteraction` and
  `WorkbenchDocumentsCore` creation; caller attachment adoption stays fenced.
- `AndroidDocumentIO.publish(uri, copy, stillCurrent): Unit`; new explicitly
  chosen destinations only, revision/source checks and truthful owned-partial
  cleanup failure. Never delete or overwrite unrelated provider content.
- `DocumentTileCache.prepare(frame: DocumentDisplayFrame,
  stillCurrent: () -> Boolean): List<PreparedDocumentTile>` with validation both
  before conversion and before adoption. Retain frame/pixel/bitmap ownership and
  reserve peak foreign copies separately; no recycling of a live draw frame.

The recovered native ABI names are accepted as the root implementation target:
`DocumentJobResult`, `DocumentRuntimeArtifact`, `DocumentRuntimeOptions`,
`DocumentJobError` (generated `DocumentJobException`), `DocumentJobCancellation`,
`DesktopDocumentRuntime.run`, `openDesktopDocumentRuntime`, `documentSourceHash`,
`validateDocumentJobResult`, and `runIsolatedAndroidDocumentJob`. Root also owns
`documentRequestJson` and `documentReply`, registration, persistence, shared
interaction/controller, manifests, runtime packaging and dependency admission.
These ABI prerequisites are not installed in the current generated bindings yet.

Implement the proposal's corrections: cancellation monitoring covers bind through
completion; close signals stop before waiting on a run lock; result-ready or
service-disconnect alone is not process retirement; uncertain cleanup retains
the actual worker/descriptors/pins/scratch and blocks new work. Strict wire bounds,
UTF-8/status/EOF checks precede adoption. Match page/source/full revision and both
generations even when returning to an apparently identical viewport.

Missing verified PDFium/qpdf resources must produce `Unavailable` or `Unsupported`
without a system-library fallback. Do not gate the pure SVG operation on an
unrelated PDFium artifact: report a precise root ABI/runtime prerequisite if the
recovered entrypoint currently does so. SVG still requires its admitted isolated
worker and memory/time bounds. Native PDF/redaction, sandbox, decoder fidelity,
provider/chooser and physical acceptance remain separate actual-execution gates.

Both owners source `/workspace/.vw-tools/cloud-setup/start.sh` in every shell.
The coordinator currently owns the heavy slot; source work and bounded harness
tests may proceed. Send ready leaf code with exact root seam/dependency requests;
generate bindings once after the coherent native caller batch is composed.
