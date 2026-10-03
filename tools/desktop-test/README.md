# Desktop app-image startup candidate

Source-only candidate for T1.09/T1.12, 2026-10-03. No build, test, launch,
credential read, device action or screenshot was performed by its author.
The central validation lane must review/import these files and apply the
literal `integration.patch` against its recorded `BASES.json`. The patch is
not applied in this worktree. It touches only Main.kt, desktop Gradle and the
root build entry point. No additional library dependency is introduced.

## Central integration and execution

The parent must serialize these commands with all other Gradle/native work:

```powershell
# Inspect BASES.json and apply only after git apply --check succeeds.
python -m unittest discover -s tools/desktop-test -p test_reports.py
pwsh -NoProfile -File tools/desktop-test/test_lease.ps1
pwsh -NoProfile -File tools/desktop-test/test_capture.ps1
pwsh -NoProfile -File build.ps1 build-desktop-distribution
pwsh -NoProfile -File tools/desktop-test/run.ps1 -Execute -TimeoutSeconds 90
```

The runner never builds implicitly and refuses execution without `-Execute`.
`build-desktop-distribution` captures source first, runs the existing offline,
Rust and Gradle license gates, builds the Windows core/host/bindgen/helper using
the existing pinned native task, then calls `:desktop:createDistributable`.
The explicit `vwNativeDir` property binds both DLLs, the helper and the generator
to the same canonical `target/debug` inputs. A fresh build nonce is embedded in
the application JAR; an unsuccessful new attempt invalidates an old receipt.
The final source must still match the source recorded before compilation.

The output is exactly:

`apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev/VisualWorkbenchDev.exe`

Its containing app image includes its own Java runtime. Neither `java.exe`, a
Gradle launch task, an installed application nor a guessed executable is used
for this startup check. The diagnostic packaging command explicitly enables
the Windows console launcher for redirected markers; ordinary builds keep the
default GUI launcher. It is still the ordinary application and initializer,
not a mocked shell or a separate test UI. No WiX/MSI, installation, signing,
firewall rule or automatic permission approval is part of this candidate.

## Readiness and shutdown

Only no arguments (ordinary launch) or exactly `--startup-smoke` are accepted.
Unknown and duplicate arguments fail before local/native initialization.
Smoke mode uses the same packaged native-byte verification, core facade,
host service, device identity, preferences, project-location selection,
controllers and real Compose window as normal launch. It waits for the real
EditorShell `drawContent()` to return, resumes on the next frame callback,
queries that actual HWND through the host service, requires per-window PMv2,
and prints three strict ordered markers. It uses the ordinary `quit()` cleanup
for connection assistance, editor, sessions and host; a cleanup exception
cannot emit the success marker. The runner also requires exit code zero, both
output streams closed, and an empty owned Windows Job. A missing frame/DPI
query or stalled shutdown fails under the bounded deadline rather than being
called a slow success.

`main_to_drawn_frame_ms` starts inside JVM `main`, so it excludes launcher/JVM
startup and is not a cold-start measurement. The process receipt separately
records wall elapsed time including shutdown. Neither is a calibrated PERF
acceptance, sharpness/visual review, mixed-monitor check, interaction test,
pen/latency measurement, day-long usage test or Gate G1 approval. Those original
T1.09/T1.12 owner and hardware clauses remain open.

No project is opened, reset, imported, exported or edited; no clipboard or
pairing/discovery action is requested. An ordinary launch **can initialize or
update the existing local preferences, installation/trust state, directory
layout and verified native cache**. This check does not snapshot/restore or
erase those normal state changes. It never closes another running instance.
The real application window may briefly appear and contend for foreground;
the runner does not focus, click, hide, screenshot or manipulate other apps.

## Evidence and ownership

The bounded parser hashes source, build/lock/generator inputs, every packaged
app-image file (including Java runtime and transitive native libraries), and
each original native binary. It streams every packaged core/host/helper
resource and compares exact size/SHA256 plus the runtime manifest. It checks
the main class and a confined launcher classpath/JVM-option allowlist. JAR
central-directory bounds are checked before the ZIP library allocates its
metadata. Unknown launch/environment overrides and stale build IDs fail.
Limits: 4,096 distribution entries, 1 GiB/file, 4 GiB/image, 256 app JARs,
16 MiB/JAR central directory, 8 MiB receipt, 1 MiB retained private raw output.

