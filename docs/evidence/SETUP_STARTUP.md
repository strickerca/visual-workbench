Status: PASS WITH GAPS

# Development setup and startup receipt

Receipt date: 2026-10-01. The project now has a buildable Rust/Kotlin/Compose development starter. Windows and approved-phone startup checks passed. Product features and full task/gate acceptance remain pending.

## Reviewed sources and scope

At intake, all 39 matching files in the active project and the archived v2 package matched by SHA-256. The reviewed package contains 189 requirements, 66 tasks across Phases 0-5, 26 prepared prompts, 30 scenarios, 19 decisions, 68 source records and three draft contracts. All original 98 requirement IDs are retained. There was no application/build project at intake. See [SETUP_ASSESSMENT.md](../SETUP_ASSESSMENT.md) for source hashes, architecture, draft-contract issues and dependency order.

The canonical plan is [IMPLEMENTATION_PLAN.md](../IMPLEMENTATION_PLAN.md). The owner's request authorized assessment, setup and startup, with the storage-warning blocker waived and the connected IN2019 approved for smoke testing. Imported prompts and their commit/merge/approval commands are source guidance. Their text does not add authorization. The original plan, prompts, contracts, ADR history, v1 references and archived packages were preserved.

The starter provides 15 shared Rust crates, two Windows host crates, a hello package, shared Kotlin startup metadata, Android and Windows Compose shells, native packaging/loading, bounded build/test runners and dependency/privacy gates. Most crates are future-module stubs. Pen, canvas editing, pairing, capture, synchronization, AI image edits and agent handoff remain unavailable.

## Toolchain and actual builds

Exact installed versions, pins, official sources and wrapper hashes are in [TOOLCHAIN.md](../TOOLCHAIN.md). Rust 1.99.0, cargo-ndk 4.1.2, cargo-deny 0.20.2, Kotlin/Compose compiler 2.4.20, AGP 9.3.1, Gradle 9.7.0, Compose Multiplatform 1.12.1 and JDK 21 were exercised. The Android build uses final SDK 37.0 revision 2, compile 37, target 36, minimum 29, Build Tools 36.1.0 and NDK 30.0.16248370. The inherited API 36 build failed real AAR checks; the selected Compose artifacts require compile 37. Existing final SDK 37 was reused, without suppressing the checks. Kotlin's supported version range guided the AGP/Gradle selection.

| Entry point or check | Result and exercised scope |
|---|---|
| `build-core` | PASS: all 18 workspace packages compile; the license gate ran first. |
| `build-android` | PASS: debug APK built; the license gate ran first. |
| `build-desktop` | PASS: Windows Uber JAR built; the license gate ran first. Uses the owner's installed JDK; app-image/MSI and bundled-JDK redistribution remain future packaging. |
| `test-all` | PASS: 46 Python policy regressions and one actual Windows Rust ABI test. Kotlin feature test tasks are NO-SOURCE, so supply zero feature tests. |
| `lint-all` | PASS: retained-ID/workspace/manifest policy, plan coverage, Rust format, Clippy with warnings denied, source secret scan and Android/shared/desktop checks. Android lint retained 9 warnings (ChromeOsAbiSupport, DataExtractionRules, MissingApplicationIcon, OldTargetApi, UseTomlInstead). |
| `license-check` | PASS: offline checks, Rust license/source/bans gate and actual Maven metadata census. |
| `doctor` | PASS: zero missing starter prerequisites after the compile-SDK correction. |
| `hil-test` | PASS on IN2019: install, launch, resumed foreground, one actual instrumentation test, APK restoration and foreground verification. Activity creation loads the real Rust library. |
| `hil-test rust vw-ffi` | PASS: one actual FFI test through the configured Android Cargo runner; task-owned device files removed and absence verified. A separate built-in cargo-ndk test also passed and is distinguished from this configured runner. |
| `start.ps1` / desktop readiness | PASS: visible, responding Windows window; two packaged native DLLs loaded. Actual window DPI 168, Windows scale 1.75, Compose density 1.75 and AWT X/Y scale 1.75. No manual text-sharpness claim. |
| Hello builds/runtime | PASS: Windows ABI marker; ARM64 Android library and executable; exact Android ABI marker verified on phone. |
| APK artifact inspection | PASS: both ARM64 libraries stored uncompressed, ELF LOAD alignments at least 16 KB, ZIP alignment verified. The phone uses 4 KB pages; 16 KB-device execution remains untested. |

