# T5.04 Android caller handoff — 2026-10-18

**Disposition:** local Android caller candidate wired in source; uncompiled and
unaccepted. No installed-app or physical user outcome is claimed. Remote Dialog
activation is held for one missing path grant, with its exact patch prepared.
Krita, device activity and paid services remain deferred.

## Source and ownership

Frozen caller base: `f661eca9d2f6d6d6ebe4fc31ed2d0a5c6ec095e0`, fetched by full
SHA before creating `codex/t5-04-air-actions-callers`. Exact grant A is
[`CALLER_PARALLEL_GRANTS_20261018.md` at 30b7761](https://github.com/strickerca/visual-workbench/blob/30b77615fe0ce79b2b1a044a0704ae2c7d186c14/docs/CALLER_PARALLEL_GRANTS_20261018.md).
Read its full paths/interfaces, AGENTS, current Android callers and accepted
shared interfaces before editing. There are no repository `.agents/skills`.

Existing modified paths:

```text
apps/android/src/main/kotlin/com/visualworkbench/android/MainActivity.kt
apps/android/src/main/kotlin/com/visualworkbench/android/editor/EditorController.kt
apps/android/src/main/kotlin/com/visualworkbench/android/editor/CanvasSurface.kt
apps/android/src/main/kotlin/com/visualworkbench/android/editor/EditorPreferences.kt
apps/android/src/main/kotlin/com/visualworkbench/android/ui/WorkbenchApp.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionKeys.kt
apps/android/src/main/AndroidManifest.xml
```

New owned leaves and tests:

```text
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionCommands.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionBarrelCorrelation.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionControls.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionDialogControls.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionPreferences.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionViewRestore.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionCallerTest.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/check_caller_source.py
```

This is the only handoff document changed. No Desktop, shared, native, bindings,
protocol, Gradle, Cargo, MCP or unrelated Android file changed. Accepted shared
router SHA-256 remains
`7dc09b08cbccc73ec6861a3278a3edcc5cf70834f7a5e8a6fdcda5a87d453c04`.

## Implemented caller behavior

- Activity sends each full KeyEvent once through the accepted `AirActionKeys`
  before its original fallback. A retained editor owns the existing serial
  `CommandDispatcher` and accepted `AirActionRouter`; the Activity owns only its
  callback/timer. Pause, focus loss, replacement and disposal retire presses and
  queued offers. Activity recreation detaches its timer without closing the
  retained router; final ViewModel disposal closes it.
- Touch and hover observe accepted MotionEvent barrel state before the existing
  PrecisionButtons handler. Its contact ordering, hold clock, peek, click,
  cancellation and pen-up behavior remain authoritative. No synthetic F keys or
  second pointer pipeline was added. Ordinary canvas cancellation preserves the
  observed barrel hold/release correlation; full input/focus loss clears it and
  resets PrecisionButtons. Native renderer/reader methods are unchanged.
- The local command guard combines current activity/context tickets with the
  existing workflow, precision, draft/modal, AI/result, operation/pending, contact,
  renderer and undo/redo guards. Ownership changes retire tickets synchronously;
  native remote invalidation also increments the ticket on its calling thread
  before Main publication. Queued UI controls check ownership and freshness again.
- All eight safe commands have visible Canvas controls, independently of hardware
  setup. Lens commands use PrecisionController; tool stepping uses the actual
  visible EditorTool order, including the selected mask eraser. Undo/redo uses
  the same native commit helper as existing UI, under the existing mutation lock,
  and awaits completion. It never treats a detached launched operation as done.
- Fit/Restore saves the exact borrowed Camera, including rotation, and matches
  project epoch/document/page/viewport ownership. Intervening navigation and
  ownership replacement clear it; a second Fit is not substituted for Restore.
- Settings exposes the accepted setup/remapping panel. Stable gesture/command
  enum names and explicit null/disabled entries use the existing preference
  writer and schema, preserving brushes and unrelated settings. Unknown command
  names disable only that known slot. Support, connection, interference and key
  observations remain ephemeral. Receiving a key never infers Bluetooth support.
- MainActivity has exactly one REMOTE_ACTION registration, pointing at the
  accepted eight-key declaration. The manifest permission set is unchanged.

## Remote hook scope blocker

Grant A omits `apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteEditPanel.kt`.
That existing file owns both the separate Dialog Window and its exact
RemoteEditController.palette. Its Window does not use Activity.dispatchKeyEvent.
An async scope question is pending; this file remains untouched.

The prepared, unapplied patch is:
`/tmp/vw-air-actions-remote-dialog-hook/remote-dialog-hook.patch`.
`git apply --check` passes against this candidate. It adds one defaulted composable
slot to RemoteEditPanel and passes the existing palette plus closing/command-busy
state from inside its Dialog; WorkbenchApp connects that slot to the owned
AirActionDialogControls. No RemoteEditController edit or replacement is needed.

AirActionDialogControls is implemented as an inactive leaf pending that hook.
It borrows/restores the real Dialog Window.Callback before fallback, fences its
focus and callback owner, and borrows the exact existing palette. Only verified
Paint Undo/Redo map remotely; there is no local undo fallback, generic key
injection, new grant, guessed fit/restore or new tool order. Its serial command
waits for the palette's actual busy interval to end. Six unsupported remote
commands remain disabled. Until the slot is admitted/applied, **all remote Air
Actions remain unavailable**, without affecting the existing remote controls.

## Executed evidence and remaining gates

Source-only command:

```sh
python3 apps/android/src/test/kotlin/com/visualworkbench/android/airactions/check_caller_source.py
```

The audit passes lexical delimiter, exclusive-path, unchanged-permission,
one-Activity registration, barrel-order, shared-commit and exact frozen-method
checks. It confirms preserved giant renderer, document observers, camera/share
operations and retirement logic; it is not a Kotlin type checker or Android test.
Source-hash receipts are retained at `/tmp/vw-air-actions-callers-source-r4.json`
and subsequent final receipts. Whitespace checks pass.

Thirteen new JUnit caller regressions are written for legacy preference migration,
disabled/remapped values, malformed/unknown command refusal, unrelated preference
preservation, exact camera restore, stale project/document/page/viewport/navigation
refusal, barrel correlation across ordinary/full interruption and the closed
remote mapping. **These thirteen tests have not been compiled or executed.**
The prior 37 accepted leaf scenarios and their original receipts
remain historical evidence, not a rerun of this caller candidate.

The image lane holds the heavy slot. Grant restrictions prohibit compiler/build
processes while it is occupied; no Kotlin, Gradle, native build, AAPT2 or device
process ran for this caller slice. Preserve the existing `/tmp/vw-air-actions-*`
leaf success/failure receipts; no drafts or unrelated worktrees were removed.

Next: obtain the one-file remote slot grant, apply/review its prepared patch,
then Astra reviews the complete caller candidate before any main merge. With the
serialized slot, run the repository-pinned Android/shared compilation and focused
JUnit checks, then actual UI/lifecycle/recreation/modal and native completion
checks. Runtime capability/interference detection, S23 stylus/Bluetooth/barrel/Air
Actions, A09 and all physical/performance/product gates remain unverified.
