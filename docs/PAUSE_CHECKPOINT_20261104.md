# Pause checkpoint — 2026-11-04

The owner paused all builds to free system resources and asked for the project
to pause once the code work was done. The code work is now done; what remains
is builds, device runs and owner sessions.

## Repository state

- Updated 2026-11-06 after a full review of the session's work. Local
  `main` was at the review-fix merge (`caf8978` plus checkpoint updates).
  Those commits have since been pushed to `origin/main`. Stage 1 sign-off
  still waits for the remaining blockers.
- The review-fix merge includes two **uncompiled** code changes on main:
  `apps/shared/.../AiInteraction.kt` (caller cancellation now cancels the
  owned job synchronously; `AiInteractionTest` no longer yields) and
  `host-win/.../editor_probe_runner/frame/paint.rs` (dead overflow checks
  removed). The B5 delivery and notices changes in it are covered by passing
  PowerShell 5 fixtures and Python tests.
- Working tree: clean.
- 2026-11-13: on the owner's instruction, the three branches below were
  merged into main **before** their compile and device checks, and main was
  pushed to `origin/main`. The branches and their worktrees were deleted.
  Their pending checks (resume steps 1–3) now run on main, not in worktrees.
  Main therefore holds uncompiled Kotlin and Rust from all three.
- Merged branches (formerly waiting for builds):

| Branch | Tip | Worktree | What it is | Reviewed |
| --- | --- | --- | --- | --- |
| `task/b4-canvas-present-20261104` | `89bd3a3` | `b4-canvas` | B4 canvas make-before-break and Callback2 handshake, plus a pixel test | Fix `b562578` verified on the S23 (before: 224/226 with black frames; after: 226/226, twice). Review fixes in `89bd3a3` are **uncompiled** |
| `task/b9-physical-gesture-producer-20261104` | `d4d3e5a` | `agent-afc498451465324ed` | B9 Tier 1 physical-gesture producer (phone journal, owner-physical scenario, receiver pen history, validators, Paint per-aspect rows) | Reviewed; H1/H2/M1–M5/L1–L5 fixed. Python and PowerShell tests pass; Kotlin and Rust are **uncompiled** |
| `task/t501-phone-recapture-20261104` | `6deadfa` | `agent-a147779b1954bb954` | VERIFY-001/004 phone and tunnel verification origins | Reviewed; all 7 findings fixed. 2026-11-06 review fixes (reserved recapture labels, refused-route slot release, bounded fallback) independently reviewed twice. Node tests 23/23; Kotlin **uncompiled** |

## Stage 1 blockers

| Blocker | Status | Evidence |
| --- | --- | --- |
| B1 Windows release replays | Closed | `evidence/B1_WINDOWS_RELEASE_REPLAYS_20261104.md` |
| B2 model graphs and LiteRT scan | Graphs reproduced byte-identical; Android runtime admission still open | `ASSISTED_MODEL_LOCAL_REPRODUCTION_20261104.md` |
| B3 WiX and scrcpy licenses | Conditions met (D22). Tunnel catalog compile is still T5.03 | `evidence/B3_LICENSE_CONDITIONS_20261104.md` |
| B4 black canvas | Fixed and verified on the S23; review fixes need a build plus a device rerun | branch above |
| B5 normal APK delivery | Accepted on the S23 | `evidence/B5_NORMAL_DELIVERY_20261104.md` |
| B6 visible phone→Paint run | Prepared; not run | `.local/b6-prep/` (run-b6.ps1, README) |
| B7 Windows owned guard | Closed: 103 cases | `evidence/B7_WINDOWS_OWNED_GUARD_20261104.md` |
| B8 packaged MCP smoke | Closed: two-cycle pass, ~6.5 s per start | `evidence/B8_PACKAGED_MCP_SMOKE_20261104.md` |
| B9 physical-gesture producer | Code done; needs compile, then an owner S Pen session | branch above |
| B10 live GPT Image | Waits for an owner spend decision | — |
| LIC-003 generated notices | Merged; status `complete` (427 components) | `THIRD_PARTY_NOTICES.md`, D23 |

## Resume order (builds; one heavy build at a time, ≥10.5 GB free)

Steps 1–3 now run on main (the branches are merged; their worktrees are gone,
so native libraries must be rebuilt with `build-ffi` first). Fix failures on
main in a new task branch.

1. **B4.** Run Gradle
   `:android:testDebugUnitTest` (with `-PvwAndroidHil=true` and
   `-PvwNativeDir`/`-PvwAndroidNativeDir` pointing at the rebuilt
   `target\`). Then run `build.ps1 hil-test app` with
   `VW_ANDROID_EXPECTED_MODEL=SM-S918U`. It must pass 226/226 with
   `black_band_samples=0`. Then get a short independent re-review of
   `89bd3a3`.
2. **B9.** On main:
   - Android unit tests (`RemotePhysicalGestureJournalTest`,
     `RemoteWindowDeliveryTest`) and the remote-integration HIL variant.
   - Shared `RemotePhysicalCompatibilityTest` (expects 11).
   - Desktop compile.
   - `cargo clippy -D warnings` and `cargo test -p vw-remote-host`.

   Fix anything that fails to compile.
3. **VERIFY-001/004.** Desktop compile, then `:desktop:test` for
   `com.visualworkbench.desktop.mcp.*` and `DesktopAndroidTunnelTest`.
4. **Rebuild on merged main:**
   - `build-android`, then deliver with `build.ps1 deliver-android-normal`.
   - `build-desktop-distribution`, then rerun the packaged MCP smoke
     (`tools/desktop-test/run.ps1 -Execute -Mode mcp`).
   - `test-all` (run `build-ffi` first) and `lint-all`. These are the first
     compile of main's review fixes (`AiInteraction`, `paint.rs`); fix
     anything that fails before B6.
5. **B6.** Freeze main, then `build.ps1 build-remote-integration`, then
   `pwsh .local/b6-prep/run-b6.ps1 -Stage Check`, then `-Stage Prepare`, then
   the owner-attended `-Stage Run`. That needs about 10–15 minutes hands off,
   USB tethering on, Paint closed and its Brush tool last used. The Paint
   version (11.2605.81.0) currently matches the pinned policy.
6. **B9 owner session.** About 30 minutes over three phases: injection
   negative control, HIL receiver strokes, Paint blocks. It needs the S23 S Pen.
   - The S Pen's source (0x5002) and `verifyInputEvent` signing are unconfirmed
     on the device. If either does not hold, every sample comes out Unknown.
   - The S Pen has no eraser end, so InvertedEraser stays NotTested.
7. **Stage 1 publish.** Push main to `origin/main` and verify the remote SHA.

## Owner decisions or actions pending

- Hands-off windows for B6 and the B9 S Pen session.
- B10: approve live OpenAI spend (about $1–5), or keep the mock-only carry-over.
- Stage 0 confirmations still open: D21 (personal G5 finish line).

## Environment notes

- The task worktrees under `.claude/worktrees/` were removed on 2026-11-13.
- Downloads and private artifacts live under `.local/` (gitignored):
  - scrcpy and WiX archives;
  - LiteRT AAR scan;
  - the assisted-model graphs;
  - delivery backups;
  - MCP preflight receipts.
- WSL Ubuntu holds the B2 conversion venv in `~/vw-assisted-20261104`, if a
  re-run is needed.
