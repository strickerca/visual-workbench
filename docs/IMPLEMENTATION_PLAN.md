# Visual Workbench — Implementation Plan v2.0

Revision 2026-10-01. Companion to BUILD_SPECIFICATION.md v2.0 and REQUIREMENTS.json (189 requirements). Every requirement maps to at least one task below (checked by `tools/check_plan_coverage.py`).

## How to run this plan

1. Put this package at `C:\dev\visual-workbench` (the repository root; any path works if you tell the agent).
2. Run tasks in the order of their dependencies. Each Phase 0 and Phase 1 task has a ready-to-paste prompt in `prompts/`, and each prompt is complete on its own.
3. One task at a time per folder. A task run in parallel needs its own `git worktree` folder, and tasks that use the phone take turns.
4. Paste one prompt into a fresh coding-agent session. The agent starts from main with validated dependencies integrated, uses its task branch, writes evidence and requirement statuses, and reports results plus the next task's model/effort. Local commits and completed dependency integration are authorized without repeated Git approval.
5. Review results and complete listed owner actions. At each numbered phase conclusion the agent automatically commits and pushes validated phase-owned source/evidence under the publishing policy below. Owner confirmation of product gates and manual decisions remains separate.
6. A Phase 1 task may start as soon as its own dependencies are done; Gate G0 must pass before the Phase 1 acceptance run (T1.12). Prompts for Phases 2–5 are written after Gate G1, using real Phase 0 measurements. Generating them now would bake in guesses.

**Phase gates.** A phase ends when its gate passes. A scenario passes at a gate when all of its requirements whose phase is ≤ that gate's phase pass; requirements from later phases re-run the same scenario at their own gate. Each scenario's gate and re-runs are in `REQUIREMENTS.json → scenarios[].gate` and `rerun_at`.

**Requirement status.** A task that delivers only part of a requirement sets it to `in_progress` / `partial` and names the open clauses and the task that closes them. Only the last task that lists a requirement may set it to `passed`, and that task is always in the requirement's own phase. Each phase's acceptance task may correct the status of any requirement up to its phase, including setting `passed` when its own run verifies every clause. A P2 or P3 requirement Christian defers in a dated DECISIONS.md entry (`impl_status` deferred, `verify_status` not_applicable) does not block its scenarios. Every gate task depends, directly or through other tasks, on the last listing task of each requirement its scenarios check. `tools/check_plan_coverage.py` checks all of this.

## Owner actions (things only Christian can do)

| When | Action |
|---|---|
| Before T0.01 | Free at least 70 GB on C:. |
| T0.01 | Approve installer prompts (Git, Rust, JDK, Python, Android command-line tools, Visual Studio Build Tools) and the license agreements the agent lists. If Smart App Control is on, turn it off in Windows Security when the agent explains why (it can be turned back on later). On the phone: turn off Auto Blocker; enable Developer options, USB debugging and Stay awake; accept the laptop's adb key (approve Samsung's USB driver if asked); tap "Install anyway" if Play Protect warns. Keep the laptop plugged in with sleep off while agents run. Optionally update the Intel graphics driver to 32.0.101.7092. |
| T0.02 | Keep the phone connected and unlocked for the `hil-test` check. |
| T0.03 | One admin check: right-click Start → Terminal (Admin) → `Confirm-SecureBootUEFI`, and say True or False. Allow Location for the Wi-Fi check if asked. |
| T0.04 | Turn Air Command's pen-button trigger off and on when asked; enable Air actions for the probe if prompted. Draw about 24 short traces, including 4 tilted calibration strokes. Draw casually for 10 minutes while the phone's temperature is logged. |
| T0.05 | Allow free installs of Krita and GIMP (winget). Click Yes once for an elevated Notepad. Don't touch mouse or keyboard while injection runs. |
| T0.06 | Turn on USB tethering when asked (and after each replug); stay on 5 GHz Wi-Fi; approve temporary firewall rules (click Allow if Windows asks). |
| T0.07 | Keep the phone connected and unlocked during the decode runs. |
| T0.08 | Make sure System Protection is on; approve the admin prompts for the certificate and driver; check Secure Boot when asked; at the end, export the driver-signing key to a USB stick (or another offline place) with the two commands the agent gives you, and keep its password in your password manager. |
| T0.09 | Optional: take one 200 MP photo (and HEIF if available) and copy it to the laptop. |
| T0.10 | Do a blind feel test of two pen demos (about 10 minutes); decide whether to spend up to 2 more days on a Windows build of Google's ink library if the agent asks. |
| T0.11 | At platform.openai.com: add billing with a low monthly budget and create an API key; store the key yourself in Windows Credential Manager (the agent gives the exact steps; never paste it in chat). Provide three of your own photos. Approve about $1–2 of test spend. |
| T0.12 | Read the Gate G0 summary and confirm it, or say what to change; approve or reject any looser performance target. |
| T1.06a, T1.06b | Turn on USB tethering when asked; approve temporary firewall rules (click Allow if Windows asks). |
| T1.08a, T1.08b | Use the phone app for about 10 minutes each and say what feels slow, imprecise or confusing. |
| T1.09 | Say whether text in the PC window looks sharp at your normal scaling; approve a temporary firewall rule. |
| T1.10 | Pair the phone by scanning the QR code. When asked: rest your palm mid-stroke, press Home mid-stroke, unplug and replug the cable (then turn tethering back on), and turn the phone's Wi-Fi off for 10 seconds. For one test, turn Developer options off and back on (tap Build number 7 times again, then USB debugging and Stay awake, and accept the adb prompt); for another, unplug the router's internet (WAN) cable. Record the PC window when asked; approve firewall and route-fix prompts. |
| T1.11 | Make sure Claude desktop and ChatGPT desktop are installed and signed in. Run the Claude Code paste test in your own terminal. Expect your clipboard to be replaced during the tests. |
| T1.12 | Do the manual scenario steps (about 20 minutes) and approve the firewall rule for the installed app. Use the build for a day, then reopen the session (`claude --continue`) and report back. |
| Phase 2 | Start Claude Code with `claude --dangerously-load-development-channels server:vw` for push tests; keep Codex current; enable the Visual Workbench accessibility service on the phone (and allow restricted settings if Android asks); in your own Compose apps' debug builds, enable `testTagsAsResourceId`. |
| Phase 4 | Install Photoshop 27.9.1 if you want Photoshop support (Creative Cloud); install Affinity (free). |
| Phase 5 | Create and safely store the Android release signing key. Before any distribution: code-signing certificates (Authenticode; EV for driver attestation). |

## Repository layout

```
visual-workbench/
  README.md  CLAUDE.md  Cargo.toml  rust-toolchain.toml  deny.toml  justfile (or build.ps1)
  .gitignore  .gitattributes
  docs/        BUILD_SPECIFICATION.md DECISIONS.md REQUIREMENTS.json IMPLEMENTATION_PLAN.md
               TOOLCHAIN.md SOURCES.json evidence/ compat/
  contracts/   vw_protocol.proto package.schema.json storage.sql
  prompts/     phase-0/ phase-1/ (later phases added after G1)
  tools/       diagnostics/ diag-win/ pen-trace/ pen-inject/ bench/ pair-cli/ hello/ check_plan_coverage.py
  fixtures/    images/ pdf/ svg/ malformed/ traces/ (generator scripts + hashes; big outputs in git-ignored generated/)
  core/crates/ vw-geom vw-model vw-ops vw-store vw-ink vw-raster vw-assets vw-pdf vw-svg
               vw-pkg vw-ai vw-proto vw-net vw-sim vw-ffi
  host-win/crates/ vw-host-win vw-host-ffi      (compile to stubs on non-Windows targets)
  apps/        settings.gradle.kts shared/ android/ desktop/
  mcp/         vw-mcp stdio bridge and channel server (T2.09)
  bridges/     photoshop-uxp/  krita/ (GPL, own LICENSE, excluded from app builds)
  drivers/sudovda/ (fork; separate package; licenses per BUILD_SPECIFICATION §2.5)
  third_party/ scrcpy/ pdfium/ LICENSES (bundled binaries: file → license → SHA-256)
```

