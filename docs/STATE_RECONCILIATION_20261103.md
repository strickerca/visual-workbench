# State reconciliation — 2026-11-03

Stage 0 of the 2026-11 completion roadmap. Base: `main` = `origin/main` =
`2b6b2ea`, clean tree. Each statement below was checked against git ancestry
or local evidence; it supersedes the named stale text without relabelling the
original receipts.

## Corrections to stale statements

| Stale statement | Where | Current fact | Check |
| --- | --- | --- | --- |
| Desktop receiver candidate `dae67bb` "held" for the blit presentation-receipt defect | ACTIVE_CLOUD_LANES_20261019 (middle section), CLOUD_MODEL_TRANSITION_CHECKPOINT_20261020, CLOUD_ROOT_INTEGRATION_20261020, CLOUD_RESUMPTION_CHECKPOINT_20261020, DEFERRED_VALIDATION_LEDGER_20261018, ROADMAP_CURRENT_EXECUTION_LEDGER_20261019 (T3.02 row) | Closed in source. The bounded Desktop `18158c7` successor was accepted. Its 6 production and 8 test leaves were promoted in `6a263dd` (in main; 125-method Desktop batch plus independent review). The branch is intentionally unmerged. | `git merge-base --is-ancestor 6a263dd main` |
| "Main promotion: pending approval" | WINDOWS_S23_ACCEPTANCE_20261020 | Done. `f2302cc` and all later work are in main. | ancestry |
| "No MSI build hook exists" | RELEASE_ROOT_COMPLETION_20261020 | Superseded by MSI_RELEASE_CLOSEOUT_20261020 / `d6ab240` (in main). `build.ps1` has `package-desktop-msi` and `assemble-release`. | ancestry, `build.ps1` ValidateSet |
| Release-hook cherry-picks `4841faa…` / `558aa03…` | RELEASE_ROOT_COMPLETION_20261020 | Those objects do not exist locally. The actual commits in main are `9f2449b` and `e4c761e`. | `git cat-file`, ancestry |
| Windows build "blocked by missing MSVC lib.exe" | ROADMAP_REMAINING_PHASES_20261020, ACTIVE_CLOUD_LANES_20261019, CLOUD_MODEL_TRANSITION_CHECKPOINT_20261020 | Applies only to the Linux cloud cross-check. This laptop has MSVC 14.44.35207 (VS 2022 Build Tools 17.14) and 14.50.35717 (VS Build Tools 2026 18.5), with x64 `lib.exe`, plus Windows SDK 10.0.26100.0. | `vswhere`, file presence |
| Paint provider budget 500 ms | IMPLEMENTATION_PLAN (2026-10-07 entry) | 750 ms since R161 (parent 1,000 ms, managed 2,500 ms). A note was added under that entry. | plan R161 entries |

## Unrecorded local result

The HIL receiver focus repair `b9d541b` (in main) has a passing local balanced
replay that docs/ had not recorded:

- Receipt: `.local/retired-worktrees/windows-s23-20261020/.local/remote-normal-integration-763c9abf849d4a658b3d57655d8f9b79.json`
- `kind=normal-native-remote-usb-integration`, `status=passed`, `model=SM-S918U`,
  `carrier=QuicTether`, `physical_route_verified=true`, `owner_user=0`,
  `profiles_preserved=true`, `phone_render_observed_count=3`, artifact manifest
  `5d1d39a6…1f2e`.
- The input was software-generated. This does not pass physical S Pen, latency
  or normal Workbench UI acceptance.
- The earlier run `c4ea9b87…` failed at `owned_target_readiness`. It remains a
  failure receipt.

## Open defects carried into Stage 1

- **B4:** `.local/acceptance-evidence-20261020/PHONE_RETAINED_VISUAL_TRIAGE_20261020.json`
  shows a full black drawing region after eraser, resize or Activity recreation
  (`visual_acceptance_passed=false`).
- **B6:** the visible phone→Paint run failed twice at `baseline_wgc`. The second
  failure was `cursor_in_canvas`.
- **B7:** the Windows owned guard (103 cases) stopped before its census because
  remote edit control was not granted at initial owned-foreground readiness.
