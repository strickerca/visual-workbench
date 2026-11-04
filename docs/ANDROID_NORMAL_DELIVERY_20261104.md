# Normal Android app delivery (B5), 2026-11-04

`build.ps1 deliver-android-normal` is the reviewed route for updating the
owner's normal installed Visual Workbench app (`com.visualworkbench.android`)
on the S23 Ultra (SM-S918U). The HIL helpers still refuse the production
package. This route replaces the ad-hoc R161-era delivery: it checks the same
things (existing signer, installed bytes read back, normal launch) and leaves a
text receipt.

Source: `tools/deliver-android-normal.ps1` (entrypoint),
`tools/android-normal-delivery.psm1` (decision logic),
`tools/tests/android-normal-delivery-fixtures.ps1` (synthetic fixtures, also in
`test-all`). Everything runs in Windows PowerShell 5.1 and in PowerShell 7.

## Usage

The command does not build anything. Start with an APK that was already built
and reviewed, for example from `package-android-app`. First record its SHA-256:

```powershell
Set-Location "$env:USERPROFILE\projects\Design Workbench\visual-workbench"; (Get-FileHash -Algorithm SHA256 -LiteralPath ".\apps\android\build\outputs\apk\debug\android-debug.apk").Hash.ToLower()
```

Connect the S23, make sure the existing adb server is running (the script never
starts or restarts it), unlock the phone on the owner profile, then deliver:

```powershell
Set-Location "$env:USERPROFILE\projects\Design Workbench\visual-workbench"; powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 deliver-android-normal -DeliveryApk ".\apps\android\build\outputs\apk\debug\android-debug.apk" -DeliveryApkSha256 <64-hex-sha256> -DeliveryModel SM-S918U
```

All three inputs are required. `-DeliveryModel` accepts only `SM-S918U`.
Relative APK paths are resolved from the current directory. The delivery
inputs are refused on every other `build.ps1` command.

## What it does

1. Runs the synthetic delivery fixtures (no device).
2. Takes `.local/android-normal-delivery.lock`, so only one delivery runs at a
   time. Verifies the APK's SHA-256 and holds the file open read-only until the
   end, so the hashed bytes cannot be swapped.
3. `apksigner verify --verbose --print-certs` (build-tools 36.1.0) must pass
   with exactly one signer. `aapt2 dump badging` must report package
   `com.visualworkbench.android` and launchable activity
   `com.visualworkbench.android.MainActivity`.
4. Selects the phone with `Get-VwAndroidDevice -ExpectedModel SM-S918U`. That
   needs exactly one matching online physical device and never falls back to
   another one. The adb server must already be running and match the client
   version.
5. Requires the current user to be 0. Reads `pm list users` through the
   reviewed owner-profile parser, and requires the package to be installed for
   user 0 and **absent from every other profile**. An APK update would also
   change any other profile that holds the package, so that case is refused.
6. Backs up every installed APK (`pm path --user 0`: base and splits) with
   `adb pull` into `.local/android-normal-backups/<run-id>/`. Each pulled
   file's hash must match the phone's `sha256sum`. `backup.json` records the
   names and hashes.
7. Runs apksigner on the backed-up APKs: they must have one signer between
   them, and it must equal the new APK's signer. The new versionCode must not
   be lower than the installed one.
8. `adb install -r --user 0 <apk>`. It never uses `-d`, never uninstalls, and
   never clears app data.
9. Reads the result back: `pm path` must show a single `base.apk`. That file is
   pulled, and both its local hash and the phone's `sha256sum` must equal the
   input SHA-256. The other-profile check runs again.
10. Launches the app normally with
    `am start -W --user 0 -a MAIN -c LAUNCHER -n com.visualworkbench.android/.MainActivity`,
    then checks up to 3 times that the user-0 normal activity is resumed.
11. If a check fails after the install, it restores the backup with
    `install -r --user 0`, or `install-multiple` when there were splits. It then
    verifies every restored file's hash against the backup. This applies to a
    readback or profile mismatch, a failed launch, or a device error. If
    Android refused the install, the old bytes are confirmed still in place and
    nothing is reinstalled.

## Receipt and privacy

Each run writes `.local/android-normal-delivery-<run-id>.json`, a text-only
receipt containing:

- schema, status, phases, failure phase and failure code
- the input name, hash, signer and versionCode
- the installed-before hashes and signer
- readback, launch and rollback results

The receipt never contains device serials or local absolute paths. The
receipt writer refuses to write a receipt that contains either one. The serial
is redacted from all bounded-process output. adb, apksigner and aapt2 output
stays in memory and is never written to log files. Screenshots: none.

| Status | Meaning | Exit |
|---|---|---|
| `delivered` | Installed, read back, normal launch observed in foreground | 0 |
| `delivered-foreground-inconclusive` | Installed and read back. Foreground was not observed (shared phone), so the result is inconclusive. No other app was stopped and nothing was rolled back | 1 |
| `refused` | A precondition failed. Nothing was installed | 1 |
| `install-failed-unchanged` | Android refused the install. The previous bytes were confirmed in place | 1 |
| `rolled-back` | A check failed after the install. The backup was reinstalled and verified | 1 |
| `rollback-failed` | Rollback could not be verified. Keep the backup and do **not** uninstall the app. Diagnose manually | 1 |

## Limits

- The input must be a single APK. A split or bundle input is refused at
  readback (and then rolled back). Backups and rollbacks do support split
  installs.
- If the new versionCode is higher than the installed one
  (`rollback.may_require_downgrade=true`), the automatic `-r` rollback is a
  downgrade, and Android refuses it without `-d`. The script never uses `-d`,
  so such a rollback reports `rollback-failed`. The current app has a fixed
  `versionCode = 1`.
- `install-multiple` with `--user 0` and the Samsung `dumpsys activity`
  resumed-activity format have only been checked against synthetic fixtures.
  No real device run of this route has happened yet. The first real delivery
  is its acceptance.
- Test-only APKs (`android:testOnly`) are refused by Android because the route
  does not pass `-t`.
- Backups are kept. Delete old `.local/android-normal-backups/<run-id>`
  folders yourself once a delivery is accepted.
- A delivery does not count as physical S Pen, performance, normal paired
  session or owner acceptance evidence.
