# Visual Workbench — spec package v2.0

## Development starter — 2026-10-01

The active working folder is this `visual-workbench/` directory. The Rust/Kotlin/Compose development starter builds and runs on Windows and the approved IN2019 phone. Pen, canvas, pairing, capture, synchronization and AI features are still planned. See [the startup receipt](docs/evidence/SETUP_STARTUP.md) for tested scope and remaining setup clauses.

Read [the setup assessment](docs/SETUP_ASSESSMENT.md) for source authority, the selected compatible stack, the available-device boundary and the next tasks. The v2 authority is [docs/BUILD_SPECIFICATION.md](docs/BUILD_SPECIFICATION.md), [docs/REQUIREMENTS.json](docs/REQUIREMENTS.json), [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md) and [docs/SOURCES.json](docs/SOURCES.json). Root specification/requirement copies and the archived folders preserve v1/reference material.

### Build and start

Run from this directory in a shell with the selected JDK 21, Android SDK/NDK, Rust/Cargo, Python and gitleaks available. Local environment paths and measured verification belong to the setup report under `docs/evidence/`.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 doctor
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 build-core
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 build-android
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 build-desktop
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\start.ps1
```

`build-desktop` creates a Windows application JAR using the owner's installed JDK 21. `start.ps1` opens that built starter and leaves it running. The development JAR does not bundle a Java runtime; app-image/MSI packaging and the documented Java-runtime exception manifest remain later packaging work. `build.ps1 run-desktop` offers a bounded Gradle development session (`-TimeoutSeconds`, default 600). Every build runs the license gate first.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 license-check
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 test-all
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 lint-all
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 hil-test
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 hil-test rust vw-ffi
```

HIL requires exactly one authorized physical Android device. The app form installs the built debug APK, checks foreground startup and requires an actual passing instrumentation test. It then restores and opens the starter after the test runner removes its installation. Model IN2019 on Android 11/API 30 is suitable for starter smoke checks, while S23 Ultra hardware acceptance remains pending. `hil-test rust vw-ffi` targets the setup ABI smoke test.

Next: T0.04 pen probe/trace recorder, following the Phase 0 dependencies and current model/effort recommendations. Setup/hardware carry-overs remain explicit; the owner's storage-warning waiver continues to apply. Phase-end Git publication is authorized under the policy below.

The IN2019 is shared with another app development session. Foreground checks allow up to three brief starter relaunches; persistent focus contention is reported as inconclusive. Other apps are preserved and adb/device resets are avoided.

## GitHub publication

