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
| 2b | T3.03/04 loupe and precision lens | Geometry has a precision-lens foundation. Separately inspect precision R2 at recovery commit `473395197f6437586a8b0c05fa8cd35aa6800441`; it is not part of the original 796-file admission. Its literal app patch targets older composed inputs. | After current module tests, assign bounded precision leaf modules/tests; integration keeps shared API, FFI registration and controller/app composition. Rebase against current selection/eraser/AI/focus behavior. Its 34 written tests, A03–A05 and physical feel remain unpassed. |
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

## Integration progress after initial inspection

Portable document/image candidates and their exact pinned graphs are on the
integration branch, pending the owners' serialized phase tests and review fixes.
No active-source acceptance is inferred from archive publication. Admission and
portability checks are recorded separately in
`evidence/CLOUD_RECOVERY_ADMISSION_20261018.json` and
`evidence/CLOUD_PORTABILITY_20261018.json`.

Combined candidate `6dcdcdf` compiled the real Linux host libraries/generator,
Android arm64 core and current shared/Android/Desktop app sources. It passed 20
document-protocol tests, warnings-denied protocol Clippy and 53 focused JVM cases
(22 shared, 18 Desktop, 13 Android writer), with zero failures/skips. Android lint
completed with zero errors, 53 warnings and 12 notes. Product-core 16 KiB LOAD
alignment passed on all four segments. License census remained 262 dependencies
across 12 runtime configurations. See
`evidence/CLOUD_COMBINED_VALIDATION_20261018.json` for exact commands, artifacts,
fresh JUnit receipts and limits. Windows helper packaging, app launch and actual
chooser/provider/device behavior were not exercised by this Linux host run.

Desktop diagnostics callers and the Android cancellation-settlement repair are
integrated. Common ordinary-format and document DTO/frame/ownership contracts
are admitted and tested; exclusive next caller slices and root prerequisites are
fixed in `RECOVERY_CALLER_ASSIGNMENTS_20261018.md`. Recovered document/image
engines still need native registration, storage/facade and app adoption before
their import/render/export workflows become usable. Precision `1d0f2fd` remains
staged for the next integration batch; its module review does not pass app or
physical acceptance.

The additional precision R2 commit was fetched separately; existing lane bases
remain unchanged. Proposed next lane owns only `vw-ffi/src/precision.rs`, its
dedicated test, Android/desktop `precision/` leaf modules and their dedicated
tests. Integration admits shared precision APIs and composes the eight reserved
registration/controller/renderer/app targets. Do not apply `integration.patch`
wholesale: its frozen inputs are based on `0930c7b` and four older prerequisites.
Native geometry and phase JVM checks precede device or full-window acceptance.
Every new worker shell must source
`/workspace/.vw-tools/cloud-setup/start.sh`; the tested setup is `a79728a`, and
the inherited tools/caches do not require another installation.

## Next ordinary-format caller source checkpoint

Main is published at `7be9da7372f686d3d864ed0817fdcdb0833be138` with the
preceding combined validation scope. Image caller leaves `dafb622` are now
composed with root native registration, preparation, bounded format dispatch,
shared enum/error/facade mappings and both app import/export/notice seams on the
integration branch. The missing-codec distinction survives to user messages;
TIFF page notices check the current full revision and attachment/refresh owner.
File export hashes completed seekable bytes before the existing atomic publish.

This is an uncompiled source checkpoint, pending the image owner's focused
native tests and review, then one combined binding/app build. Only Rustfmt,
locked offline metadata, diff whitespace and Kotlin syntax checks ran in this
seam phase; the documents lane owns the heavy build slot. Exact source checks
and remaining gates are in `evidence/CLOUD_IMAGE_CALLER_SEAMS_20261018.json`.
Precision adapters `260b4f5` are recorded for the following reserved-hook batch;
their earlier module tests do not cover those new leaves.

## Document/precision native and shared seam source checkpoint

Document platform source `4c130504169f516d6825cdf1f5a09a690c5463d8` and precision
source `260b4f5e3fc47772cfa28f012c94ed1f29d48cf4` are composed on the integration
branch. Root native registration, document storage/page/overlay/copy facade,
request/reply transport, shared injected interaction and isolated Android service
registration now have concrete source implementations. Windows runtime/worker
source is admitted without packaged native artifacts; SVG no longer requires a
PDFium artifact. The new exact-runtime retirement retry must be called by the
document owner's retained Desktop worker before it can release quarantine.

