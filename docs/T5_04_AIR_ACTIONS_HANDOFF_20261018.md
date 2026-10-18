# T5.04 Air Actions leaf handoff — 2026-10-18

**Disposition:** reusable leaves implemented; app composition and PEN-009–011/A09
acceptance remain pending. No user-visible app behavior has changed yet. The
integration owner is `01a14eea-d204-762c-94b8-f07105b5001e`. Krita remains deferred.

## Source and independent ownership

Compared actual fetched main `6f3e2f3b9ed7a760d4dd7f4c6f26fa44dc5ad88d` with
recovery `0d91b70558a995fa7654021de7465b1d3f615e3d`, including its 796-file
manifest and archive search. Neither contains production Air Actions leaves.
`EditorSupport.kt` and `StylusInput.kt` match recovery; retain their serial
`CommandDispatcher` and MotionEvent abstraction. The existing probe declaration
is the source for the eight F1–F8 defaults. No `.agents/skills` or T5.04 task prompt
exists in this checkout. Read AGENTS, current plan/spec/requirements and recovery,
giant-provider and latest roadmap checkpoints before editing.

Only these new paths belong to this lane:

```text
apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/AirActions.kt
apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/AirActionRouter.kt
apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/AirActionContract.kt
apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/AirActionRouterTest.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionKeys.kt
apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionsPanel.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionKeyContract.kt
apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionKeysTest.kt
apps/android/src/main/res/xml/workbench_air_actions.xml
apps/android/src/main/res/values/air_actions.xml
tools/air-actions/check.py
docs/T5_04_AIR_ACTIONS_HANDOFF_20261018.md
```

No existing source, app entrypoint, controller, giant renderer, manifest, build
file, protocol/schema, generated binding or FFI registration was edited.

## Exact coordinator hooks

1. Own one `AirActionRouter(dispatcher, nowMs, binding, allowed, execute,
   queueRefused)` on the main thread, using the existing `CommandDispatcher` class
   with the editor's main scope and `SystemClock.uptimeMillis`. `binding` returns
   `AirActionBinding(localAdmissionEpoch, Canvas|RemoteApp|Dialog)` or null. Reuse
   the existing project/document/page/focus/grant/readiness/modal/AI/draft guards;
   this ticket creates no input authority. Bump the local epoch and interrupt
   whenever those existing owners change, even if they later become ready again.
2. Activity owns `AirActionKeys(router)`. Delegate full KeyEvents exactly once
   through `accept(event)`, with the normal key fallback for false. Do not also
   route `SamsungActions.onKey(keyCode, down)`: that placeholder loses timestamp,
   repeat and cancellation information. Call `interrupt()` on pause, focus loss,
   canceled input and availability/connection loss; `close()` before parent
   disposal. Stop the Activity timer while it is backgrounded.
3. `configure(savedBindings, setup.routingReady)` retires old mappings/offers;
   call `keys.schedule()` after configuring. Persist stable enum names through
   existing preferences, retaining unknown-name refusal/default policy there.
   `AirActionSetup` deliberately distinguishes unknown, unsupported, disabled,
   disconnected and observed Air Command interference. Supply actual host or
   explicit owner observations. Key receipt alone never proves BLE/device support.
4. Feed the accepted pen sample's MotionEvent primary-button state and timestamp
   to `router.barrel(active, timeMs)` **before** the existing `PrecisionButtons`
   barrel handler; call `keys.schedule()`. Preserve the current touch/hover packet
   ordering, `precisionInputAllowed`, hold/peek, pen-up queue and cancel behavior.
   No second pointer pipeline is needed. Invalidate first on canceled/disallowed
   packets. This observer requires a small coordinator composition hook; the
   reserved CanvasSurface and PrecisionInput sources are untouched here.
