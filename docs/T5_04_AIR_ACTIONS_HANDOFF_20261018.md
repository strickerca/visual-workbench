# T5.04 Android caller handoff — 2026-10-18

**Disposition:** public Window callback repair for the composed caller candidate;
review and final application lint remain pending. The earlier composed source
compiled and passed 151 Android tests. That evidence predates this repair. No
installed-app, physical S23/S Pen/Bluetooth/Air Actions or A09 outcome is claimed.
Krita remains deferred.

## Source and ownership

Repair base: `70475b818b2ca5bf02b4a3539fb038c864410d83` on
`cloud/capture-air-caller-integration-20261018`. Repair branch:
`codex/t5-04-supported-key-hook`. Read AGENTS, implementation plan/requirements,
latest checkpoints and `CAPTURE_AIR_CALLER_COMPOSITION_20261018.md`; compare actual
accepted main and recovered callers before editing. The composed callers match
reviewed Air source `766d1f2`, `cd3c643`, `cc606a02`, `46df4c4`. The original
lane-only base/audit remains `f661eca9d2f6d6d6ebe4fc31ed2d0a5c6ec095e0` and
is not weakened to accept the broader combined tree. No `.agents/skills` exists.

The coordinator's exclusive Android grant includes MainActivity. This repair
changes exactly these source/test paths, plus this single handoff:

```text
apps/android/src/main/kotlin/com/visualworkbench/android/MainActivity.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionWindowKeyHook.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionWindowKeyContract.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionWindowKeyTest.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/check_remote_motion.py
```

Controller, CanvasSurface, Dialog callback, remote panel/input source, manifest,
shared router and dispatcher are byte-identical to the repair base. No Desktop,
shared/native, generated bindings, build/lock/configuration, permission or MCP
source changes. No main merge, installation, device access or native rebuild.

## Repair and exact integration hooks

MainActivity's `dispatchKeyEvent` override and super call inherited AndroidX
ComponentActivity's library-group restriction, producing the composed batch's
three RestrictedApi errors. MainActivity now installs `AirActionWindowKeyHook`
after `editor.attachAirActivity(...)` and before Compose content. This uses the
public platform [Window.Callback](https://developer.android.com/reference/android/view/Window.Callback)
and [Window.setCallback](https://developer.android.com/reference/android/view/Window#setCallback(android.view.Window.Callback))
contracts. No lint suppression, baseline or cast around a restricted API is used.

The hook sends the same actual KeyEvent once to
`editor.airActivityKey(airActivityOwner, event)`. Consumed events return immediately;
unconsumed events call the previous callback exactly once and preserve its
result/exception. Other Window callbacks delegate unchanged. MainActivity closes
the hook first in `onDestroy`, then detaches its existing timer and owner. Close
retires observation even when another callback has chained it and restores the
previous callback only while it still owns the Window slot.

Existing pause/resume/focus, retained-router, replacement and disposal guards are
unchanged. Activity declines remote context; `AirActionDialogControls` continues
to own the remote Dialog Window with its exact registration token/remote/palette
references. Its touch/generic-motion observer still retires pending offers on
contact/cancel/focus loss/detach/replacement and feeds barrel correlation before
existing input. RemoteSurfaceView -> RemoteEditController.touch is unchanged.
The stale queued Undo correction remains intact: negative control one stale
Undo; hooked contact trace zero; fresh idle trace one Undo.

Existing caller behavior is preserved: closed eight-command mapping and remaps;
visible safe Canvas controls; explicit setup state without inferring Bluetooth
from keys; current activity/context/draft/modal/readiness/contact/native-completion
guards; exact owned Camera restore; current native mutation completion; verified
remote Paint undo/redo only. The manifest retains one REMOTE_ACTION registration
and unchanged permissions. No alternative input pipeline or synthetic F keys.

## Executed evidence and limits

Focused command (cached compiler, one bounded worker):

```sh
python3 apps/android/src/test/kotlin/com/visualworkbench/android/airactions/check_remote_motion.py --tools-root /workspace/.vw-tools --receipt-dir /tmp/vw-air-key-hook-validation/focused-contracts
```

**25/25 scenarios pass:** original 17 real-router remote-contact cases plus eight
new callback cases: consume/fallback order, exact event identity, retired callback,
original exception propagation, balanced/repeated key deduplication, interrupted
press and Activity/Dialog ownership. Production Window hook and MotionEvent
adapter compile against API 37 with warnings denied, cached Kotlin 2.4.0,
coroutines 1.9.0 and JVM 21. One new JUnit wrapper contains the eight callback
scenarios; this lane's standalone execution is not the full app JUnit run.
Receipt and named log: `/tmp/vw-air-key-hook-validation/focused-contracts/`.
Temporary standalone compiler artifacts were disposed.

**15/15 static comparisons pass**, proving only the stated MainActivity edits,
attach/install/content and retirement order, and unchanged guarded inputs/shared
leaves. Receipt: `/tmp/vw-air-key-hook-validation/source-ownership.json`.
Current SHA-256:

- MainActivity.kt: `1f7b1de6b858baa9445d15c012111dd57053181776f67f0d749cd414e51ccf2c`
- `bd3ab7f11ad8a18ad28965597b36629fde87b293dfe332e52ee2311bddb24589` — SHA-256 of `AirActionWindowKeyHook.kt`.
- `a739fdfdf9e3fca43a4f3e310163072ad83985643ff5b1f27f327f0c4a864724` — SHA-256 of `AirActionWindowKeyContract.kt`.
- AirActionRouter.kt (unchanged): `7dc09b08cbccc73ec6861a3278a3edcc5cf70834f7a5e8a6fdcda5a87d453c04`

**Focused pinned-AGP RestrictedApi check passes.** An isolated Android app with
AGP 9.3.1, Kotlin 2.4.20, Activity 1.11.0, Core 1.16.0 and the frozen app's
resolved-version constraints compiles the byte-identical production hook and a
minimal Activity install/close caller. The original override/super-call control
fails with **two RestrictedApi errors** at those two sites; the replacement
compiles and `lintDebug` succeeds with **zero issues**. This is a RestrictedApi-only
probe, not full Visual Workbench lint; the composed application reported three
errors. Probe source/configuration, reports, logs and exact hashes are retained
under `/tmp/vw-air-key-hook-validation/`; receipt:
`focused-lint-receipt.json`. No lint baseline/suppression or repository build edit.

Full `:android:lintDebug` cannot run here without absent generated/native/shared
outputs. Skipping their compilation fails at `:shared:processAndroidMainJavaRes`
because the KMP compile provider has no completed output. Failure logs and all
isolated diagnostic attempts remain in `/tmp/vw-air-key-hook-validation/`.
Empty standalone CLI reports that did not reproduce the original error are
inconclusive and are not passing lint evidence. No SDK package/license setup,
full native rebuild or binding regeneration was performed to bypass this limit.

Next: Astra review, then the coordinator's affected Android test/lint batch using
the preserved composed artifacts, under normal coroutine recovery. Previous
shared 499/499 and Android 151/151 results remain bound to `70475b8`; Desktop's
separate fixture correction remains its owner's lane. Actual Android callback,
UI/recreation/modal, native remote ACK/input and physical S23/Bluetooth/A09,
performance and owner acceptance remain unverified. No product gate is passed.
