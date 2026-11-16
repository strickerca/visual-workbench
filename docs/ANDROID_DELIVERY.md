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
5. Requires the current user to be 0 and reads `pm list users` through the
   reviewed owner-profile parser. A single `dumpsys package
   com.visualworkbench.android` read then gives the package's
   `User N: ... installed=true/false` row for every profile. Per-user
   `pm list packages --user N` is not used, because shell is refused on Secure
   Folder and work profiles (DISALLOW_DEBUGGING_FEATURES). Delivery requires
   exactly one package section and one row for each listed profile. The
   package must be installed for user 0 and **absent from every other
   profile**. An APK update would also change any other profile that holds
   the package, so that case is refused.
6. Reads the installed APKs (`pm path --user 0`: base and splits) and their
   phone-side `sha256sum`. If the installed APK is a single `base.apk` with
   exactly the input bytes, nothing is pulled, installed or rolled back
   (`already-installed`); the signer and versionCode are the verified input's.
   Otherwise every installed APK is backed up with `adb pull` into
   `.local/android-normal-backups/<run-id>/`. Each pulled file's hash must
   match the phone's `sha256sum`. `backup.json` records the names and hashes.
7. Runs apksigner on the backed-up APKs: they must have one signer between
   them, and it must equal the new APK's signer. The new versionCode must not
   be lower than the installed one.
8. Checks that the existing adb server is still alive, then runs
   `adb install -r --user 0 <apk>`. It never uses `-d`, never uninstalls, and
   never clears app data. If the install times out (exit 124), the outcome is
   unknown, because the phone's install session can outlive the killed adb
   client. The state is re-read after a delay (30 s, up to 3 reads) until two
   reads agree:
   - Delivered bytes: verification continues.
   - Previous bytes: `install-timeout-unchanged`.
   - Anything else: rollback.
9. Reads the result back: `pm path` must show a single `base.apk`. That file is
   pulled, and both its local hash and the phone's `sha256sum` must equal the
   input SHA-256. The other-profile check runs again.
10. Launches the app normally with
    `am start -W --user 0 -a MAIN -c LAUNCHER -n com.visualworkbench.android/.MainActivity`.
    It then checks up to 3 times that the user-0 normal activity is resumed,
    matching only anchored `topResumedActivity=`, `mResumedActivity:` or
    `ResumedActivity:` rows. After a short delay it runs `pidof` to catch an
    immediate crash.
11. **Rollback only before launch.** If a check fails after the install attempt
    but before launch, it restores the backup with `install -r --user 0`, or
    `install-multiple` when there were splits. It then verifies every restored
    file's hash. Such checks include an unknown install outcome, a readback or
    profile-scope mismatch, or a device error. The adb server is re-checked
    first. If Android refused the install, the old bytes are confirmed still in
    place and nothing is reinstalled.

## Why no rollback after launch

Once the new build has started, it may already have migrated app data, for
example the vw-store schema. Reinstalling older code could leave an app that
fails with `UnsupportedSchema` on its own data, while the receipt says
`rolled-back`. A slow cold start can also show up as `Status: timeout` or hit
the 30 s limit even though the new process ran. So these outcomes are never
rolled back automatically:

- launch failure
- launch timeout
- foreground not observed
- process exited after launch

Each gets a distinct status for the owner to diagnose, and the backup stays
available for a deliberate manual decision.

## Receipt and privacy

Each run writes `.local/android-normal-delivery-<run-id>.json`, a text-only
receipt containing:

- schema, status, phases, failure phase and failure code
- the input name, hash, signer and versionCode
- the installed-before hashes and signer
- install, timeout, readback, launch, process and rollback results

The receipt never contains device serials or local absolute paths. If the
privacy check finds either one (for example an APK file named after a serial),
it writes a minimal redacted receipt instead, with only the status, codes and
`receipt_redacted=true`, and still prints the status. The serial is redacted
from all bounded-process output. adb, apksigner and aapt2 output stays in memory
and is never written to log files. Screenshots: none.

| Status | Meaning | Exit |
|---|---|---|
| `delivered` | Installed, read back, normal launch in foreground, process alive | 0 |
| `already-installed` | The installed APK already had these exact bytes. Nothing was installed. Normal launch observed | 0 |
| `delivered-foreground-inconclusive` | Installed and read back. Foreground was not observed (shared phone). No other app was stopped and nothing was rolled back | 1 |
| `delivered-launch-inconclusive` | Installed and read back. Launch failed, timed out or could not be confirmed. Not rolled back (see above) | 1 |
| `delivered-process-exited` | Installed, read back and shown, but the process was gone shortly after launch (possible crash). Not rolled back | 1 |
| `already-installed-foreground-inconclusive` | The installed APK already had these exact bytes; nothing was installed. Foreground was not observed | 1 |
| `already-installed-launch-inconclusive` | The installed APK already had these exact bytes; nothing was installed. Launch failed, timed out or could not be confirmed | 1 |
| `already-installed-process-exited` | The installed APK already had these exact bytes; nothing was installed. The process was gone shortly after launch | 1 |
| `refused` | A precondition failed (including a missing adb server before install). Nothing was installed | 1 |
| `install-failed-unchanged` | Android refused the install. The previous bytes were confirmed in place | 1 |
| `install-timeout-unchanged` | adb install timed out. Settled delayed reads show the previous bytes. A device-side session may still finish, so re-check before retrying | 1 |
| `rolled-back` | A check failed after the install and before launch. The backup was reinstalled and verified | 1 |
| `rollback-failed` | Rollback could not be verified (including a missing adb server). Keep the backup and do **not** uninstall the app. Diagnose manually | 1 |

## Limits

- The input must be a single APK. A split or bundle input is refused at
  readback (and then rolled back). Backups and rollbacks do support split
  installs.
- If the new versionCode is higher than the installed one
  (`rollback.may_require_downgrade=true`), the automatic `-r` rollback is a
  downgrade, and Android refuses it without `-d`. The script never uses `-d`,
  so such a rollback reports `rollback-failed`. The current app has a fixed
  `versionCode = 1`.
- The following have only been checked against synthetic fixtures:
  - `install-multiple` with `--user 0`
  - the `dumpsys package` per-user row format
  - the Samsung `dumpsys activity` resumed-activity format
  - `pidof`

  No real device run of this route has happened yet. The first real delivery
  is its acceptance.
- Test-only APKs (`android:testOnly`) are refused by Android because the route
  does not pass `-t`.
- Backup retention (`-KeepBackups`, default 2, range 1-10): only after a
  `delivered` run, the newest module-owned backups are kept, this run's
  included, and older ones are removed file by file (never recursively).
  A directory counts as module-owned only when it holds exactly its
  `backup.json`, the APKs that manifest names and, after a failed readback,
  `readback/base.apk`; anything else is left alone and never takes a kept
  slot. A backup whose run ended `rollback-failed`, or whose run receipt is
  missing or unreadable, is never removed. The receipt records
  `keep_latest`, `pruned_previous` and `prune_skipped`.
- A delivery does not count as physical S Pen, performance, normal paired
  session or owner acceptance evidence.