5. Implement `allowed` and `execute` with the same current visible-command
   callbacks. Canvas uses `precision.toggle()`, existing `undo()/redo()`,
   `chooseTool()` over the allowed visible tool order, and
   `precision.magnification()` clamped to 2/4/8. `fit()` currently exists;
   **fit/restore still needs coordinator-owned camera save/restore**, fenced by
   document/page/viewport identity and cleared by intervening navigation. Do not
   label repeated `fit()` calls as restore. Remote actions use only the existing
   `RemoteEditorPalette.command`/`remoteEditorActionEnabled` and native grant path;
   refuse unmapped remote commands, without local undo or arbitrary shortcuts.
6. Embed callback-only `AirActionsPanel.render(setup, bindings, commandEnabled,
   onSetup, onRemap, onCommand)` with an owned AndroidView/dialog. It offers all
   eight safe on-screen commands regardless of Air availability. Recheck the
   existing guard inside each callback. Explain that its gesture labels assume
   Samsung's defaults; the framework can separately remap the declared actions.
7. Register one Activity's `com.samsung.android.support.REMOTE_ACTION` intent
   filter and matching metadata pointing at `@xml/workbench_air_actions`.
   No permission, SDK, Bluetooth connection, private setting read or device
   identity change is required by these leaves. The schema was checked against
   [Samsung's declarative documentation](https://developer.samsung.com/galaxy-spen-remote/air-actions.html).

The resolver requires matched uncanceled down/up events, a 250 ms resolution and
barrel-correlation window, and a 2 s press/delivery freshness bound. It suppresses
repeat/duplicate routes, expires lost releases, fences remaps/session replacement
and queued offers, and explicitly reports capacity refusal without replay. Limits:
8 simultaneous presses, 16 pending actions, 32 remembered input/gesture streams.
All Air commands wait the correlation window; this favors conservative duplicate
refusal. Interrupted second clicks discard their unresolved click group. No enum
value can Send, Submit, Apply, grant control or name an arbitrary key sequence.

## Executed checks and retained limits

Run `python3 tools/air-actions/check.py --tools-root /workspace/.vw-tools
--receipt-dir /tmp/vw-air-actions-check-NEW` on a fresh receipt path. It performs
only bounded leaf compilation/policy/resource checks; it downloads nothing.

- R3 passes **36 named standalone host scenarios** (32 shared router/setup,
  4 Android key adaptation), Kotlin **2.4.0**, coroutines **1.9.0**, JVM 21,
  warnings denied. Common production sources also pass strict explicit API mode.
- Both Android leaves compile against **API 37**. AAPT2 **36.1.0** compiles/links
  the two resources with a synthetic manifest; XML/default/key/label checks pass.
  Common import guard, whitespace and staged-byte Gitleaks checks pass (no leaks).
  No native/project build ran.
- Exact source hashes and command exit receipts are in
  `/tmp/vw-air-actions-check-r3/receipt.json`; named results in `host-contracts.log`.
  Temporary compiler/resource artifacts were disposed; only text receipts remain.
  Router SHA-256: `d7d07694eb98a2f2561432bc5d99fffd200b8119485962ba8b86bb8576dd82b0`.
- Retained failed attempts: R1 rejected Android's non-public
  `META_MODIFIER_MASK`; R2 rejected deprecated `ACTION_MULTIPLE` under `-Werror`.
  R3 uses explicit public modifier bits and a narrowly documented suppression for
  consuming that legacy callback. R1/R2 logs and failure receipts remain under
  `/tmp/vw-air-actions-check-r1` and `/tmp/vw-air-actions-check-r2`.

**Unrun:** repository-pinned Kotlin/Gradle/JUnit wrappers, whole-app/native build,
Android Handler/View lifecycle and visual behavior, manifest/controller composition,
automatic platform capability/interference observation, target-device event timing,
actual S23 stylus/Bluetooth/Air Actions/barrel gestures, A09 and all product gates.
F1–F8 can come from a keyboard; a key-only or software receipt is not physical
acceptance. Next work is coordinator T5.04 composition (M), then serialized app
checks when the shared slot is granted. Standard speed retained; no session
model/effort setting was changed or asserted from unavailable runtime metadata.
