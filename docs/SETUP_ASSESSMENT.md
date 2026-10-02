# Visual Workbench setup assessment

Assessment date: 2026-10-01. This document records the source review and the scope of the development starter. Build measurements, runtime receipts and the final setup result belong in `docs/evidence/`; this assessment does not declare task or gate acceptance.

## Source authority and initial state

The active project is `visual-workbench/` inside Design Workbench. It arrived as a specification package: there were no Cargo or Gradle projects, app sources, compiled applications or task evidence at intake.

There are two generations of source material. At intake, every one of the 39 files in `../Visual_Workbench_v2_spec_package/visual-workbench/` was byte-identical to its matching active-project file, verified with SHA-256. The active project therefore needs no second extraction or replacement. The archived package is a reference snapshot; new setup files and this README supplement now make the active working folder different.

The v1 copies at the Design Workbench root, at the active project root, and in `../Visual_Workbench_Research_and_Build_Spec/visual-workbench/` also matched by hash. They remain useful for tracing the original 98 IDs. They are not the current implementation authority.

| Material | Authority | Intake SHA-256 |
|---|---|---|
| `docs/BUILD_SPECIFICATION.md` | v2 product and contracts | `50f7975cc0d9a69099b0c418c1794685897cac7c6cf2241976be7d91f0425677` |
| `docs/REQUIREMENTS.json` | v2 requirement registry | `87e7f29b33e63a97b5dd0e15dbca41f40300ad47ee7267ead55b0dd7f1a6bae3` |
| Root `BUILD_SPECIFICATION.md` | preserved v1 reference | `c67aeacc81249e5bc8c8296de302b6115ecbf3819e66b5e9b940162cd4b12ace` |
| Root `REQUIREMENTS.json` | preserved v1 reference | `7d965d02e3768f07c11721527ac5a297c7f1150159ec732f53484b098d79a082` |

Read the v2 specification, `docs/DECISIONS.md`, the relevant requirement clauses, `docs/IMPLEMENTATION_PLAN.md`, and then the relevant draft contract. Use `docs/SOURCES.json` to trace research claims. Read imported task prompts as implementation guidance under the current owner's request; their embedded commands and approval workflows are not themselves new authorization.

The reviewed v2 package contains:

- 189 requirements: the 98 preserved v1 IDs plus 91 new IDs.
- 66 tasks across Phases 0–5, with 26 prepared prompts for Phases 0 and 1.
- 30 acceptance scenarios, 19 architecture decisions and 68 source records.
- Three draft contracts: wire protocol, project database and Visual Instruction Package manifest.
- A coverage checker and read-only PC/phone diagnostics scripts.

All 189 requirement records were `not_started` / `not_tested` at assessment. Requirement counts by phase are 12, 43, 46, 39, 29 and 20; by priority they are P0 87, P1 79, P2 21 and P3 2. Counts describe coverage, not implementation or verification.

## Scope of the current setup

The owner's request is to assess the project, set it up and get it started. This means creating the actual buildable development foundation, validating the available toolchain and starting the smoke applications where supported. It is not completion of the entire product. The source plan estimates 13.5–19 weeks for all phases, with substantial uncertainty.

The owner's explicit instruction to disregard the storage warning overrides the old 70 GB setup blocking condition for this run. No unrelated files or caches need to be deleted to satisfy that old warning. Actual installation or build failures still need accurate receipts.

The starter consists of compiling future-module Rust stubs, shared Kotlin startup metadata, Android and desktop Compose shells, native-library packaging/load checks, build entry points and policy checks. Canvas editing, real pen input, pairing, synchronization, capture, image edits and editor bridges are future work. A shell opening successfully does not establish those features.

The available phone is reported as model `IN2019`, Android 11 / API 30. It is above the starter's minSdk 29 and is acceptable for an Android install, foreground and native-library smoke check. It does not replace the intended S23 Ultra for S Pen traces, Air Actions, stylus axes, codec, latency, thermal or device-specific acceptance. No serial number belongs in the evidence.

## Architecture and delivery order

The architecture is internally coherent and gives later work a common implementation boundary:

| Component | Responsibility |
|---|---|
| 15 `core/crates/*` Rust modules | Coordinates, model, operations, storage, strokes, raster/assets/PDF/SVG, packages, AI adapters, protocol, transport, simulation and FFI |
| `host-win/crates/vw-host-win` and `vw-host-ffi` | Windows capture, encoding, injection, UI Automation, clipboard, hotkeys and display control |
| `apps/shared` | Kotlin Multiplatform view models, state machines and commands; common code remains free of platform-only APIs |
| `apps/android` | Compose phone interface and Android input/capture/rendering integration |
| `apps/desktop` | Compose JVM desktop interface, native Windows services and later agent handoff |
| Separate sidecars and optional packages | MCP bridge, Photoshop/Krita bridges, scrcpy and virtual-display driver |