This source batch is not compiled or promoted to main. It passed Rustfmt,
locked offline metadata, Cargo source/license/bans policy, Kotlin syntax,
manifest XML and whitespace checks. Newly written root interaction/storage/job
cases remain unrun. Exact provenance, corrections and execution limits are in
`evidence/CLOUD_DOCUMENT_NATIVE_SOURCE_20261018.json`. Android hard memory policy
needs actual isolated ART process verification; compilation cannot establish it.

Next root composition retains the app-owned document transport and draw leases,
then connects page/import/export UI and precision input/render/control hooks.
Those actual app flows are still open. Heavy validation remains serialized by
the parent. Image error fix `bf0d5f4` was independently published directly atop
`5cecb83`, allowing its focused run without this wider uncompiled source batch.
Image source repair `b0fee0a` adds embedded TIFF provenance. CORE-006 requires
revision and source identity in the exported file itself; BMP has no admitted
carrier. Root `2fc64bc` therefore gates BMP file preflight and both export UIs,
while retaining BMP import. Ordinary-format delivery remains partial. The image
owner now holds the focused heavy slot. Documents' last completed pure harness
passed 47 cases; its final three privacy/handoff cases are still uncompiled.
No existing hardware or runtime gate is passed by these source checkpoints.

## Verification recorded by this ledger

The initial ledger recorded only exact baseline/ref checks, clean branch creation,
manifest parsing, active-source reads and build-script inspection. No native,
Gradle, emulator, Windows, provider or physical-device test had run at that point.
Historical evidence is linked by task above and remains bound to its original
source/artifacts/device. Later integration receipts must state their own source,
commands, counts and limitations.


## Actual app caller source freeze

The next integration checkpoint wires PDF/SVG intake and private passwords,
selected page reopening, page/text/region controls, viewport tile adoption,
independent renderer leases, joined cancellation/project replacement/close, and
precision input/prediction/hover/render/control paths in both current apps. Source
and failure ownership remain explicit; this does not pass runtime or physical
acceptance. Shared production state/ownership checks pass31 cases; dependency
licenses pass264 entries. Exact source scope and limits are recorded in
`evidence/CLOUD_REAL_APP_CALLERS_20261018.json`.

Document file publication is intentionally disabled: copy requests still lack
CORE-006 revision/source context and embedded final-file provenance. This is an
independent protocol/engine integration prerequisite, not a successful export
feature. Partial-opacity overlays also remain Unsupported. Native artifact
packaging, regenerated ABI/app compilation, repaired image and focused precision
checks, actual GUI/provider/GPU behavior and hardware gates remain open. Main is
still the previously validated `7be9da7` checkpoint; Krita remains deferred.


### Caller review repairs — 2026-10-18

Frozen candidate `070758a` remains unchanged while independent validation runs.
Separate branch `cloud/document-copy-identity-20261018` adds the source-bound
CORE-006 copy DTO/handoff (`5d0e39a`), then page/eraser and fidelity notice repairs
(`968ffa0`). Both actual app page callbacks join eraser/selection owners before
changing pages and rebind them afterward. Page identity now fences shared
commands, gestures and picker tickets; native fresh PDF edits/erasures require
page membership atomically. Source-fenced admitted notice inspection runs after
runtime installation on import/reopen and retains valid warnings across same-source
edits. The copy capability remains disabled pending final-byte exporter proof.

Affected bounded checks: 70 Kotlin cases passed (including actual app eraser
adapters with injected hit input), 24 protocol cases passed, 21 Kotlin source files
syntax-parsed. Native two-page admission regression is written but unrun. The next
combined phase must regenerate bindings once for `EditPrecondition.page_index` and
`VectorEraseOptions.page_index`, then validate native APIs and actual apps. This
source batch does not pass device, UI, sandbox runtime or hardware acceptance.
See `docs/evidence/CLOUD_PAGE_AND_NOTICE_REPAIRS_20261018.json`.

### Combined caller/provenance candidate — 2026-10-18

The separate identity branch now includes document provenance leaves `cf01b29`
and the reviewed fixed-width xref repair `b16884a`, plus the two reproduced
precision hook corrections. A local page-view generation and explicit page checks
now fence both root refresh paths before snapshot/tool adoption, including after
post-read suspensions. Native page admission has a positive current-page erase
control. Android ordered refresh invalidation preserves its event sequence.

Portable SVG carrier/fidelity tests and the actual Android engine adapter's SVG
copy/host typed-adoption test pass. Only SVG publication is admitted by the common
controller, with an 8 MiB output reservation in both the pipe and parent budget.
Exact-original PDF is explicitly unsupported; other PDF copy controls remain
unavailable pending verified PDFium/qpdf final-file runtime checks. Native redaction
verification, opacity refusal and output caps remain in force. Runtime packaging,
OS isolation and actual device/provider publication are still separate gates.

