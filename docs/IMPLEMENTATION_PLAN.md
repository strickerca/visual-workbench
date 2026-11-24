# Visual Workbench — Implementation Plan v2.0

Revision 2026-10-01. Companion to BUILD_SPECIFICATION.md v2.0 and REQUIREMENTS.json (189 requirements). Every requirement maps to at least one task below (checked by `tools/check_plan_coverage.py`).

> **Owner rule (2026-11-13):** wherever this plan asks for evidence files,
> receipts, snapshots or preserved artifacts, don't produce them. Record results
> as one line in the commit or task status, and delete artifacts after use. See
> "Only do what makes the product better" in `AGENTS.md`.

## Active roadmap — 2026-11-14

Owner-selected finish line: a **personal G5 build**. Close M4-KP (Paint only),
record G0–G3 and G5, and finish with an installable MSI and APK for personal
use, without release signing (the APK keeps its existing development signer).
Krita, T4.02, T4.05, T4.06, broader editors and full G4 remain deferred (D21,
pending owner confirmation). Live provider spend is not pre-authorized. Device
time on the S23 and owner gate confirmations are the critical path.

Two tracks run together: the completion roadmap (what must be verified) and the
improvement program (what makes the product better and cheaper to verify).

### Completion roadmap

| Stage | Scope | Closes |
| --- | --- | --- |
| 0 Re-baseline | Done 2026-11-03 | Planning baseline |
| 1 Blockers | B1–B10 below | Blockers |
| 2 M4-KP | Paint catalog calibration/admission; T4.03 guards (checklist item 4); T4.04 ghost ink; T4.07 Paint compatibility record; T4.08 essential controls; T4.01 ≥30 fps / PERF-003; personal packaging; owner drawing session | M4-KP checkpoint |
| 3 G0–G2 | T0.04/05/06/10/11 carry-overs and T0.12; A01/A02/A06 plus daily use; A13/A21–A24/A26/A29 plus A06 re-run in real apps | G0, G1, G2 |
| 4 Phase 3 | T3.01–T3.11 runtime/device work and confirmed source gaps; T3.12 scenarios | G3 |
| 5 Phase 5 | Depends on M4-KP instead of T4.09 only once D21 is confirmed. T5.01–T5.05, T5.08 decisions, T5.06 MSI build/install/uninstall with admitted WiX and notices, T5.07 release-build scenarios and reports | Personal G5 |

Stage 1 blockers:

| Blocker | Status |
| --- | --- |
| B1 Windows release replays | Closed 2026-11-04 |
| B2 model graphs and LiteRT scan | Graphs reproduced byte-identical ([ASSISTED_MODEL_REPRODUCTION.md](ASSISTED_MODEL_REPRODUCTION.md)); Android runtime admission open |
| B3 WiX and scrcpy licences | Conditions met (D22); the tunnel catalog compile is T5.03 |
| B4 black canvas | Closed 2026-11-23: re-checked on the S23 with the merged code and the canvas log on: 6 rotations and 5 Home→return cycles, no black or stale frame in 35 screen samples, no `redraw_fallback`, no buffer timeout; each return presented 18–37 ms after its new surface |
| B5 normal APK delivery | Accepted on the S23 ([ANDROID_DELIVERY.md](ANDROID_DELIVERY.md)) |
| B6 visible phone→Paint run | Prepared, not run. Owner-attended, 10–15 minutes hands off, USB tethering on, Paint closed with Brush last used |
| B7 Windows owned guard | Closed: 103 cases |
| B8 packaged MCP smoke | Closed: two-cycle pass, about 6.5 s per start |
| B9 physical-gesture producer | Code merged 2026-11-13; needs an owner S Pen session (about 30 minutes). The S Pen source (0x5002) and `verifyInputEvent` signing are unconfirmed on the device |
| B10 live GPT Image | Waits for an owner spend decision (about $1–5) |
| LIC-003 generated notices | Complete (427 components, D23) |

S23 session 2026-11-23 (main a824a9f plus this task branch; isolated `.hil` package, owner user 0; effort high):
instrumented suite 228/228 after two project-screen fixes (with no projects the guide hid Pair a computer
and Diagnostics; in landscape the project list and both actions were off screen and nothing scrolled).
Injected stylus strokes (`input stylus swipe`) with every pen-path switch off: at a human pace (60–80 ms
strokes, about 150–200 ms apart) all 500 landed on fresh pages and on a 1,009-stroke page, but 382 of
1,100 were dropped while that page grew to 1,827 strokes; at a 50 ms gap 236 of 400 landed on a fresh
page, 331 of 400 with "next stroke starts while the last one saves" on. The dropped pen-downs are
fixed by the canvas rebuild below. No
"checkpoint on background" was due at 1,827 strokes (opening had already checkpointed); a project
opens in 11–77 ms. Not covered: real S Pen, latency, PC pairing (B6, B9).

Canvas rebuild (D25, owner instruction 2026-11-23): the screen works like a paint program, marks stay
objects, saving happens in the background, one path with no switches.

| Step | Status |
| --- | --- |
| 1 Ink never refused; own stroke added without a read-back | Done 2026-11-23. A pen-down for ink is no longer dropped while earlier strokes save or lists refresh; up to 63 strokes or 400,000 samples may wait (was 3 strokes); a saved stroke comes back from the core and is added to the held document, so the work no longer grows with the page; instruction and marker lists are not re-read after a stroke; Diagnostics counts refused pen-downs by reason. The "quick refresh" and "next stroke starts while the last one saves" switches and the old path are removed (six switches remain). OnePlus 8 (IN2019, Android 11, isolated `.hil` package, user 0), `PenStrokeBurstInstrumentedTest`: before 261 of 400 rapid strokes, 42 of 80 on a 1,200-stroke page and 102 of 150 saved; after 400, 80 and 150, none refused, held document equal to a read. Full suite there 228 of 232; the 4 failures are the canvas-recreation pixel checks in `EditorSurfaceInstrumentedTest` (one black sample at 0–1 ms), which fail the same way on main on that phone and pass on the S23. Independent review: one defect (controls left waiting after an announced own stroke) and two should-fix items, all fixed. Still refused right after a stroke on tone-adjusted pages, giant images and pages with AI results (the page picture is rebuilt); step 2 removes it for pages with AI results. Not run: S23, real pen. Effort: xhigh (delegated), review high |
| 2 Kept picture; a finished stroke is drawn into it once | Done 2026-11-24. The canvas keeps a graphics-processor picture of the saved page (one screen-sized buffer, about 5 MB on the S23, given back on Home, rotation, project close and any memory warning); ink frames and the frames after a save draw it and put the wet ink on top, and a finished stroke on a plain top layer is drawn into it once. Anything else (undo, edits, erase, moves, changes from the computer, page, background, layer or view changes) makes the picture again, a view change once the movement stops; giant images, paged documents, AI results and in-progress moves, erases and drafts draw the whole scene as before. The pen loupe finds the strokes near the nib through a 32×32 grid instead of every object. A shown AI result is carried across an own stroke, so ink is no longer refused on such pages; tone-adjusted pages and giant images still are. The "draw only the new stroke" switch is removed (five remote switches remain). S23 (SM-S918U, Android 17, `.hil`, user 0): frames from the picture equal a whole-scene draw after open, undo, edit, move, delete, pan, zoom, turn, recreation, rotation, Home and the memory warning; after added strokes within one step per channel except at most 2 edge pixels; no picture or buffer left after closing. Ink frame draw p50 0.2 ms and pen-up to saved stroke drawn 26 ms on a 5,000-stroke page, against 151 ms on main (200 strokes: 34 against 50 ms). `PenStrokeBurstInstrumentedTest` 3/3, full suite 236/236 before the review fix (which touched only the picture surface). Independent review: one plausible defect (a frame the platform renderer drew late could be taken as the new picture, leaving a stroke off screen until the next rebuild) fixed by closing a renderer whose frame did not come out, with a 162-render colour check; a construction leak fixed. Open: no wait for the picture to finish on Android 10–12 (no platform call before 13); the picture may go unused in landscape, which waits for the landscape fix (the editor canvas has no height in landscape on the S23); loupe cost follows the strokes under it. Not run: real pen, AI-page ink on the device, Android 10–13 phones. Effort: xhigh (delegated), review high |
| 3 Undo, erase, move and changes from the computer redraw only the touched area | Next |
| 4 One picture per layer; tiled zoom | Not started |

