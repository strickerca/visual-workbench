# B5 normal Android delivery — first real run — 2026-11-04

Stage 1 blocker B5, accepted. `build.ps1 deliver-android-normal` (reviewed
route, merge `cb44ef2`) delivered a fresh `build-android` debug APK from
main `88c1f4a` to the owner's S23 (SM-S918U, Android 17) normal app.

- **APK:** `apps/android/build/outputs/apk/debug/android-debug.apk`,
  315,422,506 bytes, SHA-256
  `6cd4fe87942eea8d10a12298a6de243717c5f2ea26c7417a733f3de3a83c87e4`.
- **Receipt:** `.local/android-normal-delivery-3dde7177fba74da8b875cf4aa36e5a4d.json`
  (local, not in Git): **status=delivered**. Every phase passed: inputs,
  apk-signer, apk-identity, device-selection, owner-scope, backup,
  installed-signer, install, readback, launch, foreground, process-check.
- **Device facts confirmed:**
  - The phone has a Secure Folder profile (user 150). `pm list packages
    --user 150` from the shell fails with SecurityException, as predicted.
    The single `dumpsys package` read parsed {0: installed, 150: not
    installed}.
  - Installed bytes read back equal the input. The app launched normally and
    stayed in the foreground with its process alive.
- **Defect found and fixed on the first attempt:**
  - In PowerShell 7, `Task.GetAwaiter().GetResult()` emits a
    `VoidTaskResult` object into the pipeline, so device selection returned
    a two-item array.
  - The route refused safely (`device_model_mismatch`, app unchanged).
  - Now discarded at the source and at the call site. Fixtures still pass
    694/694 on PowerShell 5.1 and 7.
- **Not covered:** physical S Pen, performance and owner acceptance remain
  separate.