Git-ignored: `target/`, `build/`, `.gradle/`, `local.properties`, `*.apk`, `fixtures-private/`, `fixtures/generated/`, keystores and `*.pfx`.

## Conventions

- **Branches and commits:** start from up-to-date main with validated dependencies integrated; use task/<task-id>-<slug> and conventional commit messages. Standing owner authorization permits local task commits and completed dependency integration. Automatically commit and push phase-owned work at each numbered phase conclusion under the policy below; do not ask for Git approval again.
- **Rust:** edition 2024; `rustfmt`; `clippy -D warnings`. No `unwrap`/`expect`/`panic` in library code (use `thiserror`). `unsafe` only in FFI and platform modules, with SAFETY comments. Workspace crates: `publish = false`, `license = "LicenseRef-VisualWorkbench-Proprietary"`.
- **Kotlin:** official style; explicit API mode in shared modules; coroutines for async work; nothing blocking on the main thread.
- **Tests** live next to the code. Hardware tests are marked and run through `hil-test`; Rust tests run on the phone through `hil-test rust <crate>`.
- **Scripts:** PowerShell scripts run as `powershell -NoProfile -ExecutionPolicy Bypass -File <script>`.
- **Evidence:** `docs/evidence/<task>.md` starts with `Status: PASS`, `Status: PASS WITH GAPS` or `Status: FAIL`, then one line per acceptance check. A dependency blocks a later task only when its evidence is missing or says FAIL. Screenshots come from a Per-Monitor-V2-aware capture (`tools/diag-win --screenshot`, from T0.03), cropped to the window under test, with QR codes, addresses, serials, user names and chat content removed.
- **Definition of done for every task:**
  - Deliverables exist.
  - `test-all`, `lint-all` and `license-check` pass.
  - REQUIREMENTS.json status and evidence are updated per the status rule above.
  - `docs/evidence/<task>.md` lists commands, numbers, and what was not tested.
  - No secrets or identifiers anywhere.

## Automatic phase publishing

