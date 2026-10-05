Status: IN PROGRESS
SOFTWARE CHECKPOINT — native helper, editor-identity and app foundations validated.
OPEN — integrated Windows/Android delivery, live editor controls, physical S Pen,
performance, practical drawing and milestone acceptance remain pending.
BUILD TASKS LEFT — 6. The S23 returned on 2026-10-04 and is authorized for
applicable tests; separately selected IN2019 checks remain authorized.

# M4-KP — Krita and Paint remote editing

This checkpoint follows the owner's D20 scope decision. Active work is T4.01,
T4.03, T4.04 and the Krita/Paint subsets of T4.07/T4.08, followed by focused
acceptance and personal-use packaging. Expanded Phase 4 work and Phase 5 remain
deferred. Earlier product gates retain their existing status.

## Necessary dependencies

| Component | Current evidence | Remaining milestone check |
|---|---|---|
| Physical window identity and DPI | Exact HWND, PID, thread, process creation, physical client/window rectangles and DPI are checked by the admitted native code. Selected image inspection and bounded PE/version parsing pass native fixtures. | Real selected Krita/Paint process, current canvas, coordinate mapping, move/resize/minimize/focus and elevated-target refusal. |
| Capture and hardware encoding | The isolated WGC/Intel HEVC helper builds. Access units, configuration and coded/visible dimensions have explicit bounds; capture timestamps fence input receipts. | Actual hardware descriptor and controls, continuous capture without contact freeze, at least 30 fps and retained failure behavior. |
| Paired CONTROL/MEDIA/INPUT | The scoped protocol, exact pen-flag extension and eight new regressions pass the central network check. FFI adapters and app composition are being integrated. | Real paired media/input delivery, exact target/source/geometry/session binding, disconnect/background cleanup and fresh reconnect grants. |
| Guarded input ownership | Helper regression tests cover admission, startup gates, contact completion deadlines and retirement predicates. Input is guarded per call; a partial finite batch seals the session. | New-helper live pressure/flags and 100 target-change trials, no stale input, actual helper/process/IO retirement, and visible state on both devices. |
| Frame/render ownership and ghost ink | Decoder and ghost source is admitted. Shared ghost logic and isolated Android decoder lifecycle fixtures pass; the decoder engines in those phone tests are fake. | Integrated hardware decode/render, retained native frame tickets, immediate lifecycle clearing, first covering-frame fade at 150 ms, hard expiry within 500 ms, and wet latency at most 25 ms p95. |
| Krita/Paint compatibility and controls | Exact selected-image identity plumbing passes. Portable Krita 5.3.4 was verified against its published SHA256; installed Paint was inventoried. | Live Windows Pointer Input pressure in Krita, basic Paint drawing, per-version/tool results, current native tool/settings proof and tested essential shortcuts with a visible destination. |
| Personal delivery | Prior normal APK and Windows distribution exist for Phase 2. | Fresh milestone APK and Windows app image, exact artifact hashes/notices, setup guide and practical owner drawing session. |

## Startup and observation validation, 2026-10-04

The shared-parser correction passed **254 native tests**, strict lint and
formatting. The nine new Paint-reader cases pass; the nine existing PE parser
cases now execute once. The earlier 263-test run and its duplicate-module lint
failure remain retained. The real-render receipt parser separately passed
**seven tests**.

The latest helper check passed **62 tests**, strict lint, formatting and all five
fresh license gates. This includes the harness startup correction and the
input-free Paint and bounded Krita control-candidate observation paths. All eight
nested process phases and the outer Job retired with completed output streams;
all 115 bound source paths remained unchanged. Initial guard readiness requires
a new balanced receiver mouse pair, released buttons and the exact owned target
foreground. Automatic activation alone cannot start the run. Receiver messages
do not establish independent physical mouse provenance. The 100-trial live rerun
and live editor observations remain pending.

A fresh, explicitly selected S23 read-only check confirmed that its route command
returns two text lines despite the JSON option. The integration runner needs to
validate that actual format against the current USB destination/interface/source;
the original JSON-only route contract does not match. No network identifiers
are published and no device or Windows settings were changed.

[The second integration progress receipt](M4-KP-integration-progress-r2.json)
retains exact source bindings, counts, phase cleanup and the sanitized route
shape. No verification images were created. **Six build tasks remain.** Next is
the full Windows/Android integration build and app checks (L), with Sol/xhigh
feature executors; this checkpoint does not pass streaming, editor effects,
target-device performance or milestone acceptance.

The subsequent measured-format route correction passed **23 pure parser cases**,
with actual Job/stream retirement and unchanged source bytes. The first full
integration build then stopped at `:desktop:compileKotlin`: the new test host
uses the correct generated core package, but the desktop module lacks a direct
core-bindings dependency. Windows and Android native builds, generated bindings
and import/alignment checks passed; complete app artifacts and APK verification
did not. The Android native build also reported two host-only unused functions.
All 11 nested phases and the outer Job retired, and all 115 source paths were
preserved. [The failed build receipt](M4-KP-integration-build-r17-failure.json)
retains these outcomes. A separately frozen dependency/warning correction is
required before rerunning. No app was installed or launched in this build.

The separately reviewed correction then passed the **complete integration
build**: both native targets, generated bindings, the desktop test-host classpath,
both isolated Android test APKs and APK native-packaging verification. All 12
nested phases and the outer Job retired with output streams complete; the
115-path source snapshot remained unchanged. The outer run took 470.149 seconds
and sampled a peak tree working set of 2,898.4 MiB.
[The successful build receipt](M4-KP-integration-build-r18.json) binds the exact
source and generated artifacts. The earlier failure remains retained. This build
performed no install, launch, input or capture and does not pass the app tests,
live streaming, editor controls or physical/performance acceptance. Fresh app and
native helper checks are next; **six build tasks remain**.

