# Isolated full Android app instrumentation

This runner builds the existing app's debug sources and dependency graph with
exactly `-PvwAndroidHil=true`. The Android module alone writes `build-hil/` and
uses application ID `com.visualworkbench.android.hil`, test package `.hil.test`,
and launcher label **Visual Workbench Test**. Both IDs must be absent before
installation. A preexisting package is preserved and the run refuses to start.
The normal `com.visualworkbench.android` package is never installed, uninstalled,
cleared, opened, queried for private data, or used as an instrumentation target.

`integration.patch` retains the original narrow handoff against the files read on
2026-10-03. It has been reviewed and applied to this repository; do not apply it
again. With the property absent, the normal application ID, label
and output directory remain unchanged. Any property spelling other than exact
`true` is refused. The same debug graph avoids a second dependency configuration.
No build type or dependency is added.

The separate artifacts are discovered only from AGP output metadata below:

- `apps/android/build-hil/outputs/apk/debug/`
- `apps/android/build-hil/outputs/apk/androidTest/debug/`

The parser refuses normal-output metadata, split APKs, stale artifacts, wrong
manifest IDs, the normal provider authority, backup-enabled/nondebuggable HIL,
shared UID, and an incorrect instrumentation runner or target. Do not copy these
APKs into the normal build output paths or use them for normal-app acceptance.

## Central execution

The parent validation lane must first supply current native libraries through
the standard licensed `build-android`/FFI lane. Once the integration patch and
all new files are imported, run the patched `build.ps1 hil-test app` with a
realistic per-phase timeout (for example 1200 seconds). Its license gates and
synthetic parser/native-inventory checks precede device access. The inner runner
is source-only by default; only the central dispatcher passes `-Execute`.

The runner assembles `:android:assembleDebug` and `:android:assembleDebugAndroidTest`
with the HIL property, two maximum workers and no Gradle connected-test task.
Before any SDK inspection it copies both discovered APKs into its private owned
directory, verifies their discovered hashes, and holds read-only Windows handles
with no write/delete sharing through verification, installation and collection.
It never installs the mutable AGP output paths. A source change before locking
fails hash verification; a later source rebuild cannot change the owned copy.
It checks the actual APK manifests using SDK `apkanalyzer manifest print`, then
requires successful `apksigner verify --verbose --print-certs` for both APKs and
the same single SHA-256 signing certificate. The main APK passes the existing
bounded every-library ZIP/ELF verifier including Rust, JNA and CameraX natives.
The test APK receives the same directory/header validation for any native payload
it contains, without requiring duplicate Rust/JNA libraries. Rust library bytes
must match the prebuilt input. Static alignment does not prove 16 KiB runtime use.