Open code and admission gaps:

- T5.06: QUALITY-006 has no comparison tool for checking reproducibility.
- T5.08: FORMAT-012 lacks its OS decoder routes (Android platform DNG, Windows
  WIC Raw Image Extension); FORMAT-009 opens PDF-compatible AI as a PDF. Both
  are P2 and may be deferred. INSTR-005 needs a dated owner decision.
- T3.11: PERF-007 — automatic links move back to a returning higher-priority
  carrier; shown in simulation, hardware timing unconfirmed.
- Runtime admissions, not code: SEL-001 (empty LiteRT catalog, B2);
  TUNNEL-002, TUNNEL-005, SEM-007 and CAPTURE-008 (no compiled approved
  catalog, B3); FORMAT-006, QUALITY-001 and QUALITY-002 (PDFium, qpdf and LPAC).

### Improvement program

Each stage ends with green automatic checks and one independent code review.
Effort levels are in `AGENTS.md`. Hardware-dependent changes land switched off
or behaviour-preserving until they are run on the laptop or the S23.

| Stage | Scope | Status |
| --- | --- | --- |
| Step 0 | Compile and test the code merged uncompiled (B4, B9, T5.01) | Linux, 2026-11-14: Rust lint clean, 1,993 tests pass; Kotlin compiles, shared 633 / Android 273 / desktop 692 tests pass. Fixed one call-slot ordering bug and one invalid test method. Windows-only code compiles, lints clean and passes its tests on a hosted Windows runner (5 failures were the runner's short temp path, fixed in `tools/ci/windows-rust.ps1`). Device re-checks pending. Effort: high |
| 0 Cloud build setup | Android SDK and pinned tools in cloud sessions | Done 2026-11-14: `tools/ci/install-tools.sh`; cloud environment `visual-workbench` saved (first fresh start unproven). Effort: high |
| 1 Automatic checks | `.github/workflows/ci.yml`, `tools/ci`, vulnerability scan | Done 2026-11-14: workflow and scripts in place; 13 known vulnerabilities fixed by upgrades, 3 unmaintained notices tolerated in `deny.toml`. All nine jobs green on a4fa2ab8 with the Stage 5 core; `ci-required` now also waits for the desktop app built and tested on Windows (679 tests). Effort: high |
| 2 Cleanup | Only files that help build, test or run the product remain | Done 2026-11-14: 513 receipts, handoffs, old copies and dead probes removed; secret-scan exceptions 278 → 43 lines; this plan 1,099 → about 690 lines. Old GitHub branches still to delete. Effort: high |
| 3 Proof | End-to-end loop test; requirements linked to tests; speed budgets; parser property tests; emulator pixel tests | Mostly done 2026-11-14: loop test passes on Linux (`vw-ffi/tests/golden_path.rs`); 140 of 175 active requirements linked to 297 existing tests, checked on every push; stroke-save benchmark added (2.5 ms empty, 56 ms at 250 strokes, 120 ms at 500, fails at the 769th object in a layer). Corrupted-input tests (deterministic, 6,000 to 20,000 mutations each) pass for the AVIF, HEIF, video, PDF-metadata, capture-data and project-archive readers. A stroke-save speed budget (10 ms at 2,000 strokes) runs on every push. The emulator job runs 14 on-screen tests. All 14 passed on two of three runs after Stage 7 (2026-11-15); earlier screen-pixel failures were the emulator's own launcher dialog dimming the screen, and one was a real refresh-timing defect fixed in Stage 7. Still intermittent (about 1 run in 3): after an activity recreation or a resize the canvas sometimes shows the old layout, or one black frame, for up to seconds. A delegated diagnosis (2026-11-15) matched the canvas's own lifecycle log to the platform's: the emulator froze the app's drawing for Android's fixed 4 s buffer timeout (`dequeueBuffer failed, error = -110`) about eight times in a run (about 33 s frozen in 104 s of tests); the system's own 1 s sync timeout then shows the window before the canvas has drawn, and every canvas step after the freeze ran in order and promptly. Conclusion: an emulator drawing stall, not a canvas defect (about 85%). The job now leaves two of the runner's four cores to the host's software renderer. Two canvas defects found on the way are fixed (a new drawing layer could start from the picture for the old size; rotation or a return from the background built two layers), and the canvas can write its lifecycle to the log (`VW_CANVAS_LIFE`, off unless switched on; on the S23: `adb shell setprop log.tag.VW_CANVAS_LIFE VERBOSE`). The B4 re-check on the S23 should look for the old layout or a black frame after rotating or returning to the app. 2026-11-16: the log now only observes (records go into a fixed ring that a writer thread prints while a log-on canvas is open; delegated xhigh, review high), after the log-on copy of a test twice failed on one black frame 1 ms after a recreation while the log-off copy passed. If such a frame is real it happens with the log off too (about 50%): a black frame that starts at the old surface's `release_callback` and ends when the new surface's window frame goes up, with every canvas step after it prompt, is the platform keeping the window; a canvas cause shows `redraw_fallback` or a late `presented` after `redraw_request presented=false`. That is what to read in the B4 re-check log. The other intermittent failure, a new canvas never ready in the setup wait (`new-project-and-background`, twice), was a real phone defect (delegated xhigh; review high: no blocker, three items fixed): the core replays its latest change to a new subscriber from its own thread, and when that notice, or a PC edit, landed while a plain page was decoding, the decode was dropped as stale and never retried, leaving no background and drawing refused until the project was reopened. A finished decode is now kept while the same page and revision are shown and retried once after a newer revision; the wait's timeout now names the part that was missing. Emulator since the logging fix (2026-11-16): 7 of 8 complete runs passed, the failure being that defect; the first run with the fix passed 16 of 16. The job stays informational. Effort: high |
| 4 Diagnosability | Timing ring and logging over FFI; error causes kept; plain-language error catalog | Core part done 2026-11-14: failures keep a fixed-text cause and timed stages record durations in a bounded in-memory ring (`core_diagnostics`); the apps can read it (`readCoreEvents`) and `ErrorCatalog` gives every failure a plain message and an action. Linux: core tests pass, shared 638 tests pass, both apps compile. The exported problem report can now carry those core events (schema 2) under the same closed privacy audit, and a save that takes 50 ms or more is recorded. Still open: showing it in a problem-report screen (Stage 8) and replacing the remaining discarded errors. Effort: high |
| 5 Core engine | A layer must hold far more than 768 objects (order keys run out today); edit cost independent of document size; dependency tidy | Done 2026-11-15; independent review found no violation of hash, wire, schema, revert or convergence rules and two low items (old full layers stay full; a test-only cross-check also lands in the debug `vw-pair-cli` built by `build.ps1 build-network`): new layers take 100,000+ objects; saving a stroke takes about 0.6 ms at 2,000 strokes (was 655 ms) and stays under 1.2 ms at 8,000; stroke prediction no longer copies the stroke; fixed a bug that made a project unopenable after recolouring an unfilled shape. Linux: lint clean, 2,030 Rust tests pass, simulator and crash suites pass; hash, wire, schema and bindings unchanged. Still open: the recovery checkpoint every 500 saves still writes the whole history (0.5 s at 2,000 strokes) and snapshots are never pruned (needs a format change); layers already filled to 768 by old builds need re-keying; every ordinary build of the core is now optimised (2026-11-15, `[profile.dev]` in `Cargo.toml`; at 2,000 strokes an ordinary save went from 1.9 ms to 0.8 ms and the every-500th save from 3.4 s to 0.45 s; the phone package shrank from 165 MB to 117 MB; not yet built on the laptop); dependency tidy done 2026-11-16 (delegated xhigh; review high after the merge: every point passes, checked against the fs2 and rustix sources): the unmaintained fs2 is replaced by vw-store's `os_fs` module making the same OS calls (flock and statvfs on every Unix including Android, LockFileEx and GetDiskFreeSpaceW on Windows), the empty vw-pkg crate is gone, vw-capture and mcp/native carry the workspace lints, 28 shared pins live in `[workspace.dependencies]` with no resolved version changed, and vw-avif-os is vw-codec-os's avif module; Windows locking and disk numbers still need the Windows job to run them. Effort: xhigh (delegated) Follow-up done 2026-11-15 (delegated xhigh, review high: no blocker; three should-fix and most low items fixed): a save no longer writes a recovery checkpoint and the file keeps at most two. At 8,000 strokes the slowest save went from 1.7 s to 13-26 ms (a busy disk has produced 40-110 ms), the project from 801 MB to 34.7 MB and reopening from about 3.3 s to about 1.3 s; existing projects shrink on their first open. Opening replays the edit log; replay compares revision and state hash, not notice wording, and a project a later engine decides differently is refused with a fixed cause and left untouched (pinned by a two-device script and two fixtures written by the 2026-11-13 build). A later checkpoint is written at open, while the project is left alone, or at close, only when replay would be costly. The no-follow file flag was wrong on arm64 (the phone) in five places and is fixed. Engine follow-up done 2026-11-15 (delegated; review high: no blocker; its should-fix and low items fixed): at 8,000 strokes every edit that is not a new stroke (move, restyle, delete or erase an older stroke, layer, title, instruction, mask, result) went from 125-210 ms to 7-9 ms; reopening after 200 such edits from 42.6 s to about 3 s with no checkpoint due; the phone's work per sync update from about 11 s to about 1 ms; the PC's answer to a phone's sync request from 2.9 s to 0.03 ms. Live sync no longer compares notice wording; a save that would push the history past what a new device can be given (256 MB, roughly 30,000 strokes) is refused; a failed sync update can no longer turn the phone into a host, and the phone checks its copy against the host's hash. Still open: a host past 256 MB of history becomes read-only (owner decision); editing something early in a very large document still costs about 8 ms per 20 MB re-hashed. Workflow admission follow-up done 2026-11-16 (delegated; review high: no blocker; all five items fixed, among them two places where the new charge was too small, which on the phone means running out of memory): the memory check that decides whether a tool may run charged sixteen copies of the whole history, so with real pen strokes (100 samples each) the eraser was refused from about 20 strokes and instructions, the layers panel, selections and file export from about 100; the check itself took 56 ms at 500 strokes and 220 ms at 2,000. Each tool is now charged what it really allocates, compared on every call with a counting allocator: every tool runs at 500 and 2,000 strokes on the PC and on the phone, also with 2,000 strokes queued on an offline phone (the eraser then uses 252.6 of the 256 MB allowance, so a much longer offline queue is refused); at 8,000 strokes only instructions are refused, because hashing that page really needs about 209 MB. Not measured by that test: verification compiling, AI, semantics and giant images (their own unit tests use the same charge). Effort: xhigh (delegated), review high. Correction (2026-11-16): the main session's full run found three of that test's cases over their charge (a marker beside a 2 MB colour profile 96.3 MiB used against 40.9 charged; a marker on a 500-stroke page 27.0 MB against 5.6; a phone replaying 2,000 queued strokes 199.5 MiB against 196.7) and first recorded them here as charges too small for the apps. That was wrong: they happen only in the workspace test build, where vw-ops is built with its `cross-check` feature (switched on for every crate because two crates use it in their own tests) and every edit re-hashes the whole project to check the incremental hash. The apps never build it; there the same calls use 0.1 MiB, 0.1 MiB and 97.0 MiB. Commit charges now also cover that check in that build (no change in the apps), and the test keeps the background checkpoint out of measured calls; it passes in both builds (delegated xhigh). That test took about 16 minutes in the cross-check build; it now builds the PC pages ready-made (a new test checks a built page equals a drawn one) and takes about 60 s there and 32 s in the product build, with a byte-identical product report. Review (high, 2026-11-16, after the merge): no blocker; CI ran this test only in the cross-check build, where it tolerates refusals the checks explain, and one bound (planning a marker) had been loosened in both builds. Fixed in the main session: the bound is strict again in the apps' build, unknown cross-check charges fail loudly, and `tools/ci/rust-test.sh` runs the test once more as the apps build the core. Outline transfer done 2026-11-16 (delegated xhigh; review high: no blocker, five low items fixed): after every save the apps re-read the document, and each stroke's outline crossed to the app as two boxed lists of numbers, so the read hit its 64 MB limit from about 400 strokes of 100 samples and the phone stopped refreshing. New additive calls send each outline as one compact byte buffer, decoded exactly; both apps use them and the old calls stay. Strokes a full read admits: 404 → 2,592 at 100 samples, 801 → 4,852 at 50, 135 → 899 at 300; one stroke of more than about 85,700 samples is refused, as the plain read refuses it, because the read now also charges each stroke's build peak. Live ink allocates nothing more per batch. Hash, stored data, wire format and schema unchanged. Phone project upkeep done 2026-11-16 (delegated xhigh; review high: no blocker, five of six items fixed): the phone writes the recovery checkpoint closing would write once the app has been out of sight for 2 s and the editor is idle (about 1.1 s at 2,000 strokes, 4.2 s at 8,000 on Linux; Android freezes a cached app after about 10 s), so the next open replays less; returning drops a request not yet started. Projects with more than 256 MiB of live data now compact on Android too, through a checked private copy renamed over the database (320 MiB live in 1.1 s at 13 MB peak memory on Linux); the desktop still compacts in place. Listing projects opens each one, so such a project compacts once while the list loads. S23 pending: whether the checkpoint finishes before the freeze, the reopen gain, and compaction time on the phone. Limits left (2026-11-16): the layers panel, export frames and tone controls refuse a document of more than 4,096 objects (each stroke is one), although a layer now holds 100,000+; the instruction tools hash the whole visible page on every call, which is why they are refused at about 8,000 strokes. |
| 6 Connect without typing | Pair once, then discover, recognise by certificate and connect; carrier upgrade | Code complete 2026-11-15, switched off by default (`AUTOMATIC_BY_DEFAULT`) until run on the laptop and the S23. On loopback with real encryption: a paired device reconnects with no address supplied; a device with the right name and the wrong certificate is refused; the link moves to a better carrier in under 0.1 s, returns to a replugged cable in about 0.35 s and leaves a dead route about 1.5 s after the cable is pulled, with every edit accepted exactly once. Independent review: no blocker; three should-fix items (switch-off race, revoked device's addresses kept on disk, slow return to a replugged cable) and four low items fixed. Open: the PC cannot yet receive a phone-owned project automatically; on the PC each network adapter is chosen once; Windows discovery (mDNS) and Android finding it are untested anywhere; hardware timing for PERF-007 unmeasured. Known low items left: stalled strangers can hold the two listening slots for 15 s; the pairing store is read on each route probe; a saved carrier label is trusted as-is; Forget and relabel wait for the live connection to end; no damping if a cable link keeps stalling; retries every 5 s continue in the background. Checks: Linux Rust lint clean and 2,038 tests pass; Kotlin shared 676, desktop 688, Android 270 tests pass; all nine hosted jobs green on 96eef9e9 including Rust and the desktop app on Windows. Effort: xhigh (delegated), review high |
| 7 Pen path | Remote-edit input fixes; wet ink draws only new strokes; no wait behind save; latency shown in diagnostics | Code complete 2026-11-15; nothing run on the S23, an emulator pen or Windows. Two canvas switches (quick refresh; next stroke starts while the last one saves) became the only path on 2026-11-23, and the third (draw only the new stroke) on 2026-11-24 (D25, canvas rebuild above). Originally eight developer switches, all off after install (phone: Settings → Developer options): three for the canvas (quick refresh after a save; next stroke starts while the last one saves; draw only the new stroke) and five for remote drawing (samples at the pen's rate; exact sample times; drawing through a hitch; typed calls to the core; frames shown sooner). Nine pen-path timings on the phone (typical, slow end, worst) and `tools/pen-trace/trace_tool.py timings` to compare before and after; the measuring session is in `docs/PERF_METHODS.md`. Fixed without a switch: after a save the editor could briefly report ready while still holding the old document. Independent review: no blocker; two should-fix items (extra document reads with all switches off; a second Highlighter layer with switch 2) and the low items fixed. Not built: 60 fps remote video and sample times sent to the PC (both need Windows-side changes that cannot be compiled here); low-latency ghost ink on its own surface (cannot be judged without the phone). Nothing new on the wire; database, state hash and `host-win` untouched. Checks: Linux Rust lint clean and 2,053 tests pass; Kotlin shared 678, desktop 688, Android 324 pass; on-screen tests compile; all nine hosted jobs green on af1c7138. Effort: xhigh (delegated), review high |
| 8 Product interface | Shared UI module, Material 3, icons, strings, plain copy, first-run guide | Started 2026-11-14: eight logic files that existed as identical copies in both apps (about 1,400 lines each side) now live once in `apps/shared`; Linux: shared 654, desktop 679, Android 260 tests pass. 2026-11-15: the eight screens that existed as identical copies in both apps (884 lines each side) now exist once in `apps/shared-ui`, compiled into each app with that app's own Compose (a shared source folder, not a separate module, so no dependency or lock file changes); Linux: shared 678, desktop 688, Android 324 tests pass. The problem report both apps export now carries the core's failure reasons and stage timings (fixed words and numbers only; the report's own privacy check still gates it). A core failure is now shown in both apps as what happened and what to do, made more specific by the core's own reason when it recorded one (the reason itself is never shown). Both apps' empty states now carry the same four-step guide (open, mark up, use with the other device, hand off) in each app's own words, drawn off screen in a test. All nine hosted jobs green on 2c03e700, including the first Windows compile of `apps/shared-ui`. The most technical everyday wording on the PC is rewritten (projects copied from the other device, the clipboard's second image format, publishing a package for an assistant). A second pass rewrote about thirty failure messages a person can meet in ordinary use on both apps (transfers, copy and drag, live view, export frames, AI workspace). Still open: about 140 messages for rare internal failures (start-up integrity checks, clean-up that did not finish), left technical on purpose so a problem report stays precise; one strings object; Material 3 with tokens and icons (needs lock-file changes for the Windows build and cannot be judged without screenshots), screenshot tests. Effort: high |
| 9 AI handoff | Real MCP tool descriptions and structured results; evaluation set | Partly done 2026-11-14: all eight tools carry a description written for the agent, the server sends usage guidance, and every refusal now says what to do next (retry, refresh, ask the owner, or stop); MCP tests 77/77 on Node 24.21.0. Still open: the evaluation set (live runs need the owner's spend approval). Effort: high |
| 10 Portability and installers | Paint catalog as data with re-measure; GPU capability probe; MSI and APK built automatically | Partly done 2026-11-15: the `build-apps` workflow builds both apps as downloads on request and whenever the packaging changes — the Android package (checked by `check_apk`; 50 MB download, development key) and the Windows app as a ready-to-run folder with optimised native libraries and the local agent server (135 MB download; no installer). Both jobs green on 0dbf830c. Neither package has been started on a device from this route. Still open: MSI (needs the WiX licence step), Paint catalog, GPU probe, release shrinking, a stable signing key (owner). Effort: high |

Only the owner can: merge pull requests and protect main on `ci-required`; run
the S23 sessions (latency baseline, B6, B9, pen-path re-measure,
Jetpack Ink A/B); decide live AI spend; check Paint's version and auto-update.

Kept in mind, not to be built until the owner asks (2026-11-15):
- **Editable image file.** Saving would produce one PNG that any app shows as
  the edited picture, with the original and the marks inside it, so that
  Visual Workbench can reopen it with the marks still movable; the working
  project would become invisible housekeeping and the project list "recent
  images". Ordinary single-image work only.
- **One-file project copy.** The core can already pack a project into a single
  `.vwbz` file (specification 4.5); neither app offers it. Not wired while the
  editable image file is undecided.
- **Folding old edit history into the current state** to shrink a project
  further. Not planned: the gain is small once old checkpoints are gone, and
  syncing and conflict review read that history.
- **Material 3 redesign and new icons.** Wait until the owner has used the
  current build and can look at it.

## How to run this plan

1. Put this package at `C:\dev\visual-workbench` (the repository root; any path works if you tell the agent).
2. Run tasks in the order of their dependencies. Each Phase 0 and Phase 1 task has a ready-to-paste prompt in `prompts/`, and each prompt is complete on its own.
3. One task at a time per folder. A task run in parallel needs its own `git worktree` folder, and tasks that use the phone take turns.
4. Paste one prompt into a fresh coding-agent session. The agent starts from main with validated dependencies integrated, uses its task branch, records one-line results and requirement statuses, and reports results plus the next task. Local commits and completed dependency integration are authorized without repeated Git approval.
5. Review results and complete listed owner actions. At each numbered phase conclusion the agent automatically commits and pushes validated phase-owned source under the publishing policy below. Owner confirmation of product gates and manual decisions remains separate.
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
| Phase 4 first milestone | Use the free Krita website installer and existing Microsoft Paint. Perform S23 pressure/feel and target-change tests; no Photoshop, other-editor or driver installation is required. See D20. |
| Phase 5 | Create and safely store the Android release signing key. Before any distribution: code-signing certificates (Authenticode; EV for driver attestation). |

## Repository layout

```
visual-workbench/
  README.md  CLAUDE.md  Cargo.toml  rust-toolchain.toml  deny.toml  justfile (or build.ps1)
  .gitignore  .gitattributes
  docs/        BUILD_SPECIFICATION.md DECISIONS.md REQUIREMENTS.json IMPLEMENTATION_PLAN.md
               TOOLCHAIN.md SOURCES.json compat/
  contracts/   vw_protocol.proto package.schema.json storage.sql
  prompts/     phase-0/ phase-1/ (later phases added after G1)
  tools/       diagnostics/ diag-win/ pen-trace/ pen-inject/ bench/ pair-cli/ hello/ check_plan_coverage.py
  fixtures/    images/ pdf/ svg/ malformed/ traces/ (generator scripts + hashes; big outputs in git-ignored generated/)
  core/crates/ vw-geom vw-model vw-ops vw-store vw-ink vw-raster vw-assets vw-formats
               vw-codec-os vw-pdf vw-svg vw-mask vw-package vw-instructions
               vw-semantics vw-capture vw-stream vw-remote vw-document-protocol vw-ai
               vw-ai-platform vw-ai-provider vw-proto vw-net vw-sim vw-ffi
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
- **Results:** one line per task in this plan or the commit message: what ran, pass or fail, the device and the effort level. No evidence files (`AGENTS.md`). A dependency blocks a later task only when its result is missing or failed. Screenshots come from a Per-Monitor-V2-aware capture (`tools/diag-win --screenshot`, from T0.03), stay outside Git and are disposed after inspection.
- **Definition of done for every task:**
  - Deliverables exist.
  - `test-all`, `lint-all` and `license-check` pass, and `ci-required` is green.
  - REQUIREMENTS.json status and its one-line result are updated per the status rule above.
  - The one-line result names what was not tested.
  - No secrets or identifiers anywhere.

## Automatic phase publishing

Owner instruction, 2026-10-01: the repository
[strickerca/visual-workbench](https://github.com/strickerca/visual-workbench) is
public, and completed phase work is published without asking for Git permission
again. Since 2026-11-14 publishing goes through the automatic checks.

At each phase conclusion, and for any earlier slice worth publishing:

1. Finish the work and record the result in one line in this plan or the commit
   message. Pending owner decisions stay pending; publishing does not pass a
   gate or authorize the next phase. Never loosen a performance target without
   the owner's explicit decision.
2. Check Git status, worktrees and the remote. Preserve unrelated or concurrent
   changes and resolve conflicts deliberately.
3. Stage explicit paths. Leave out build output, caches, credentials, signing
   keys, private fixtures and verification images, and inspect the staged diff.
4. Push the task or session branch and wait for `ci-required` to be green on its
   head commit. Fix failures; never bypass a failing check.
5. Merge into main. On the laptop the standing authorization covers the merge
   and a normal `git push origin main` once the checks are green. Cloud sessions
   open a pull request for the owner to merge.
6. Confirm the remote main commit, report its hash and what is still unverified,
   and delete the merged branch and its worktree.

Never force-push, rewrite shared history, discard unrelated changes or change
repository visibility. If authentication, divergence, branch protection or
another concrete blocker prevents a normal push, keep the local work and report
it. Publishing does not authorize deployments, releases, notifications or
security or device changes.

## Phase overview

| Phase | Goal | Estimate | Gate | Scenarios at gate | Re-runs at gate |
|---|---|---|---|---|---|
| 0 | Hardware truth and setup | 1–1.5 weeks | G0 | — (evidence) | — |
| 1 | Markup core: daily use | 2.5–3.5 weeks | G1 | A01, A02, A06 | — |
| 2 | Your two first wins (W1 GPT Image, W2 app screens) | 2–3 weeks | G2 | A13, A21, A22, A23, A24, A26, A29 | A06 |
| 3 | Precision, capture, sync | 3–4 weeks | G3 | A03, A04, A05, A07, A10, A11, A12, A14, A25 | A01, A02, A21 |
| 4 | Krita + Paint first milestone; expanded scope deferred (D20) | Re-estimate after dependency review | M4-KP checkpoint; full G4 pending | Focused A08/A27 checks on harness, Krita and Paint | Disconnect/reconnect safety; full A14 later |
| 5 | Verification, Android tunnel, release | 2–3 weeks | G5 | A09, A18, A19, A20, A28, A30 | A10, A15, A21, A24, plus every scenario once on the release build |

Agent effort levels are set by the table in `AGENTS.md`.

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
  - `docs/HARDWARE_TRUTH.md`.
  - `docs/PERF_METHODS.md` with a method for every PERF requirement.
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
- **Execution, 2026-10-01:** probe/recorder/replay software implemented; required S23 owner corpus and thermal/capability measurements remain pending. This task is not an accepted dependency for T0.10 and does not pass G0.
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
- **Execution checkpoint, 2026-10-02:** the host-only recorder, guarded injector, bounded HIL runner and strict analyzers are implemented. Owner-authorized native harness testing measured pressure r = 1.000 and a 26.480 ms maximum normal contact interval; combined inverted/eraser flags did not pass through. Editor and elevation acceptance remain open; this checkpoint does not close T0.05, T4.03's dependency or G0. No phone was accessed.

### T0.06 — Transport spike
- **Size:** M · **Depends on:** T0.02, T0.03 · **Prompt:** `prompts/phase-0/T0.06-transport-spike.md`
- **Informs:** D7, CONNECT-001, CONNECT-004, PERF-002, QUALITY-004 (recovery baseline)
- **Deliverables:**
  - A Rust echo/throughput benchmark (quinn QUIC with the `ring` provider, and plain TCP), as a PC binary plus an Android binary run via adb shell.
  - Runs over USB tethering (QUIC), 5 GHz Wi-Fi (QUIC), and adb reverse (TCP), with a temporary firewall rule and a firewall-block check.
  - RTT p50/p95/p99, jitter and throughput for 64 B, 4 KB and 1 MB messages; carrier recovery time after unplug/replug and a Wi-Fi drop.
  - A route-table check proving whether tethering takes over the PC's default route, with the fix and its revert.
- **Acceptance:** numbers recorded; D7 carrier order confirmed or revised in DECISIONS.md.
- **Execution checkpoint, 2026-10-02:** bounded Windows/Android transport benchmark implemented; authorized S23 adb TCP measured 3,000 verified echoes and 256 MiB at 32.41 MiB/s. Phone Wi-Fi is disabled, USB tethering is absent and physical recovery is unmeasured. D7 remains provisional; T0.06 remains incomplete and does not unblock T1.06a.

### T0.07 — Video encode/decode spike
- **Size:** M · **Depends on:** T0.02 · **Prompt:** `prompts/phase-0/T0.07-video-spike.md`
- **Informs:** D9, STREAM-002, PERF-003
- **Deliverables:**
  - A PC tool: Windows.Graphics.Capture of a test window → Quick Sync HEVC low-latency encode (Media Foundation or oneVPL), measuring capture-to-encoded latency per frame at a 1440×3088 portrait crop and at full 3840×2160.
  - A phone tool: MediaCodec low-latency decode of the recorded stream (pushed by adb), measuring decode latency.
  - A JPEG dirty-tile encode-time measurement for the D9 frame stream.
- **Acceptance:** p50/p95 encode and decode latencies recorded; D9 confirmed or revised.

- **Execution checkpoint, 2026-10-02:** T0.07 measurements completed with gaps on the PC and authorized S23. Isolated native workers and both 130-frame hardware decoder profiles passed; recorded 10%/25% dirty tiles reached 17.872/18.695 fps over adb including phone decode/posting. Coarse WGC regions, a retained Intel unloaded-library crash and unadvertised decoder low-latency controls remain limitations; no integrated stream/performance requirement passed. Continue safe preparation and independent implementation.

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

- **Execution checkpoint, 2026-10-02:** T0.08 is an incomplete software preparation checkpoint (`Status: FAIL`), with reviewed source hashes, an independent Rust probe and guarded preflight. Missing Spectre libraries, unresolved EDID output licensing, signing/install/uninstall deliverables, owner restore/security actions and live driver/fallback measurements remain open. DISPLAY-002 statuses are unchanged and D11 stays provisional.

### T0.09 — Giant image spike and fixtures
- **Size:** S · **Depends on:** T0.02 · **Prompt:** `prompts/phase-0/T0.09-image-spike.md`
- **Requirements:** FORMAT-001 · **Informs:** D15, PERF-005, PERF-006
- **Deliverables:**
  - Fixture generator scripts (fixed seeds) with SHA-256 lists and provenance notes: synthetic 16320×12240 JPEG, a 1440×20000 screenshot, multipage PDFs, SVGs with scripts and external references, malformed files; big outputs in git-ignored `fixtures/generated/`.
  - A synthetic 200 MP HEIF generated on the phone; the owner's real 200 MP photo (if provided) in git-ignored `fixtures-private/`.
  - libvips pyramid timing and memory on the laptop.
  - Android BitmapRegionDecoder timing per region (top vs bottom of a 200 MP JPEG) and HEIF decode memory on the phone.
- **Acceptance:** numbers recorded; D15 confirmed or revised.
- **Execution checkpoint, 2026-10-02:** T0.09 produces 19 hashed/provenanced synthetic fixtures and PC/S23 decoder measurements (`Status: PASS WITH GAPS`). Host fixtures reproduce byte-for-byte after correcting PDF image IDs; HEIF has 200 MP dimensions from a scaled small pattern. PC preview/pyramid and HEIF memory measurements miss or challenge existing targets; no target is relaxed. Full-size HEIF is guard-skipped and the real sanitized owner photo remains absent, so FORMAT-001 stays partial.

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
- **Execution checkpoint, 2026-10-02:** the Rust/JNI and Jetpack Ink comparison builds and all three instrumentation tests pass on IN2019. Four source-pinned Windows Google Ink attempts remain unsuccessful; the last reaches an upstream constexpr/MSVC incompatibility after 137/205 actions. Owner preferences, recorded S Pen traces, S23 presentation latency and final D6 remain pending; the spike is incomplete. Continue provisional Rust software under the owner instruction; no phase gate is closed.

### T0.11 — GPT Image round-trip spike
- **Size:** S · **Depends on:** T0.02, T0.09, owner API key · **Prompt:** `prompts/phase-0/T0.11-gpt-image-spike.md`
- **Informs:** AIEDIT-001, AIEDIT-002, AIEDIT-003
- **Deliverables:**
  - The current OpenAI image-edit endpoint, model IDs, limits (including minimum pixels and aspect range) and prices, verified on official docs.
  - A Rust CLI: image + mask + prompt → crop-and-stitch → edit request → composite → proof (changed_outside), with timings and a running cost total capped at $2.
  - Five test edits on the owner's photos: a color change, an object removal, a sign-text change, a mask touching the border, and a mask under 100×100 px.
- **Acceptance:** `changed_outside` = 0 for all edits, confirmed independently; limits and prices recorded in the task status; the API key stays in Credential Manager.
- **Execution checkpoint, 2026-10-02:** local crop/stitch/proof, request adapter and cost guards are implemented. All five synthetic mock cases independently preserve the exterior; 28 Windows Rust, 27 IN2019 native and10 Python tests pass after correcting normalized-float resizing. No live API call, credential read or spend occurred; owner-photo/model/latency/billing acceptance remains open, so the measurement spike stays incomplete. Requirement statuses are unchanged. Continue the G0 gap report and T1.05 software under owner authorization.

### T0.12 — Phase 0 gate review
- **Size:** S · **Depends on:** T0.01–T0.11 · **Prompt:** `prompts/phase-0/T0.12-gate-g0.md`
- **Requirements:** QUALITY-004
- **Deliverables:** a consolidated Phase 0 summary; a dated DECISIONS.md entry confirming or adjusting every PERF target (loosened targets need Christian's OK); checks that dated D6, D7, D9, D10, D11 and D15 entries exist; corrected statuses for every phase-0 requirement; the G0 decision; a list of carry-overs into Phase 1.
- **Acceptance:** Gate G0 recorded, confirmed by Christian.
G0 remains BLOCKED: six spikes are incomplete, physical baselines/current-main gate health and owner confirmation are absent. All twelve phase-0 requirements remain in_progress/partial and all nine PERF targets are retained unchanged. The dated PERF proposal and chronological hardware update record the carryovers. Independent software continues under the owner's full-continuation instruction; this does not conclude Phase 0 or waive T1.12's gate dependency.

**Gate G0:** hardware-truth.md complete; D6, D7, D9, D10, D11 and D15 confirmed or revised with evidence; PERF targets confirmed or adjusted (QUALITY-004); toolchain pinned; license gate working.

---

## Phase 1 — Markup core (daily use)

Goal: by the end, you screenshot something, mark it on the phone, see it live on the PC, and paste the clean or marked image into Claude Code, every day.

### T1.01 — vw-geom: coordinate spaces and transforms
- **Size:** M · **Depends on:** T0.02 · **Prompt:** `prompts/phase-1/T1.01-vw-geom.md`
- **Requirements:** CORE-002, TUNNEL-007 (math and property tests; hardware verification in T3.01)
- **Deliverables:** all spaces of BUILD_SPECIFICATION §4.2 (R, D, V, P, L, F, H, A), affine type with inverse, camera model, EXIF orientation transforms, PDF page transforms, pixel-snap helpers, property tests (round trip ≤ 1e-6 px for scales 1/64–64 and any rotation; F→H integer-exact).
- **Acceptance:** proptest suites pass with ≥ 10,000 cases each, on Windows and on the phone.
Five 10,000-case suites and eight known-answer tests pass on Windows and the physical SM-S918U; checked affine mapping measures 10.590 ns/point (95% interval 10.226–10.962 ns, <50 ns target). The Android wrapper's initial verified compiler-telemetry survivor failure is retained; corrected full run and owned cleanup pass. CORE-002 and TUNNEL-007 remain in_progress/partial for downstream app/hardware clauses; PEN-006 receives math evidence only. No numbered phase gate or phase-end push is due.

### T1.02 — vw-model and vw-ops: document, operations, transactions, undo
- **Size:** L · **Depends on:** T1.01 · **Prompt:** `prompts/phase-1/T1.02-model-ops.md`
- **Requirements:** CORE-001, CORE-005, SYNC-004, SYNC-005, SYNC-001, HOOK-001, EDIT-001, EDIT-004
- **Deliverables:**
  - Document model per §4.3 (all object kinds including adjustment; capture info; timeline reserved).
  - Ops and transactions per §4.4: host sequencing, idempotent txn IDs, LWW properties, the offline conflict rule with conflict records, fractional ordering with cycle checks.
  - Per-device undo/redo; provisional gesture state with expiry and cancel; canonical state hash.
- **Acceptance:** property tests (undo/redo restore identical hashes over 500 random sequences); duplicate transactions are idempotent; a stale offline write produces a conflict record; gesture expiry and cancel are unit-tested.
- **Execution checkpoint, 2026-10-02:** T1.02 implements the shared model, draft code generation, atomic host sequencing, offline conflict retention, optimistic replica/rebase, per-device inverse history and provisional gestures. Owner-requested parallel model/engine implementation and independent review used separate file ownership; heavy builds and S23 tests were serialized. No numbered phase gate or phase-end push is due.

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
- **Execution checkpoint, 2026-10-02:** shared SQLite storage, immutable blobs, offline queue/conflicts, sequencer checkpoints, bounded caches, schema migrations and portable archives are implemented. Independent review is clear. CORE-008, FORMAT-003, SYNC-001 and HOOK-001 remain in_progress/partial for downstream app, transport and gate clauses. No numbered phase gate or phase-end push is due.

### T1.04 — vw-ink: deterministic stroke modeling
- **Size:** M · **Depends on:** T1.01, T0.10 decision · **Prompt:** `prompts/phase-1/T1.04-vw-ink.md`
- **Requirements:** PEN-001, PEN-002, DEV-006
- **Deliverables:**
  - The stroke algorithm chosen in T0.10 (pressure curve, stabilization, pen/marker/highlighter families).
  - An incremental API for wet ink.
  - Geometry quantized to 1/256 px.
  - Golden tests from recorded traces, identical on Windows and Android builds.
- **Acceptance:** goldens are byte-identical across `x86_64-pc-windows-msvc` and `aarch64-linux-android` (Android goldens run on the device via `hil-test rust`).

- **Execution checkpoint, 2026-10-02:** T1.04 implements the provisional Rust D6 candidate under the owner's full-continuation instruction. Twenty-one engine tests and one JNI registry test pass on Windows and physical IN2019; sixteen synthetic geometry hashes match exactly, three real Android instrumentation tests pass, and CPU append p95 is 1.354 us on IN2019 (5.300 us on Windows). PEN-001, PEN-002 and DEV-006 remain in_progress/partial for app, export and physical clauses. Owner traces, S23 presentation latency and final D6 selection remain deferred; G0/G1 are open. No phase-end push is due yet.

### T1.05 — vw-raster: exports
- **Size:** M · **Depends on:** T1.02, T1.04 · **Prompt:** `prompts/phase-1/T1.05-vw-raster.md`
- **Requirements:** EXPORT-001, EXPORT-002, FORMAT-004, FORMAT-005, FORMAT-010, FORMAT-011, CORE-006, DEV-006
- **Deliverables:**
  - Clean and marked exports (full and region) at source resolution: tiny-skia rendering of all annotation kinds (built so output is identical on x86-64 and arm64), bundled fonts, deterministic text layout that the apps also use.
  - PNG/JPEG/WebP encoders with preflight; EXIF orientation and ICC handling; alpha and 16-bit PNG round trips.
  - Revision and asset-hash metadata in exports; golden tests.
- **Acceptance:** byte-exact goldens on both platforms; oversize WebP refused with alternatives; clean exports equal source pixels.

- **Execution checkpoint, 2026-10-02:** T1.05 native raster software passes 62 tests on Windows and IN2019, with nine matching PNG/layout goldens. The explicit Windows 50MP PNG16 scratch/marked-export test passes and cleans its temporary files. Strict lint/format and license gates pass; three inspected verification images were disposed. The eight task requirements remain in_progress/partial. No phase-end push is due.

### T1.06a — vw-proto and vw-net transport: protocol, carriers, simulation
- **Size:** L · **Depends on:** T1.02, T0.06 · **Prompt:** `prompts/phase-1/T1.06a-transport.md`
- **Requirements:** CONNECT-004, CONNECT-005, CORE-007, DEV-005, SYNC-004, CONNECT-001, CONNECT-002
- **Deliverables:**
  - `contracts/vw_protocol.proto` finalized; prost codegen.
  - Session layer with the six channels of §4.6; reconnect semantics.
  - QUIC carrier (quinn with the `ring` provider) and TCP-mux carrier over adb with TLS 1.3 inside (test certificates until T1.06b).
  - `vw-sim` deterministic fault simulation (drop, duplicate, reorder, partition, offline).
- **Acceptance:** 10,000 seeded simulation runs converge with zero lost or duplicated transactions; MEDIA keeps latest-frame semantics under 2× overload on both carriers on hardware; ops exchange over tethering and adb on hardware. CONNECT-001/002 stay partial until T1.10.

- **Execution checkpoint, 2026-10-02:** T1.06a protocol v1.2, TCP/QUIC, durable resync and bounded blob transport pass 74 ordinary Windows and 58 IN2019 tests. The 10,000-seed simulation converges in 222.96 seconds with no lost/duplicate edits or replayed INPUT. ADB TLS synthetic OPS exchange passes. Physical MEDIA overload/tether checks remain open.

### T1.06b — Pairing, trust store, discovery and route safety
- **Size:** M · **Depends on:** T1.06a · **Prompt:** `prompts/phase-1/T1.06b-pairing.md`
- **Requirements:** CONNECT-003, SEC-003, CONNECT-001, CONNECT-002
- **Deliverables:**
  - Device identity keys and self-signed certificates; an app-level trust store interface (Windows DPAPI implementation; Android implementation in T1.10).
  - QR pairing (HMAC over the TLS exporter); code pairing through a PAKE; single-use, expiry and lockout; revocation.
  - DNS-SD with a rotating DiscoveryId; route safety API (Windows).
  - `tools/pair-cli` (PC console plus an adb-pushed phone binary) for hardware tests before the apps exist.
- **Acceptance:** unit tests for expiry, reuse, replay, lockout, revoked refusal and TXT contents pass; hardware pairing with pair-cli over tethering succeeds and a revoked peer is refused; no key material in project files.

- **Execution checkpoint, 2026-10-02:** T1.06b QR/SPAKE2, DPAPI, lockout, revocation, discovery and route APIs are implemented. IN2019 ADB QR pairing, exact synthetic OPS acknowledgment and refusal after revocation pass with cleanup. Keystore, app flows, project/export secret scans, tethering and security acceptance remain open.

### T1.07 — vw-ffi and the Kotlin shared module
- **Size:** M · **Depends on:** T1.02, T1.03, T1.04, T1.05 · **Prompt:** `prompts/phase-1/T1.07-ffi-shared.md`
- **Requirements:** HOOK-002
- **Deliverables:**
  - UniFFI surface (async-friendly, batch calls for samples) for projects, gestures, queries, render lists, text layout and exports; session and pairing calls are added by T1.09 and T1.10.
  - Generated Kotlin bindings used by Android (JNA, arm64 `.so`, 16 KB aligned) and desktop (JNA, Windows `.dll`).
  - `apps/shared` KMP module (commonMain with no Android-only or JDK-only imports, plus an enforcing check); Gradle tasks invoking cargo-ndk and the Windows build.
- **Acceptance:** a smoke test creates a project, draws a stroke and exports from both Android and desktop through the bindings, matching the Rust golden; the alignment check passes; the commonMain import check passes and is proven to fail on a violation.

- **Execution checkpoint, 2026-10-03:** T1.07 generated/native wrappers pass on Windows and IN2019 with matching state/PNG bytes, 100,000 samples, cancellation and owned cleanup. The guarded commonMain import boundary passes its negative fixture; HOOK-002 passes that exact criterion. Native alignment is checked without claiming a 16 KB runtime device.

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
- **Execution checkpoint, 2026-10-03:** integrated app source and the isolated physical IN2019 run pass 65 total instrumentation cases shared across T1.08a/T1.08b and synthetic T1.10/T1.11 coverage. Exact APK/native binding and owned cleanup pass. S23 presentation timing, owner gestures, airplane-mode acceptance and large-image display remain open.

### T1.08b — Android app: shapes, text, selection, brush settings, layout
- **Size:** M · **Depends on:** T1.08a · **Prompt:** `prompts/phase-1/T1.08b-android-tools.md`
- **Requirements:** EDIT-001, EDIT-004, PEN-002, UX-002, UX-003, UX-004
- **Deliverables:**
  - Line, arrow, rectangle, ellipse and text tools (text laid out and outlined by the core, never by Android text); object eraser; select, move and resize with handles; color and width quick controls.
  - Brush settings: Bézier pressure-curve editor with live preview stored per brush; stabilization slider; hover width cursor.
  - Layout per §4.17: tool rail (mirrored for left-handed use, movable), status bar, context bar; dark and light themes; 48 dp targets; reduced motion; contrast check.
- **Acceptance:** phone text matches the export golden at 1:1; every tool creates editable objects; the curve editor and stabilization change replayed-fixture output; both themes pass WCAG AA contrast.
- **Execution checkpoint, 2026-10-03:** native tool/reopen/text-golden, pressure/stabilization, saved-preference, declared contrast and exercised touch-target cases pass in the same 65-case IN2019 run. Full rendered accessibility, physical hover, owner layout and cross-device acceptance remain open. Gates remain pending; software continuation is explicitly authorized.

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

- **Execution checkpoint, 2026-10-03:** T1.09 actual packaged desktop startup draws a frame, confirms PMv2 at density 1.75 / DPI 168 and exits through ordinary cleanup. The initial redirected-native-cache failure is retained; canonical-root/handle repairs pass 19 targeted cases and the final aggregate passes 96 desktop cases. Visual, window-mode, interaction and pairing acceptance remain open.

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

- **Execution checkpoint, 2026-10-03:** T1.10 native sessions, provisional gestures, recovery, durable blobs and platform trust adapters are integrated. Aggregate simulation passes all 10,000 fault seeds; the 68-case isolated IN2019 app increment includes real Keystore encryption/tamper refusal and native loopback pairing/revocation persistence. These are not live cross-device or physical reconnect/performance acceptance.

### T1.11 — Import, export and handoff basics
- **Size:** M · **Depends on:** T1.05, T1.08b, T1.09, T1.10 · **Prompt:** `prompts/phase-1/T1.11-import-export.md`
- **Requirements:** EXPORT-006, FORMAT-005, CORE-006, HOOK-001, SYNC-001, UX-005
- **Deliverables:**
  - Import on Android (share target, photo picker, camera) and on the PC (drag-drop, open, paste); blob sync to the peer, including a video asset; imports over 50 MP refused with a message.
  - Exports to the PC clipboard (PNG plus DIBV5), file drag (temporary PNG), and Android share (FileProvider).
  - Revision and resolution shown on export.
- **Acceptance:** tested into Claude desktop, ChatGPT desktop, a browser chat and Claude Code (A13, partial; Christian runs the Claude Code paste himself); a `.mp4` asset syncs with an identical hash; in airplane mode the phone exports a marked PNG; SYNCED, SYNCING, RECONNECTING, OFFLINE (n pending) and EXPORT READY each observed on both devices.

- **Execution checkpoint, 2026-10-03:** T1.11 import/export, clipboard formats, drag and Android share/camera/provider paths are integrated and exercised by native/JVM/isolated-app fixtures. Real recipient apps, owner clipboard workflows, airplane mode and full status observations remain pending.

### T1.12 — Phase 1 acceptance run
- **Size:** S · **Depends on:** T0.12 and every Phase 1 task · **Prompt:** `prompts/phase-1/T1.12-phase1-acceptance.md`
- **Requirements:** CORE-001, CORE-002, CORE-003, CORE-005, PEN-001, PEN-003
- **Deliverables:** scripted A01, A02 and A06 runs with evidence; PERF-001/002 numbers; corrected statuses for every phase ≤ 1 requirement; a list of gaps; an installable APK and desktop app image; `docs/QUICKSTART.md`.
- **Acceptance:** Gate G1 — recorded as pending until Christian confirms daily use after a day with the build.
- **Execution checkpoint, 2026-10-03:** T1.12 closes the current software checkpoint: aggregate tests pass 458 ordinary Rust, two release simulator and 99 JUnit cases; all 10,000 simulation seeds pass; aggregate lint/license checks pass after retained repairs. The final 68-case IN2019 app suite passes with cleanup, the normal APK is signature/hash verified and installed/foregrounded, and the packaged desktop draws and shuts down cleanly. G0/G1, physical/manual scenarios and S23 performance remain pending.

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


- **Phase 2 software checkpoint, 2026-10-03:** T2.01–T2.11 source is integrated. Aggregate validation passes 776 Rust and 339 JVM cases plus all 10,000 simulation seeds; the separate 50 MP PNG16 export lane and 154/154 isolated S23 cases pass. Final lint/licenses, exact-hash normal S23 delivery and fresh packaged MCP startup/shutdown pass. CAPTURE-001 paired capture now appends to the retained journal and existing OPS/BLOB link; the earlier replacement defect and validation failures remain documented. G0/G1/G2 and all 46 Phase 2 requirement acceptance statuses remain partial/pending; real apps, providers, physical inputs and owner scenarios were not substituted by software fixtures. Automatically publish this validated software checkpoint under the owner's Git policy. Preserve broader Phase 3 candidates and deferred Phase 4 scope; this checkpoint does not start Phase 5 or pass M4-KP.

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

## Phase 4 — First milestone: remote pen editing in Krita and Paint

**Owner-approved scope, 2026-10-03 (D20):** deliver one usable Windows/Android
remote-edit product with two free editor targets: Krita and Microsoft Paint.
Krita is the primary pressure-sensitive drawing target; Paint supports basic
remote drawing and shortcuts, with its installed-version pressure behavior
measured and documented. Use an existing physical Windows display and one
validated USB carrier first. No paid editor, AI API, cloud service, custom editor
plugin or virtual-display driver is needed for this milestone.

**Execution priority:** review the necessary capture, physical-pixel/DPI mapping,
paired transport, frame identity, input ownership and lifecycle dependencies;
then T4.01 -> T4.03 -> T4.04 -> the Krita/Paint subset of T4.07 -> the essential
controls subset of T4.08 -> milestone checkpoint M4-KP. T4.07 and T4.08 may be
implemented independently after guarded input is integrated; serialize heavy
builds and device/editor input tests.

The original full-task dependencies below remain the dependency graph for G4.
For M4-KP, validate the exact shared components it uses. The owner's existing software-progression exception
allows work while hardware-only evidence is pending; it does not waive failed
required software or hardware checks for declaring the milestone usable.
Unrelated PDF/SVG, giant-image, additional-format or paid-provider work is not a
new M4-KP prerequisite. Preserve existing earlier-phase work and open gates.

### Milestone acceptance and delivery (M4-KP)

1. Installable Android APK and Windows app image, artifact hashes, third-party
   notices and a short setup guide. Open Krita or Paint, connect the S23, select
   the target window, explicitly grant control, draw, erase, navigate, undo/redo
   and save through the editor. Confirm a practical owner drawing session.
2. T4.01 keeps the existing HEVC target of at least 30 fps and PERF-003's USB
   pen-to-host-to-phone latency of at most 80 ms p95. Measure on the S23; record
   actual carrier/codec/version and retained failures. No silent fallback or
   performance relaxation; ghost ink must also meet PERF-001.
3. Harness pressure correlation at least 0.95; visibly pressure-sensitive Krita
   strokes using Windows 8+ Pointer Input. Record hover, conditional tilt, eraser,
   barrel button and shortcut results per tool/version. Paint pressure remains a
   measured compatibility result; basic Paint drawing must work without it.
4. Zero stray events in 100 target-change trials, no freeze on pen contact, and
   visible grant/pause state on both devices. Check expiry/revoke, move, resize,
   minimize, focus switch, higher-integrity refusal, phone backgrounding and link
   loss. Disconnect/reconnect invalidates the input session, releases held input
   and delivers zero stale events; control requires a fresh valid grant.
5. Local ghost ink fades using acknowledged input sequences and expires within
   500 ms. Verify focused A08/A27 behavior in the harness and Krita, plus Paint
   drawing/control. Essential palettes name the destination and offer undo/redo,
   brush/eraser, brush size and canvas navigation with tested editor shortcuts.
6. Record affected software/lint/license checks, hardware measurements, editor
   compatibility and remaining gaps as one-line results in this plan. Keep
   verification images outside Git and dispose of them after inspection. A
   compiling APK or a passing harness alone does not pass M4-KP.

M4-KP is a focused usable-build checkpoint, not full G4 or G5 acceptance. The
full carrier-switch A14 re-run, A15/A16 Photoshop, A17 virtual monitor and broader
editor acceptance remain deferred. Do not report those scenarios as passed or
use M4-KP to satisfy the original T4.09 dependency of Phase 5 automatically.
Minimal milestone packaging is independent of the full T5.06 release bundle;
public distribution signing/certificates remain separate, with no purchase or
security-setting change required for this personal milestone.

### Retained tasks and deferred scope

**Do not automatically start deferred work after M4-KP.** Report the usable build
and remaining gaps; additional Phase 4 scope resumes only on a later owner
instruction. The same deferral applies to repeat editor installations or spikes
in earlier tasks when they exist solely to support deferred Phase 4 features.

| Task | Size | Depends on | Requirements | Deliverables and acceptance |
|---|---|---|---|---|
| T4.01 HEVC streaming pipeline (active) | L | T3.12, T0.07 | STREAM-002, PERF-003 | WGC -> Quick Sync HEVC low-latency -> MEDIA -> MediaCodec low-latency -> SurfaceView; at least 30 fps; frames report last_input_seq_applied; PERF-003 measured on the S23. |
| T4.02 Lossless text tiles (deferred) | M | T4.01 | STREAM-003 | Retained for a later owner-selected milestone: lossless refinement after 500 ms idle; decoded static regions hash-equal to source. Reconsider if preview text/fine-detail quality needs improvement. |
| T4.03 Injection, guards, control grant (active) | L | T4.01, T0.05 | TUNNEL-004, TUNNEL-006, TUNNEL-008, SEC-002, CAPTURE-005 | Pen/mouse/keyboard injection with conditional tilt conversion; per-batch guards; control grant and lifecycle cleanup; 0 stray events in 100 trials; no stale input after disconnect/reconnect. Full carrier-switch coverage remains required for full G4. |
| T4.04 Ghost ink (active) | M | T4.03 | GHOST-001, GHOST-002 | Local echo overlay with input-sequence fade rules; focused A27 in harness/Krita, and Paint behavior recorded. |
| T4.05 Virtual monitor integration (deferred) | L | T0.08, T4.01 | DISPLAY-001, DISPLAY-002, DISPLAY-003, DISPLAY-004 | Retain signed driver install/uninstall, display modes, negative-origin/rotated mapping, watchdog recovery and A17 for a later milestone. Existing physical-display capture remains active. |
| T4.06 Photoshop: drawing, dual view, UXP bridge (deferred) | L | T4.03, T4.05, owner installs Photoshop | ADOBE-001, ADOBE-005, ADOBE-006, ADOBE-007 | Photoshop drawing, New Window on a virtual display, UXP pixel/selection/layer commands and A15/A16 are deferred. No paid Adobe installation or subscription is part of M4-KP. |
| T4.07 Krita and Paint compatibility; other editors and bridge deferred | M | T4.03 | EDITOR-001, EDITOR-002, EDITOR-003, EDITOR-004, ADOBE-002, ADOBE-003, ADOBE-004, ADOBE-008, LIC-004 | Active: Krita remote pen using Windows 8+ Pointer Input, Paint drawing/control, per-version/tool compatibility records and honest layered/multiview limits. Deferred: Krita pixel/layer GPL plugin, GIMP, Affinity, Photopea/other web editors, Illustrator and plugin packaging. EDITOR-001 stays partial if only its pen clause is verified. |
| T4.08 Essential Krita/Paint controls; broader palettes deferred | S | T4.03 | EDITOR-005, TUNNEL-009 | Active: visible command destination, correct undo/redo, brush/eraser, brush size and canvas navigation for Krita/Paint. Other editor palettes and their acceptance are deferred; preserve full requirement clauses and report partial coverage. |
| T4.09 Full Phase 4 acceptance (deferred) | S | T4.01–T4.08 | (verification) | Original full G4 remains pending: A08, A15, A16, A17, A27 and A14 re-run. Deliver and assess M4-KP separately using the focused checklist above; do not erase or relabel full-gate acceptance. |

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