The fresh native helper check passed **62 cases**, strict lint/format and all
five license gates on this snapshot. The normal app run completed **391 cases**
with **five failures**: one shared cancellation-identity assertion and four
desktop runtime-anchor/inventory cases. Android's six unit cases passed. All
nested/outer Jobs and streams retired and the 115 source paths stayed unchanged.
[The failed app-check receipt](M4-KP-app-check-r18-failure.json) retains the test
census and hashes of all 53 privately preserved XML reports. The failure is being
reviewed before the integrated phone test; production retirement and runtime
inventory predicates are not relaxed.

Separate input-free observations successfully opened, queried and retired new
owned blank Paint and Krita windows. Paint **11.2605.81.0** verified through the
explicit installed-package reader and yielded a complete 224-node selected-root
tree with zero traversal errors. It retained the process/final image and verified
the OS package namespace; it does not claim ordinary ancestor-lease equivalence.
Krita **5.3.4.0** yielded both bounded control-candidate groups completely. Its two
toolbar numeric controls have no name, automation ID or label witness, so their
ranges/positions do not establish brush-size authority.
[The query-only observation receipt](M4-KP-editor-observations-r18.json) binds both
actual images, probe binaries, source and retirement evidence. No input or
verification images were created; no profile, shortcut, drawing or physical
acceptance follows from these queries. Actual effect/restoration measurements
and essential controls remain open. **Six build tasks remain**.

The separate R19 app ownership repair passed **392 cases**: 178 shared, 208
desktop and six Android unit cases, with zero failures/errors/skips. The actual
cleanup path retains its captured primary error while retirement runs on the
worker dispatcher; cancellation during retirement now has a deterministic
regression. Synthetic runtime fixtures contain the same six native payloads as
production, with the namespace/anchor assertions preserved. All nested/outer Jobs
and streams retired and the 120-path snapshot remained unchanged. The original
R18 failures remain retained. [The R19 app receipt](M4-KP-app-check-r19.json)
binds the source and all 53 privately retained XML reports.

The new retained-source and quantitative-frame harness was separately admitted.
Its offline lock normalization changes only two local dependency edges to already
pinned png 0.18.1 and blake3 1.8.7; all 408 package records and the 368 external
package versions remain present. Three new synthetic UUIDv4 identifiers were
rejected by the existing UUIDv7 contract before the intended assertions.
[The original frame-check failure](M4-KP-frame-check-r1-failure.json) is retained.
A fixture-only correction subsequently passed **all 73 native cases**, but the
complete strict run failed on two new lint diagnostics; formatting was not
reached. A separate equivalent lint repair is required, with no allows or
production validation/deadline relief. These samples do not establish an editor
effect or grant a control profile. The fresh read-only S23 check reports its USB
interface and correlated Windows adapter up; no settings were changed or network
identifiers published. **Six build tasks remain**. Next: strict native checks
and the integrated USB run (L), then measured essential controls (S), with
Sol/xhigh feature executors and physical/performance acceptance still pending.

The strict frame attempt and its two lint diagnostics are retained in
[the R2 lint failure receipt](M4-KP-frame-check-r2-lint-failure.json). Separate
equivalent source corrections have been admitted. A bounded query confirmed that
Windows supplies its temporary directory with a trailing separator; the frame
runner now removes exactly that one OS-supplied separator before applying the
existing strict namespace checks. Both new shape regressions await the fresh
helper check. No general path, lease or deadline predicate was relaxed.

The subsequent FFI attempt failed when its orphan-quota fixture tried to allocate
a 256 MiB file, followed by a secondary poisoned-mutex failure. Only its first
96-case suite ran; [the original failure](M4-KP-ffi-check-r4-storage-failure.json)
is retained. A separately reviewed test-only repair marks the newly owned file
sparse and asserts the same logical size and quota refusal. All **254 FFI cases**,
strict lint and formatting then passed against the 121-path source snapshot;
all three inner phases and the outer Job/streams retired. The
[R20 FFI receipt](M4-KP-ffi-check-r20.json) binds that run. Production quota and
parser behavior are unchanged. The exact owned-cache cleanup disposed of 30
obsolete test debug-symbol files, retaining their executable/fingerprint pairs,
newest symbols, all source, evidence and production payloads. The
[unblocking receipt](M4-KP-build-unblocking-r20.json) retains hashes, counts and
disposal evidence. Zero verification images were created. **Six build tasks
remain**; these validation repairs do not complete an integrated milestone task.

The subsequent complete helper check passes **75 cases**, strict lint,
formatting and all five fresh license gates, with all eight inner phases and
the outer Job/streams retired and the same 121-path source snapshot preserved.
The two OS temporary-directory shape regressions ran successfully. The
[current helper receipt](M4-KP-helper-check-r4.json) retains exact source and
process evidence. These native unit checks establish no frame image, editor
effect or phone render.

The fresh **R20 complete integration build passes** all 12 nested phases,
including current native payloads, generated bindings, the desktop host and both
isolated Android test APKs. All actual Jobs and streams retired, preserving the
121-path source snapshot. [The build receipt](M4-KP-integration-build-r20.json)
binds the generated artifacts. The first strict artifact inventory stopped before
installation because Gradle named a missing, empty Java NO-SOURCE output. An
administrative repair created only that verified empty build-output directory;
source, classpath bytes and bytecode remained unchanged. A permanent producer
correction is separately frozen for review.