The laptop has 15.65 GiB usable RAM. At the recorded resource sample, C: had 48.83 GiB free, compared with 54.53 GiB at intake. Rust jobs and Gradle workers are capped at two, with one heavy build at a time. The owner's storage override was honored; unrelated storage was preserved. The sample's free RAM was measured during validation, rather than as an idle-system baseline.

Android lint warnings remain visible in the generated report. Target 36 and ARM64-only are intentional starter selections; icon/version-catalog cleanup and Android 12+ backup/device-transfer rules are open before user project data or release packaging. No warning was suppressed to obtain this result.

## License and privacy evidence

The actual Maven census passes for 164 external modules across four runtime configurations (Android debug/release, desktop and shared desktop), one hash-bound parent POM and three first-party generated-output bindings. It retains transitives, rejects unresolved/unreviewed dependencies and uses the nearest explicitly bound license declaration. Forty-six policy fixtures include 21 parent-chain regressions. This is metadata-policy evidence; full bundled native license/notices and redistribution acceptance remain open. The development JAR does not bundle a JDK. `third_party/LICENSES` currently contains no separately vendored external binaries.

A synthetic GPL-3.0-only Cargo dependency failed both `license-check` and `build-core` before compilation or Gradle began (11.937 s and 7.344 s). Manifest and lock bytes were restored exactly, with zero compiled probe artifacts. Because this newly initialized repository has no commits and no commit authorization, the isolated probe did not use the imported prompt's throwaway-branch workflow. Gradle GPL rejection has fixture evidence, rather than a published GPL module injected into the real application graph.

The source/evidence and retained-log secret scans pass. Direct APK/JAR scan attempts reported zero bytes and are retained as insufficient coverage. A follow-up scanned byte-identical ZIP copies outside Git with archive depth two and no generated-output exclusions: 11,176,282 bytes scanned with no findings. Those task-owned copies were disposed of. See [PACKAGE_SECRET_SCAN.json](PACKAGE_SECRET_SCAN.json). Gitleaks is a heuristic scan; it does not establish exhaustive binary absence or close SEC-004.

An earlier cargo-ndk invocation used an invalid platform flag and panicked, printing inherited GitHub and Exa credentials in tool output. The exact retained failure log was sanitized with hash receipts. Later child processes strip credential-bearing environment entries and redact output/paths; the desktop launcher also strips inherited credential entries before launching Java. This does not erase the transcript exposure. The owner was told to rotate the GitHub and Exa credentials; rotation is not verified, and SEC-004 remains partial. No values are reproduced in evidence.

## Device sharing, preserved failures and cleanup

The owner shares IN2019 with another app development session. Focus loss can be normal. The starter helper allows at most three brief, scoped `am start`/foreground checks, with 15-second ADB phases and two-second pauses after focus loss. Persistent contention is inconclusive. Four synthetic cases passed: immediate readiness, recovery after focus loss, bounded persistent contention and immediate failure for missing installation. A final live foreground check passed. Other apps are preserved; no device/adb reset or other-app stop was used.

