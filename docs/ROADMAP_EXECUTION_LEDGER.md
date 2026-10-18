# Roadmap execution ledger — 2026-10-18

Baseline: `67b0af376612f999a0ea2543ad99a98fddd69b22`.
Recovery source: `0d91b70558a995fa7654021de7465b1d3f615e3d`,
`recovery/source-candidates/MANIFEST.json` (796 source files).
The archive is preserved outside the active build roots. Its publication and
syntax checks do not establish compilation, integration, or product acceptance.

The owner's roadmap continuation supersedes the historical pause and M4-only
scope. Latest steering defers Krita implementation/reactivation; preserve its
existing source. Paint and independent non-Krita roadmap work proceed. The owner
reports an S23 connected for separately coordinated Windows/device validation.
Original requirement clauses, performance targets and physical gates
remain in force. This ledger records software disposition separately from gates;
it does not change `REQUIREMENTS.json` or approve a product gate.

| Order | Planned work | Actual baseline and candidate disposition | Required validation / remaining acceptance |
|---|---|---|---|
| 0 | Reproducible cloud build | Cloud setup has a separate owner. Shared Gradle hard-codes Windows DLLs, generator executable and alignment tooling. Minimal portability changes belong to integration. | Use the setup owner's tested script. Preserve Windows checks and dependency/license pins. Serialize heavy builds. Linux results cannot validate Windows capture/input. |
| 1 | Phase 0/1/2 foundations and acceptance | Implemented core, storage, raster, pairing, markup, masks, explicit OpenAI request flow, capture/semantics, packages and MCP. T2.11 records historical 776 native, 339 JVM and 154 isolated S23 cases plus delivery checks. | These are source-bound historical receipts. G0/G1/G2 and real/manual scenarios remain open. No live provider call or hardware action is authorized by this cloud slice. |
| 2a | T3.01/02 picker and frame stream | M4 capture/HEVC code exists. Local picker repair is owned separately. Recovery `transport/t3-stream-finish` supersedes the earlier stream sketch. | Reconcile the repair with current Main, then promote deliberate stream hunks after format/color admission. Test grant, freeze, frame identity, stale revision and retirement. A07, physical DPI mapping and frame-rate targets remain open. |
| 2b | T3.03/04 loupe and precision lens | Geometry has a precision-lens foundation; source inspection does not establish complete Android/desktop loupe controls or A03–A05. | Implement/adopt platform interaction and rendering with identical document geometry, latch/restore and cancellation tests. Physical feel remains separate. |
| 2c | T3.05/06 selections, tilt and assisted selection | Rectangle/lasso/mask behavior exists. Recovery has selection and tilt R2 compositions. No complete assisted-selection implementation was found in active source. | Integrate bounded selection/model changes and conditional tilt conversion before app wiring. Assisted selection needs an approved model/license and correction/fallback tests; no model is selected by this ledger. |
| 2d | T3.07 document controls | Recovery has controls R2, local views R2, rotation, image output R2 and output UI R3. Current controllers remain authoritative. | Apply compatible hunks after model/DTO review; verify undo/revision, original preservation, export, bookmarks and 20-document behavior. |
| 2e | T3.08/10 giant images and formats | `vw-assets` is a stub; `vw-formats` and `vw-avif-os` are absent. Prefer foundation R2, Windows producer R3, Android intake/viewport R2, PNG export R2 and ordinary-format composition. Color changes overlap raster and stream code. | Image lane reviews its three crates/fixtures; integration owns store, raster, FFI and apps. Test bounds, source hashes, cache leases, stale viewport, original-byte preservation, BMP/TIFF and AVIF refusal. Native decoder, GPU, A10/A11 and PERF-005/006/008 measurements remain open. |
| 2f | T3.09 PDF/SVG/redaction | `vw-pdf` and `vw-svg` are stubs. Prefer SVG R3, protocol/raster-redaction/Windows-worker R2 and restricted PC redaction. | Document lane reviews PDF/SVG crates/fixtures. Integration owns protocol, worker registration and apps. Validate malformed input, deny-all SVG resources, bounded framing, password privacy, cancellation and verifiable redaction refusal. PDFium/qpdf runtime and A12 remain open. |
| 2g | T3.11/12 sync and Phase 3 acceptance | Existing transport/reconnect code is partial evidence; no integrated T3.11 or G3 acceptance was found. | Implement scale rebase/conflict review, carrier switching and phone-only AI sends after relevant contracts. A14/A25 and complete G3 reruns remain open. |
| 3 | Phase 4 Paint and deferred scope | Main contains R164/R166/R167 source with historical streaming, input, controls and packaging receipts. Normal transfer/rendering, Paint palette/Zoom/Undo eligibility and shutdown remain open. Krita work is deferred; preserve its source and acceptance gaps. | Integrate the separately owned picker repair without replacing current Main with the R169 diagnostic overlay. Preserve original receipts. Five scoped build tasks remain per checkpoint, with Krita clauses deferred; physical S Pen, 30 fps, 80 ms p95, A27 and owner drawing acceptance are unpassed. Full G4 includes retained deferred tasks. |
| 4 | Phase 5 verification/providers/diagnostics/tunnel/release | No complete verification loop, Gemini/FLUX image adapters, Android tunnel, Air Actions or release acceptance is established. Nine R169 diagnostic source files are preserved; their 20 written tests were unrun at preservation. | Finish preceding software dependencies, then offline provider/verification/diagnostic contracts and tests. Explicit Send, budget and credential boundaries persist. Signing, live providers, drivers, physical tests and owner gate decisions need their actual external authority. G5 remains open. |

## Current ownership and integration order

- Integration: root Cargo manifests/lock, protocol/schema, store/raster shared
  changes, FFI registration, Kotlin shared APIs and generated bindings, Android
  manifests, both EditorControllers, Desktop Main, WorkbenchApp and central
  evidence. Shared contracts are in `RECOVERY_INTEGRATION_CONTRACTS.md`.
- Document lane `01a14eeb-5d7a-72a8-85ed-f02d687f001c`: `vw-pdf`, `vw-svg`
  and dedicated fixtures. Image lane `01a14eeb-8086-76af-9670-dd5ffa5eb032`:
  `vw-assets`, `vw-formats`, `vw-avif-os` and dedicated fixtures.
- Setup lane `01a14eda-0d89-721c-8a1b-4f6a5aa5d014`: `tools/cloud-setup`.
  Local owner retains dirty picker and private worktree backups. Integration
  uses an isolated branch; main publication is coordinated through the parent.
- Establish contracts, review dependency/licenses, integrate native foundations,
  run phase-level tests, then wire FFI/shared APIs and apps. Generate bindings once
  for the integrated phase boundary, and repeat only after relevant API changes.
- Commit and push coherent validated slices. Merge only reviewed candidates with
  meaningful checks. Preserve failed receipts and exact runtime gaps. Dispose old
  worktrees only after their changes have an explicit integrated, superseded or
  retained disposition and their backups are verified. No local backup cleanup
  is performed by the cloud integration lane.

## Verification recorded by this ledger

Initial inspection only: exact baseline/ref checks, clean branch creation,
manifest parsing, active-source reads and build-script inspection. No native,
Gradle, emulator, Windows, provider or physical-device test has run in this lane.
Historical evidence is linked by task above and remains bound to its original
source/artifacts/device. Later integration receipts must state their own source,
commands, counts and limitations.