The selected **S23 real USB integration then failed** after its controller became
ready: no validated render callback arrived within 30 seconds. The physical USB
route passed. The runner's original `phone-usb-route` failure label is stale; the
owned TestRunner exception identifies the later render deadline. The exact failing
capture/encode/transport/decode stage is not yet known. All owned OS process trees
and output streams retired; both isolated test packages were removed and private
work was disposed of. Internal phone/host retirement and final profile preservation
were not observed, and this attempt proves no profile mutation. The
[failed render receipt](M4-KP-usb-render-r20-failure.json) retains the actual source,
artifact, postmortem and cleanup bindings. Zero input grants or verification images
were produced. **Six build tasks remain**; stage diagnostics are the next repair.

The fresh owner-authorized guard attempt stopped safely at initial readiness:
zero TARGET clicks were recorded within 45 seconds, so zero cases or mutation
trials ran and no test input was injected. The owner saw only a window labelled
"sync"; the native captions are TARGET and SINK. Window visibility remains the
cause to investigate. Both owned surfaces, the driver/outer Jobs and all output
streams retired, with the 121-source snapshot unchanged. The
[readiness failure receipt](M4-KP-guard-readiness-r20-failure.json) retains this
inconclusive attempt. It does not replace the earlier 68/100 result or pass the
required 100 trials. Zero verification images were created; six build tasks remain.

## Native foundation validation

### Integration progress, 2026-10-04

The later native FFI check passed **245 tests**, strict lint and formatting.
The control/observer and destination-input changes then passed **55 helper
tests**, strict lint, formatting and all five fresh license gates, followed by
a fresh helper build. The updated protocol check passed **93 tests** and all
**10,000 deterministic fault seeds**, with strict lint and formatting. Each
result is bound to its actual source snapshot; later admissions are not covered
merely because an earlier snapshot passed.

The latest live guard attempt completed **70 cases, including 68 of the required
100 target-change trials**. The separate receiver recorded zero events in all six
categories. The next move/restore trial stopped with `TargetChanged` after
successful foreground checks. The old log cannot identify the exact failing
operation or original failure-time geometry. Actual native windows, outer and
driver Jobs, and output streams retired. This remains a failed attempt. Separate
harness-only stage diagnostics and instructions explaining the deliberate rapid
window movement have been admitted; production guard predicates and deadlines
remain unchanged.

A separately reviewed Paint installed-package reader has been admitted. It uses
the selected retained process, current-user Store registration and package/path
checks plus a retained strong final-image descriptor. It does not weaken ordinary
ancestor leases or enable editor/input authority. The lock update adds only the
local host-FFI edge to the existing pinned Windows package; all **408 packages**
and external versions remain unchanged. All five fresh license gates pass. Its
native run passed **263 tests**, including nine new reader cases and nine repeated
existing parser cases. Strict lint then rejected the duplicate parser module.
The failure is retained; the single-module repair subsequently passed the
254-test check recorded above. No live package-reader observation has yet been
performed.

The S23 tether inventory now observes an up phone RNDIS interface and an up
Windows adapter correlated to the exact Samsung USB device, with one IPv4 address
on each. Private addresses and serials are withheld. This is route inventory,
not media streaming. No OnePlus command or machine security/network change was
performed. The real generated-session/controller/hardware-decoder USB test and
its interrupted-install cleanup correction are admitted for build validation.

[The integration progress receipt](M4-KP-integration-progress-r1.json) retains
exact source bindings, counts, failures, log names, event census and cleanup.
No verification images were created. These foundations and source admissions
do not complete a build task; **six remain**. Next is the integrated app/build
validation (L) using the owner-selected `gpt-6.1-sol` / `xhigh` feature lanes.

Central `build.ps1 test-remote-helpers` passed **28 tests, zero failed or ignored**
across five suites. Strict all-target clippy with warnings denied, formatting and
all five license checks passed. `build.ps1 build-remote-helpers` produced both
native helper binaries. These are development artifacts; no helper was launched
against an editor in this checkpoint.

Central `build.ps1 test-ffi` passed **238 tests, zero failed or ignored** across
12 suites, including 16 editor-identity cases. Strict clippy and formatting
passed. The unchanged dependency manifest reused its successful license receipt;
the subsequent helper checks also ran fresh license gates.

All reported successful bounded jobs and nested phases confirmed process-tree
cleanup and completed output streams. Earlier parser, missing Windows import,
IPC discriminator, fixture lint and helper lint failures remain retained. Repairs
were admitted from separately hashed source freezes; failed attempts were not
relabeled as passing runs.

Exact source hashes, counts, artifact hashes and retained log names are in
[the native foundation receipt](M4-KP-native-foundation.json).

## App foundation validation

The repaired central app check passed **365 cases**: 152 shared, 207 desktop and
6 Android unit cases, with no failures, errors or skips. All 26 new shared
compatibility, palette and ghost cases passed. The earlier failed run remains
retained; its coroutine-receiver compile error and immutable-list test error
were repaired separately without changing production expiry or retirement rules.

The strict Android report parser passed **27 cases** and binds the entire
166-method source inventory. Isolated owner-user-0 instrumentation on the S23
SM-S918U passed **166/166 cases**, including 12 new decoder fixtures. These use
fake decoder engines and real Surface/thread lifecycle fixtures; they do not
measure hardware decoding or the normal app's remote-edit behavior.

All 45 phone-run process phases confirmed cleanup. Both exact-hash test packages
were removed from owner user 0, both Android profiles were preserved, and the
owned private workspace was disposed of. JVM outer and inner process trees also
retired with completed output streams. No verification images were created.
Counts, current app source hashes and isolated APK hashes are in
[the app foundation receipt](M4-KP-app-foundation.json).