| Earlier attempt | Disposition |
|---|---|
| Gradle generated-source/Provider wiring failures | Corrected; actual application builds pass. |
| Old license-report plugin artifact ambiguity | Replaced by exact runtime graph/POM audit; application consumer selection preserved. |
| SDK/compiler files over-classified as runtime | Shipping audit scope corrected with exact output/producer bindings; this was not an SDK image-transform failure. |
| Guava POM without a direct license | Gate failed closed; exact parent-chain evidence added; actual gate and affected regressions pass. |
| Android compile SDK 36 AAR check | Real build required SDK 37; supported final platform reused; corrected APK build passes. |
| First app HIL post-test reopen | Instrumentation passed, but the test runner uninstalled the app. Restore-install added; the full corrected HIL command passes. |
| First V4 two-second timeout probe | Child readiness inconclusive; retained separately. Five-second retry verified exit 124 and owned child cleanup. |
| Evidence version shaped like an IP address | Exact JDK pin remains in TOOLCHAIN.md; evidence splits update/vendor-build components. Scanner rule was preserved. |
| Bounded wrapper default directory on PowerShell 5 | Default moved after parameter binding; Windows PowerShell probe passes. |

Eight process V4 probes verified exit preservation, credentials/path handling, grace shutdown, wrapper-exits-first failure and timeout tree cleanup. The runner uses Windows Job Objects and retains progress/phase receipts. A recorded zero sampled peak on a short process is not zero memory consumption. No verification screenshots were created: zero retained, zero in Git. Package scan copies, temporary policy/probe fixtures and owned phone test directories were disposed of; text-only hashes/counts/cleanup receipts remain. Unrelated files, processes, applications and installations were preserved.

## Artifact bindings and phase measurements

| Artifact | SHA-256 |
|---|---|
| Final Android debug APK, also verified installed | `7d54e4c45e7865b06152fa758b367e88e976c05b6d323858d42469b44fadf348` |
| Running desktop application JAR | `f5e40621a65f20431f71a311087b89aad1cd8dc69f4d6750cab611933d4c46f8` |
| Android hello library | `1701d65dd0c6e4e040ed10ad224df962506c583c6c72fb5ddfb91e46815dd06d` |
| Android hello executable | `8360a274540701e3542bee17b081c6d4f6ffd7971e488c30095af2c84588dfd4` |

[SETUP_METRICS.json](SETUP_METRICS.json), [APK_VERIFICATION.json](APK_VERIFICATION.json), [ANDROID_STARTUP.json](ANDROID_STARTUP.json) and [DESKTOP_STARTUP.json](DESKTOP_STARTUP.json) bind the measured outcomes. Raw text logs and JSON process receipts are in the task's external temporary text-log directory. Basenames below avoid account/device identifiers. Timings are phase durations and memory values are sampled process-tree working sets; they exclude separate preceding license phases. Desktop readiness duration measures the readiness check against the running window, not full launch time.