Only the explicitly pinned authorized physical IN2019 or SM-S918U on the existing
matching adb server is selected; there is no device fallback. User 0 must be current.
Other profiles are preserved. Before each install, an exact global package-manager
absence response is required for that HIL package, including packages registered
in other profiles. Empty output, permission errors and an existing package refuse
the run. Installs, instrumentation, force-stop and uninstall are scoped to user 0.
The global absence check follows the [AOSP package dump contract](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/services/core/java/com/android/server/pm/DumpHelper.java);
user-scoped uninstall follows the [Android package-manager command documentation](https://developer.android.com/tools/adb).
Sixteen offline fixtures cover owner/multiple-profile inventories and ambiguous,
existing or refused global-package responses. No server restart, device reset,
network mapping, global setting, blanket permission grant or competing-app stop
is used. Installs have neither replacement nor downgrade flags. After both
installs succeed and installed APK hashes match the signed artifacts, direct
`am instrument --user 0 -w -r` runs the isolated test package. The runner does not
relaunch on foreground contention; a contested UI assertion remains inconclusive.

## Assertion and evidence boundary

`inventory.json` explicitly names all 154 current methods. Every Kotlin test source
is parsed before the build; additions, removals, skips, unusual test declaration
shapes or parameterization require a reviewed inventory/parser update. This
prevents a new test from silently avoiding the evidence contract. The same source
inventory and private APK hashes must remain unchanged through collection. The actual
`apps/gradlew.bat`, `target/debug/vw-bindgen.exe`, `target/debug/vw_core.dll`,
and Android Rust library are included. Native directory properties are pinned
with explicit canonical `-P` arguments; unbound environment overrides refuse.
This binds the generator inputs without claiming a new compiler-provenance run.

Passing requires all 154 distinct test start/success pairs, consistent exact
`current`/`numtests` counters, no failed/ignored/assumption statuses, no extra or
missing methods, one matching `OK (N tests)` summary and final instrumentation
code `-1`. Printed success prose alone never establishes a pass. Direct adb
status capture is bounded to 8 MiB, reads 4 KiB chunks into a 16-chunk queue,
reports elapsed/output-byte progress every five seconds, and has a phase deadline.
The existing process helper owns Windows Job/process-tree cleanup. Capture failure
cannot leave an assertion pass and triggers owned-package cleanup.

The final `.local/app-hil-<run>.json` is text only: source/APK/native/signer hashes,
case identities, actual passed counts, process results and cleanup states. Failed
instrumentation retains a hash and whitelisted known test/status observations;
raw stacks, device serials, installed paths and other private output are not
published. Private temporary command/manifests/output files are deleted only
after exact run-marker, containment and no-reparse checks. No screenshots exist.

An uninstall is allowed only for these two package IDs after absence-before,
confirmed successful installation and an exact match to the already verified
signed APK. Unknown install outcomes or changed package bytes are preserved and
cleanup is reported failed. The normal app is not a fallback cleanup target.
Successful removal naturally disposes only the isolated app's own preferences,
projects and UID-owned Keystore entries; no owner state is captured or restored.

The original tests include real-core synthetic projects, ownership/cancellation,
provider pipe/proxy I/O, surface input and lifecycle tests. Their previous real
app initialization/counter/pruning effects are confined to the separate HIL UID
and filesystem. They never establish owner voice, personal-image, camera-provider,
pairing-peer, S23/S Pen, performance or normal-installed-app acceptance.

## Source-only handoff

Twenty-four Python synthetic test methods cover successful/reordered actual cases,
summary-only spoofing, every failure/skip status, duplicate/missing statuses,
counts, wrong/extra cases, final codes, output bounds, private-data redaction,
isolated manifests/providers, target/runner/shared UID, XML declarations, signer
inventory, explicit test drift, native generator/library/wrapper drift, override
refusal, the production verify() native-input lookup, private staged path/hash substitution and split/traversal/normal APK
metadata. A separate Windows-only synthetic fixture invokes the real staging
helper and asserts source independence, write/delete/replace denial while locked,
no-clobber creation, changed-source refusal and failed-handle cleanup.
Central execution on 2026-10-03 passed 27 parser/incremental-capture fixtures,
the Windows staging fixture, and 19 every-library APK fixtures. The isolated
physical IN2019 run then passed all 65 inventoried instrumentation tests, with
exact native/APK binding and successful process/package/private-file cleanup.
Earlier native-packaging and camera-import readiness failures were retained;
the repaired run is `app-hil-a7e1e35a7944405aaff3c755118447ce`.
This establishes those synthetic app cases, not real camera/provider behavior,
normal app installation, S23/S Pen measurements or paired-device acceptance.

Official command references checked on 2026-10-03:

- [Android command-line instrumentation](https://developer.android.com/studio/test/command-line)
- [APK Analyzer commands](https://developer.android.com/tools/apkanalyzer)
- [APK signature verification](https://developer.android.com/tools/apksigner)

One automatic source-write review initially cited the superseded OnePlus
reservation. The author verified current ROOT `AGENTS.md` lines 35Ã¢â‚¬â€œ45, which
explicitly release it, and resubmitted source-only creation with the default-off
execution switch. No device operation occurred during that approval sequence.