The separate new-helper guard harness has 100 written target-change trials,
covering six receiver event categories. After separately admitted pen-flag, BSTR
and fixture lint repairs, the current central strict check passed **37 cases**,
with zero failures or ignores; fresh license checks, all-target clippy with
warnings denied and formatting passed. A fresh build produced both helpers,
the guard harness and the read-only editor probe. Earlier failures remain retained.

The live guard attempt stopped with an ungranted-control error before completing
any case or mutation trial. Both owned surfaces recorded zero events in all six
categories. Actual outer and driver Jobs retired and their streams completed.
A runner receipt-writing error happened after its cleanup loop: four bare
PowerShell `false` tokens were separately repaired to `$false`. The missing
native receipt was not reconstructed, and the native foreground handoff still
needs diagnosis. This attempt establishes no live guard acceptance. Exact source,
artifact and cleanup bindings are in
[the guard foundation receipt](M4-KP-guard-foundation.json).

The harness, full app composition and live editor observations remain open.

## Network foundation validation

The central network check passed **88 cases**, including all eight new remote-edit
protocol cases, and all **10,000 deterministic fault seeds**. Strict all-target
clippy with warnings denied and formatting passed. One top-level ignored entry is
the private process-death worker invoked by its owning recovery fixture. All
seven inner process phases and the outer job confirmed actual cleanup and completed
streams; all 59 bound source paths were preserved.

These cases preserve exact pen flags through authenticated protocol delivery,
require bilateral capability, refuse malformed scopes and legacy raw commands on
the new route, and retain existing capture/disconnect behavior. They do not prove
hardware pen flags, native input, editor effects or normal app remote editing.
Exact source hashes, counts, simulation totals and log names are in
[the network foundation receipt](M4-KP-network-foundation.json).

## Acceptance limits

The suspended, read-only editor probe and its retirement fault fixtures pass a
separate strict check with **42 cases**, followed by a fresh build. It successfully
bound and observed an owned native surface and Krita 5.3.4.0. Krita's complete
322-node snapshot contains a canvas and an observed eraser toggle. Several nodes
report keyboard focus, and the observed shortcut properties are empty; these
facts do not enable any actions or establish current runtime canvas authority.

The first Krita attempt stopped at its 30-second startup bound. A subsequent
diagnostic run recorded actual resource-loading CPU progress, reached readiness,
and completed the read-only observation. Paint reached window readiness but its
source binding reported an IO failure. A separate directory access repair has
been admitted for validation; its cause and effect remain unverified. Both failed
attempts are retained. All successful and failed observation trees, Jobs and
streams retired. No input or verification images were produced. The public
[editor probe receipt](M4-KP-editor-probe.json) retains counts, hashes, selected
positive facts and limits; full UI trees remain private text.

The newer [R23 helper check](M4-KP-helper-check-paint-r23.json) passes 80 cases,
strict lint/format and fresh license gates. The matching [complete integration
build](M4-KP-integration-build-r23.json) passes all twelve stages, preserving all
123 bound source paths and retiring actual nested/outer Jobs and streams.
The [selected-S23 USB attempt](M4-KP-usb-render-r23-failure.json) reaches a
configured host stream and a Ready hardware decoder with current scope/config,
but records zero renders before the unchanged callback deadline. Owned packages,
OS processes/streams and private work were cleaned up. An observed phone
retirement marker does not establish the complete internal lifecycle case.
The reviewed first-keyframe startup repair is admitted for affected validation;
its source review is not hardware-delivery evidence.

The [corrected Paint controls query](M4-KP-paint-controls-r23.json) observes
size, zoom and canvas but leaves brush/style Unknown. It grants no palette or
numeric input authority. The [unattended blank Krita frame attempt](M4-KP-frame-r7-color-refusal.json)
passes native bind, exact-owned foreground rebind and current tool/settings
observation, then fails with the hash-bound typed capture refusal ColorDepth.
It sends no input and produces no verification image. Its empty owned TEMP
sample/marker was subsequently disposed of after all actual native, editor and
outer Jobs/streams retired; text-only cleanup evidence is retained separately.
Explicit unattended guard readiness is integrated for the next rebuilt live
run; successful software readiness still requires actual balanced receiver
input, released buttons and exact foreground. These changes complete no scoped
build task. Six remain, with physical S Pen, performance and owner acceptance
separate and pending.

The [R24 app run](M4-KP-app-check-r24.json) passes 400 cases across 54 retained
XML reports, including all eight new decoder-startup cases. Its matching
[native check](M4-KP-helper-check-r24.json) passes 80 cases, strict lint/format
and fresh license gates; the [complete rebuild](M4-KP-integration-build-r24.json)
passes twelve stages with 125 source bindings preserved and all actual build
Jobs and output streams retired. The [unattended guard run](M4-KP-guard-unattended-r24.json) then passes
103 cases and all 100 target-change trials, preserving all six sink categories
and retiring every old input owner. All four modes have 25 trials, with 200
distinct old/fresh session identities. Actual guarded software input and a new
balanced receiver pair satisfy readiness; physical human provenance is false.
This completes the scoped software guard check, with editor, integrated input
and physical pen acceptance still open.

The [matching S23 USB test](M4-KP-usb-render-r24-failure.json) remains failed
with zero native-validated renders. Its retained diagnostic is now
RecoveryRequired(RenderedIdentity), which distinguishes callback-path rejection
from R23's Ready-with-no-render state. The precise timing/identity predicate is
not yet measured. OS processes/streams, isolated packages and private work were
cleaned up. [The separate R7 cleanup receipt](M4-KP-empty-frame-disposal-r7.json)
confirms disposal of its exact empty owned sample/TEMP marker; zero verification
images existed and none remain available for review. Six build tasks remain.

