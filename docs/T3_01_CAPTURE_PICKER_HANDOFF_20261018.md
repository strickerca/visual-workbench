# T3.01 capture-picker integration handoff

Source branch: `task/t3.01-capture-picker-leaves-20261018`, frozen source head
`ff218817e63d541462b7ed5395416abadd697afd`, based on main
`6f3e2f3b9ed7a760d4dd7f4c6f26fa44dc5ad88d`.
The three source commits are native `fca3bd2`, host `0639851`, desktop `ff21881`.
Only the 13 assigned, previously absent leaves were added. Their source is
unchanged by this handoff. Recovery `0d91b705` and later `47339519` contain the
same picker candidate; all 13 archived sizes/SHA-256 values were checked.

## Exact prerequisite hunks

[capture-picker-prerequisites.patch](../integration/t3-01/capture-picker-prerequisites.patch)
is **unapplied**, coordinator-owned integration material. It passes
`git apply --check` against the source branch. It adds only:

- The existing private `vw-geom` dependency and its `Cargo.lock` edge.
- Core picker/Windows module registration and `capture_picker` re-export.
- Distinct `Request::{PickerCatalog, PickerCapture}` and
  `Response::{PickerCatalog, PickerFrame}` variants, helper dispatch/watchdog
  deadlines, and process-side admission/timeouts.
- `region` admission in `FrameIdentity` and the core capture importer.
- Host `capture.rs` child-module registration/re-export.

The patch contains no archived integration postimage and changes no picker leaf.
No `windows/wgc.rs` prerequisite is needed: the owned monitor readback stays
private in `windows/picker_capture.rs`, preserving current window diagnostic and
canvas paths. Reconcile these hunks with current coordinator changes, format
them, and regenerate host bindings once in the coherent build phase.

## Shared and desktop root hooks

Coordinator supplies the recovered common `CaptureRegionEditor` API:

```kotlin
class CaptureRegionEditor(map: (Camera, Boolean, List<Point>) -> List<Point>) {
    fun bind(source: String, width: UInt, height: UInt, camera: Camera)
    fun begin(physicalPoint: Point): Boolean
    fun move(physicalPoint: Point): Boolean
    fun finish(physicalPoint: Point): Boolean
    fun cancel()
    fun detach()
    val contacting: Boolean
    val edges: List<Double>?
    fun completedEdges(): List<Double>?
}
```

Use current `Point`, `Camera` and `WorkbenchCore.mapPoints`; the declaration
above specifies signatures, not a replacement implementation. No shared target
DTO is required. The host leaves retain the recovered catalog, selection,
`PickerFrame.info/preview/cropRegion/collect`, service and region-hotkey APIs.
`cropRegion` reads the exact hashed monitor snapshot rather than recapturing.

Retain one picker on the UI dispatcher. The constructor call shape is:

```kotlin
val pickerModal = PickerModal(controller::instructionPicker)
val picker = CapturePickerController(
    scope, core, location.path, device,
    openService = {
        openDesktopCaptureService(
            nativeRuntime.directory.resolve("vw-capture-helper.exe"),
            nativeRuntime.packagedHash("vw-capture-helper.exe"),
        )
    },
    saved = controller::captureDestination,
    modal = pickerModal::set,
    mayOpen = ::currentPickerAdmission,
)
```

`currentPickerAdmission` means a local predicate over **current** capture
protection, quitting, command and competing-modal state; it is not a new
controller API. Include project/native/document choosers, diagnostics, settings,
instruction/AI/export/paste/text dialogs, remote view and MCP/Codex state as
applicable. Every open, including a hotkey wakeup, calls this predicate. Do not
freeze these values in `remember`. Acquire this picker's independent focus lease
synchronously before showing it; retain the lease on failed close.

