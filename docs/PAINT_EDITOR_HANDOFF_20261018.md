# Paint/editor lane candidate — 2026-10-18

P lane activated by owner delegation from thread
`01a118d0-0d97-758f-8db3-446b7bbf4005`. Integration coordinator:
`01a14eea-d204-762c-94b8-f07105b5001e`.

Base: `6d505416963fb42009921bd3fb8639994e73752a`, exactly as prescribed by
`docs/AI_EDITOR_RESERVED_BOUNDARIES_20261018.md` at
`79d7d7fc20b8bc99c4b5990507a7cf65570f12f7`.
Candidate branch: `task/paint-editor-controls-20261018`.
Code tip: `3a19fd0d97b26e24f4531cb9b6a0ec5c65cae2c7`.

Commits:

- `62773c3`: native Paint Size eligibility, bounded observation predicates and
  dedicated native probe.
- `3a19fd0`: shared palette/session fences, both actual palette panels,
  compatibility presentation and focused shared regressions.

Read `AGENTS.md`, the implementation plan/roadmap, current native profile and
Paint implementations, shared remote contracts and both actual panels before
editing. Neither this checkout nor `/workspace/.agents` contains relevant
`.agents/skills` files. Existing controls, guarded injection, source/package
leases and runtime ownership were reused. No recovered diagnostic overlay was
applied. Krita source was preserved and its implementation/reactivation remains
deferred.

## Exact ownership and changed files

Native, under `host-win/crates/vw-remote-host/src/platform/shortcuts/`:

- `paint_controls.rs`
- `paint_controls/production.rs`
- new `paint_controls/compatibility.rs`
- new `paint_controls/tests/run_probe.py`

Shared, under `apps/shared/src/`:

- `commonMain/kotlin/com/visualworkbench/shared/RemoteEditorPalette.kt`
- `commonMain/kotlin/com/visualworkbench/shared/RemoteEditorCompatibility.kt`
- new `commonTest/kotlin/com/visualworkbench/shared/RemotePaintPaletteCases.kt`
- new `commonTest/kotlin/com/visualworkbench/shared/RemotePaintPaletteTest.kt`
- new `commonTest/paint/run_probe.py`

Actual callers:

- `apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteEditorPalettePanel.kt`
- `apps/desktop/src/main/kotlin/com/visualworkbench/desktop/remote/RemoteEditorPalettePanel.kt`

This dedicated handoff is the only documentation addition. The central deferred
ledger is coordinator-owned, so its proposed rows are below. No controller,
Main/WorkbenchApp/Activity, RemoteEditPanel, dispatcher, shared RemoteEditApi,
NativeRemoteEdit, input guard, WGC/HEVC, process/job root, package identity,
manifest, build root, lockfile, protocol or generated binding was edited.

## Resulting behavior and additive interfaces

Paint Size now requires the current Slider itself to be eligible, enabled and
onscreen in addition to its independently checked Thumb. Current numeric facts
and the admitted shape are revalidated before authority/preparation/final
validation. Display-only Value text remains allowed when the reviewed RangeValue
route is writable. Failed Size discovery is reused only within that single
bounded authority observation, avoiding a second identical failed provider walk
for the opposite Size action. No element/failure cache crosses requests or
stages. The pure numeric observation predicate preserves the previous read-only
observation semantics; it supplies no pixel mapping or input permission.

A Paint frame witness now rejects QPC values whose nanosecond conversion would
overflow at every witness-validation caller, including unchanged bracketing.

The shared palette refuses stale producer snapshots at command admission and
checks current producer session/grant state before adopting a receipt/error.
Observed grant, binding, identity, pause or canvas-owner changes advance a UI
revision. A validated partial-input receipt seals its input session even when
that observational revision changed or the target temporarily disappeared.
Geometry/proof refresh does not clear that seal. Only a distinct connection or
input-session ID clears it. Serial admission, no queued/retried commands and
joined cancellation remain intact. An Injected message still says editor effect
is unverified; it never claims a visible result or invokes local undo.

Both real panels use the shared availability explanation. Invalid/dual/stale
proofs and absent current routes are visibly unavailable. Canvas class/type and
physical rectangle checks match native structural bounds, retaining negative
origins. Selected-canvas authority still permits only the original closed
essential set. The action enum/order and command signatures used by Air are
unchanged.

Compatibility rows name their observed tool/settings, method, actual Mouse or
WindowsPointerInput API, and device when provided. Untested aspects remain
untested. Unknown editor/input observations cannot become supported evidence.
These imported historical records never enable commands or infer current tool
support.

Additive shared surface, already used by both owned panels:

