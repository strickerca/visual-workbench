# Android remaining callers — 2026-10-19

Branch `codex/android-remaining-callers` builds on `f60eca3`, preserves the reviewed
Android caller commit `398c0f4` as `3f90c9c`, commits AND1–4 repair `a21f4d4`,
and merges the accepted shared giant composition dependency `e72f2f1` at
`7bcea97`. This branch is for coordinator review, not a main merge or release.
The other checkout's uncommitted work was preserved. Krita remains deferred.

## Owned changes and hooks

- `a21f4d4`: frame controls remain visible when their owner blocks input; the
  giant picked-project owner survives post-cleanup cancellation; URI retirement
  retries before another giant open; native export receipt validation runs before
  opening a provider writer or creating a share lease. The callback is mandatory
  through `ProjectRepository.exportImage/sharePng` and `RasterTransfers.export/sharePng`.
  AndroidExportReceipt checks the exact immutable export intent/revision and O
  dimensions. Existing provider fixtures assert zero writer opens on refusal.
- `ef45ffe`, `faa4e58`: Android ordinary tone display uses `editingBackground`
  with project identity, page, epoch, full revision, source and size checks;
  unadopted bitmaps are recycled. Ordered changes revoke stale composition before
  refresh. The renderer receives a filtered scene that omits only canonical
  content baked into matching pixels; the controller retains full geometry for
  selection and handles. Moved saved artwork has outline feedback until commit.
  GiantAndroidCanvas binds immutable Source/Composed mode and the source-specific
  color choice to one existing viewport owner. Draw admission stays atomic on the
  graphics thread. Native levels above 2 remain an explicit composed-tile limit.
- `8f94190`, `8e75eaa`, `4399716`: the document picker exposes PDF-compatible AI
  to the PDF/password importer, whose native intake requires a PDF signature.
  The bounded 64 MiB ordinary provider staging now calls native PSD/PSB
  flattened-composite or DNG/CR2 embedded-preview creation by provider name or
  exact MIME. NEF/ARW are refused. RAW UI labels pixels as an embedded preview;
  frame and tone controls are unavailable for it. Assisted selection reuses the
  actual selection document, mutation mutex, native mask import and manual
  correction. Its worker-readable source gate uses project reference identity
  and is revoked on ordered change, page, epoch, departure and backgrounding.
  Pen prompts are committed on pen-up, so cancellation drops an interrupted tap.

The production assisted catalog is empty under app-private `noBackupFilesDir`
and pins are null. The constructor permits a reviewed catalog/pins pair; only
that pair constructs `liteRtAssistedSelectionBackend` for the actual project.
The coordinator must supply reviewed model/runtime/APK/license/memory admissions
and the app's constructor injection before local inference can run. No model,
download, permission or credential was added. ProjectRepository's import method,
native/shared APIs, protocol/schema, Gradle/manifests and Ai controllers remain
unchanged by the remaining-caller slice. `RasterTransfers.importImage` is the
additional format staging seam used under the remaining-caller directive.

## Checks and limits

`git diff --check` and staged-byte secret-pattern checks passed. Earlier focused
source-order checks for AND1–4 passed. Newly authored Android unit and device
tests cover baked draw classification, composed giant mode, provider refusal and
source-reference fencing; they have **not run**. No aggregate Gradle/ABI/lint,
native rerun, app installation, model inference, provider/device runtime or
physical S23 stylus/Bluetooth/A09 check was run without the shared build slot.
Compilation is not claimed from source checks. Real user-visible Android results
remain unverified; product gates remain open. The composed giant bridge's parent
25 focused cases belong to `e72f2f1`, not to this Android caller validation.