The R28 checkpoint passes [101 native helper/capture cases](M4-KP-helper-check-r28.json),
[254 FFI cases](M4-KP-ffi-check-r28.json), [409 JVM cases across 55 retained XML
reports](M4-KP-app-check-r28.json) and [31 diagnostic grammar cases](M4-KP-diagnostic-grammar-r28.json).
The [full integration rebuild](M4-KP-integration-build-r28.json) passes all twelve
stages with 131 source bindings preserved, strict lint/format, fresh license
checks and all actual build Jobs/output streams retired. These checks bind the
historical R28 sources; later admitted changes require their own validation.

The [actual S23 tether test](M4-KP-usb-render-r28-failure.json) still fails with
zero native-validated renders. Its closed diagnostic isolates mask 16: the
codec's render timestamp precedes the actual local release request. Media PTS
matches exactly and callback enqueue delay is 20.766302 ms, within the unchanged
500 ms limit. The render/request and callback/render differences are clamped
to their diagnostic bounds; they are not exact measured clock offsets. OS
processes/streams, isolated user-0 packages and private work are cleaned up.
The phone retirement phase marker does not prove internal decoder retirement;
internal host retirement and the final profile inventory comparison also remain
unproved because the run stopped at the render-callback phase.

The [owned blank Krita frame attempt](M4-KP-blank-frame-r28-failure.json) identifies
the original ColorDepth refusal as a nonopaque pixel with alpha 228. Monitor
color-space and texture descriptor gates pass. Pixel location, count and cause
are not measured, so no capture predicate is relaxed. The run sends no input
and creates no image. Actual editor/native/outer Jobs and streams retire, and
[the exact empty owned sample/TEMP cleanup](M4-KP-empty-frame-disposal-r28.json)
removes only its marker and empty directories nonrecursively. Zero verification
images were created or retained; none are available for review.

R30's reviewed explicit local Surface scheduling and the Paint query R2 reader
are admitted with 132 source bindings. Their [affected native check](M4-KP-helper-check-r30.json)
passes 103 cases, strict lint/format and fresh licenses. The [fresh app check](M4-KP-app-check-r30.json)
passes 414 cases across 56 retained XML reports, including all five local
Surface scheduling cases; actual process Jobs and output streams retire.
Hardware render, causal editor effects and production control activation remain
pending. Six build tasks remain. Next are finite-input ownership (L), the
bounded alpha-location diagnostic (S), active editor compatibility/effects (M)
and essential controls (S), with both feature executors at Sol/xhigh.

The shared finite-input owner and terminal correction are composed with 136
source bindings. Two independent source reviews confirm the correction to the
reported Pending-to-immediate-Complete exit gap: even after release succeeds,
the helper must consume exact Stop and return Stopped before parent uncertainty
clears. The intermediate uncorrected source was never executed. The [affected
native run](M4-KP-finite-check-r31-lint-failure.json) passes all 138 cases but fails
strict clippy on one nested optional keyboard-destination comparison. Actual
Jobs and output streams retire and the complete source map is preserved.
The narrow ordered let-chain repair is admitted for validation, with no lint
waiver or style-only regression. Input-capable editor effects still await their
separate terminal-rendezvous consumer and actual outer retention. Six build
tasks remain; this foundation is not an editor effect or production input pass.

The R33 rerun passes [all 138 native cases](M4-KP-finite-helper-check-r33.json)
and [all 254 FFI cases](M4-KP-finite-ffi-check-r33.json), including strict lint and
formatting. Actual nested/outer Jobs and output streams retire, and all 136 source
bindings remain unchanged. A matching fresh helper build passes its six stages.
The subsequent [unattended guard run](M4-KP-guard-unattended-r33.json) passes
103 cases and all 100 target-change
trials with the corrected owner; the actual software readiness click and balanced
receiver pair are observed. Physical human provenance and integrated editor effects
remain separate. Six build tasks remain.

The following [Paint controls query](M4-KP-paint-controls-query-r33.json) completes
without input and retires the exact owned editor, probe and outer Jobs/streams.
Actual brush primary-button and size/zoom thumb witnesses remain Unknown, so the
settings digest and frame witness are unavailable. The measured 275438 us
whole-harness query does not prove the production 180 ms budget or grant control.

The corrected alpha diagnostic passes [156 native cases](M4-KP-alpha-helper-check-r35.json),
strict lint, formatting and fresh license checks with 137 source bindings. The
[integration rebuild](M4-KP-integration-build-r35-link-failure.json) fails MSVC
LNK1140 while linking vw_core.dll; its observed failed PDB is 114356224 bytes,
which does not identify the exceeded internal PDB limit. All actual Jobs and
streams retire, including the compiler telemetry child. No new current complete
artifact set, alpha census or phone render result is established. The S23's
USB tether route is freshly confirmed Up through read-only, model-selected
queries; no OnePlus command or settings change occurs. Six build tasks remain.

No integrated decoder, production pen-input, editor action, pressure, frame-rate,
cross-device latency or owner drawing result is established by this checkpoint.
The existing T0.05 harness measured pressure correlation 1.000 and scoped guard
trials, but its combined inverted/eraser flags failed and it used the earlier
injector. Those results cannot pass the new runtime's 100-trial acceptance.
The earlier video spike also does not pass T4.01's integrated 30 fps/80 ms targets.

A later source-bound native check passed **43 cases**, strict lint, formatting
and fresh license gates, followed by a fresh helper build. Krita's new complete
318-node read-only snapshot confirmed the actual focused element inside
`KisOpenGLCanvas2`. This is a selected-window measurement, not persistent input
authority or an editor effect. Paint's bind still reported an IO error after
the least-access directory repair; typed error-stage diagnosis remains open.