Bounded Kotlin checks on this batch pass 77 cases. The composed native batch before
the final xref leaf repair passes 248 cases; portable document leaves pass 131 with
2 required-runtime contracts ignored. The final source is now entering serialized
native lint/rebuild, one phase binding-generation pass and actual app validation;
no final combined acceptance is claimed yet.

### Completed cloud composition checks — 2026-10-18

Code candidate `b0ace63b075fee190515f8e0c45fb6062f3809d0` is published on
`cloud/document-copy-identity-20261018`. Frozen `070758a` and main `7be9da7`
remain unchanged. The combined software check phase is complete:

- 443 native cases passed; two required PDFium runtime contracts were ignored.
- Actual shared, desktop and Android app compilation passed. The full host/unit
  suites passed 718 cases: shared 287, desktop 312 and Android 119. Seven desktop
  Windows-only cases were skipped, with their exact names in the receipt.
- Android instrumentation sources compile, including the actual controller's
  delayed-page-refresh regression; none were device-run. Android lint reports
  zero errors, 58 warnings and 12 hints. All four arm64 LOAD segments pass16KiB
  alignment. The real desktop controller page-race/eraser-target regression passes.
- Native integration strict Clippy passed on Linux. Windows-target document
  runtime/worker/probe Clippy passed with warnings denied after correcting pinned
  Win32 Boolean fields. This is a type check, not Windows runtime evidence.
- Fresh Linux core/host/generator and Android arm64 libraries built. Core/host
  bindings were generated once for this phase; retries reused unchanged output.
  Cargo policy and the264-module Gradle license census passed.

SVG publication alone is enabled, with verified final-file identity and bounded
output. PDF exact-original is explicitly unsupported; other PDF publication stays
unavailable until actual PDFium/qpdf compatibility and final redaction verification
are performed. OS worker isolation/retirement, Windows packaging/launch and actual
Android provider/service delivery are still unverified. Krita and physical product
and performance gates remain deferred. The source does not claim a gate pass.

Resolved build failures and artifact/generated-binding hashes are recorded in
`docs/evidence/CLOUD_COMPOSED_APP_VALIDATION_20261018.json`. The test-only Skiko
cache now uses the owned build tree; three real Windows file/owner tests have
explicit platform assumptions. The bounded API29 pin reader and unsigned maximum
identity wire tests pass. No home/security/credential/signing/device change occurred.

Next integration work remains: independent combined-source review/main promotion,
then deliberate giant-image root viewport and stream-finish consumer integration,
remaining controls/selection/tilt promotions and Phase5 software verification gaps.
Current ordinary-format callers, image foundations and their source-bound tests do
not establish complete giant-image UI or stream acceptance. Parent-owned old
worktrees and dirty picker remain untouched pending their separate disposition.

## Reviewed publication and PDF runtime feasibility

Independent source review accepted restricted candidate `5f4f581` with no remaining
source blocker. A normal, non-forced fast-forward push advanced remote main from
`7be9da7` to exact `5f4f581c84d9215c06c0029f8dd042fc8453f26d`. The remote SHA
was verified. GitHub returned no commit statuses or PR workflow runs for this
commit; protection/check-run REST reads returned Forbidden. No protection,
required check, security or workflow configuration was changed or overridden.
This records server acceptance of the normal push, not Windows runtime evidence.
The frozen roadmap validation ref remains `070758ac3b0537b4ea9efc0f6d21b23792d47762`.
Durable sanitized execution excerpts supplement the full source-bound receipt in
`evidence/CLOUD_COMPOSED_APP_RECEIPT_EXCERPTS_20261018.txt`; original logs and failed
attempts remain in the cloud execution environment.

The two ignored PDFium engine cases cannot safely run with the available approved
inputs: the project requires verified build 7881 while the preinstalled Linux
library is build 7999, and no reviewed artifact/trust pin or actual target worker
boundary is available. System qpdf is 12.2.0 rather than required 12.4.2; its
Windows LPAC path is unavailable on Linux. No substitute native binary was loaded
or installed. Six strict pypdf cross-parser checks on actual Rust-stamped
synthetic copies passed for LF, CRLF and CR xref rows and repeated stamping.
This diagnostic does not close either ignored PDFium case, native isolation,
Windows/Android execution or any product gate. PDF publication stays refused.
Exact artifact identities, reproducible diagnostic sources, bounds and results
are in `evidence/CLOUD_PDF_RUNTIME_FEASIBILITY_20261018.json`.

