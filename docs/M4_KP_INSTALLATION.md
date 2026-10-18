# Visual Workbench: Krita/Paint personal installation

Local Paint delivery, 2026-10-08. The ordinary apps are packaged and launched;
normal paired controls and owner drawing acceptance remain pending. Krita is
deferred. These results do not close M4-KP.

| Delivery field | Value |
|---|---|
| APK | `android-debug.apk` â€” ordinary `com.visualworkbench.android` |
| APK SHA-256 | `3d59e86176c83d1db4b9e9cc6a104b57b330cceadf6bc33fc241ef09de0734da` |
| APK signer SHA-256 | `662bebe5f3839662f8a3eac9ddd00c278cfbb1c872f01f2ca5c108327368c30c` |
| Windows image | Complete `VisualWorkbenchDev` folder |
| Complete output archive receipt SHA-256 | `59973e6b1e3a9d592b695e5b47c3ad11d1ab34efd3588e51be20c2850999fccb`; contains exact per-file image hashes |
| Tested editor version | Paint `11.2605.81.0`; scoped automated effect/restoration receipts retained |
| Packaged Paint commands | Undo, Redo, Brush, Eraser, Smaller brush, Larger brush, Zoom in, Zoom out; actual normal eligibility and effects pending |
| Installation/startup checks | S23 owner-user same-signer update, exact installed APK readback and normal foreground launch passed; complete Windows image opened normally; normal paired session and normal shutdown pending |

This personal build uses your existing Windows display and the S23 Ultra. It requires neither a virtual display driver nor the deferred Krita pixel/layer plugin. It is a development APK and Windows app image; no production-signing or installer claim is made.

## Install and launch

Keep the complete Windows folder together, including `app` and `runtime`, and launch `VisualWorkbenchDev.exe`. A separate Java installation is not needed for the app image. Preserve its notices and runtime legal files. Use a new folder for updates rather than mixing files from different builds; close the old app normally before switching.

Install the supplied ordinary APK on the S23 Ultra as an update to Visual Workbench. The delivery coordinator verifies the signing identity and installed bytes. Keep the existing app data. If Android refuses a signing mismatch, keep the existing installation and diagnose the mismatch rather than uninstalling it. The test/HIL app is a separate package and is not the delivered app.

Connect the phone and Windows app through the existing project connection and pairing flow. If using the optional Developer USB connection panel, select your installed `adb.exe`, inspect devices and choose only the S23. Use the matching displayed PC/phone ports. No ADB binary is bundled. Do not select or reconfigure the OnePlus. Normal network connection does not require this developer-USB helper.

## Select an editor and draw

Open a document in the validated Paint version on Windows. In `Remote Krita / Paint`, use `Refresh windows` and select the exact editor window. Check the displayed destination. Grant that window on the PC with `Grant selected window`; on the phone, open the remote computer view and use `Request PC control`. Drawing is available only while the current exact target is granted and rendered. A refused grant needs current editor focus/identity validation; never bypass the canvas guard.

The phone shows the selected window and a separate remote destination palette. Remote Undo/Redo and tool controls operate on the editor's history and settings; Workbench undo is separate. Only commands listed as enabled in the final delivery receipt are supported. A visible but disabled command is unavailable for the current catalog/target/focus. A missing applicable catalog can leave independently safe pen control available without enabling shortcut authority.

## Pause and recover

Use `Pause` on the phone or `Pause control` on Windows before changing destination. Backgrounding, disconnection or decoder recovery retires input authority; reconnecting must not silently grant control again. Verify the destination and obtain a fresh PC grant before drawing resumes.

If the app says retirement is pending, keep the owning view/application open so it can finish releasing input. Retry its normal Close only after the retained cleanup path settles; do not use a forced process kill as a normal recovery step. If video is missing, use `Show diagnostics`, retain the error/status text, and check the selected project connection and editor window. A connection label alone does not prove that video is rendering.

For an update or removal, close the remote view and application normally after input retirement. Back up needed projects before any uninstall. Android uninstall deletes that app's local data. Removing the Windows image folder does not require a driver/certificate uninstall because this milestone installs neither; preserve project/data folders separately. Do not remove unrelated app files or settings.

Physical S Pen feel/fidelity, frame rate, latency, physical A27 and owner drawing acceptance remain separate from software completion. Consult the final receipt for which checks actually passed.

## Paint command budget contract (delivery coordinator)

