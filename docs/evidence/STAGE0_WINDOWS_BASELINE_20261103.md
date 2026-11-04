# Stage 0 Windows baseline — 2026-11-03

This is the first full Windows workspace run of the 2026-11 completion roadmap, on
this laptop, starting from a cold `target/`. Source: branch
`task/s0-rebaseline-20261103` (main `2b6b2ea` plus Stage 0 docs and test
repairs). First runs at `0f232b0`; final Kotlin tests and `lint-all` at `7897e1e`. Raw logs stay outside Git, in the session scratch directory.
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
| Kotlin `:shared:allTests` (desktop) | 632 passed, 0 failed (after the repairs below) |
| `build.ps1 lint-all` (setup, plan coverage, rustfmt, clippy `-D warnings`, gitleaks, Kotlin lint/check) | pass |

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

## Later repairs (`9108b0f`, `7897e1e`)

- **Review fixes:** the independent review of `0f232b0` found four
  `android_bridge` async tests that still took the input permit without the
  shared lock. They now hold it. The untagged-candidate refusal keeps its own
  assertion, and Windows gets an `OwnerAdb` parent-directory negative test.
  Correction to `0f232b0`'s message: the FLUX exterior test had failed since it
  was written. The untagged refusal predates it.
- **Clippy:** five existing test-only errors are cleared (`needless_update`,
  `type_complexity`, `let_and_return`, missing test-file allow attributes).
- **`AiInteractionTest` race:**
  - Cause: the late-preparation test settled its request before the cancelled
    caller cancelled the owned job.
  - Effect: the request sometimes became active, and the fixture's `Storage`
    close failure then hid the real assertion. The `Storage` error comes from
    the test fixture, not from disk. The earlier free-space explanation was
    wrong for this test.
  - Fix: yield first. It passes 3/3 repeated runs.
- **Gitleaks:** the first Windows `gitleaks dir` scan flagged 219 lines that
  were already on main:
  - 212 are file-name keys bound to SHA-256 digests.
  - 7 are observed Krita `5.3.4.0` versions.

  An exact-line allowlist now covers them, following the existing policy.

## Environment notes

- **Disk:** free space on C: fell to 8.1 GB during the cold build. The product
  keeps a disk reserve of max(5 GB, min(5% of capacity, 10 GB))
  (`vw_store::disk_reserve_bytes`), so `GiantImportNativeCallerTest` correctly
  returned `Limit` at that point. At the owner's go-ahead,
  `~/.gradle/caches/build-cache-1` was deleted; this project builds with
  `--no-build-cache`. About 29–35 GB were free for the final runs.
- **`test-all` prerequisite:** `test-all` assumes `build-ffi` outputs
  (`target/debug/vw_core.dll`, `vw-bindgen.exe`) already exist. Run
  `build-ffi` first after a cold `target/`.
