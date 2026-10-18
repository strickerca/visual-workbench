# T3.01 Desktop capture caller candidate

Source: `490ed8ba471fe13391c542fa389a7261f6d21af3` on
`task/t3.01-desktop-capture-callers-20261018`, based on exact
`f661eca9d2f6d6d6ebe4fc31ed2d0a5c6ec095e0`.
Authority: grant B in
`30b77615fe0ce79b2b1a044a0704ae2c7d186c14:docs/CALLER_PARALLEL_GRANTS_20261018.md`.
This is a source candidate for Astra review and the coordinator's coherent
compile/test batch. No compiler or Kotlin test ran in this lane; the coordinator
owns the heavy slot. No product, Windows, device or performance gate is passed.

## Changes and exact caller hooks

- Main retains the recovered window/monitor/drawn-region picker and its separate
  capture service, opened with the packaged helper path and packaged SHA-256.
  Foreground capture continues to own its original service. Neither controller
  registers a hotkey on construction; both hotkeys require an explicit UI action.
- Both caller admission closures use `rememberUpdatedState` and read current
  editor/modal/workflow/diagnostics/protection/quitting state. The picker also
  refuses foreground busy, cleanup-pending or stopping state. The MCP check uses
  the real `McpLifetimeState.owner` and its actual coordinator state, including
  active capture, closed/start/failure state. It adds no MCP DTO, grant or selector.
- Picker focus acquisition calls `cancelInput()` before the independent
  `PickerModal` lease and before publishing open state. A failed lease release
  retains the lease. Main's visible **Retry capture cleanup** action remains
  available outside normal command gating when an excluded picker disappears.
- Main HWND protection is bound to the actual window instance and settles before
  editor visibility. Exclusion cancellation propagates. A refused exclusion shows
  the editor with capture unavailable; the picker never shows an unexcluded UI.
- `CapturePickerController.save(preview)` accepts only the exact currently
  presented `PickerPreview` object. `previewPresented(preview)` is acknowledged
  after that bitmap's Canvas draw and a subsequent frame callback. Save and
  region input stay disabled during asynchronous bitmap preparation. This is
  Compose draw sequencing, not proof of physical/GPU presentation.
- Region input supplies the exact preview and camera to begin/move/finish/cancel;
  stale pointer cleanup cannot erase another source/camera's contact. The common
  helper validates physical viewport and camera transforms, detaches invalid
  replacements, retires invalid moves and fences reentrant mapping. Public shared
  declarations are unchanged. Edges remain unrounded original-frame pixels;
  native crop owns negative host origin, freshness and final pixel-edge snapping.
- Main first settles diagnostics and rechecks abortable departure predicates,
  then calls `withRetiredCaptureCallers(picker::shutdown, foreground::shutdown)`
  before any parent teardown. Both sources must finish before the parent callback
  runs. Cancellation cannot skip joins. A source failure keeps native runtime,
  host, editor and other parents alive. An aborted departure does not permanently
  retire capture. Stopping state prevents advertising a partly retired picker.
- Foreground now publishes busy/stopping/cleanup state, joins its poll and capture
  work, retains late frames, and keeps failed native release owners. Repeating a
  no-op destructor cannot turn an earlier unknown outcome into success. Multiple
  cleanup failures retain all affected owners and the original failure.
- `deliverCaptureProject` passes the actual native project unchanged. For the
  frozen `EditorController.captureDestination` implementation, ordinary delivery
  failures are handled there and its finally closes under `NonCancellable`.
  Escaping non-cancellation close failure retains that project and a sticky
  failure in the caller. Ordinary callback cancellation after successful close
  stays cancellation. This interpretation is bound to the current destination,
  `NativeProject.close`, `releaseNative` and typed native error mapping; it is not
  a promise for arbitrary project implementations. Their unchanged hashes are in
  the receipt. No project wrapper or second native close is used.
- Private picker workspace cleanup retains final directory-deletion progress.
  With an exact file key, retry accepts only the same empty directory. A new child
  or replacement refuses deletion. Normal marker-owned deletion works when the
  provider has no file key. An unmarked retry without that identity remains held;
  no timestamp/path identity substitute is used. OpenJDK's Windows Java 17
  [provider source](https://raw.githubusercontent.com/openjdk/jdk17u/master/src/java.base/windows/classes/sun/nio/fs/WindowsFileAttributes.java)
  returns a null key, so Windows final-deletion refusal remains a runtime limit.

The initial native prerequisite patch is already present in the frozen base.
**No additional shared/native/registration/binding prerequisite hunk is needed.**
Main's existing EditorShell/rendering suffix is byte-identical to the base.
Neither EditorController, diagnostics module, MCP coordinator, Android root,
native module, manifest, lockfile nor generated binding was edited. The recovery
archive and older leaf handoff are preserved; no archived integration patch was
applied. No ordinary file chooser is added around region capture.

Capture project/PNG provenance does not become a retained OS target. There is no
intersection with private `McpVerificationTargets` or phone requested-region/live
stream composition in this candidate. A future exact native target transfer
still requires its explicit separate owner contract.

## Evidence and pending validation

Run the cheap check with:

```sh
source /workspace/.vw-tools/cloud-setup/start.sh
python3 evidence/T3_01_DESKTOP_CALLER_STATIC_CHECK_20261018.py
```

The retained [JSON receipt](../evidence/T3_01_DESKTOP_CALLER_SOURCE_20261018.json)
binds all 15 source/test paths to the source commit. Checks passed for exclusive
path ownership, source hashes, whitespace, `git diff --check`, lexical delimiters
(not a Kotlin parser), unchanged shared declarations/platform-neutral imports,
unchanged rendering suffix, source hook/order assertions and independent fixture
arithmetic. A bounded credential-marker scan and manual staged-diff review found
only source and fake test fixtures; no verification images or native artifacts.

There are **60 written tests, 24 new**, across CaptureAdmissionTest,
CaptureCallerRetirementTest, CaptureDeliveryTest, CapturePickerTest and
CaptureRegionEditorTest. They cover delayed/cancelled joins, abortable departure,
failed native/destination cleanup, exact preview identity, file-key/no-key cleanup
branches, stale/reentrant geometry, physical viewport/density/fractional borders.
**Zero Kotlin tests executed here.** Historical projected leaf tests do not
validate this caller candidate.

The coordinator must allocate the focused shared/Desktop JVM tests and coherent
app/binding compatibility build, then review actual Compose/Windows behavior:
window/monitor/region selection, stale/destroyed targets, mixed DPI and negative
monitor origin, exclusion success/refusal/cancellation and hidden retry, bitmap
draw/region/save identity, hotkey conflicts/disable, delayed cancellation and
shutdown failure retention. Windows final unmarked cleanup without a file key
must retain its refusal. UIA/WGC quality, hardware input, GPU/physical presentation
and latency/performance acceptance remain separate and unrun. No device, install,
paid service, security, credential or destructive cleanup action was performed.

Next: coordinator composition plus focused validation and independent Astra
review, medium task, requested `gpt-6.1-sol` / `xhigh`. Reused delegated reviewer
and shared-helper executor used those requested settings; this primary session's
actual model/effort is not exposed, and no app settings were changed.
