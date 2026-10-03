# Shared Kotlin/native instrumentation

`run_shared_android.ps1` is the T1.07 runner wired through
`build.ps1 hil-test shared-ffi -TimeoutSeconds 1200`. The parent owns task enumeration, parser fixtures,
Gradle compilation, APK installation and the one shared IN2019 device lane.
No S23, S Pen, latency, UI, session or provider acceptance follows from this run.

## Runner entry points

The reviewed runner consists of these five files:

- `tools/ffi-test/run_shared_android.ps1`
- `tools/ffi-test/shared_reports.py`
- `tools/ffi-test/test_shared_reports.py`
- `tools/ffi-test/test_package_inventory.ps1`
- `tools/ffi-test/SHARED_ANDROID_RUNNER.md`

`build.ps1` recognizes `shared-ffi` in `HilMode`. Its `hil-test` branch runs
`Run-LicenseGate`, prepares the normal developer environment, and uses
`pwsh.exe` for the child runner. The native Windows/Android
libraries, generated bindings and reviewed golden files must already exist from
`build-ffi`; do not silently regenerate goldens after a mismatch. The entry point
runs these parser fixtures first through the bounded parent process helper:

```powershell
python.exe -m unittest discover -s tools/ffi-test -p test_shared_reports.py -v
pwsh.exe -NoProfile -File tools/ffi-test/test_package_inventory.ps1
```

`tools/hil-test.ps1` dispatches `shared-ffi` before the generic app branch. It uses
`Invoke-VwProcess` with an explicit total limit and fails on a nonzero exit.
It inherits the prepared developer environment:

```powershell
$run = Invoke-VwProcess -FilePath 'pwsh.exe' -ArgumentList @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
    'tools/ffi-test/run_shared_android.ps1', '-ExpectedModel', 'IN2019',
    '-TimeoutSeconds', "$TimeoutSeconds"
) -WorkingDirectory $projectRoot -Phase 'hil-shared-ffi' `
  -TimeoutSeconds ($TimeoutSeconds * 3 + 300 + 1800 + 300)