- `RemoteEditorPaletteState.targetRevision: ULong = 0uL`
- `RemoteEditorPaletteState.sealedBinding: RemoteTargetBinding? = null`
- `remoteEditorActionUnavailableReason(RemoteTargetState?, RemoteEditorAction): String?`
- `remoteEditorPaletteStateReason(RemoteEditorPaletteState): String?`
- `remoteEditorCompatibilitySummary(RemoteEditorPaletteState): List<String>`

No FFI/schema/enum registration or generated ABI change is required by this batch.
The new state fields are UI fences, not a native capability or grant.

## Focused evidence actually executed

Source every worker shell with `/workspace/.vw-tools/cloud-setup/start.sh`.
These checks use cached tools, no dependency resolution, full Gradle/Cargo build,
Windows/editor/device execution, provider request, signing or installation.

| Command from repository root | Actual result | Scope and limit |
|---|---|---|
| `python3 apps/shared/src/commonTest/paint/run_probe.py` | Exit 0; strict production compilation and 14 synthetic scenarios pass | Actual unchanged production RemoteEditApi, palette, compatibility and profile-evidence sources; cached Kotlin compiler 2.4.0, explicit API, warnings denied, Java 21 target and pinned coroutines 1.9.0. No generated/native/Compose stubs. This is a leaf probe, not the project's Kotlin 2.4.20 Gradle build. |
| `python3 host-win/crates/vw-remote-host/src/platform/shortcuts/paint_controls/tests/run_probe.py` | Exit 0; 3 Rust tests pass, zero failed/ignored | Exact pure production module compiled unchanged as a child with edition 2024 and warnings denied; no Windows replacement types or native grants. |
| `rustfmt --edition 2024 --check host-win/crates/vw-remote-host/src/platform/shortcuts/paint_controls.rs` | Exit 0 | Recursively includes the changed production/compatibility children. |
| `git diff --check` and staged text credential-pattern scan | Exit 0 | All 11 code/test files; heuristic private-key/provider/GitHub/AWS token checks, not exhaustive secret or product-security acceptance. |

Final shared output excerpt:

```text
Paint palette synthetic cases passed: 14
Exact production leaves passed explicit API / warnings denied; temporary classes disposed.
```

The 14 scenarios include malformed proofs, unavailable routes, delayed observed
revoke/restore success/refusal/failure, partial input/new-session behavior,
delayed partials with missing/restored targets and geometry changes, toolbar
focus transition, busy/no-queue/joined close, scoped compatibility claims,
queued-observer admission and old receipt completion before target collection.
The JUnit wrapper invokes these scenarios as one aggregate test; no JUnit/Gradle
suite was run here. Two additional Windows production Size tests are written but
unrun. They cover an ineligible Slider behind a visible Thumb and invalid current
numeric values even with an otherwise matching shape.

Intermediate receipts are retained: the original 10-scenario leaf run and 3 pure
Rust tests passed; review found a delayed-partial local-seal bug, which was fixed
with two regressions. The first strict compile after that repair failed solely
on an unnecessary `!!` warning, ran zero scenarios and was corrected. A subsequent
12-scenario run passed, then the final queued-observer addition passed 14. An
initial scratch runner emitted a duplicate-classpath warning; the committed
runner uses one classpath and the final run emitted none. This history is not
promoted into Windows/runtime evidence. Task-owned local review/failed-source
receipts are under `.local/paint-editor-review/`; existing drafts/recovery
archives/data were not removed. No screenshots were created. Probe classes and
executables were disposed by their temporary-directory owners.

## Independent review and coordinator hooks

Independent read-only review was requested with `gpt-6-astra / xhigh`. The tool
accepted that requested configuration; the child session could not independently
expose its actual model/effort. It found the delayed-partial issue and a missing
input-method label; both were repaired and reviewed. Its final review found no
remaining in-grant blockers, including the queued-observer addition. It ran no
builds or editor/device actions. Reviewed SHA-256:

- Palette: `b9eaf488679520d7603d142c545118c71abf3ac14b268cfdef054af013baa2a5`
- Synthetic scenarios: `dd6f5099e2cba1a2ff9a73150d85be1fa77d5586ebae168e3aa0a99396730245`

The current native `core/crates/vw-ffi/src/session/remote_host.rs` grant path
creates a fresh `input_session_id` at lines 787–790 and installs the binding before
publication at lines 837–843. Preserve and verify that existing contract during
integration; no ABI expansion is needed for it. Air sources are not on this base,
so the coordinator must check the actual composed Air caller against the unchanged
enum/order and serial command contract.

Known bound: `targetRevision` observes a conflating StateFlow; it cannot detect a
revoke/restore that happens entirely between collections and returns to identical
producer values/binding. The direct producer checks cover a changed current
binding/grant even while collection is delayed. The queued tests prove that case,
not impossible reconstruction of skipped same-binding history. If a future root
path restores grants with identical input-session IDs, root must supply a producer
authority generation or nonconflating invalidation. That hypothetical behavior
must not be enabled by this lane. Native guards remain authoritative for every
actual input, regardless of UI observation timing.