The service factory must return a separately owned hash-pinned service, never
the handle owned by `DesktopCaptureController`. The existing
`controller::captureDestination` factory suffices: it captures route/revision
before capture, and its returned callback owns and closes the project in a
`NonCancellable` finally even on refusal/cancellation. No additional editor
factory or phone transport is required for these local leaves.

Collect `picker.state`; render `CapturePickerWindow(picker, controller::report)`;
add a gated local action invoking `picker.open()`. Include `state.open ||
state.cleanupPending` in competing-modal/shortcut/route checks. Ctrl+Alt+R is
explicitly enabled/remapped in the picker and only requests `open(regionOnly =
true)`; it never captures or chooses a source itself. Construction starts no hook.

Use `initializePickerProtection { excludeCaptureWindows(listOf(windowHandle)) }`
for the main HWND: Pending keeps it hidden; Ready permits capture; Unavailable
shows the editor with capture disabled. Cancellation propagates. Every secondary
HWND must be excluded before visibility; the picker handles its own HWND.

Expose **Retry capture cleanup** from the visible editor/menu whenever
`state.cleanupPending`, outside ordinary command gating. Launch `picker.dismiss()`,
catch/report refusal, and retain its owner. Initial exclusion failure can leave
the picker hidden, so its own close button is insufficient. Do not show an
unexcluded fallback. On app/route retirement, await `picker.shutdown()` before
closing core/runtime/scope. A thrown cleanup failure retains those owners and
the modal; a second no-op native destruction is not retirement proof.

## Private retained-recapture intersection

Verification proposal `6591961d582a012b23b3b0cec58c1c7f711f1b93`,
`docs/T5_01_VERIFICATION_HANDOFF_20261018.md`, remains separately awaiting
admission. Its hook is `rememberCapture(info: PackageInfo, selector: String)`
after exact capture/package settlement while the target route remains retained.

These leaves keep the selection in private `Draft.selected` and retire that draft
after successful save/dismiss. Their saved callback has no `PackageInfo` or
selector. A saved project/PNG is not a retained OS target. Coordinator admission
must connect the existing exact picker/native target owner and its retirement
to the opaque selector at save/package creation, then invoke the proposed hook.
No second target model, verification identity, selector field, grant or recapture
API was added here. Do not assume the verification proposal or its patches merged.

## Checks and remaining acceptance

- Native pure projection: **16/16 passed**, using exact method bodies with serde
  removed and required region admission projected. Three serde/wire cases and
  the 32-bit case remain unrun; this is not full crate compilation.
- Desktop helper projection: **27/27 repository test methods passed**, with
  literal `Point`/`Camera`/`PickerPreview` DTO projections. The controller compiled
  against explicit API projections; all six files passed Kotlin PSI parsing.
  These are not generated-binding/Compose/Gradle app checks.
- Seven Rust leaves passed rustfmt; whitespace, explicit staged-path/privacy
  checks and independent host FFI source review passed. Eight host PNG/crop tests
  are written but unrun. No heavy build slot was granted or used.
- Full native/serde/Clippy, actual bindings/Compose, Windows capture/exclusion/
  hotkey/lifecycle, physical DPI/negative-origin, GPU, device and performance
  acceptance remain unrun. No product/phase gate or milestone task is passed.

Own-window checks remain fail closed. Actual Compose HWND behavior needs Windows
validation: [GetWindowDisplayAffinity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdisplayaffinity)
documents layered-window/DWM conditions. Published preview images follow the
existing Compose/Skia render lifetime; native/GPU retirement is unmeasured.
No captures, screenshots, credentials or machine/device identifiers are included.
The bounded runner fetched temporary JUnit 4.13.2/Hamcrest 1.3 test jars; no system
tool or repository dependency installation/change was performed.

Next: coordinator admission and serialized real native/binding/app validation
(M; standing owner override `gpt-6.1-sol` / `xhigh`). Main, reserved source,
AWT Import/Open repair, phone/live-stream composition and deferred Krita remain
unchanged by this branch.
