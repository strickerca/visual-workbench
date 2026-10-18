# Air Actions preference continuation boundary

Superseded by the whole Android caller grant in CALLER_PARALLEL_GRANTS_20261018.md.
Do not assign this to a second owner; its preference contract remains useful to lane A.

The parent may assign the existing Air Actions owner this source-only slice from
f661eca9d2f6d6d6ebe4fc31ed2d0a5c6ec095e0 on the preserved
cloud/capture-air-verification-draft-20261018 branch. It already includes accepted
2a66b8 ninth-press handling and the accepted editing correction. Do not overlay
controllers, Activity, CanvasSurface, WorkbenchApp, AndroidManifest, shared APIs,
resources or generated bindings from older roots. Parent assigns the heavy slot;
source work is independent of root's native and coordinator validation.

## Exact three-file ownership

- apps/android/src/main/kotlin/com/visualworkbench/android/airactions/AirActionPreferenceCodec.kt (new)
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/EditorPreferences.kt (existing)
- apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionPreferenceCodecTest.kt (new)

Use the existing shared AirActionBindings/AirActionGesture/AirActionCommand.
Add a final defaulted airActionBindings: AirActionBindings property to the existing
EditorPreferences so existing constructors remain valid. Keep EditorPreferencesStore
and its existing conflated/durable settings writer as the sole persistence owner;
no additional preference file, writer, dependency or permission.

The new pure codec has these internal functions:

    encodeAirActionPreferences(bindings: AirActionBindings): Map<String, String?>
    decodeAirActionPreferences(values: Map<String, String?>): AirActionBindings

Encode exactly the eight snapshot entries using stable gesture/command enum
names. A JSON null represents an explicit disabled mapping; a missing entry uses
the existing shared default. Bound input to eight entries and 64 characters per
name. Unknown names, non-string/non-null JSON values, oversized input or an invalid
subrecord fall back only the Air Actions subrecord to defaults. Preserve the
other existing preferences and brushes when this optional subrecord is invalid.
Do not add Send/Submit/Apply/arbitrary keys to the closed command set.

Read/write an optional airActions JSON object within the existing bounded settings
record. Missing optional data keeps defaults; preserve explicit JSON null through
round-trip. Keep existing settings size/write failure handling and optional-field
backward compatibility. Avoid tests that require a new JSON/Android mocking
dependency: test the pure codec with ordinary host unit fixtures, and return the
small JSON adapter changes for actual Android compilation in root's next batch.
Cover all default mappings, every command/remap, explicit disabled values, partial
maps, unknown names and limits. No per-commit binding generation.

Persist bindings only. Support, Bluetooth connection, enabled state, Air Command
interference, observed keys, device identifiers, clocks, session epochs, targets
and grants remain session observations. Reloaded bindings cannot enable routing
without root's current setup/focus/lifecycle/command guards.

## Root retains platform admission and callers

Root will own the Activity key callback exactly once before fallback, main-thread
AirActionKeys lifetime and timer removal on pause/focus loss/availability change;
barrel observation before the existing PrecisionButtons handler in both touch
and hover routes; guarded EditorController commands and exact Fit/Restore camera
ownership; existing remote command/grant admission; panel/fallback composition;
and the REMOTE_ACTION manifest/resources registration. Worker preferences must not
claim any of those hooks, BLE delivery or physical S Pen acceptance completed.
No vendor SDK, live provider, toolchain research, device operation or permission
change belongs to this slice. Return source SHA/tests and any tiny root usage
hunk; do not edit the retained roots or assign another worker.