The new live harness attempt completed 65 cases, including 63 target-change
trials, with zero events in the separate sink. It then stopped when Windows
refused a fresh owner focus grant during the next focus-switch trial. All owned
native windows, outer and driver Jobs, and output streams retired. The failed
attempt remains retained; it does not satisfy the required 100 trials.
[The fresh measurement receipt](M4-KP-native-measurements-r8.json) binds the
sources, artifacts, counts, journals and cleanup. No editor input, phone action
or verification image was involved. Six build tasks remain.

The early native helper advertises no shortcut actions until current tool/settings
observation is implemented and verified. Distribution metadata and profile strings
do not substitute for that observation. Editor identity is selected-process image
identity; it is not an attestation of all loaded DLLs.

The subsequent directory experiment disproved the R7 attributes-only lease
equivalence: an empty directory could be renamed while that handle remained open.
The original `FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES` access blocked the same
rename with Windows error 32. R10 restores that access. Its independent regression
opens each of three directory levels alone, with no final-image handle masking the
result; each rename is blocked while held and succeeds after the lease closes.
All three experiment directories and their handles were disposed of.

That regression and both new focus-transition cases passed in a **46-case** run.
Strict lint then stopped on test-module placement; the complete app integration's
next compile stopped on a missing `windows::core::BOOL` import in the native picker.
Both failures and actual owned process/stream cleanup remain retained. Narrow
source corrections were separately admitted for validation. Paint's read-only
binding now reports the precise failure: access denied opening ancestor ordinal 2
of its protected package path. It establishes no input or profile authority. A
separate package-aware identity contract is still required; ordinary helper
namespace leases remain strong. Exact measurements, source hashes, failures and
cleanup are in [the directory correction receipt](M4-KP-directory-lease-correction.json).
These repairs and foundations do not complete a build task; six remain.

The R34/R36 installed Paint frame proofs, R37 cdylib link fix, finite terminal
effect path and R3 primitive queries are admitted with 143 source bindings.
[All 183 native cases](M4-KP-paint-helper-check-r37.json), strict lint/formatting
and fresh license gates pass. The [full integration rebuild](M4-KP-integration-build-r37.json)
passes all 12 stages, including Windows/Android native builds, golden bindings,
desktop classes and isolated APK native gates. This does not prove a phone render
or input effect. The later [FFI run](M4-KP-ffi-r37-link-and-space-failure.json)
fails the separate lib-test Windows PDB limit and a parallel archive write fails
OS112; actual compiler/outer Jobs and streams retire and all source bytes remain
unchanged. Its core/host DLL rewrites require another successful runtime rebuild.

The [actual input-free Krita census](M4-KP-blank-frame-r37-alpha-census.json)
finds 84 nonopaque pixels in the bottom 12 client rows, all outside the freshly
verified canvas. The 1852 by 1202 canvas has zero nonopaque pixels. Full-client
capture correctly preserves its opaque-255 color-depth refusal; the cause and
a separate canvas-only capture pass remain unproved. The [disposal receipt](M4-KP-empty-frame-disposal-r37.json)
confirms exact editor/native/outer retirement and removal of the empty owned
temporary fixture. Zero image files were created or retained.

The [actual corrected Paint query](M4-KP-paint-controls-query-r37.json) now observes
the brush primary button, size/zoom thumbs, current settings digest and frame
witness. Undo/Redo are Known disabled on the blank fixture. Size/zoom RangeValue
readOnly is false in this measurement. The 246444 us isolated query does not pass
the production 180 ms lookup budget. No input or profile authority is granted.

[Project symbol disposal](M4-KP-project-symbol-disposal-r37.json) retains exact
counts, hashes and ownership checks for superseded Cargo PDBs only. Source,
evidence, executables, payloads and fingerprints are preserved. Three independently
identified normal-runtime repairs and exact owned-request admission are being
reviewed before the next run. Six build tasks remain; root Sol/max is limited to
administration/review/validation, feature coding remains Sol/xhigh, and independent
review uses Astra/xhigh. Physical pen, performance and owner acceptance remain
pending.

No requirement or G0–G5 gate is passed here. M4-KP still requires its complete
focused acceptance checklist. Zero verification images were created for this
native checkpoint; no verification image is staged or retained.

Next: T4.01/T4.03 integration (L), T4.04 (M), active T4.07 compatibility (M) and
T4.08 essential controls (S). Owner-selected coding settings are
`gpt-6.1-sol` / `xhigh`. Both feature coding lanes were verified at those settings;
the existing root session remains Sol/max for coordination, source admission,
review and validation. Settings were not silently changed.

The [retained owner contracts](M4-KP-retained-owner-contracts-r6.json) pass all 69
pure cases after actual C# compilation and PowerShell parsing. The subsequent
[actual no-input setup failures](M4-KP-retained-owner-setup-failures-r6-r7.json)
reach zero cases and no native binding: the ordinary launch lease refuses access
to Paint's protected installation ancestor. All actual owned processes, Jobs and
streams retire. This is setup diagnosis, not actual finite-owner OS validation or
editor-effect evidence. Windows permissions are unchanged.

The [PDB flag correction](M4-KP-pdb-flag-correction-r37.json) preserves the original
R37 build receipt and records an observed 128929792-byte MSF7 PDB named `NONE`.
`/PDB:NONE` names that output rather than omitting it; Microsoft's documented
omission option is `/DEBUG:NONE`. A narrow ordered source correction and fresh
build are pending. The original build's successful stages and retirement remain
historical facts; its claimed PDB omission is not confirmed. This binary cache
file will not be published. No verification images were created or retained.