if ($run.ExitCode -ne 0) { throw 'Shared FFI instrumentation or cleanup failed' }
```

The script has three potentially heavy sequential phases: test-APK assembly,
desktop tests, and connected tests. Its default bound is 1200 seconds per phase;
inventory is 300 seconds (cold configuration on this host exceeded 120 seconds)
and every other subprocess is bounded. The parent can
set a larger per-phase limit before starting a run. Its
outer `build.ps1` HIL budget must accommodate the child total plus cleanup. A
timeout is a failed/inconclusive run, never an automatic retry. The shared
process helper reports progress every 15 seconds and kills only its Windows Job.

The integration must retain the existing single-heavy-build lane and at most
two Gradle workers. Do not run this mode alongside another shared test/build.
A `.local/shared-ffi-hil.lock` adds a same-checkout exclusion lock; it is not a
machine-wide build lock and does not authorize concurrent other worktrees.

## Actual tasks and packaging

The current root catalog was inspected at AGP 9.3.1. The official Android KMP
guide now uses 9.4.0 in its example. This runner changes neither pin. The actual
module calls `withDeviceTest`, with `androidDeviceTest` source set and
`AndroidJUnitRunner`. KMP uses a single variant, so the app module's
`connectedDebugAndroidTest` task is not an appropriate substitute.

The runner inventories `:shared:tasks --all` and requires the registered
`assembleAndroidTest`, `connectedAndroidTest`, and `desktopTest` anchors before
using them. The two Android anchor names were also found in the installed
official 9.3.1 `AndroidTestTaskManager` class constants. The central first paired
run enumerated the real graph and passed Windows plus physical IN2019 execution.
If a later AGP graph differs, the run refuses rather than choosing a fallback.

Primary sources:

- [Android KMP plugin and single-variant/device-test configuration](https://developer.android.com/kotlin/multiplatform/plugin)
- [withDeviceTest API and androidDeviceTest defaults](https://developer.android.com/reference/tools/gradle-api/9.2/com/android/build/api/dsl/KotlinMultiplatformAndroidLibraryExtension#withDeviceTest(kotlin.Function1))

The fresh APK's AGP output metadata and textual merged manifest must identify
exactly `com.visualworkbench.shared.test` and self-target that same package with
the expected instrumentation runner. No starter APK is installed or launched.
Split test APKs are outside this first runner. The existing root `check_apk.py`
must pass its bounded ZIP/ELF/16KiB checks before the exact embedded
`lib/arm64-v8a/libvw_core.so` and `ffi-golden.properties` bytes are compared with
the selected build outputs. The latter is a Java resource at APK root; if actual
AGP packaging differs or the resource is missing, validation fails rather than
accepting an unverified fixture path. Direct `adb am instrument` was avoided so
the installed plugin retains responsibility for the real library-test graph.
The separate byte-binding parser also bounds the central directory before
opening `ZipFile`, independently of the earlier packaging-check process.

## Scope, ownership and evidence

An already-running default local adb server is required. A bounded read of its
smart-socket version must match `adb version` before any device selection; no
server is started, reset or killed. `Get-VwAndroidDevice -ExpectedModel IN2019`
requires one online authorized physical match and honors an existing expected
serial. There is no fallback, emulator use, reverse mapping or device setting.
`ANDROID_SERIAL` and the existing helper's model/serial guards are restored in
`finally`. A disconnected/contested device remains inconclusive.

Before connected tests, the exact shared test package must be absent. AGP normally
uninstalls it after testing. On failure/timeout the runner only force-stops and
uninstalls that exact package if its installed base APK hash matches the final
owned build artifact. A different installation is preserved and cleanup fails.
The device must have only user 0, currently active; another user/profile is a
refusal because AGP's installation or cleanup may affect packages across users.
Package absence is checked through `pm list packages --user 0`; only one exact
present match permits `pm path`, whose nonzero absent exit is never swallowed.
It never stops, clears or reinstalls the starter or another project app. The
private UUID work directory is deleted only after marker, containment and
reparse checks. Uncertain cleanup prevents a passing receipt.

Both wrappers `DesktopCoreSmokeTest.rustGoldenAndBoundary` and
`AndroidCoreSmokeTest.rustGoldenAndBoundary` must appear as real passing cases in
new/changed XML written after that phase began. Declared counters must match
actual testcase elements; failed/skipped/disabled/duplicate cases, missing
goldens, malformed/oversized/DTD XML and old reports cannot pass. Both wrappers
currently exercise the same native state/export PNG golden, 100,000 samples,
duplicate commit, cancelled native handle acquisition and session ownership.
The receipt counts JUnit cases honestly; 100,000 samples are not 100,000 tests.
The desktop identity includes KMP's observed `[desktop]` suffix exactly; the
runner does not remove arbitrary parameter suffixes to manufacture a match.

The script leaves one new text-only `.local/shared-ffi-android-<UUID>.json`
receipt even after a failed execution once its exclusion lock is acquired. It
contains static test identities, counts, source/fixture/native/APK/report hashes,
phase statuses and cleanup booleans. It never publishes raw XML filenames,
device serials, suite properties, exception stacks or captured subprocess
output. Temporary parser state is deleted. AGP's normal raw XML/HTML remains in
ignored build directories as private diagnostic data; never stage it.

Source and prebuilt native files are hashed before either target and rechecked
between targets and at collection. Added source files and native drift fail.
These hashes establish the bytes co-observed during validation, **not** proof of
which compiler invocation created a prebuilt binary. Preserve the parent native
build receipts alongside this run. A valid parser result alone is insufficient:
only the PowerShell receipt with `status=passed`, both wrapper reports and all
cleanup booleans true is the runner's acceptance result.

Fifteen synthetic Python test functions cover report freshness, actual counts,
wrong/zero/duplicate cases, failures/skips, XML admission, redirection, duplicate
JSON keys, source/binary drift, both-target collection, self-target packaging
and exact native/golden binding with bounded ZIP admission. One symlink fixture may skip when the host
cannot create links. The central Windows lane passed all 15 cases on 2026-10-03,
including the source/lockfile binding checks; no symlink case was skipped.
Ten PowerShell fixtures extract only the actual inventory function from its AST
and inject a strict fake adb to check absence, exact matches, scoped commands,
ambiguous output, split packages and nonzero failures. They execute no runner,
Gradle or device command. All ten passed in the same central validation lane.