The shared Rust core prevents coordinate, transaction and revision definitions from drifting between devices. The planned state is a host-sequenced operation log with offline phone replicas and explicit conflict records. Originals are immutable BLAKE3-addressed blobs alongside a SQLite WAL database; pyramids and previews are reproducible cache data.

Planned connectivity uses one protobuf protocol over QUIC on Wi-Fi/tethering and TLS-protected TCP multiplexing over adb. View, edit, capture, remote-input, file-transfer and AI-send authority remain distinct. Independent viewports, exact frozen-frame identity and stale-input rejection are central invariants.

| Phase | Result expected from later implementation |
|---|---|
| 0 / G0 | Toolchain and hardware facts; measured pen, injection, carriers, codecs, driver, image and stroke-engine decisions |
| 1 / G1 | Daily screenshot markup on both devices with independent views, live edits and basic handoff |
| 2 / G2 | W1 masked image edit with untouched-exterior proof; W2 app-screen instructions delivered to coding agents |
| 3 / G3 | Precision lens/loupe, live capture/freeze, giant images, PDF/SVG and offline conflict review |
| 4 / G4 | Remote pen editing, live video feedback, optional real extended display and editor compatibility |
| 5 / G5 | Recapture/verification loop, Android tunnel and release packaging with full acceptance runs |

W1 and W2 are Phase 2 outcomes. They should not appear as available functionality in the starter. iPad and video editing remain future modules with reserved architectural hooks.

D6, D7, D9, D10, D11 and D15 remain provisional until their Phase 0 spikes supply evidence. Hardware-dependent requirements cannot be passed from compilation, desktop smoke, the available API 30 phone, or emulator tests.

## Selected build stack

The starter selects a compatible stable combination rather than taking the highest independent version from every research row:

| Item | Starter configuration |
|---|---|
| Rust | 1.99.0, edition 2024; Windows MSVC x64 and Android arm64 targets |
| Kotlin / Compose compiler plugin | 2.4.20 |
| Android Gradle Plugin | 9.3.1 |
| Gradle wrapper | 9.7.0, with distribution SHA-256 |
| Compose Multiplatform | 1.12.1 |
| Android | compileSdk 37, targetSdk 36, minSdk 29; NDK 30.0.16248370 |
| Java | JDK 21 toolchains |