The repository is public at [strickerca/visual-workbench](https://github.com/strickerca/visual-workbench).
The owner authorized the initial T0.01–T0.03 checkpoint and automatic commit/push
to origin/main at the conclusion of each numbered phase. Follow
[Automatic phase publishing](docs/IMPLEMENTATION_PLAN.md#automatic-phase-publishing).
Phase gates and physical acceptance remain separate; Phase 0 is still in progress.
Local caches, build outputs, credentials, signing material, private fixtures and
verification screenshots stay outside Git.

## Preserved original package README

The original package overview below is retained unchanged. Its statement that nothing had been built describes the package at intake, before the development starter was added.

Revision 2026-10-01. Supersedes the 2026-09-22 package (v1).

**This package is a specification and a build plan, not the app.** Nothing in it has been built or tested on hardware yet. Facts marked *verified* in the spec come from the owner's diagnostics run on 2026-10-01; everything else stays unverified until Phase 0 evidence exists.

## Start here

1. Extract the zip into `C:\dev` so you get `C:\dev\visual-workbench`. That folder becomes the repository root.
2. Open a fresh Claude Code session in that folder and paste the whole of `prompts/phase-0/T0.01-toolchain.md`.
3. When the agent finishes, read its five-part report, do anything listed under "What Christian needs to do next", and say yes when it asks to merge if you're happy.
4. Paste the next prompt (T0.02, T0.03, …). The dependencies are in `docs/IMPLEMENTATION_PLAN.md`; run one task at a time unless you set up a separate worktree folder.

Prompts for Phases 2–5 are written after Gate G1, from real Phase 0 measurements.

## Contents

| Path | What it is |
|---|---|
| `docs/BUILD_SPECIFICATION.md` | The product, hardware facts, architecture and every contract (coordinates, document model, sync, storage, protocol, pairing, packages for AI, capture, remote editing, virtual display, editor bridges, formats, performance budgets, invariants, gates, scenarios). |
| `docs/REQUIREMENTS.json` | 189 requirements, each with priority, phase, dependencies, measurable acceptance, test method and status. All 98 v1 IDs are kept. Also holds scenarios A01–A30 (with each one's gate and re-runs) and the hardware truth table. |
| `docs/IMPLEMENTATION_PLAN.md` | Phases 0–5, 66 agent-sized tasks, owner actions, repository layout, gates, status rules and risks. |
| `docs/DECISIONS.md` | Architecture decisions D1–D19, with alternatives, confidence and the spike that confirms each provisional one. |
| `docs/TOOLCHAIN.md` | Known-good tool and library versions as of 2026-10-01. Task T0.01 pins exact versions. |
| `docs/SOURCES.json` | 68 sources checked 2026-09-30 / 2026-10-01, each with the narrow claim it supports. |
| `contracts/` | Draft wire protocol (`vw_protocol.proto`), AI package manifest schema (`package.schema.json`) and project database schema (`storage.sql`). |
| `prompts/phase-0/`, `prompts/phase-1/` | 26 standalone Claude Code prompts, one per task. Each carries its full context, rules, definition of done and report format. |
| `tools/check_plan_coverage.py` | Checks that every requirement is assigned to a task that can pass it at its own gate, that every gate task depends on the tasks its scenarios need, that the plan and prompts use only real IDs, and that each prompt's requirements and dependencies match the plan. |
| `tools/diagnostics/` | Read-only PC and phone diagnostics scripts. They print no serial numbers, account names or addresses. |

## What changed from v1

- **Built around your first two wins.** Marking up an image so GPT Image edits it (W1) and marking up app screens so Claude Code or Codex changes the UI (W2) are Phase 2, right after the markup core.
- **91 new requirements** for semantic capture (UI Automation and the Android accessibility tree), the AI instruction package, agent hand-off (MCP, Claude Code channels, Codex), masked image edits with proof that nothing outside the mask changed, verification, sync, streaming, editor bridges, licensing, security, developer setup and performance budgets. Eight of them split a v1 requirement whose parts land in different phases, so every gate can actually pass.
- **Architecture decided (D1–D19):** one Rust core shared by both apps; Kotlin and Compose on the phone and the PC; a host-authoritative operation log so the phone works offline and syncs on reconnect; QUIC over USB tethering or Wi-Fi with no developer mode needed for normal use; QR pairing with pinned TLS.
- **Phase 0 spikes first.** After two setup tasks, nine measurement tasks (T0.03–T0.11) prove the risky parts (pen data, Windows pen injection, transport, video, the virtual display driver, giant images, stroke geometry, GPT Image masks) on your actual laptop and phone before anything depends on them, and a gate review locks the performance targets.
- **No GPL or AGPL code in the app**, so it stays sellable. Each dependency's license is checked automatically.
- **Hooks for later:** video editing and the iPad are reserved in the data model and protocol so they don't need a rewrite.

## Checks

```
python tools/check_plan_coverage.py --v1 <path to the v1 REQUIREMENTS.json>
```

## Diagnostics ready (T0.03)

The Windows diagnostics and bounded selected-window screenshot wrapper are in
[tools/diag-win](tools/diag-win/README.md). Dated PC/IN2019 facts and explicit
untestable S23/S Pen states are in [hardware-truth.md](docs/evidence/hardware-truth.md).
[PERF-template.md](docs/evidence/PERF-template.md) defines all nine measurement
methods. Screenshots are inspected and disposed outside Git; text receipts remain.
T0.04 is the next slice: pen probe/trace recorder, size M, recommended
`gpt-6-astra` with `xhigh` reasoning. Actual S23 pen traces require that device;
IN2019 startup results do not verify S Pen behavior. Settings are owner-controlled.
