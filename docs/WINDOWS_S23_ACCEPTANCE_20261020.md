# Windows and S23 bounded software checkpoint — 2026-10-20

**Release ready: false.** Current software tests, fresh Desktop build and packaged startup smoke passed. The owner-approved MCP cache action was aborted on a path-identity mismatch before mutation; packaged MCP and product acceptance remain open.

Local source `fff9212e1bf2b98907540efb0655e7cc290543d2` and published task checkpoint `4f3b569f584e2e65d2f6ef5d78ef866a6340d317` share tree `a49b3accfb333d351c3097a3a28248505e4aea5d`. The recorded branch is `task/windows-s23-acceptance-20261020`. The companion JSON contains complete SHA256 pins for every proof below; proof names are basenames, with no private paths, device/window identifiers, raw output or media.

## Current software verification

| Scope | Recorded result | Proof ID |
| --- | --- | --- |
| Isolated S23 instrumentation | 225/225 cases passed, zero skipped; 1,250 source inputs; 45 owned processes all exited zero with cleanup true | `s23` |
| Managed Windows execution | 1,502 tests across 200 fresh suites: shared 632, Desktop 638, Android JVM 232; zero failures, errors or skips | `managed` |
| Windows native core/integration/host | 479 unit passes, exact intentional quarantine probe accounted by its passing isolated wrapper; 155 integration passes across 23 nonempty targets; 89 host passes | `native` |
| Native policy checks | Clippy with warnings denied and format passed; all eight phases prove Job/EOF cleanup; caller budgets, assertions and 10,000,000,000-byte reserve retained | `native` |

The S23 run cleaned its owned packages and private work, left no pending installer, and preserved the normal app and other profiles. It tested an isolated application under owner user 0; it does not establish normal-app, physical S Pen, latency or 16 KiB runtime acceptance. The current managed census includes MCP capacity 8, startup 15, runtime-smoke 6 and progress 4 cases with zero skips.

## Generated Windows GPU smoke

All five isolated smoke workers completed with exit zero: portrait 1440×3088 and 4K 3840×2160 hardware HEVC, plus raster 10%, raster 25% and 4K raster 25%. Each HEVC phase measured 12 frames after 10 warmups, used GPU input with zero CPU pixel copies for HEVC, and reported no software fallback. Raster phases measured five frames each; 4K PNG/QOI lossless roundtrips verified.

Finite inspection covers 13 build-input hashes, seven process receipts and all five generated media files; the preview was visually inspected. All five media files were subsequently disposed, with none retained for phone work. Proof `gpu` pins the finite inspection SHA256 `a0380eddd0528f469e94331e9ab938898eb1217fe7c87f2d7728a1eab4ac16f2`. Generated content contains no user content. This is an isolated smoke result; full product streaming, sustained frame rate, physical/end-to-end latency and a phone decoder benchmark remain unpassed.

## Task output preservation and storage

Three compression proofs retain exact pre/post logical paths, lengths and hashes: 2,688 native dependency files, 18,013 managed build files and 3,516 additional native build files. No files were deleted; source, archives, persistent cache and system compression policy were not changed. The native dependency observation predates the final source checkpoint and is not a new runtime test. Recorded net free-space increases of 78,327,808, 525,709,312 and 283,770,880 bytes are point observations influenced by other storage activity, rather than isolated allocation savings. The reserve remains unchanged.

Original WIP and original worktrees remain preserved. The original Desktop edit backup is `local-desktop-Main-20261020.patch`, SHA256 `117b7cb1bce0770b9d0ab1a9133c1946257778c3fda29a62bbbd7e4d07e5f345`; this checkpoint does not import or modify that WIP.

## Open runtime, authority and product gates

- **Fresh Desktop distribution and packaged startup smoke: passed.** Current build took 454.616s, with 1,439 source inputs and 205 image files totaling 212,902,698 bytes. Packaged `--startup-smoke` passed in 27.982s: 4,895ms main-to-drawn frame, DPI168/density1.75, process/window PMv2 and owned cleanup confirmed. Proof `desktop` pins the image and smoke receipts. This does not establish normal-session or paired acceptance.
- **Packaged MCP two-cycle acceptance: not executed.** Owner approval covered the same old task-generated A's temporary retirement, current-key reconciliation, smoke and restoration, with abort on use/mismatch and no deletion. The read-leased preflight failed `canonical_path_changed` at `pin_cache_ancestors`; cleanup completed before any owner lock, manifest/payload read, intent or rename. Proof `mcp_abort` pins this final disposition. The cache still has eight entries: A remains in its original place, its archive is absent, and current C and its archive are absent. No move or restoration is claimed. The current built C manifest has 1,514 files/119,499,486 payload bytes. The unchanged conservative any-node refusal is a secondary blocker with 49 observed node processes. No alias bypass, guard relaxation or process termination occurred. Failed, partial or unbound new bundles cannot be automatically retired under the unchanged guards.
- **Windows owned guard: not passed.** Initial readiness failed before its case census because remote edit control was not granted. Owned cleanup completed; no case acceptance or retry is claimed. This is not evidence of an ordinary production catalog-forwarding defect.
- **Paint acceptance: not passed.** Fresh measured catalog and normal paired Paint journey acceptance remain separate. Krita is deferred scope and has not been reactivated.
- **Normal Android app: unchanged.** Its current delivery entrypoint and admitted R165 inputs are absent. Existing HIL helpers refuse the production package; that source-route limitation is not a new permission denial or normal-app delivery result.
- **Physical/runtime gates: not passed.** Physical S Pen gestures, owner drawing acceptance, target latency, 16 KiB runtime and full streaming acceptance remain separate.
- **MSI admission: not passed.** Cached WiX is an observation-only candidate. Tool/license authority and actual compatibility, MSI construction/install/uninstall/runtime evidence remain unestablished; no new explicit legal terms were accepted.
- **Other runtime admission: open.** Model, decoder and tunnel runtime/license admission remain unestablished. Previously blocked403 artifact access was not bypassed.
- **Main promotion: pending approval.** Recorded `main` remains `106884244ebdaee57d47da6b93fe18eb4b5afb74`. Automatic approval review rejected the earlier promotion before execution because authorization did not clearly cover publishing the unfinalized acceptance checkpoint to main. No retry or alternate route is claimed. Task-branch publication is a separate recorded result.

The companion JSON identifies the exact source receipts and retained blockers. A later evidence-only commit retains the tested fff/4f source binding above.
