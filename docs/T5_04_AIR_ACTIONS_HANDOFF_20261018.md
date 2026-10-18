# T5.04 Android caller handoff — 2026-10-18

**Disposition:** Android local and remote caller candidate wired in source; full
caller compilation and acceptance remain pending. No installed-app or physical
user outcome is claimed. The extra remote Dialog grant is resolved. Krita, device
activity and paid services remain deferred.

## Source and ownership

Frozen caller base: `f661eca9d2f6d6d6ebe4fc31ed2d0a5c6ec095e0`, fetched by full
SHA before creating `codex/t5-04-air-actions-callers`. Exact grant A, including its additional RemoteEditPanel path, is
[`CALLER_PARALLEL_GRANTS_20261018.md` at 074b6c6](https://github.com/strickerca/visual-workbench/blob/074b6c603f68f19e137b7d5c0ee0a60f88c8e855/docs/CALLER_PARALLEL_GRANTS_20261018.md).
Read its full paths/interfaces, AGENTS, current Android callers and accepted
shared interfaces before editing. There are no repository `.agents/skills`.

Existing modified paths:

```text
apps/android/src/main/kotlin/com/visualworkbench/android/MainActivity.kt
apps/android/src/main/kotlin/com/visualworkbench/android/editor/EditorController.kt
apps/android/src/main/kotlin/com/visualworkbench/android/editor/CanvasSurface.kt
apps/android/src/main/kotlin/com/visualworkbench/android/editor/EditorPreferences.kt
apps/android/src/main/kotlin/com/visualworkbench/android/ui/WorkbenchApp.kt
apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteEditPanel.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionKeys.kt
apps/android/src/main/AndroidManifest.xml
```

New owned leaves and tests:

```text
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionCommands.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionBarrelCorrelation.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionRemoteMotion.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionRemoteMotionEvent.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionControls.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionDialogControls.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionPreferences.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionViewRestore.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionCallerTest.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/check_caller_source.py
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionRemoteMotionContract.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionRemoteMotionTest.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/check_remote_motion.py
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

## Remote Dialog composition

The coordinator's explicit extra grant at `074b6c6` resolves the earlier path
blocker. Applied the exact prepared patch: RemoteEditPanel adds one defaulted
composable slot inside its existing Dialog and passes its exact
RemoteEditController.palette plus closing/command-busy state. WorkbenchApp wires
that slot to AirActionDialogControls. Its prior input, lifecycle and retirement
body is byte-for-byte preserved after removing this one slot call. No new remote
controller or palette is created, and no existing native owner is replaced.

AirActionDialogControls borrows/restores the real Dialog Window.Callback before
fallback, fences focus and the exact callback/palette owner, and exposes the
same guarded visible controls. A retired Dialog callback cannot dispatch into a
replacement palette. Only verified Paint Undo/Redo map remotely; there is no local
undo fallback, generic key injection, new grant, guessed fit/restore or new tool
order. Its serial command waits for the palette's actual busy interval to end.
Six unsupported remote commands remain disabled. The former unapplied patch is
preserved at `/tmp/vw-air-actions-remote-dialog-hook/remote-dialog-hook.patch` as
historical preparation evidence; it is now composed in this candidate.

## Astra remote-motion correction

Astra found that a delayed Air Undo could survive pen contact and execute after
its covering ACK made native commands eligible again. The Dialog callback now
observes touch AND generic motion before forwarding each original event exactly
once, preserving the previous delegate result/exception. Read-only snapshots
contain only tool/pointer/contact/cancel/primary-barrel information; the real
RemoteSurfaceView and RemoteEditController input source remains unchanged.

Pen-down retires pending/queued Air offers before marking contact active. Air
commands are refused throughout contact; up/cancel retire again before clearing.
Barrel observations use the existing correlation/router/timer. Focus, detach and
replacement retire/clear the observer. Every callback now also carries an exact
per-Window registration token, so even replacement reusing the same remote/palette
cannot let an obsolete callback change the new owner's state. Conditional Window
callback restoration is preserved. Observer failure blocks Air before forwarding.

The new production observer + real AirActionRouter contract contains **17 named
scenarios**, including the exact 1000 configure, 1001/1002 SwipeLeft, 1010–1040 pen,
1100 simulated ACK, 1252 advance case (expected zero commands), with an unhooked
negative control (expected Undo). It also covers queued offers, contact keys,
cancel/focus/detach/replacement, barrel-remapped click suppression, fresh idle
Undo, and motion forwarding identity/order/result/exception. Native readiness/ACK
is a fixture flag, not an actual native or device measurement.

**Authorized focused execution passes all 17/17 scenarios on exact source
`cc606a0243fe18d2468f8aabab0ceee560c2f907`.** The isolated resource check found a
16 GiB local memory limit, about 162 MB in use, over 17 GB headroom, and no other
compiler/build process. Cached jars/SDK are read-only; outputs used a unique
private temporary directory. Compiler heap was capped at 512 MiB, runtime at
256 MiB. No Gradle/native build, daemon, installation or shared build/cache writes.
The standalone command is:

```sh
python3 apps/android/src/test/kotlin/com/visualworkbench/android/airactions/check_remote_motion.py --tools-root /workspace/.vw-tools --receipt-dir /tmp/vw-air-remote-motion-NEW
```

Executed with cached Kotlin **2.4.0**, coroutines **1.9.0**, JVM 21 and `-Werror`;
the MotionEvent adapter also compiles against API 37. The negative control produces
**one stale Undo**, the hooked exact timing trace **zero commands**, and fresh
idle input **one Undo**. All contact/queued/cancel/focus/detach/replacement/barrel
and exactly-once delegate scenarios pass. Exact command exit codes, source hashes
and limits: `/tmp/vw-air-remote-motion-executed-r1/receipt.json`; named results:
`host-contracts.log`; resource/collision evidence:
`/tmp/vw-air-remote-motion-resource-check.json`. Temporary compiler artifacts were
disposed and the post-run build-process count is zero.

Current source hashes (SHA-256):

- AirActionRouter.kt: `7dc09b08cbccc73ec6861a3278a3edcc5cf70834f7a5e8a6fdcda5a87d453c04`
- AirActionRemoteMotion.kt: `6b744bbf7f80c57b28a78fe2f962a61b2ffda9a23d49cd820682a2abc2425a02`
- AirActionRemoteMotionEvent.kt: `ad8aecb1086f55b9ee8efc9e444aea2742257cc5864aa4e2c060797e39a3da4c`
- AirActionRemoteMotionContract.kt: `08380de852aa37d6e8b415ced4f7b1537d56e24243640222e7391370bc109932`

The source/ownership audit also passes **113 distinct checks**, including byte
identity of both real remote-input source files and exact callback-registration
fences. Receipt: `/tmp/vw-air-remote-motion-source-final.json`. Whitespace and
staged secret checks pass. This was standalone host execution, not a JUnit runner
or full Compose/EditorController compilation. Native ACK/input, actual MotionEvent
framework execution, Android runtime and physical S23/A09 acceptance stay unverified.

## Executed evidence and remaining gates

Source-only command:

```sh
python3 apps/android/src/test/kotlin/com/visualworkbench/android/airactions/check_caller_source.py
```

The audit passes lexical delimiter, exclusive-path, unchanged-permission,
one-Activity registration, barrel-order, shared-commit and exact frozen-method
checks. It confirms preserved giant renderer, document observers, camera/share
operations and retirement logic; it is not a Kotlin type checker or Android test.
The earlier caller audit passed **91 distinct source/ownership checks**; source-hash
receipts are at `/tmp/vw-air-actions-remote-source-final.json`. Whitespace and
staged-byte Gitleaks checks pass. These counts are static checks, not JUnit cases.

Thirteen new JUnit caller regressions are written for legacy preference migration,
disabled/remapped values, malformed/unknown command refusal, unrelated preference
preservation, exact camera restore, stale project/document/page/viewport/navigation
refusal, barrel correlation across ordinary/full interruption and the closed
remote mapping. **These thirteen tests have not been compiled or executed in this lane.**
The first local candidate used source-only validation while the image lane held
the build slot. After the coordinator reported the slot free, the existing
bounded leaf checker was rerun: **37/37 host scenarios pass**, strict explicit-API
shared leaf compilation passes, the current native-Android key adapter/settings
panel compiles against API 37, and the two declarative resources compile/link.
This uses cached Kotlin 2.4.0, coroutines 1.9.0, JVM 21 and AAPT2 36.1.0, with
warnings denied; it is not whole shared/app compilation. Exact commands, source
hashes and results are in `/tmp/vw-air-actions-callers-leaf-recheck/receipt.json`.
Temporary compiler/resource artifacts were disposed. No Gradle, native build,
installation, provider or device activity ran. Existing `/tmp/vw-air-actions-*`
receipts and preparation drafts remain preserved.

Next: Astra reviews this complete caller source candidate before any main merge.
The coordinator owns combined repository-pinned Android/shared compilation and
execution of the thirteen new JUnit checks, followed by actual UI/lifecycle/
recreation/modal and native completion checks. The new Compose Dialog/caller code
has not been compiled in this lane. Runtime capability/interference detection,
S23 stylus/Bluetooth/barrel/Air Actions, A09 and all physical/performance/product
gates remain unverified.