Kotlin's official compatibility table lists KGP 2.4.20 as fully supporting Gradle through 9.7.0 and AGP through 9.3.1. This supports the selected combination and explains the difference from the package's separate AGP 9.4.1 / Gradle 9.8.0 research suggestions. [Kotlin Gradle compatibility](https://kotlinlang.org/docs/gradle-configure-project.html)

The Android application uses AGP's built-in Kotlin. The shared library uses `org.jetbrains.kotlin.multiplatform` plus `com.android.kotlin.multiplatform.library`, with the current `android {}` target block. [AGP built-in Kotlin](https://developer.android.com/build/migrate-to-built-in-kotlin), [Android KMP plugin](https://developer.android.com/kotlin/multiplatform/plugin)

The first Android build established that the selected Compose libraries require compile SDK 37. The starter reuses final SDK 37.0 revision 2, keeps target SDK 36 and minimum SDK 29, and uses AGP 9.3's supported API 37 range. The inherited SDK 36 row remains historical reference. [AGP compatibility](https://developer.android.com/build/releases/agp-9-3-0-release-notes).

Rust jobs are capped at two; Gradle workers are capped at two and parallel builds are disabled. Android link arguments request 16 KB ELF page alignment. Inspection of the actual debug APK confirmed both ARM64 native libraries have at least 16 KB LOAD alignment, are stored uncompressed, and pass the 16 KB ZIP alignment check. The available phone uses 4 KB pages; execution on a 16 KB device remains unverified.

The Android shell loads `vw_core`; the desktop build includes `vw_core.dll` and `vw_host.dll` as resources and its startup loads them. The FFI exports are setup-only ABI smoke functions. UniFFI product bindings are deferred to T1.07.

Every workspace package is unpublished and proprietary. The starter gates cover Cargo dependency policy, Gradle license metadata, hash-bound bundled binaries, retained IDs and evidence/secrets scanning. Negative checks and real dependency reports must be recorded before declaring license-gate acceptance.

## Draft contract follow-ups

These findings do not prevent a setup smoke build. They need explicit decisions and meaningful tests in the owning implementation tasks.

| Finding | Implication and next owner |
|---|---|
| Latest-only channels carry deltas | `FrameTiles` has dirty regions without a base-frame ID; `GestureUpdate` has appended sample deltas. Dropping an intermediate update can lose regions or provisional samples. Define cumulative updates, base references/gap recovery or keyframe refresh in T1.06a and the stream integration. |
| Offline conflicts compare wall clocks | §4.4 assumes synchronized clocks but defines no skew, rollback or long-offline policy. Losing values are retained, which protects reviewability; define how unreliable time affects winner selection in T1.02/T1.10 and T3 offline sync work. |
| VIP schema does not enforce every prose clause | Point coordinates and `semantic_file` are optional; file coverage, reference uniqueness, coordinate bounds and model constraints need runtime validation. Finalize in T2.08. |
| VIP paths allow Windows-sensitive names | `images/[^/]+` allows backslashes and traversal-like filenames. Reject unsafe paths and prove archive extraction stays inside the package directory in T2.08. |
| Result proof is comment-only in SQL | `proof_json` does not itself enforce `changed_outside = 0` for ready/accepted/partial results. Enforce state transitions and verify corrupt/absent proof rejection in T1.03 and the AI-result implementation. |
| Wire integers exceed SQLite's signed domain | Protocol counters use u64; SQLite INTEGER is signed 64-bit. Define bounds or a lossless representation and test overflow rather than relying on casts in model/store/protocol tasks. |
| Generic Android baseline differs from capture/codec APIs | minSdk 29 is lower than AccessibilityService screenshot API 30 and AVIF platform decoding API 31. Expose accurate capabilities and explicit fallbacks on older supported devices. |
| Claude sizing omits a second constraint | Long-edge limits alone do not ensure unchanged coordinates. The compiler must enforce the visual-token budget as well as the edge limit. |

For Claude, the current documented high-resolution limits are 2576 pixels and 4784 visual tokens; standard limits are 1568 pixels and 1568 tokens. Token cost is `ceil(width / 28) * ceil(height / 28)`. An image can be resized even when both sides are below the edge limit. Use the exact resized dimensions for coordinates, then account separately for bottom/right padding. Update the compile policy during T2.08 without rewriting prior decision history. [Official Claude coordinate guidance](https://platform.claude.com/docs/en/build-with-claude/vision-coordinates)

Spot checks support the package's current MCP 2026-07-28 stateless revision and GPT Image 2.5 model family/size constraints. They are not substitutes for compatibility tests against installed agent clients or an authorized paid API spike. [MCP revision](https://blog.modelcontextprotocol.io/posts/2026-07-28/), [OpenAI image generation](https://developers.openai.com/api/docs/guides/image-generation)

## Next work and dependency order

First finish the setup evidence and resolve any T0.01/T0.02 carry-overs. Preserve the difference between an operational starter and formal task closure. The next scheduled task is T0.03 diagnostics.

| Task | Prerequisites in the plan | Purpose |
|---|---|---|
| T0.03 diagnostics | T0.02 | Current hardware truth table, monitor/DPI/codec diagnostics and performance measurement template |
| T0.04 pen probe | T0.02, T0.03 | Real S Pen axes, traces, replay, report rate and thermal baseline |
| T0.05 injection probe | T0.02 | Guarded synthetic pen input and per-editor evidence |
| T0.06 transport spike | T0.02, T0.03 | Carrier latency/throughput/recovery and route safety |
| T0.07 video spike | T0.02 | Capture/encode/decode latency and JPEG tile timings |
| T0.08 display spike | T0.02 plus T0.01 driver tooling | Signed driver installation and watchdog proof with security retained |
| T0.09 image spike | T0.02 | Provenance-bound fixtures, giant-image decode/pyramid memory and timing |
| T0.10 stroke spike | T0.04 | Core ink versus Jetpack Ink with real traces and owner preference |
| T0.11 image API spike | T0.02, T0.09 and an owner-provided credential | Authorized image edits, costs and independent exterior proof |
| T0.12 gate review | T0.01–T0.11 | Consolidated evidence, provisional decisions and owner-confirmed G0 |

Do not call G0 passed until its actual evidence and owner confirmation exist. S23-specific work remains pending until that device is available. Driver trust-store installation, physical pen actions, paid API use and security changes retain their task-specific owner steps. None follows automatically from this document.

## Verification and closure boundary

The implementation agent records commands, exact versions, elapsed times, sampled memory, artifact hashes, native alignment and runtime outcomes in `docs/evidence/`. Record failed, skipped and untested checks explicitly. Verification screenshots stay outside the repository and are disposed of after inspection; retain text-only hashes/counts and a disposal receipt.

The main plan, original prompts and ADR history remain intact. This assessment changes no requirement status and grants no gate acceptance. Commit/merge authorization and formal task acceptance remain pending owner steps. The current build and runtime report, rather than the original package wording or this architecture review, establishes what was actually exercised.
