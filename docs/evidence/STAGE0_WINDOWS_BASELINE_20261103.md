# Stage 0 Windows baseline — 2026-11-03

This is the first full Windows workspace run of the 2026-11 completion roadmap, on
this laptop, starting from a cold `target/`. Source: branch
`task/s0-rebaseline-20261103` at `0f232b0` (main `2b6b2ea` plus Stage 0 docs and
test-fixture repairs). Raw logs stay outside Git, in the session scratch directory.
No device, GUI, provider or network step ran.

## Results

| Step | Result |
| --- | --- |
| `build.ps1 doctor` | 0 missing checks (toolchain listed in `hardware-truth.md`) |
| License gate (offline policy, npm, cargo-deny 0.20.2, Gradle `checkDependencyLicenses`, MCP node and installed licenses) | pass |
| Python suites (ai-verifier 10, tools 110, desktop reports 41, plus 9 more) | pass |
| MCP node, package runtime, device-selection, app device-scope and store crash-receipt fixtures | pass |
| `cargo test --workspace --exclude vw-sim` (240 test binaries) | **2,259 passed, 0 failed, 15 ignored**, after the repairs below |
| `vw-sim` release simulation (10,000 seeds) | 4 passed, 0 failed |
| `build.ps1 build-ffi` (Windows and Android native, FFI golden, shared bindings) | pass |
| Kotlin `:android:testDebugUnitTest` | 232 passed, 0 failed |
| Kotlin `:desktop:test` | 638 passed, 0 failed |
| Kotlin `:shared:allTests` (desktop) | **630 passed, 2 failed (environmental; see below)** |
| `build.ps1 lint-all` | **not run**: free disk is below the floor |

## Rust repairs (`0f232b0`)

The first workspace run had 12 failures. None was a product defect:

- **Stale expectations after later intentional refusals.** Four tests still
  expected outcomes from before deliberate source changes:
  - `vw-ai` FLUX candidate verification now refuses untagged color first.
  - `vw-store` is now at schema 3 (`1783ccc`).
  - `vw-model` binds the TIFF first-page notice to TIFF originals (`0d8558b`).
  - `vw-remote-host` Paint witness validation refuses an overflowing clock
    first (`b159203`).

  Each test now asserts the current refusal.
- **Windows-invalid fixture paths.** Two `vw-remote-host` tunnel tests used
  paths that do not work on Windows: a POSIX `/tmp/...` adb path, and a
  verbatim `\\?\` path produced by canonicalizing the temp directory, which
  the plain drive-path policy refuses by design.
- **Test isolation.** Five `vw-ffi` giant-image tests raced for the
  process-wide two-slot input permit under `--test-threads=2`. They passed
  single-threaded. All tests that take the permit now serialize on one shared
  test lock.

## Open environmental items

- **The two shared Kotlin failures are free-space refusals.** Free space on C:
  fell from 23 GB to 8.1 GB as the cold build repopulated `target/` (about
  12 GB) and the Gradle caches. The product keeps a disk reserve of
  max(5 GB, min(5% of capacity, 10 GB)) (`vw_store::disk_reserve_bytes`). At
  8.1 GB free, giant import correctly returns `Limit`
  (`GiantImportNativeCallerTest`) and AI preparation returns `Storage`
  (`AiInteractionTest`).
  - Rerun both, and `lint-all`, once C: is back above the 10 GB + 512 MiB floor.
  - Deleting `~/.gradle/caches/build-cache-1` (7.1 GB; this project builds
    with `--no-build-cache`) was proposed but not performed; it needs the
    owner's go-ahead.
- **`test-all` prerequisite.** `test-all` assumes `build-ffi` outputs
  (`target/debug/vw_core.dll`, `vw-bindgen.exe`) already exist. Run
  `build-ffi` first after a cold `target/`.