Windows handle leases pin every distribution file against writes/deletion
and each path directory against renaming, refuse reparse points using the
opened handle, then recheck all hashes before launching. The same leases stay
held through exit and the final source/runtime check. The repository's existing
`Invoke-VwProcess` provides the outer wrapper's process-tree ownership and
15-second progress. Application stdout/stderr goes directly to `capture.psm1`'s
two fixed 4 KiB byte-stream pumps, never to the helper's line queue. The reader
admits at most 1 MiB combined output (including its joining newline), using two
fixed 1 MiB backing arrays and a final at-most-1 MiB byte copy. It refuses even a
single huge unterminated line without allocating a line/string first. Overflow
is failure: fixed chunks continue draining while the exact inner Job is killed
and reaped; no truncated output can pass readiness. The wrapper emits only
fixed short status markers to its outer Job. Both inner and outer cleanup must
pass, and UTF-8/output-limit observations are required by the strict parser.
Only that invocation's Job is terminated on a deadline; process-name
kills, unrelated-instance cleanup and unrestricted recursive deletion are
absent. This reuses the existing process helper's immediate post-start Job
assignment contract; it does not claim hostile process-spawn containment.

The final path-free JSON is written exclusively under the system temporary
directory's `VisualWorkbench-desktop-text-receipts/<run-id>.json`. It includes
relative source/distribution paths, hashes and allowlisted numeric/boolean
observations. Raw stdout/stderr stays in an owned temporary directory until
parsing, then is removed along with private intermediates after exact owner
marker/child/containment checks. Unknown files are preserved and cleanup fails
honestly. Shared helper logs retain only a sensitive-output placeholder and
process metadata. Failure phase and cleanup receipts survive; arbitrary Java
exception strings and personal absolute paths do not enter the public receipt.
No images are made or retained. The receipt is build binding, not a signature
or proof against a malicious local administrator.

## Source fixtures and pending acceptance

The Python tests exercise production parsing and synthetic app-image binding,
including missing/duplicate/reordered markers, invalid DPI/finite limits,
failed Job cleanup, traversal/override refusal, JSON duplicate/no-clobber,
fresh/stale nonce, actual native bytes rather than only their manifest,
unlisted external classpaths/options, malformed ZIP directory, PE console
subsystem, exact source drift and JRE replacement. The Windows fixture checks
write/delete/rename denial, traversal refusal and release after failed lease
acquisition. Synthetic PE/class/JVM bytes are never executed. Central validation
passed 24 parser/binding tests, ten real bounded-child-capture cases and the
Windows lease fixture on 2026-10-03. Explicit diagnostic capture can return
bounded valid UTF-8 after a clean nonzero child exit; it preserves that exit
status and still refuses overflow, encoding and cleanup failures. Normal
runner behavior is unchanged. The first real package built and bound correctly,
then startup refused a virtualized native-cache path. That failure receipt is
retained. The repaired app image then drew an actual frame with process/window
PMv2 and completed ordinary shutdown; the startup receipt and its limits are in
`docs/evidence/T1.09.md`. Real directory-rename tests also verified the access
flags required to make Windows sharing restrictions effective.

## Primary packaging provenance

Read 2026-10-03: Kotlin's official
[native distribution guide](https://kotlinlang.org/docs/multiplatform/compose-native-distribution.html)
describes `createDistributable` as an app-image task and the Windows console
launcher option. The current upstream
[AbstractJPackageTask](https://github.com/JetBrains/compose-multiplatform/blob/master/gradle-plugins/compose/src/main/kotlin/org/jetbrains/compose/desktop/application/tasks/AbstractJPackageTask.kt)
shows confined `$APPDIR` resource and Skiko options. These references describe
packaging, not the unrun candidate's behavior. The local already-resolved
Compose plugin 1.12.1 JAR was inspected read-only for the same option literals;
its SHA256 is `f02445941fc50c7fdde9fe6da709b793199c7dc97ee08cf15fee6e99ccb4dcca`.
No new dependency download or version change was made.
