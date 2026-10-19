# AND4 staged export validation boundary

Parent activates this immediately in the existing Android platform owner
01a1507a-7956-7594-8687-9e7a9a18900b, extending its current seven-file and
remaining-caller grants exclusively to:

- apps/android/src/main/kotlin/com/visualworkbench/android/editor/ProjectRepository.kt
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/RasterTransfers.kt
- apps/android/src/androidTest/kotlin/com/visualworkbench/android/RasterTransfersInstrumentedTest.kt
- NEW dedicated Android staged-validation tests

Root will not edit these leaves concurrently. This resolves the missing file
ownership for AND4; the owner implements it together with its four pending
caller repairs. Native/shared manifests, ProviderIo, MediaHandoff and all other
ungranted files remain unchanged. Request an exact additional seam only if the
existing instrumented provider fixture cannot establish writer-open counts.

Freeze this mandatory callback parameter, placed before stage so the existing
stage trailing lambda remains clear:

```kotlin
validateReceipt: suspend (FileExportResult) -> Unit
```

Thread it through ProjectRepository.exportImage/sharePng into
RasterTransfers.export/sharePng. Immediately after the native exportTransfer or
exportFile result returns, invoke validateReceipt(result) in the original caller
context before entering the provider publication block. It must run before
provider.open(uri,"wt") and before MediaHandoff.create/FileOutputStream or any
share lease/URI is published. A refusal/cancellation propagates through existing
workspace/destination/lease cleanup; no writer or published share is created.
Do not provide a production no-op/default validator or an unsafe fallback
overload that silently skips this boundary.

The Android controller supplies a closure over the immutable ExportCallerIntent
and exact captured project reference, binding and projected settings. Check
result.revision.projectId, hostSeq and stateHash against the intent binding,
then current project/attachment/page/epoch/camera/settings intent identity and
the same projected geometry used for preflight. Check result metadata/encoded
shape through existing truthful receipt contracts as applicable. Native OPS
can advance revision despite the controller mutex; preflight or cached UI info
is not validation of the native render's actual receipt. The closure executes
inside the controller's existing mutations borrow: it must not reacquire that
mutex, create another project owner or re-project the rectangle.

Keep the existing post-return intent/currentness check before setting ExportReady
or assigning shareLease/shareUri. Preserve the precise incomplete-owned-Create-
Document deletion/reporting behavior and the existing share lease transfer and
cleanup. Do not open/truncate a mismatching provider destination and then call a
late refusal successful. No unexpected destination is deleted or overwritten.

Required regression: a synthetic exporter finishes a real staged output but
returns a different project ID or hostSeq/stateHash. The actual staged-transfer
validator refuses; the provider fixture's writer-open count remains zero and
owned scratch/incomplete destination cleanup is accounted for. Also cover
changed intent, positive receipt, callback cancellation and share refusal before
lease creation. Author device-only cases honestly when they cannot run here;
focused source/host checks may run, while aggregate Android/ABI/device execution
stays deferred. Do not claim these callbacks close the other three AND findings
or tonal/format/assisted implementation gaps.