Owner instruction, 2026-10-01: create a public GitHub repository, commit/push all
work completed so far, and automatically commit/push after each phase concludes.
The delivery remote is origin, repository
[strickerca/visual-workbench](https://github.com/strickerca/visual-workbench),
and delivery branch main.

This applies to numbered Phases 0–5, not an instruction to push after every task.
Tasks may use local commits and integrate validated dependency work as needed.
The initial T0.01–T0.03 publication is an explicitly authorized progress checkpoint;
Phase 0 remains in progress until its gate criteria and owner decisions are met.

At each phase conclusion:

1. Complete implementation/validation and record the phase gate result,
   requirement statuses, measured facts and remaining gaps. Pending owner
   decisions remain pending; publishing a checkpoint does not pass a gate or
   authorize the next phase. Never loosen a performance target without the
   owner's explicit decision.
2. Inspect Git status, worktree/branch ownership and the configured remote.
   Preserve unrelated/concurrent changes. Integrate only completed, validated
   phase/dependency work into main. Resolve conflicts deliberately and preserve
   relevant requirement, decision and toolchain changes from both sides.
3. Run required affected tests, lint, license and hardware checks. Record
   physical/external gaps honestly. Resolve failed required software, license or
   secret checks before publication and retain their failure receipts.
4. Stage explicit phase-owned source and text evidence paths. Exclude local
   build/cache/runtime data, credentials, signing keys, private fixtures and
   verification images. Inspect the exact staged diff and scan staged bytes for
   secrets/identifiers. Dispose task-owned verification images before closure.
5. Commit with a conventional message identifying the phase, then use a normal
   git push origin main. Commit, safe integration and push are already authorized;
   do not ask for another Git permission.
6. Verify remote main equals the local published commit. Report commit hash,
   repository URL, publication result, open verification limits and the next
   task/phase model and effort. If nothing changed, verify/report parity without
   making an empty commit.

Never force-push, rewrite shared history, discard unrelated changes or change
visibility again during automatic publication. If authentication, divergence,
branch protection or another concrete blocker prevents a normal push, preserve
the local work and report it; do not bypass the blocker. Git publication does
not authorize deployments, releases, notifications or security/device changes.

Carry this policy into future phase prompts. Existing task prompts defer to it;
their Git checklists record integration/publication status rather than asking
for repeated merge/push approval.

## Phase overview

| Phase | Goal | Estimate | Gate | Scenarios at gate | Re-runs at gate |
|---|---|---|---|---|---|
| 0 | Hardware truth and setup | 1–1.5 weeks | G0 | — (evidence) | — |
| 1 | Markup core: daily use | 2.5–3.5 weeks | G1 | A01, A02, A06 | — |
| 2 | Your two first wins (W1 GPT Image, W2 app screens) | 2–3 weeks | G2 | A13, A21, A22, A23, A24, A26, A29 | A06 |
| 3 | Precision, capture, sync | 3–4 weeks | G3 | A03, A04, A05, A07, A10, A11, A12, A14, A25 | A01, A02, A21 |
| 4 | Remote pen editing and virtual monitor | 3–4 weeks | G4 | A08, A15, A16, A17, A27 | A14 |
| 5 | Verification, Android tunnel, release | 2–3 weeks | G5 | A09, A18, A19, A20, A28, A30 | A10, A15, A21, A24, plus every scenario once on the release build |

### Recommended Codex settings

For maximum quality, Astra 6 leads every phase and Sol 6.1 handles the bounded tasks listed below. These are project-specific recommendations, not experimentally proven optimal settings. They apply to the coding agent running a task, not the app's image-provider models. Select and verify the model and reasoning effort manually before implementation; these annotations do not configure running sessions.

| Phase | Recommended model | Reasoning effort | Reason |
|---|---|---|---|
| 0 — Hardware truth and setup | Astra 6 (`gpt-6-astra`) | `xhigh` | Design meaningful measurements and reconcile provisional architecture decisions. |
| 1 — Markup core | Astra 6 (`gpt-6-astra`) | `max` | Establish deterministic geometry, transactions, crash recovery, pairing, and synchronization. |
| 2 — Image editing and agent handoff | Astra 6 (`gpt-6-astra`) | `xhigh` | Integrate semantic capture, image providers, instruction packages, and agent interfaces. |
| 3 — Precision, capture, sync | Astra 6 (`gpt-6-astra`) | `max` | Handle frame identity, coordinate correctness, giant images, redaction, and offline conflicts. |
| 4 — Remote pen and virtual monitor | Astra 6 (`gpt-6-astra`) | `max` | Coordinate native input guards, video feedback, driver lifecycle, and editor behavior. |
| 5 — Verification and release | Astra 6 (`gpt-6-astra`) | `xhigh` | Complete remaining integrations and assemble release evidence across the product. |

### Task model and effort exceptions

Every task inherits its phase default unless listed below. Sol means `gpt-6.1-sol`; Astra means `gpt-6-astra`.

| Phase | Exceptions |
|---|---|
| 0 | **Sol / high:** T0.01 toolchain, T0.03 diagnostics. **Sol / xhigh:** T0.02 build infrastructure, T0.09 image fixtures and benchmarks. **Astra / max:** T0.08 virtual-display spike, T0.10 stroke-engine decision, T0.11 image-edit proof, T0.12 gate review. |
| 1 | **Sol / xhigh:** T1.08b Android editing tools and layout. **Astra / xhigh:** T1.09 desktop shell, T1.11 import/export integration. |
| 2 | **Sol / xhigh:** T2.04 markers and instruction UI. **Sol / high:** T2.05 voice and handwriting entry. **Astra / max:** T2.03 crop-and-stitch proof, T2.06 semantic capture, T2.08 package compiler, T2.09 MCP server, T2.11 acceptance. |
| 3 | **Sol / xhigh:** T3.05 additional selection tools and tilt. **Astra / xhigh:** T3.01 capture picker, T3.03 loupe, T3.06 assisted selection, T3.07 document editing controls, T3.10 additional formats and color. |
| 4 | **Astra / xhigh:** T4.02 lossless text tiles, T4.07 editor compatibility and Krita bridge, T4.08 shortcut destinations. |
| 5 | **Sol / xhigh:** T5.02 additional image adapters, T5.04 Air Actions. **Astra / max:** T5.01 verification loop, T5.06 release packaging, T5.07 full acceptance. |

The assignments resolve all 66 tasks: 35 Astra/max, 21 Astra/xhigh, 7 Sol/xhigh, and 3 Sol/high. Every phase gate receives Astra/max.

Higher effort is assigned to unresolved contracts involving dropped delta updates, unreliable offline clocks, integer bounds in storage, package-path validation, proof enforcement, and stale remote input. It still requires validation and does not establish hardware acceptance.

Each existing Phase 0 and Phase 1 prompt states its resolved settings and a task-specific reason before `## Context`. Keep Phase 2–5 recommendations in this plan and carry them into their prompts when those prompts are authored after G1, using the measured evidence.

Sources: [official model guidance](https://developers.openai.com/api/docs/guides/latest-model) and [official effort guidance](https://developers.openai.com/api/docs/guides/deployment-checklist).

Total: about 13.5–19 weeks of agent-built work; ±40% (estimate).

Task sizes (agent working time): S ≤ 0.5 day · M ≤ 1.5 days · L ≤ 3 days.

---

## Phase 0 — Hardware truth and setup

Goal: replace every "unverified" with a measurement, install a working toolchain, and settle the provisional decisions (D6, D7, D9, D10, D11, D15) before feature work.

### T0.01 — Laptop toolchain and version pins
- **Size:** M · **Depends on:** owner frees ≥ 70 GB · **Prompt:** `prompts/phase-0/T0.01-toolchain.md`
- **Requirements:** DEV-001
- **Deliverables:**
  - Smart App Control state checked first; the phone's prerequisites (Auto Blocker off, USB debugging, Stay awake) confirmed.
  - Repository initialized with `.gitignore` and `.gitattributes` before the first commit; downloaded files unblocked.
  - Installed and verified: Git, Rust stable, cargo-ndk, cargo-deny, JDK, Android command-line tools (SDK 36, NDK r30, platform-tools), Gradle wrapper version, Visual Studio Build Tools (MSVC, Windows SDK, Spectre-mitigated libraries), WDK and SDK.CPP NuGet packages (download only, cached in `C:\dev\cache\nuget`).
  - `docs/TOOLCHAIN.md` "Pinned" column filled with exact versions and source URLs.
  - `rust-toolchain.toml`.
  - Hello-world builds for `x86_64-pc-windows-msvc`, `aarch64-linux-android` (cargo-ndk), an Android APK and a Compose desktop window.
  - Disk and RAM usage recorded.
- **Acceptance:** every hello-world builds and runs (APK installs on the S23 Ultra via adb; the desktop window opens and reports the Windows display scale).

### T0.02 — Repository skeleton, build entry points, license gate
- **Size:** M · **Depends on:** T0.01 · **Prompt:** `prompts/phase-0/T0.02-repo-skeleton.md`
- **Requirements:** DEV-002, LIC-001, LIC-002, SEC-004, QUALITY-007
- **Deliverables:**
  - Repo layout as above, with empty crates and modules that compile.
  - `justfile` or `build.ps1` with build-core, build-android, build-desktop, test-all, lint-all, license-check and hil-test (including `hil-test rust <crate>`); every `build-*` runs the license gate first.
  - `deny.toml` (allow-list per BUILD_SPECIFICATION §2.5), a Gradle license check, a `third_party/LICENSES` manifest check, and a secrets-and-identifiers scan (gitleaks-style rules, permissive tool).
  - `CLAUDE.md` with project conventions.
  - `tools/check_plan_coverage.py` wired into `lint-all`.
- **Acceptance:** all entry points run; adding a GPL test dependency makes `license-check` and a `build-*` entry point fail (proved on a throwaway branch, then removed).

### T0.03 — Diagnostics and hardware truth table
- **Size:** S · **Depends on:** T0.02 · **Prompt:** `prompts/phase-0/T0.03-diagnostics.md`
- **Requirements:** QUALITY-005, QUALITY-004
- **Deliverables:**
  - `tools/diag-win` (Rust, Per-Monitor-V2): per-monitor resolution, DPI, refresh and HDR; Media Foundation hardware codecs; a `--screenshot` mode for evidence images.
  - Runs of both diagnostics scripts (Smart App Control, Developer Mode, OneDrive-synced Documents, Wi-Fi band and channel with Location allowed).
  - `docs/evidence/hardware-truth.md`.
  - `docs/evidence/PERF-template.md` with a method for every PERF requirement.
- **Acceptance:** every row of BUILD_SPECIFICATION §2.1 is verified or explicitly marked untestable with a reason.

### T0.04 — Pen probe app and trace recorder
- **Size:** M · **Depends on:** T0.02, T0.03 · **Prompt:** `prompts/phase-0/T0.04-pen-probe.md`
- **Requirements:** PEN-017, DEV-003 · **Informs:** QUALITY-004 (thermal baseline), PEN-014 (tilt sign calibration)
- **Deliverables:** a minimal Android probe app (`tools/pen-trace/probe-android`) that:
  - logs every MotionEvent with historical samples (tool type, pressure, tilt, orientation, distance, buttonState, flags) and hover events;
  - logs declarative Air Actions KeyEvents (REMOTE_ACTION meta-data);
  - measures report rate and front-buffer draw latency estimates;
  - exports traces as JSON to `fixtures/traces/`.

  Also an instrumentation replay harness that injects traces with stylus tool type, pressure, tilt, hover and buttons. Report: tilt yes/no, hover yes/no, report rate, barrel behavior with Air Command on and off, Air Actions delivery, battery temperature over a 10-minute pen session.
- **Acceptance:** at least 24 owner traces recorded (lines, circles, small handwriting, fast flicks, hover-only passes, palm-down drawing, tilt calibration); replay reproduces sample counts exactly and every sample's timestamp within 1 ms.
- **Execution, 2026-10-01:** probe/recorder/replay software implemented; required S23 owner corpus and thermal/capability measurements remain pending. See `docs/evidence/T0.04.md`. This task is not an accepted dependency for T0.10 and does not pass G0.
- **Owner deferral, 2026-10-02:** continue code work while S23 measurements are deferred and the OnePlus is reserved by another project. Integrate the validated T0.04 software checkpoint; retain the incomplete physical acceptance and T0.10 dependency block. T0.05 depends only on T0.02 and may proceed independently.

### T0.05 — Windows pen-injection probe and test harness
- **Size:** M · **Depends on:** T0.02 · **Prompt:** `prompts/phase-0/T0.05-injection-probe.md`
- **Requirements:** DEV-004 · **Informs:** D10
- **Deliverables:**
  - `tools/pen-inject/harness`: a Rust test window that logs WM_POINTER pen info (pressure, tilt, rotation, flags, timestamps).
  - `tools/pen-inject/inject`: a Rust CLI with the foreground/rectangle guard built first, then a synthetic PT_PEN device drawing scripted strokes (pressure ramp, tilt sweep, barrel and eraser flags, ≥ 20 Hz keepalive).
  - Runs against the harness, Paint, Krita (Windows 8+ Pointer Input), GIMP and Photopea in Edge (and Chrome if installed).
  - Results in `docs/compat/injection-smoke.md`, with screenshots of pressure-varying strokes; a dated D10 entry.
- **Acceptance:** harness pressure tracks commanded pressure (Pearson r ≥ 0.95); each editor's result is recorded as yes, no or partial with evidence; 0 stray events in 100 guard trials.
- **Execution checkpoint, 2026-10-02:** the host-only recorder, guarded injector, bounded HIL runner and strict analyzers are implemented. Owner-authorized native harness testing measured pressure r = 1.000 and a 26.480 ms maximum normal contact interval; combined inverted/eraser flags did not pass through. See `docs/evidence/T0.05.md` for live guard results and retained source-bound reports. Editor and elevation acceptance remain open; this checkpoint does not close T0.05, T4.03's dependency or G0. No phone was accessed. The next independent task is T0.06 (M, gpt-6-astra / xhigh), with physical transport measurements deferred while the phone is reserved.

### T0.06 — Transport spike
- **Size:** M · **Depends on:** T0.02, T0.03 · **Prompt:** `prompts/phase-0/T0.06-transport-spike.md`
- **Informs:** D7, CONNECT-001, CONNECT-004, PERF-002, QUALITY-004 (recovery baseline)
- **Deliverables:**
  - A Rust echo/throughput benchmark (quinn QUIC with the `ring` provider, and plain TCP), as a PC binary plus an Android binary run via adb shell.
  - Runs over USB tethering (QUIC), 5 GHz Wi-Fi (QUIC), and adb reverse (TCP), with a temporary firewall rule and a firewall-block check.
  - RTT p50/p95/p99, jitter and throughput for 64 B, 4 KB and 1 MB messages; carrier recovery time after unplug/replug and a Wi-Fi drop.
  - A route-table check proving whether tethering takes over the PC's default route, with the fix and its revert.
- **Acceptance:** numbers recorded; D7 carrier order confirmed or revised in DECISIONS.md.
- **Execution checkpoint, 2026-10-02:** bounded Windows/Android transport benchmark implemented; authorized S23 adb TCP measured 3,000 verified echoes and 256 MiB at 32.41 MiB/s. Phone Wi-Fi is disabled, USB tethering is absent and physical recovery is unmeasured. D7 remains provisional; T0.06 remains incomplete and does not unblock T1.06a. Source and exact validation are in `docs/evidence/T0.06.md`. Next independent task: T0.07, M, gpt-6-astra / xhigh.

### T0.07 — Video encode/decode spike
- **Size:** M · **Depends on:** T0.02 · **Prompt:** `prompts/phase-0/T0.07-video-spike.md`
- **Informs:** D9, STREAM-002, PERF-003
- **Deliverables:**
  - A PC tool: Windows.Graphics.Capture of a test window → Quick Sync HEVC low-latency encode (Media Foundation or oneVPL), measuring capture-to-encoded latency per frame at a 1440×3088 portrait crop and at full 3840×2160.
  - A phone tool: MediaCodec low-latency decode of the recorded stream (pushed by adb), measuring decode latency.
  - A JPEG dirty-tile encode-time measurement for the D9 frame stream.
- **Acceptance:** p50/p95 encode and decode latencies recorded; D9 confirmed or revised.

- **Execution checkpoint, 2026-10-02:** T0.07 measurements completed with gaps on the PC and authorized S23. Isolated native workers and both 130-frame hardware decoder profiles passed; recorded 10%/25% dirty tiles reached 17.872/18.695 fps over adb including phone decode/posting. Coarse WGC regions, a retained Intel unloaded-library crash and unadvertised decoder low-latency controls remain limitations; no integrated stream/performance requirement passed. D9 updated; source-bound results and media disposal are in `docs/evidence/T0.07.md`. Next: T0.08 (M, gpt-6-astra / max); current session xhigh differs, and driver/security owner actions remain pending. Continue safe preparation and independent implementation.

### T0.08 — Virtual display spike (SudoVDA, Memory Integrity on)
- **Size:** M · **Depends on:** T0.02 (and the NuGet cache from T0.01) · **Prompt:** `prompts/phase-0/T0.08-sudovda-spike.md`
- **Informs:** D11, DISPLAY-002
- **Deliverables:**
  - A restore point confirmed before any install (System Protection on).
  - SudoVDA forked into `drivers/sudovda` and built from source with the WDK and SDK.CPP NuGet packages.
  - A project-only self-signed code-signing certificate; after signing, the private key is exported to an offline password-protected `.pfx` and deleted from the store.
  - Install and uninstall scripts.
  - A Rust IOCTL client that adds and removes a 3088×1440@60 display with watchdog pings.
  - Crash test (kill the client → display removed); full uninstall including certificate removal.
- **Acceptance:** works with Secure Boot and Memory Integrity on, or a documented failure with the VirtualDrivers VDD fallback tested.

- **Execution checkpoint, 2026-10-02:** T0.08 is an incomplete software preparation checkpoint (`Status: FAIL`), with reviewed source hashes, an independent Rust probe and guarded preflight. Missing Spectre libraries, unresolved EDID output licensing, signing/install/uninstall deliverables, owner restore/security actions and live driver/fallback measurements remain open. DISPLAY-002 statuses are unchanged and D11 stays provisional. See `docs/evidence/T0.08.md`. Continue independent T0.09 (S, gpt-6.1-sol / xhigh); current gpt-6-astra / xhigh differs in model.

### T0.09 — Giant image spike and fixtures
- **Size:** S · **Depends on:** T0.02 · **Prompt:** `prompts/phase-0/T0.09-image-spike.md`
- **Requirements:** FORMAT-001 · **Informs:** D15, PERF-005, PERF-006
- **Deliverables:**
  - Fixture generator scripts (fixed seeds) with SHA-256 lists and provenance notes: synthetic 16320×12240 JPEG, a 1440×20000 screenshot, multipage PDFs, SVGs with scripts and external references, malformed files; big outputs in git-ignored `fixtures/generated/`.
  - A synthetic 200 MP HEIF generated on the phone; the owner's real 200 MP photo (if provided) in git-ignored `fixtures-private/`.
  - libvips pyramid timing and memory on the laptop.
  - Android BitmapRegionDecoder timing per region (top vs bottom of a 200 MP JPEG) and HEIF decode memory on the phone.
- **Acceptance:** numbers recorded; D15 confirmed or revised.

### T0.10 — Stroke engine spike
- **Size:** M · **Depends on:** T0.04 · **Prompt:** `prompts/phase-0/T0.10-stroke-spike.md`
- **Informs:** D6, PERF-001
- **Deliverables:**
  - A minimal Rust pressure-width stroke modeler (deterministic, libm) called from Kotlin.
  - An Android demo with front-buffered wet ink (graphics-core) plus motion prediction, using the Rust geometry.
  - A second demo using Jetpack Ink 1.0.0 `InProgressStrokes`.
  - Latency estimates for both.
  - Owner feel test (blind A/B across three stroke types).
  - An attempt to build google/ink's C++ core for Windows, time-boxed to 4 hours.
- **Acceptance:** a D6 decision recorded under the D6 rule, with numbers and the owner's preference.

### T0.11 — GPT Image round-trip spike
- **Size:** S · **Depends on:** T0.02, T0.09, owner API key · **Prompt:** `prompts/phase-0/T0.11-gpt-image-spike.md`
- **Informs:** AIEDIT-001, AIEDIT-002, AIEDIT-003
- **Deliverables:**
  - The current OpenAI image-edit endpoint, model IDs, limits (including minimum pixels and aspect range) and prices, verified on official docs.
  - A Rust CLI: image + mask + prompt → crop-and-stitch → edit request → composite → proof (changed_outside), with timings and a running cost total capped at $2.
  - Five test edits on the owner's photos: a color change, an object removal, a sign-text change, a mask touching the border, and a mask under 100×100 px.
- **Acceptance:** `changed_outside` = 0 for all edits, confirmed independently; limits and prices recorded in `docs/evidence/T0.11.md`; the API key stays in Credential Manager.

### T0.12 — Phase 0 gate review
- **Size:** S · **Depends on:** T0.01–T0.11 · **Prompt:** `prompts/phase-0/T0.12-gate-g0.md`
- **Requirements:** QUALITY-004
- **Deliverables:** a consolidated Phase 0 summary; a dated DECISIONS.md entry confirming or adjusting every PERF target (loosened targets need Christian's OK); checks that dated D6, D7, D9, D10, D11 and D15 entries exist; corrected statuses for every phase-0 requirement; the G0 decision; a list of carry-overs into Phase 1.
- **Acceptance:** Gate G0 recorded, confirmed by Christian.

**Gate G0:** hardware-truth.md complete; D6, D7, D9, D10, D11 and D15 confirmed or revised with evidence; PERF targets confirmed or adjusted (QUALITY-004); toolchain pinned; license gate working.

---

## Phase 1 — Markup core (daily use)

Goal: by the end, you screenshot something, mark it on the phone, see it live on the PC, and paste the clean or marked image into Claude Code, every day.

### T1.01 — vw-geom: coordinate spaces and transforms
- **Size:** M · **Depends on:** T0.02 · **Prompt:** `prompts/phase-1/T1.01-vw-geom.md`
- **Requirements:** CORE-002, TUNNEL-007 (math and property tests; hardware verification in T3.01)
- **Deliverables:** all spaces of BUILD_SPECIFICATION §4.2 (R, D, V, P, L, F, H, A), affine type with inverse, camera model, EXIF orientation transforms, PDF page transforms, pixel-snap helpers, property tests (round trip ≤ 1e-6 px for scales 1/64–64 and any rotation; F→H integer-exact).
- **Acceptance:** proptest suites pass with ≥ 10,000 cases each, on Windows and on the phone.

### T1.02 — vw-model and vw-ops: document, operations, transactions, undo
- **Size:** L · **Depends on:** T1.01 · **Prompt:** `prompts/phase-1/T1.02-model-ops.md`
- **Requirements:** CORE-001, CORE-005, SYNC-004, SYNC-005, SYNC-001, HOOK-001, EDIT-001, EDIT-004
- **Deliverables:**
  - Document model per §4.3 (all object kinds including adjustment; capture info; timeline reserved).
  - Ops and transactions per §4.4: host sequencing, idempotent txn IDs, LWW properties, the offline conflict rule with conflict records, fractional ordering with cycle checks.
  - Per-device undo/redo; provisional gesture state with expiry and cancel; canonical state hash.
- **Acceptance:** property tests (undo/redo restore identical hashes over 500 random sequences); duplicate transactions are idempotent; a stale offline write produces a conflict record; gesture expiry and cancel are unit-tested.

### T1.03 — vw-store: SQLite project store and blobs
- **Size:** M · **Depends on:** T1.02 · **Prompt:** `prompts/phase-1/T1.03-vw-store.md`
- **Requirements:** CORE-008, FORMAT-003, SYNC-001, HOOK-001
- **Deliverables:**
  - `contracts/storage.sql` finalized.
  - WAL store with op log, snapshots, pending queue (phone) and conflicts.
  - Content-addressed blobs (BLAKE3), including a video asset.
  - RAM cache-budget API; disk budget and reserve (max(5 GB, min(5% of the drive, 10 GB))).
  - Crash-recovery tests (kill during write).
  - Migrations framework; `.vwbz` export/import (labels stripped).
- **Acceptance:** kill-during-write tests never corrupt the store; a reopened store has an identical state hash; the disk reserve blocks caching and never touches originals.

### T1.04 — vw-ink: deterministic stroke modeling
- **Size:** M · **Depends on:** T1.01, T0.10 decision · **Prompt:** `prompts/phase-1/T1.04-vw-ink.md`
- **Requirements:** PEN-001, PEN-002, DEV-006
- **Deliverables:**
  - The stroke algorithm chosen in T0.10 (pressure curve, stabilization, pen/marker/highlighter families).
  - An incremental API for wet ink.
  - Geometry quantized to 1/256 px.
  - Golden tests from recorded traces, identical on Windows and Android builds.
- **Acceptance:** goldens are byte-identical across `x86_64-pc-windows-msvc` and `aarch64-linux-android` (Android goldens run on the device via `hil-test rust`).

### T1.05 — vw-raster: exports
- **Size:** M · **Depends on:** T1.02, T1.04 · **Prompt:** `prompts/phase-1/T1.05-vw-raster.md`
- **Requirements:** EXPORT-001, EXPORT-002, FORMAT-004, FORMAT-005, FORMAT-010, FORMAT-011, CORE-006, DEV-006
- **Deliverables:**
  - Clean and marked exports (full and region) at source resolution: tiny-skia rendering of all annotation kinds (built so output is identical on x86-64 and arm64), bundled fonts, deterministic text layout that the apps also use.
  - PNG/JPEG/WebP encoders with preflight; EXIF orientation and ICC handling; alpha and 16-bit PNG round trips.
  - Revision and asset-hash metadata in exports; golden tests.
- **Acceptance:** byte-exact goldens on both platforms; oversize WebP refused with alternatives; clean exports equal source pixels.

### T1.06a — vw-proto and vw-net transport: protocol, carriers, simulation
- **Size:** L · **Depends on:** T1.02, T0.06 · **Prompt:** `prompts/phase-1/T1.06a-transport.md`
- **Requirements:** CONNECT-004, CONNECT-005, CORE-007, DEV-005, SYNC-004, CONNECT-001, CONNECT-002
- **Deliverables:**
  - `contracts/vw_protocol.proto` finalized; prost codegen.
  - Session layer with the six channels of §4.6; reconnect semantics.
  - QUIC carrier (quinn with the `ring` provider) and TCP-mux carrier over adb with TLS 1.3 inside (test certificates until T1.06b).
  - `vw-sim` deterministic fault simulation (drop, duplicate, reorder, partition, offline).
- **Acceptance:** 10,000 seeded simulation runs converge with zero lost or duplicated transactions; MEDIA keeps latest-frame semantics under 2× overload on both carriers on hardware; ops exchange over tethering and adb on hardware. CONNECT-001/002 stay partial until T1.10.

### T1.06b — Pairing, trust store, discovery and route safety
- **Size:** M · **Depends on:** T1.06a · **Prompt:** `prompts/phase-1/T1.06b-pairing.md`
- **Requirements:** CONNECT-003, SEC-003, CONNECT-001, CONNECT-002
- **Deliverables:**
  - Device identity keys and self-signed certificates; an app-level trust store interface (Windows DPAPI implementation; Android implementation in T1.10).
  - QR pairing (HMAC over the TLS exporter); code pairing through a PAKE; single-use, expiry and lockout; revocation.
  - DNS-SD with a rotating DiscoveryId; route safety API (Windows).
  - `tools/pair-cli` (PC console plus an adb-pushed phone binary) for hardware tests before the apps exist.
- **Acceptance:** unit tests for expiry, reuse, replay, lockout, revoked refusal and TXT contents pass; hardware pairing with pair-cli over tethering succeeds and a revoked peer is refused; no key material in project files.

### T1.07 — vw-ffi and the Kotlin shared module
- **Size:** M · **Depends on:** T1.02, T1.03, T1.04, T1.05 · **Prompt:** `prompts/phase-1/T1.07-ffi-shared.md`
- **Requirements:** HOOK-002
- **Deliverables:**
  - UniFFI surface (async-friendly, batch calls for samples) for projects, gestures, queries, render lists, text layout and exports; session and pairing calls are added by T1.09 and T1.10.
  - Generated Kotlin bindings used by Android (JNA, arm64 `.so`, 16 KB aligned) and desktop (JNA, Windows `.dll`).
  - `apps/shared` KMP module (commonMain with no Android-only or JDK-only imports, plus an enforcing check); Gradle tasks invoking cargo-ndk and the Windows build.
- **Acceptance:** a smoke test creates a project, draws a stroke and exports from both Android and desktop through the bindings, matching the Rust golden; the alignment check passes; the commonMain import check passes and is proven to fail on a violation.

### T1.08a — Android app: shell, stylus input, canvas, wet ink
- **Size:** L · **Depends on:** T1.04, T1.05, T1.07 · **Prompt:** `prompts/phase-1/T1.08a-android-canvas.md`
- **Requirements:** PEN-003, PEN-015, PEN-001, CORE-004, UX-006, PERF-001, SYNC-001
- **Deliverables:**
  - Compose app: Projects, Canvas, Settings, Diagnostics (trace recorder), Pairing placeholder (completed in T1.10).
  - Stylus abstraction with capability flags and a generic-stylus profile; finger-drawing setting (off by default).
  - Canvas for images up to 50 MP (larger ones refused with a message), opened for now through the system photo picker (full import paths come in T1.11); camera through vw-geom; gesture map per §4.16.
  - Front-buffered wet ink with prediction using the core geometry; pen, highlighter and marker; undo/redo.
  - Offline local projects through vw-store.
- **Acceptance:** PERF-001 measured (≤ 25 ms p95 target); injected-event tests of the gesture map, palm/cancel and the generic profile pass; the app works in airplane mode.

### T1.08b — Android app: shapes, text, selection, brush settings, layout
- **Size:** M · **Depends on:** T1.08a · **Prompt:** `prompts/phase-1/T1.08b-android-tools.md`
- **Requirements:** EDIT-001, EDIT-004, PEN-002, UX-002, UX-003, UX-004
- **Deliverables:**
  - Line, arrow, rectangle, ellipse and text tools (text laid out and outlined by the core, never by Android text); object eraser; select, move and resize with handles; color and width quick controls.
  - Brush settings: Bézier pressure-curve editor with live preview stored per brush; stabilization slider; hover width cursor.
  - Layout per §4.17: tool rail (mirrored for left-handed use, movable), status bar, context bar; dark and light themes; 48 dp targets; reduced motion; contrast check.
- **Acceptance:** phone text matches the export golden at 1:1; every tool creates editable objects; the curve editor and stabilization change replayed-fixture output; both themes pass WCAG AA contrast.

### T1.09 — Desktop app: window, canvas, PC editing, pairing, shortcuts
- **Size:** L · **Depends on:** T1.06b, T1.07, T1.08b · **Prompt:** `prompts/phase-1/T1.09-desktop-shell.md`
- **Requirements:** UX-001, UX-004, CORE-002, EDIT-004
- **Deliverables:**
  - Compose desktop app: main window (windowed, maximized, borderless fullscreen), Per-Monitor-V2 DPI awareness.
  - Overview canvas with independent camera (mouse, touchpad, Ctrl+wheel; one-finger touch pan) and an optional peer viewport outline; follow-peer and match-view commands.
  - PC editing with the mouse: select, move and resize with handles; color and width; delete; undo/redo; arrow-key nudges. Each edit is one undoable transaction.
  - Status chips with the fixed status words; keyboard shortcuts for every command.
  - Default project location per §4.5 (OneDrive-aware).
  - PC pairing: QR code, code fallback, paired-devices list, revoke; session and pairing calls added to vw-ffi; a temporary firewall rule.
  - Placeholders for the instruction panel and transfer shelf, labelled "not available yet" (never inert).
- **Acceptance:** all three window modes keep state; the window reports the Windows display scale and evidence screenshots are crisp; objects can be selected, moved, resized and recolored with the mouse, each as one undo step; pairing with the pair-cli phone binary succeeds; every command has a shortcut.

### T1.10 — Phone pairing and live sync integration
- **Size:** L · **Depends on:** T1.06b, T1.08b, T1.09 · **Prompt:** `prompts/phase-1/T1.10-live-sync.md`
- **Requirements:** CORE-003, CORE-005, CORE-007, SYNC-005, SYNC-001, PERF-002, CONNECT-001, CONNECT-002, CONNECT-003, EDIT-001, EDIT-004, UX-005
- **Deliverables:**
  - Phone pairing: QR scan (CameraX + ZXing core), code entry, fingerprint confirmation, Keystore-backed trust store, NsdManager discovery, paired-devices list and revoke.
  - Provisional streaming of strokes, handle drags and slider moves (≤ 120 Hz), commit swap without flash, and cancel propagation (palm, ACTION_CANCEL, backgrounding); PC edits shown on the phone.
  - Reconnect mechanics: `adb reverse` re-created within 1 s; tethering shortcut; status words RECONNECTING → SYNCED, or OFFLINE (n pending).
  - Route safety wired into the app: the §4.6 warning and the one-click metric fix with its revert.
  - Offline phone work with a forced conflict; temporary firewall rules, removed at the end.
- **Acceptance:** A02 and A06 pass on hardware; PERF-002 measured; unplugging mid-gesture produces no duplicate; pairing and editing work over tethering with Developer options off, and over Wi-Fi with the router's internet unplugged; the route-safety check runs on every tether connection; the phone's trust list is Keystore-wrapped and no key material is in projects or exports; the forced conflict writes a conflicts row; camera independence verified with the phone.

### T1.11 — Import, export and handoff basics
- **Size:** M · **Depends on:** T1.05, T1.08b, T1.09, T1.10 · **Prompt:** `prompts/phase-1/T1.11-import-export.md`
- **Requirements:** EXPORT-006, FORMAT-005, CORE-006, HOOK-001, SYNC-001, UX-005
- **Deliverables:**
  - Import on Android (share target, photo picker, camera) and on the PC (drag-drop, open, paste); blob sync to the peer, including a video asset; imports over 50 MP refused with a message.
  - Exports to the PC clipboard (PNG plus DIBV5), file drag (temporary PNG), and Android share (FileProvider).
  - Revision and resolution shown on export.
- **Acceptance:** tested into Claude desktop, ChatGPT desktop, a browser chat and Claude Code (A13, partial; Christian runs the Claude Code paste himself); a `.mp4` asset syncs with an identical hash; in airplane mode the phone exports a marked PNG; SYNCED, SYNCING, RECONNECTING, OFFLINE (n pending) and EXPORT READY each observed on both devices.

### T1.12 — Phase 1 acceptance run
- **Size:** S · **Depends on:** T0.12 and every Phase 1 task · **Prompt:** `prompts/phase-1/T1.12-phase1-acceptance.md`
- **Requirements:** CORE-001, CORE-002, CORE-003, CORE-005, PEN-001, PEN-003
- **Deliverables:** scripted A01, A02 and A06 runs with evidence; PERF-001/002 numbers; corrected statuses for every phase ≤ 1 requirement; a list of gaps; an installable APK and desktop app image; `docs/QUICKSTART.md`.
- **Acceptance:** Gate G1 — recorded as pending until Christian confirms daily use after a day with the build.

---

## Phase 2 — Your two first wins

Prompts are written after G1. Tasks and acceptance are fixed now.

| Task | Size | Depends on | Requirements | Deliverables and acceptance |
|---|---|---|---|---|
| T2.01 Selections and masks (W1 subset) | M | T1.12 | EDIT-005, EDIT-006, EDIT-007, EXPORT-003 | Rectangle, lasso and painted-mask selections; all mask math (add, subtract, intersect, invert, expand, shrink, feather); stroke, object and mask erasers; mask and cutout export. Golden mask math within 1 level per pixel. |
| T2.02 Provider interface and GPT Image adapter | M | T0.11, T1.12 | AIEDIT-001, AIEDIT-004, AIEDIT-005 | Provider capabilities (including minimum pixels and aspect range) and cost config; GPT Image adapter; keys in Credential Manager / Keystore; cost estimate and explicit Send; per-day soft budget. |
| T2.03 Crop-and-stitch, proof, compare, partial accept | L | T2.01, T2.02 | AIEDIT-002, AIEDIT-003, EDIT-010, EXPORT-011, PERF-009 | §4.9.2 steps 1–8; Result layers; wipe/blink/split/difference on both devices; brush partial acceptance. changed_outside = 0 always; PERF-009 measured. |
| T2.04 Markers, roles, instructions, focus follow | M | T1.12 | EDIT-002, EDIT-008, INSTR-001, INSTR-002, UX-007 | Numbered markers, roles, instruction model; PC instruction panel with focus follow ≤ 200 ms; phone instruction editor. |
| T2.05 Voice and handwriting entry | S | T2.04 | INSTR-003, INSTR-004 | Android on-device dictation; Windows voice typing verified in fields; stylus handwriting in fields; entry_method recorded. |
| T2.06 Capture with semantics; HEIC import | L | T1.12 | CAPTURE-001, CAPTURE-006, SEM-001, SEM-002, SEM-003, FORMAT-008 | Ctrl+Alt+A foreground-window capture (WGC, lossless) + UIA snapshot; own windows excluded; Android accessibility capture (screenshot + node tree) via Quick Settings tile, shortcut and notification; Chromium via UIA; HEIC import on both devices. |
| T2.07 Snap-to-element and element references | M | T2.06 | SEM-004, SEM-005, SEM-006 | Snapping within 12 px; ≥ 95% accuracy on 100 targets; element refs in documents; untrusted-text notice and code-span quoting with an injection-string golden. |
| T2.08 Package compiler | M | T2.04, T2.07 | PKG-001, PKG-002, PKG-003, PKG-004, EXPORT-004, EXPORT-008, SEM-005, SEM-006 | Manifest per schema (files list with hashes); claude (2576/1568 px tiers), openai, gemini and generic targets; overview and crops; prompt.md; golden package tests. |
| T2.09 MCP server | M | T2.08 | AGENT-001, AGENT-005, SEC-001, TUNNEL-003 | SDK choice recorded (Kotlin SDK in the desktop app vs TypeScript sidecar); tools per §4.9.1; stdio bridge via named pipe; token in Credential Manager; fixed port; 127.0.0.1 + Origin + token; both protocol versions checked with the official test client; capture grant and indicator; works from Claude Code and Codex. |
| T2.10 Push adapters, fallbacks, staging | M | T2.09 | AGENT-002, AGENT-003, AGENT-004, EXPORT-005, EXPORT-007, EXPORT-009, EXPORT-010, PEN-012 | Claude Code channel (the stdio bridge is the channel server); Codex App Server push with a runtime schema check; two-step clipboard and drag; transfer shelf; large-file fallback; nothing sends without explicit action. |
| T2.11 Phase 2 acceptance | S | T2.01–T2.10 | (verification of the above) | A13, A21, A22, A23, A24, A26, A29 on real apps, plus the A06 re-run → Gate G2. |

## Phase 3 — Precision, capture, sync

| Task | Size | Depends on | Requirements | Deliverables and acceptance |
|---|---|---|---|---|
| T3.01 Capture picker and DPI mapping | M | T2.11 | CAPTURE-007, TUNNEL-007 | Window/monitor/region picker and Ctrl+Alt+R; region requested from the phone; F→H verified on hardware at the laptop's scaling (negative origin with an external monitor if available). |
| T3.02 Frame stream, ring buffer, live annotate | L | T3.01 | STREAM-001, STREAM-004, HOOK-003, CAPTURE-002, CAPTURE-003, CAPTURE-004, TUNNEL-001, PERF-004 | JPEG dirty-tile frame stream ≥ 15 fps under STREAM-001's conditions; lossless ring buffer; freeze state machine per §4.18; A07. |
| T3.03 Loupe and hover-aim | M | T2.11 | PEN-004, PEN-013, PEN-016 | Contact and hover loupe; placement and edge rules; identical geometry with loupe on/off; A03. |
| T3.04 Precision lens | M | T3.03 | PEN-005, PEN-006, PEN-007, PEN-008 | Lens 2/4/8×, latch, queued toggles, exact restore; A04, A05. |
| T3.05 Ellipse and polygon selections; tilt | M | T2.11 | EDIT-013, PEN-014 | Ellipse and polygon selections; tilt recorded in strokes if the device reports it, with the §4.11 conversion and sign calibration. |
| T3.06 Assisted selection | L | T3.05 | SEL-001, EDIT-011 | Segmentation model on the phone (NPU/GPU) with CPU fallback; manual correction; license-checked model. |
| T3.07 Layers, rotation, crop/tonal, bookmarks, multi-document | M | T2.11 | EDIT-003, EDIT-012, EDIT-009, CORE-009 | Layer panel, groups, locks; rotation handles; nondestructive crop/rotate/resize/levels; region bookmarks; ≥ 20 documents. |
| T3.08 Giant images | L | T2.11 | FORMAT-002, PERF-005, PERF-006, PERF-008 | libvips pyramids on the PC (`cfg(windows)`); tile streaming; phone-originated tiling with platform decoders; GPU texture budgets; A10. |
| T3.09 PDF, SVG, redaction, sandboxing | L | T2.11 | FORMAT-006, FORMAT-007, QUALITY-001, QUALITY-002 | PDFium pages, text selection, overlays, exports; true redaction in the PC worker (qpdf) and full rasterization on the phone alone; resvg with deny-all resolver and fidelity warnings; SVG export; malformed corpus; A12. |
| T3.10 BMP, TIFF, AVIF and color | M | T3.08 | FORMAT-013, FORMAT-014 | BMP/TIFF import and export; AVIF via OS decoders; color-managed display rendering; A11. |
| T3.11 Offline rebase, conflict review, carrier switching, phone AI sends | L | T2.11 | SYNC-002, SYNC-003, PERF-007, AIEDIT-007 | Rebase at scale and the conflict review-and-swap UI per §4.4; USB ↔ Wi-Fi switching ≤ 2 s; the W1 loop from the phone alone; A14, A25. |
| T3.12 Phase 3 acceptance | S | T3.01–T3.11 | (verification) | A03, A04, A05, A07, A10, A11, A12, A14, A25, plus the A01, A02 and A21 re-runs → Gate G3. |

## Phase 4 — Remote pen editing and virtual monitor

| Task | Size | Depends on | Requirements | Deliverables and acceptance |
|---|---|---|---|---|
| T4.01 HEVC streaming pipeline | L | T3.12, T0.07 | STREAM-002, PERF-003 | WGC → Quick Sync HEVC low-latency → MEDIA → MediaCodec low-latency → SurfaceView; ≥ 30 fps; frames report last_input_seq_applied; PERF-003 measured. |
| T4.02 Lossless text tiles | M | T4.01 | STREAM-003 | Dirty-region lossless refinement after 500 ms idle; decoded static regions hash-equal to source. |
| T4.03 Injection, guards, control grant | L | T4.01, T0.05 | TUNNEL-004, TUNNEL-006, TUNNEL-008, SEC-002, CAPTURE-005 | Pen/mouse/keyboard injection with the §4.11 tilt conversion; per-batch guards; control-grant state machine; 0 stray events in 100 trials; no stale input after reconnects or carrier switches; A08. |
| T4.04 Ghost ink | M | T4.03 | GHOST-001, GHOST-002 | Local echo overlay with input-sequence fade rules; A27. |
| T4.05 Virtual monitor integration | L | T0.08, T4.01 | DISPLAY-001, DISPLAY-002, DISPLAY-003, DISPLAY-004 | Signed SudoVDA install/uninstall in the app; modes; negative-origin and rotated mapping on hardware; watchdog recovery; A17. |
| T4.06 Photoshop: drawing, dual view, UXP bridge | L | T4.03, T4.05, owner installs Photoshop | ADOBE-001, ADOBE-005, ADOBE-006, ADOBE-007, ADOBE-008 | Pressure in Photoshop 27.9.1; New Window on the virtual display; UXP plugin via UPIA; pixel/selection/layer commands with explicit IDs; A15, A16. |
| T4.07 Free and web editors; Krita bridge; compatibility matrix | M | T4.03 | EDITOR-001, EDITOR-002, EDITOR-003, EDITOR-004, ADOBE-002, ADOBE-003, ADOBE-004, LIC-004 | Krita, GIMP, Affinity, Photopea, Paint (and Illustrator only if installed) tested; Krita GPL bridge in `bridges/krita`; matrix in `docs/compat/`. |
| T4.08 Shortcut palettes and command destinations | S | T4.03 | EDITOR-005, TUNNEL-009 | Visible command destination; app-aware undo/redo; per-editor palettes. |
| T4.09 Phase 4 acceptance | S | T4.01–T4.08 | (verification) | A08, A15, A16, A17, A27, plus the A14 re-run → Gate G4. |

## Phase 5 — Verification, Android tunnel, release

| Task | Size | Depends on | Requirements | Deliverables and acceptance |
|---|---|---|---|---|
| T5.01 Verification loop | L | T4.09 | VERIFY-001, VERIFY-002, VERIFY-003, VERIFY-004, AGENT-006 | Recapture and alignment; per-marker before/after; pass/fail; failures package; MCP tools; A30. |
| T5.02 More image models | M | T4.09 | AIEDIT-006 | Gemini and FLUX.1 Fill adapters; candidate grid. |
| T5.03 Android tunnel | M | T4.09 | TUNNEL-002, TUNNEL-005, SEM-007, CAPTURE-008 | Pinned scrcpy using the owner's adb (never bundled); screencap + uiautomator capture from the PC; feedback-loop refusal when a streamed or captured display shows the scrcpy window; scrcpy's DLL licenses verified (for example `avcodec_license()`) and listed in `third_party/LICENSES`. |
| T5.04 Air Actions module | M | T4.09 | PEN-009, PEN-010, PEN-011 | Declarative Air Actions; dispatcher dedup; onboarding; A09. |
| T5.05 Diagnostics bundle and privacy | S | T4.09 | QUALITY-003 | Diagnostics export with redaction preview; automated identifier/secret scan. |
| T5.06 Packaging and release | M | T5.01–T5.05 | QUALITY-006, LIC-003 | Signed APK; MSI (jpackage + WiX + bundled runtime); driver, `.ccx` and Krita packages; checksums; notices; `third_party/LICENSES`; Authenticode signing plan; toolchain record; uninstall and recovery guide; A20. |
| T5.07 Full acceptance and compatibility matrix | M | T5.06, T5.08 | QUALITY-008 | Every scenario A01–A30 re-run on the release build; release performance, thermal and recovery report; compatibility matrix; untested list → Gate G5. |
| T5.08 Optional formats and deferred-item decisions | S | T5.01–T5.05 | FORMAT-009, FORMAT-012, INSTR-005 | PSD/PSB/AI composite import and RAW/DNG previews through OS decoders (P2; if Christian defers one, a dated DECISIONS.md entry); a recorded decision for INSTR-005 (next-release plan or drop with owner approval). |

---

## Risk register

| # | Risk | Likelihood | Impact | Mitigation | Trigger to act |
|---|---|---|---|---|---|
| 1 | Builds are slow or memory-starved on the 4-core, 16 GB laptop | High | Medium | Incremental builds, sccache, capped daemons, no Android Studio; move release builds to CI later | A full build takes > 20 min, or the laptop swaps heavily |
| 2 | S Pen reports no tilt | Medium | Low | PEN-014 is conditional; nothing else depends on tilt | T0.04 result |
| 3 | Synthetic pen pressure fails in some editors | Medium | Medium | Compatibility matrix; Windows Ink settings per editor; WinTab-only apps documented as unsupported | T0.05 / T4.07 results |
| 4 | SudoVDA blocked by Memory Integrity or policy | Low–Medium | Medium | VirtualDrivers VDD fallback (SignPath-signed); DISPLAY is P2 and can slip | T0.08 failure |
| 5 | Claude Code channels or Codex App Server change (experimental) | High | Low | Version-gated adapters; MCP pull and clipboard always work | Adapter self-test fails after an update |
| 6 | OpenAI image API changes model IDs, limits, prices or verification rules | Medium | Medium | Provider config, not code; Gemini/FLUX adapters in Phase 5 | T0.11 or a later adapter self-test |
| 7 | Compose desktop limits (pen pressure on the laptop, touchscreen gestures, native surfaces) | Medium | Low–Medium | Rust hooks; D2 alternative (WinUI 3) documented | A blocking defect in T1.09 or T4.01 |
| 8 | Intel legacy driver bugs in Quick Sync low latency | Medium | Medium | Software H.264 fallback (OpenH264, BSD) or a JPEG frame stream for remote edit at lower fps | T0.07 or T4.01 anomalies |
| 9 | Android restricts enabling accessibility services for sideloaded apps | Medium | Medium | Install via adb; onboarding explains "Allow restricted settings"; MediaProjection fallback for screenshots | T2.06 on device |
| 10 | Android developer verification for sideloading (rolling out 2026–2027) | Medium (future) | Low now / High for a product | adb installs are exempt for personal use; register as a developer before distributing | Before any distribution |
| 11 | 16 GB RAM with Photoshop + JVM + builds | Medium | Medium | Memory budgets (PERF-006); avoid concurrent heavy work; measure in Phase 4 | Swapping during Phase 4 tests |
| 12 | Wet-to-dry ink "pop" if engines differ | Low | Medium | One geometry engine for wet and dry (D6) | T0.10 / T1.08a visual diff > 0.5 px |
| 13 | Smart App Control blocks locally built programs | High if on | High | T0.01 checks first; Christian turns it off for development (reversible since April 2026); sign every binary before any distribution | Smart App Control is on or in evaluation |
| 14 | Windows Firewall silently blocks QUIC on tether or Wi-Fi links (often classified Public) | Medium | Medium | Temporary rules in development, an installer rule limited to the local subnet, and a firewall check before declaring a carrier failed | A carrier times out while adb works |
| 15 | Parallel tasks in one folder or unmerged dependencies corrupt shared files (REQUIREMENTS.json, DECISIONS.md) | Medium | Medium | Start from `main` with dependencies merged; one task per folder; worktrees for parallel tasks | A merge conflict in a shared doc |