The [actual input-free retained owner lifecycle](M4-KP-retained-owner-inputfree-lifecycle-r8.json)
passes three cases with the exact owned blank Paint retained through each effect
owner and all actual effect/editor/root Jobs and output readers retired. The
explicit protected final-image launch lease establishes launch integrity only;
native InstalledPaint identity and canvas remain independent authority. No input,
editor history restoration or physical result is claimed.

Current151 strict [helper checks](M4-KP-runtime-helper-check-r43.json) pass 183 native
cases; [FFI checks](M4-KP-runtime-ffi-check-r43.json) pass 265 cases, including eleven
real post-auth-handler retired-MEDIA/source-epoch regressions. The
[app checks](M4-KP-app-check-r43.json) pass 433 cases across 59 retained XML reports,
including all nineteen new request/lifecycle/publication cases. All current source
bytes are preserved and actual process Jobs/output readers retire.

The [actual pinned linkage and parser record](M4-KP-linker-and-parser-r43.json)
binds the exact Cargo-selected lib-unit executable to its actual 107-case run and
records 26 pure parser passes. Explicit OPT:REF,NOICF remains before DEBUG and final
DEBUG:NONE. There is no CodeView record, matching PDB or new named NONE output;
other type13 POGO metadata remains. An initial administrative projection refusal
that wrongly assumed every PE debug-directory record disappears is retained by
hash. The [named output disposal](M4-KP-named-pdb-disposal-r37.json) and
[superseded project test cache disposal](M4-KP-test-cache-disposal-r43.json) retain
exact nonrecursive ownership/count/hash checks and unchanged source/current payloads.
No verification images were created or retained. A fresh final runtime build and
S23/editor integration remain necessary; six build tasks remain.

Unattended continuation: the ordered R45/R46 canvas implementation passed 198 native cases plus strict checks against 153 exact source paths; the private common owner R11 and canvas orchestration contracts passed 84 and 60 cases respectively. These results do not prove live editor causality or device behavior. The independently reviewed R9/R12 Krita finite-input implementation was then admitted against 155 source paths, but its first helper build failed before test execution with E0432 (missing direct sha2 import dependency). The complete failure receipt is M4-KP-krita-compile-failure-r12.json; every actual build Job and output reader retired, and all 155 source paths were preserved. A narrow source repair is pending. Six integrated milestone build tasks remain, with physical acceptance separate.

The narrow R14 correction reuses the existing SHA256 implementation and resolves
the retained Krita compile failure. The separately retained
[catalog Gradle configuration failure](M4-KP-catalog-gradle-compile-failure-r48.json)
is corrected by explicit Java NIO import aliases in R49. Independent review
clears both repairs and the ordered R13/R15/R16 essential-control catalog schema.
Its legacy evidence bypass and single-variant lint blocker are corrected without
intermediate execution. Standalone legacy profiles remain separately supported;
no catalog or editor action is authorized from syntax alone.

Current160 [strict helper validation](M4-KP-catalog-krita-helper-check-r16-r49.json)
passes **222 native cases**, formatting, warnings-denied lint and all five fresh
license checks. The [integration rebuild](M4-KP-integration-build-r13-r49.json)
passes all **12 stages** in 446.3 seconds, including Windows/Android native builds,
desktop classes, golden bindings and isolated APK native alignment. The
[pure render diagnostic check](M4-KP-render-diagnostics-r13-r49.json) passes 31
cases. Exact source and artifact hashes are retained, and every actual build or
diagnostic Job and output reader retires. These checks execute no editor input
or phone tests and create zero verification images. Live causal editor checks,
matching final app/FFI validation, S23 integration and personal packaging remain
pending. **Six integrated build tasks remain.** Feature work continues with
Sol/xhigh; root Sol/max performs administration, source review and validation.

Current continuation (R89–R95): the owner-selected executor settings supersede
the historical model recommendations above. The S23 is authorized; the OnePlus
is reserved and must not be used. The current software checks are recorded in
[R92 validation](M4-KP-software-validation-r92.json) and the
[13-stage R89 integration build](M4-KP-integration-build-r89.json). The six test
compilation errors from the first focused run remain retained; their correction
changed test assertions only.

[S23 streaming recovery](M4-KP-stream-r89.json) passed actual initial,
background-return and reconnect hardware-decoder renders, with actual native
and phone retirement. This completes the streaming software recovery task;
**five build tasks remain**. Physical pen fidelity, frame rate, latency and owner
drawing acceptance remain unmeasured. A subsequent balanced-input run rendered
all three cases and retired all owners, but strict receiver validation rejected
extra Windows mouse events. Its phone ACK/ghost observations do not establish
input acceptance. Reviewed R49/R94 changes add fixture pointer lifecycle handling
and retained receiver diagnostics; their live validation remains pending.

[Obsolete Android output cleanup](M4-KP-obsolete-build-cleanup-r95.json) removed
5,975 generated files, including four superseded APKs, and increased available
space by 1,329,876,992 bytes. All 1,123 protected source/artifact hashes remained
unchanged. Current integration artifacts, failure receipts, drafts and worktrees
were preserved. No verification images were created or deleted in this cleanup.

The [R96 balanced attempt](M4-KP-balanced-r96-cancellation-failure.json) received
zero mouse events and an uncancelled pen Down/Move/Up. It still failed strict
validation because a subsequent same-pointer, zero-pressure, out-of-range UPDATE
carried CANCELED. All three renders and actual owner cleanup passed; input and
editor acceptance remain false. The retained numeric journal now identifies the
failure. Native retirement currently destroys a still-in-range synthetic pointer
after Up. A guarded noncontact Leave before destruction is being implemented;
the cancellation parser remains strict pending that production correction.