- **B8:** the packaged MCP two-cycle smoke never ran (`smoke_executed=false`):
  the owner-approved cache preflight failed at `pin_cache_ancestors` with
  `canonical_path_changed` before any smoke started.

## Environment at entry

- Free space on C: was about 1.1 GB, below the 10 GB + 512 MiB build floor.
  `.local/` is about 17 GB and `~/.gradle` about 16 GB. Space was freed
  outside this task (nothing in the repo or `~/.gradle` was deleted), and C:
  had 23 GB free when the baseline run started.
- Toolchain: Rust 1.99.0 (pinned in `rust-toolchain.toml`) and the MSVC
  versions listed above.

## Requirement status sync

A read-only source audit of the 51 `not_started` requirements
([receipt](evidence/STAGE0_REQUIREMENT_AUDIT_20261103.md)) found:

- **38 with integrated implementing source and tests.** They are now
  `in_progress` / `not_tested`, each citing the audit receipt.
- **6 partial:** FORMAT-009, FORMAT-012, VERIFY-001, VERIFY-004, QUALITY-006
  and LIC-003. These are also now `in_progress`; each evidence entry states what
  is present and what is missing.
- **6 measurement-only:** PERF-001, PERF-005, PERF-006, PERF-007, PERF-008 and
  QUALITY-008. They stay `not_started` until measured. PERF-007 also has the
  unconfirmed code question listed below.
- **INSTR-005:** no source by design. It needs a dated owner decision (T5.08).

The audit is not a verification run, and verify status is unchanged. "Source present" means the source and test files
exist and are wired into the app roots; it does not mean they compiled or passed.
`check_plan_coverage.py`: OK (189 requirements, 66 tasks, 26 prompts).

Code gaps carried into later stages (the partial requirements above, plus one open question):

- **T5.01:** VERIFY-001 and VERIFY-004. Phone-screen and tunnel captures never
  register a verification target (`McpVerificationTargets` is fed only by Desktop
  window and picker captures).
- **T5.06:** LIC-003. Notices are not generated from the resolved
  Cargo/Gradle/npm tree plus bundled binaries. QUALITY-006 has no comparison tool
  for checking reproducibility.
- **T5.08:** FORMAT-012 is missing its OS decoder routes (Android platform DNG,
  Windows WIC Raw Image Extension). FORMAT-009 opens PDF-compatible AI as a PDF
  document. Both are P2 and may be deferred.
- **T3.11, unconfirmed:** PERF-007. Confirm the client moves back to a
  higher-priority carrier that returns while a lower one is live
  (`vw-ffi/src/session/link.rs`).

Runtime admission blockers, not code: SEL-001 (empty LiteRT catalog, B2);
TUNNEL-002, TUNNEL-005, SEM-007 and CAPTURE-008 (`COMPILED_APPROVED_CATALOG = None`,
B3); FORMAT-006, QUALITY-001 and QUALITY-002 (PDFium, qpdf and LPAC runtime admission).

## Branch and worktree disposition

- **Deleted** tree-identical or superseded local branches:
  `task/source-sync-20261020` (same tree as main),
  `task/windows-s23-acceptance-20261020` (same tree as `4f3b569`, which is in main),
  `task/windows-s23-focus-acceptance-20261020` (main minus two acceptance docs).
- **Archived, then deleted:** `recovery/original-main-edit-20261020-2335` and
  its identical stash `c3a72c1` became local tags
  `archive/original-main-edit-20261020` and `archive/stash-original-main-20261020`.
  Main already contains the evolved `ProjectFilePickerRequest` form of that
  `Main.kt` draft.
- **Deleted (already in main):** `task/phase2-editor-integration` (`679dc28`).
- **Kept:** remote branches, `recovery/source-candidates-20261018` and
  `task/m4-krita-paint`. All 6 locked retired worktrees are kept. Five
  (`desktop`, `image-spike`, `ink-core`, `m4-input-retention`, `transport`) hold
  uncommitted early-phase work. `windows-s23-20261020` holds ignored local
  evidence receipts (about 1.7 GB).