The current delivery work targets Paint; Krita acceptance is deferred. Before
packaging a Paint catalog, bind measured action/guard/restoration evidence and
use the exact essential-profile schema-3 values `providerBudgetMs: 750` and
`parentExchangeBudgetMs: 1000`. All other evidence digests and delivered hashes
must come from actual reviewed receipts; a template or older 180/250 ms Paint
receipt is not current acceptance. Krita's existing 180/250 ms catalog contract
is preserved for its deferred work.

Paint preparation and final validation, including both Size stages, share one
750 ms deadline. The full action-availability query shares that same limit.
The 1,000 ms parent limit applies only to shortcut or authority-refresh requests
matching exact Paint authority adopted from the pinned helper's Ready for the
opened target/catalog. Pen, Check, finite clicks/wheels and unknown, mismatched,
legacy or Krita authority remain at 250 ms. Late completion is refused and actual
input owners remain retained until release; focus and target guards are never
bypassed to meet a budget.

These are toolbar processing bounds, not S Pen latency or sustained-video
measurements. Retain separate resident-calibration and normal production-path
evidence, including all eight configured action availability/refresh outcomes.
The build and resource checks below have passed for this local delivery. Do not
repeat them for documentation changes. Normal paired palette behavior,
automatic Undo eligibility after a real pen lift, first-use Zoom, sustained
normal viewing and physical acceptance still need their own evidence.

## Final package verification (delivery coordinator)

Build sequentially from the repository root using the existing `build.ps1 build-android` and `build.ps1 build-desktop-distribution` targets. Supply the desktop target's `-RemoteEditorCatalog` and `-RemoteEditorCatalogSha256` only for the exact admitted catalog. Do not enable HIL/integration variants for the delivered APK/native payloads. Each build runs the existing license gate; retain its exact source/artifact receipt. Later builds/checks can replace binaries, so bind final files after the last such operation.

Before delivery, run the additive resource verifier against fresh outputs. The catalog path/digest variables are actual reviewed values, never placeholders submitted to the tool. Without an admitted catalog, omit both catalog arguments and explicitly record missing shortcut authority.

```powershell
python.exe -B tools/desktop-test/personal_package.py --root . --catalog $admittedCatalogPath --catalog-sha256 $admittedCatalogSha256 --output .local/m4-kp-personal-package.json --notice-index .local/M4-KP-THIRD-PARTY-NOTICES.txt
```

Both output names must be new files directly inside `.local`; preserve previous receipts by choosing fresh names. The verifier makes no package edits. It calls the existing desktop receipt check, retaining its exact six native payload, runtime, launcher, nonce and MCP contracts. It checks the admitted catalog pair (or rejects a stale pair when absent), all tracked notice bytes in desktop JARs and Android assets, ARM64 APK native layout, and the final runtime's legal files. Its JSON includes all delivered runtime file hashes and actual upstream legal-file hashes. The readable notice index lists the tracked license metadata and runtime legal locations; preserve the complete upstream notices. Include these generated text files as delivery companions; do not insert them into an already hash-bound image and claim the old image receipt still applies.

This resource receipt intentionally does not authenticate the APK signature or application ID, establish its fresh Android source/build provenance, launch either app, grant an editor action, or pass physical acceptance. Separate ordinary Android/Windows build receipts, SDK manifest/signature verification and S23 installed readback establish those delivery facts. Normal Windows startup has been observed; paired behavior and normal shutdown remain pending.

The resource receipt is `b80629e8e6cb317eaddd2a1961018eaeb0d19bd308b001600c539d9e46912359`;
the S23 install/readback/launch receipt is
`d59cce76b071a3d0f917582ec5582b8a05bfeb68b9403ff6b1368675bf4b01ce`.
The package contains all 30 tracked notices, runtime legal files and the
reviewed eight-action catalog
`113ecec42bba7a1bcea78af3fbd528dc45a46851116f766985a02fc86ddb1e67`.
Normal builds used source map
`d8405c3eb56f11d19e42d71db0a65ae4037edb3f9b222a538a840a2093117178`.
Historical 121/32 checks were not rerun or relabeled as current coverage.
The existing app was updated with replacement installation; no uninstall or
app-data clear was performed. This does not establish byte equality of hidden
pairing or project data.

The ordinary APK path is `apps/android/build/outputs/apk/debug/android-debug.apk`. The complete Windows image path is `apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev`. Its existing central target deliberately builds a console-capable personal launcher for its smoke test. The helper executables inside app resources are exactly `vw-connection-helper.exe`, `vw-capture-helper.exe`, `vw-hevc-helper.exe`, `vw-input-helper.exe`, alongside `vw_core.dll` and `vw_host.dll`; they are not standalone user tools.