| Successful phase | Elapsed / sampled tree peak | External receipt basename |
|---|---|---|
| `build-core` | 0.810 s / 17.3 MiB | `build-core-b6b2e22d3a324d9d82a754a08de8b6aa.log.json` |
| `build-android` | 71.979 s / 1945.8 MiB | `build-android-42c608a0edcc458abf369f4c32a33632.log.json` |
| `build-desktop` | 47.863 s / 980.4 MiB | `build-desktop-b2890d4cbd014e6cb335727c8eac2369.log.json` |
| `test-tools` | 1.874 s / 22.9 MiB | `test-tools-5b66afd50eb649a6a4d5b4e61c5698f1.log.json` |
| `test-rust` | 5.928 s / 130.3 MiB | `test-rust-189075d8e1d342f797aa2ea2861e9696.log.json` |
| `test-kotlin` | 63.967 s / 1104.7 MiB | `test-kotlin-35ed212f55554bfa96d6c6d50d9d2623.log.json` |
| `lint-setup` | 0.522 s / 27.4 MiB | `lint-setup-a6dcf9409deb4103b1691ef650362668.log.json` |
| `lint-plan` | 0.297 s / 20.2 MiB | `lint-plan-aa49c61dec5e4294bb4a6a2a01212b06.log.json` |
| `lint-rust-format` | 0.426 s / 27.7 MiB | `lint-rust-format-a8efdf9166d14302bd9a50ecedb5e742.log.json` |
| `lint-rust-clippy` | 0.316 s / 28.0 MiB | `lint-rust-clippy-6058f3596fee4df28d70795de2958940.log.json` |
| `lint-secrets` | 1.211 s / 34.9 MiB | `lint-secrets-3c5f16bb828f4cca9d64e7db3c320e7a.log.json` |
| `lint-kotlin` | 92.560 s / 1424.0 MiB | `lint-kotlin-e939c2d2f5ca43d2913437e6c286b7e9.log.json` |
| `license-rust` | 1.158 s / 27.6 MiB | `license-rust-a003c5b06f304bbeb0aca346bae1ce53.log.json` |
| `license-gradle` | 26.597 s / 605.2 MiB | `license-gradle-89e79ddd5b814fbd87d1e51d1058f2a7.log.json` |
| `doctor` | 9.087 s / 136.5 MiB | `doctor-713d664d24ee42e8bb1a3213934c7de3.log.json` |
| `hil-test` | 110.907 s / 1826.6 MiB | `hil-test-b83674635fe04c4b88ccd63ddcba3fbe.log.json` |
| `hil-android-instrumentation` | 100.689 s / 1687.2 MiB | `hil-android-instrumentation-a1f244c1e1234015a0a553228d77fce2.log.json` |
| `shared-focus-probes` | 1.462 s / 63.0 MiB | `shared-focus-probes-519edaffb1a84f049a7dff2869b13659.log.json` |
| `shared-focus-live-launch-1` | 0.495 s / 18.6 MiB | `shared-focus-live-launch-1-231aea8d23834f1d8483b94f875cc518.log.json` |
| `shared-focus-live-foreground-1` | 0.486 s / 18.5 MiB | `shared-focus-live-foreground-1-8aedd6aca9b94881a4c1ef139f09f7d4.log.json` |
| `apk-artifacts-final` | 0.660 s / 17.8 MiB | `apk-artifacts-final-a5a8cda5fd8b4d44b9d37921319b1415.log.json` |
| `scan-built-package-archives` | 5.686 s / 95.6 MiB | `scan-built-package-archives-b4dba07e7683453887a00d7c3197214a.log.json` |
| `scan-retained-log-secrets` | 3.419 s / 76.9 MiB | `scan-retained-log-secrets-344bef7aaaf34b0e8467b25629804306.log.json` |
| `wrapper-default-directory-probe` | 2.718 s / 29.2 MiB | `wrapper-default-directory-probe-06f09b27b9a948f3a1de0123563228c1.log.json` |
| `installed-apk-digest` | 0.479 s / 18.5 MiB | `installed-apk-digest-e6bad1af973d48eaacd9aeb0fe06682f.log.json` |
| `final-source-and-evidence-scan` | 16.395 s / 83.0 MiB | `final-source-and-evidence-scan-fd8670fc5d904c7a8274d532fbffffa4.log.json` |
| `final-retained-log-secrets-scan` | 6.235 s / 79.2 MiB | `final-retained-log-secrets-scan-de3aed88bfc248eb8573772c759f3e47.log.json` |

## Requirement and next-task boundary

Only DEV-001, DEV-002, LIC-001, LIC-002, SEC-004 and QUALITY-007 are updated to `in_progress` / `partial`, with task evidence paths. The other 183 requirement records, all scenarios and scope definitions remain unchanged. Open clauses and later owners are listed in [T0.01.md](T0.01.md) and [T0.02.md](T0.02.md).

T0.03 diagnostics is next after accounting for setup carry-overs. S23 Ultra/S Pen traces, pressure/tilt/hover, Air Actions, latency, thermal, injection, transport, codecs, driver signing/trust, paid API work, product features and G0 acceptance remain pending. Driver packages are cached but not installed/integrated; Spectre prerequisites remain open for T0.08. No security settings or driver trust were changed. No commits, merges or pushes were made; no remote is configured. The operational development starter is ready for continued work under the existing plan.