## T3.05 composed candidate and next independent batch

Leaf source `2df316f` is deliberately composed on
`cloud/t3-05-integration-20261018`, with validated code `dc6d469`.
Ellipse and tapped-polygon pixel selections now use the existing native mask
transaction, source binding, version and undo paths. Both app callers preserve
only an admitted idle polygon between real contacts; Finish publishes one step,
and cancellation, camera/page/revision/project/tool/refinement changes retire the
draft. Actual desktop controller/native-journal tests exercise that lifecycle.
The corresponding Android instrumentation is compiled, not device-run.

Reported raw stroke axes remain unchanged. The calibrated native tilt conversion
and exact scoped remote axis mask are composed through Android, protobuf, private
wire and Windows injector source. Absent/unmeasured axes retain ordinary pressure
with no tilt/twist claim; normal app construction still supplies no measured
calibration. Physical calibration, Windows injection and pen feel remain open.

Phase checks passed 566 native cases across this batch's six-package scope and
744 executable JVM/unit cases (308 shared, 314 Desktop, 122 Android), with seven
Windows-only skips. Fresh Linux and arm64 libraries, one binding-generation pass,
actual app/instrumentation source compilation, zero-error Android lint, four
16-KiB LOAD segments and dependency policy checks passed. The recovered opaque
Mask equality assertions, synthetic fixture color-consent setup and one existing
fast-IO diagnostics test assumption were corrected; failed receipts remain.
The Windows cross-check stopped in ring's C build before project source checking:
host GNU compilation was selected and lib.exe was unavailable. No alternate
compiler, SDK or binary was installed/substituted. Windows target validation and
independent review remain required considerations before main promotion; this
candidate leaves main at `bca245e`. Exact source/artifact bindings and sanitized
execution excerpts are in `evidence/T3.05-CLOUD-INTEGRATION-20261018.json` and
`evidence/T3.05-CLOUD-RECEIPT-EXCERPTS-20261018.txt`.

Exact runtime research `2400eac` is retained without changing pins or PDF gating.
Publisher downloads returned403. Separate Android packaging trace `a3875e2`
confirms that unextracted APK JNI bytes cannot satisfy the current physical-file
PDFium hash/load contract; neither a Java-only hash nor an APK-relative string is
a verified native-loader repair. That source repair belongs to a separate batch
and still needs actual admitted artifacts and isolated-device validation.

Giant viewport/JPEG stream leaves `ca773ed` and their51 written/unrun tests are
queued for a separate coherent integration slice. Root first admits the minimal
viewport API, source/manifest/full-revision/attachment fences, native provider
contracts and session-wide memory/retirement accounting. Bounded producer leaves
can then return to the image owner under exact new-file ownership while root
keeps registration, store APIs, shared contracts and actual controller/draw wiring.
No recovered controller postimage or unfinished stream workflow is promoted by
this T3.05 candidate. Five scoped M4 build tasks and all physical/product gates
retain their existing pending status; Krita remains deferred.

## T3.05 promotion and viewport/stream contract handoff

Astra's bounded review accepted restricted `83663107`; a normal fast-forward
push published that exact commit to main and the remote SHA was verified. No
Windows, mixed-version peer, measured calibration, device or product gate was
closed. The earlier T3.05 receipt's main value describes its pre-publication
validation boundary, not the current delivery ref.

The next isolated branch is `cloud/giant-stream-integration-20261018`, stacked on
that reviewed main. It admits `ca773ed` leaves, the minimal common viewport API,
native DTO/error contracts, a separate foreign-resource charge interface, and
bounded store access prerequisites. Root/native contract compilation, one
pre-allocation cache-read regression, locked graph, imports, format and dependency
policy passed. The51 written leaf tests and shared contract compilation are
pending; no bindings are regenerated or giant/stream user workflow enabled.

Exact producer/provider and pure-stream accounting file ownership, factory types,
mandatory shared pool and test obligations are published in
`GIANT_STREAM_OWNER_HANDOFF_20261018.md`. The image owner may take the single
heavy slot after parent forwards that handoff; root continues reserved native/
Kotlin/renderer source without a concurrent heavy build. Root retains all public
FFI/shared contracts, registration, store APIs and app composition. Recovered
local view counters alone do not satisfy session-wide resource accounting.
Source-bound contract checks are in `evidence/GIANT_STREAM_CONTRACTS_20261018.json`.