## Proposed deferred ledger rows (coordinator copies into central ledger)

All rows are for code tip `3a19fd0` plus its eventual coherent integration tree.
None is passed by the cheap leaf checks above. Allocate one heavy slot globally,
at most two build workers, and retain exact final source/artifact receipts.

| Requirement / phase | Exact deferred command or journey | Prerequisites | Expected outcome | Status |
|---|---|---|---|---|
| T4 native Paint/profile/guards | On Windows, `./build.ps1 test-remote-helpers -TimeoutSeconds 900` | Final reviewed tree; approved Windows Rust/toolchain; existing license gate passes; root heavy-slot allocation | Affected helper/remote/capture tests, including the new real production Size cases, strict Clippy and format pass; no forged capability/effect admission | Deferred; only 3 pure predicates executed here |
| T4 shared palette and compatibility | `bash apps/gradlew -p apps :shared:desktopTest --tests 'com.visualworkbench.shared.RemoteEditorPaletteTest' --tests 'com.visualworkbench.shared.RemotePaintPaletteTest' --tests 'com.visualworkbench.shared.RemoteEditorCompatibilityTest' -Pkotlin.incremental=false --no-daemon --no-parallel --max-workers=2` | Final-tree dependency/license check; normal root-managed generated/native prerequisites; heavy slot | Existing palette/compatibility cases and the new 14-scenario JUnit wrapper pass under the pinned project compiler | Deferred; leaf compiler/scenarios pass separately |
| Actual Paint palette callers and Air composition | `bash apps/gradlew -p apps :desktop:compileKotlin :android:compileDebugKotlin :android:testDebugUnitTest :android:lintDebug -x :desktop:packageConnectionRuntime -Pkotlin.incremental=false --no-daemon --no-parallel --max-workers=2` | Final Air/controller composition and normal ABI/runtime prerequisites; licenses pass | Both actual Compose panels compile; Air uses unchanged action mapping and no queue/retry; zero lint errors | Deferred; no Compose/app acceptance claimed |
| T4 fresh-grant producer binding | Revoke a controlling grant, issue a fresh grant while palette collection is delayed, complete the old command before delivering the target update, then inspect the published target and message | Real final native/shared caller; separately authorized owned blank Paint window or root's current native session fixture | Regrant has a distinct input-session ID; no stale dispatch/message/compatibility; input retirement joins before reuse | Deferred; native source contract reviewed, queued shared cases pass |
| T4.07/T4.08 normal Paint controls | Select exact installed Paint/source/profile, connect the final paired apps, explicitly grant the owned blank window; exercise Undo/Redo, brush/eraser, smaller/larger and first-use Zoom in/out; verify visible effect and exact restoration per action. Check Fit/100%/Pan stay visibly unavailable without admitted routes. Draw, lift, then verify automatic Undo eligibility and final Undo. | Windows/Paint matching admitted package/image/profile; current packaged catalog; source-bound final artifacts and separately authorized selected S23/user 0 | Actual eight catalog actions operate only on the current eligible tool/settings; missing routes stay unavailable; Injected remains distinct from effect; no local-history fallback; restoration independently proved | Unrun; installed version/runtime/catalog availability is an external gate |
| T4.03/04 lifecycle and retirement | In the same exact owned target, test active contact, grant expiry/revoke, focus switch, target change/move/resize/minimize, phone background, disconnect/reconnect and normal app shutdown; repeat target changes 100 times; verify balanced release and fresh grant after reconnect | Final native/apps, separately authorized Windows/device endpoints; preserve unrelated apps/profiles; exact process/IO retirement receipts | No stale/stray input, no queued commands, joined retirement; partial input stays sealed for its session and normal shutdown retires all owners | Unrun; 14 synthetic UI scenarios do not pass native/device guards |
| T4 viewing/ghost ink/performance and owner acceptance | Final normal paired Paint session: sustained viewing, physical S Pen drawing, acknowledged ghost-ink fade/500 ms expiry; measure at least 30 fps and pen-to-host-to-phone p95 <=80 ms; obtain actual owner drawing acceptance | Final source-bound artifacts, actual selected S23 and physical owner gestures; Windows capture/input runtime | Record measured device/version/carrier/codec data and retained failures; no inferred physical pressure/hover/tilt/button support | Open; all physical/product gates remain pending |

Final normal package assembly, license inventory, notices/checksums, actual device
installation and all broader phase acceptance remain in the coordinator's final
batched validation. No paid provider call, purchase, credentials, new external
access, signing, public release, device action or merge to main occurred. Five
scoped Paint milestone build/acceptance tasks remain open; this candidate does
not reduce that count or pass M4-KP/G4. Krita and broader editor work stay deferred.