The R99 guarded noncontact retirement correction is now implemented and
independently reviewed. Its [affected Windows validation](M4-KP-guarded-retirement-build-r100.json)
passes 20 retirement test executions across ordinary and fixture configurations,
strict lint and formatting, and eight ordinary binary builds in 69.4 seconds.
All actual build processes and output readers retired. The earlier runner
preflight failure is preserved: the installed Cargo proxy was a symbolic link,
so the corrected runner binds and invokes the six plain pinned toolchain files.
Android and JVM were not rebuilt by this Windows-only validation. The strict
S23 input rerun remains pending; this build does not pass input or ghost acceptance.

The [strict R100 S23 balanced-input run](M4-KP-balanced-input-r100.json) passes
actual phone Surface Down/Move/Up, Windows pen delivery without cancelled-pointer
or mouse events, native covering ACK and ghost fade. The ghost disappeared
150.602 ms after its first covering ACK, with a 320.670 ms total lifetime on the
same phone-local clock. Initial, background-return and reconnect hardware-decoder
renders passed again. All input/native/phone/process owners and owned test packages
retired. This establishes the basic automated harness path; held-contact lifecycle,
actual editor acceptance, physical pen fidelity and performance remain separate.
Five integrated build tasks still require full closure.

The preserved [R101 editor cursor failure](M4-KP-editor-cursor-failure-r101.json)
accepted finite drawing/history actions and restored settings, but whole-image
causality failed. Cursor differences remain failures rather than being masked
out. The [R102/R104 diagnostic attempts](M4-KP-editor-diagnostic-failures-r104.json)
retain a Paint input refusal and Krita foreground refusal; neither establishes
editor acceptance. All actual owners retired, and the inspected verification
images were disposed of with text-only receipts.

The held-input fixture build exposed two separate tooling failures. The
[R103 producer failure](M4-KP-android-producer-json-failure-r103.json) compiled
successfully but the installed cargo-ndk wrapper consumed the compiler artifact
records required by the producing witness. Direct pinned Cargo with the exact
validated NDK environment corrected that boundary. The subsequent
[R110 publication failure](M4-KP-jni-publication-failure-r110.json) produced a
valid compiler witness and matching output, then failed replacing the JNI file.
The existing JNI file already contains identical bytes. Recovery verifies those
bytes and resumes the remaining packaging work; it does not repeat the passing
compiler, host or license phases. Both failures remain preserved. No held-input
or editor acceptance is inferred from these build results.

An independent [Windows publication regression](M4-KP-windows-publication-regression-r111.json)
reproduces that replacement failure on the installed .NET runtime. With the
old destination open for read and delete sharing, overwriting `File.Move`
returns access denied while `File.Replace` succeeds and preserves the old
handle's bytes. Six actual scratch cases also check no-handle operation and
refusal under a reader that does not share deletion. The current recovery can
avoid replacement entirely because compiled and existing JNI bytes are equal.

The [R111 held fixture build](M4-KP-held-feature-build-r111.json) completed its
remaining packaging with exact native compiler witnesses and without repeating
successful compilation. The [first held grant failure](M4-KP-held-grant-failure-r111.json)
remains retained. A [desktop-only diagnostic build](M4-KP-grant-desktop-build-r118.json)
then passed five focused JVM cases in 49.336 seconds, reusing native and Android
bytes. The [subsequent held run](M4-KP-held-lifecycle-parser-failure-r118.json)
confirmed grant and witness arming, three hardware-decoder renders and actual
owner retirement, but failed lifecycle evidence validation. Its key parser
excluded digits and discarded the required `remote_lifecycle_native_prefix_sha256`
field. This is a reproducible tooling defect; full causal lifecycle acceptance
remains pending a corrected run.

[Native Paint diagnostics](M4-KP-paint-native-ancestry-r118.json) established that
both Windows UI Automation APIs return the passive Image directly inside the
exact observed Brush button. The existing exact-element comparison rejects this
valid destination. The reviewed correction is limited to that observed Brush
relationship; actual corrected editor behavior remains to be validated.
All diagnostic owners retired and task-owned verification images were disposed.
The earlier [managed/native discrepancy](M4-KP-paint-native-point-r115.json) is
retained separately, including the bounded diagnostic traversal failure.

[Two redundant JNI staging copies](M4-KP-duplicate-jni-cleanup-r111.json) were
removed after exact retained-copy verification, reclaiming 365,800,336 logical
bytes in addition to the earlier obsolete-build cleanup. Compiler outputs needed
for current validation, archives, drafts, worktrees and failure receipts remain.

A further [obsolete shared-test intermediate cleanup](M4-KP-obsolete-shared-test-cleanup-r121.json) removed exactly two generated native copies totaling 357,048,944 bytes. Their exact bytes remain in the preserved historical shared Android test APK. Current integration artifacts and native compiler caches were not removed.

The [final R122 pause preflight](M4-KP-pause-preflight-r122.json) stopped at S23 selection: ADB listed no devices. No artifact inventory, retained parent, installation or input was started. The 135 focused parser checks passed, but corrected real-device lifecycle acceptance remains unverified. No rebuild or repeat device attempt was made before the owner-requested pause.

[Ordinary artifact restoration](M4-KP-pause-restoration-r122b.json) passed after the no-owner device refusal and verification of the earlier R118 owners' actual retirement. All archived ordinary hashes were checked; feature bytes remain preserved. This restores historical artifacts and does not claim current-source ordinary software revalidation. Five scoped build tasks remain; work is paused at the owner's request.
