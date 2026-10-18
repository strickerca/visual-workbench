# Parallel caller and privacy grants — root boundary

Parent assigns/reuses owners; root does not spawn duplicate lanes. These grants
supersede the initial reservation of all platform roots to integration: the
Android and Desktop caller roots below are now exclusive to their platform lane.
Use Sol 6.1/xhigh as requested without silently changing executor settings.

Root's active source edits are ONLY:

- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/mcp/DesktopMcpCoordinator.kt
- apps/desktop/src/test/kotlin/com/visualworkbench/desktop/mcp/McpCoordinatorTest.kt

Root also owns module exports, Cargo/lock, protocol/schema, public FFI/shared DTO
contracts, native host registration, generated bindings and final composition.
Root will not concurrently edit the platform roots granted below. Existing
EditorController document-observer correction and giant reader/renderer ownership
must survive all caller edits. No stale root/controller overlays. No live provider,
credential, signing, deployment, device identity or hardware authority changes.

## Frozen bases and validation order

- G1 validation freeze: 02f5766d236b9cfbef2c88257fc38785a4e21d07 on
  cloud/giant-foreign-retirement-fix-20261018; exact code is unchanged from
  conditionally accepted 0dca21f. Four focused native cases, strict Linux/arm64
  Clippy, fresh libraries, 949 JVM cases, instrumentation source compilation and
  compiled retiredNative `(J)I`/arm64 export pass. Main remains a1ad07f pending
  parent review/results reconciliation. No device execution is claimed.
- Platform caller freeze: f661eca9d2f6d6d6ebe4fc31ed2d0a5c6ec095e0 on
  cloud/capture-air-verification-draft-20261018, pushed as an uncompiled draft.
  It includes accepted capture/Air Actions/T5.01 leaves, root's initial capture
  wiring, the accepted editing correction and G1 source correction. It does not
  make these callers accepted or validated. Start each caller lane here.
- Native verification/privacy freeze: 02f5766 above. No public verification FFI
  or durable-review activation has been admitted. The four-file contract below
  defines the next native leaf work, not a whole workflow overlay.
- Picked input d9fd8a5 remains held for PI1 and is NOT composed. Its current owner
  retains picked.rs/picked_tests.rs/input.rs. No new giant public intake work may
  depend on that held implementation; root composes a reviewed repair later.

All lanes below can do source/cheap static work concurrently. Parent assigns
focused test/build slots; no compiler/build process while another heavy owner
holds the slot. Root currently owns it and can release after its coordinator
phase freezes. Next root batch composes accepted callers, performs one required
host/core public binding generation for the phase, then relevant native/JVM/app
checks. No per-small-commit ownership/binding regeneration or repeated broad
setup research. Preserve all old worktrees until parent disposition.

## A — Whole Android Air Actions caller slice

Reuse the Air Actions owner. Exclusive existing paths:

- apps/android/src/main/kotlin/com/visualworkbench/android/MainActivity.kt
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/EditorController.kt
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/CanvasSurface.kt
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/EditorPreferences.kt
- apps/android/src/main/kotlin/com/visualworkbench/android/ui/WorkbenchApp.kt
- apps/android/src/main/AndroidManifest.xml
- apps/android/src/main/kotlin/com/visualworkbench/android/airactions/ (all existing/new lane files)
- apps/android/src/test/kotlin/com/visualworkbench/android/airactions/ (lane tests)
- apps/android/src/main/res/xml/air_actions.xml
- apps/android/src/main/res/xml/workbench_air_actions.xml

New lane tests may live in android editor tests if needed for an existing
controller helper; do not change unrelated tests or public shared APIs. This is
a full caller implementation grant, replacing the smaller preference-only
proposal in AIR_ACTIONS_PREFERENCES_NEXT_LANE_20261018.md; do not assign a second
preference owner. Add private helpers only where they simplify real lifecycle
ownership; no artificial adapter hierarchy is required.

Shared contract is the accepted AirActionBindings, AirActionSetup, AirActionBinding,
AirActionRouter and closed eight AirActionCommands from 2a66b8. Keep those shared
files unchanged. Use the existing serial CommandDispatcher and current guards;
never invent Send/Submit/Apply, generic key injection or a new grant. Activity
receives the actual KeyEvent exactly once before fallback. Retire timer/press/
queued offers on focus, pause, context, project/document/page/target/grant changes
and availability loss. A balanced key pair cannot infer Bluetooth support.

Observe physical barrel state before existing PrecisionButtons in touch AND
hover paths. Preserve its contact/hold/peek authority and don't turn MotionEvent
into synthetic F keys. On-screen fallbacks remain available when hardware setup
is unsupported/unknown, and use the same current local/remote command guards.
Remote commands use existing RemoteEditorPalette/native grant; never local Undo
as an implicit remote fallback. If existing controller methods launch work,
preserve actual serial completion and freshness rather than accepting detached
work as a completed Air command. Retain the native/renderer/rotation/offline
retirement and failed-owner behavior already present in EditorController.

Fit/Restore must retain an exact camera with its project/document/page/viewport
ownership; a second Fit is not Restore. Invalidate stale restoration after
intervening navigation/ownership changes. Persist stable binding enum names via
existing EditorPreferencesStore/writer, preserve explicit disabled mappings and
unrelated preferences, and keep setup/connection/observed device state ephemeral.
Register only the declared REMOTE_ACTION metadata/action contract; no BLE/vendor
SDK permission or dependency. Compile/on-device evidence remains separate.

## B — Whole Desktop capture-picker caller slice

Reuse capture-picker owner. Exclusive:

- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/Main.kt
- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/capture/ (all existing/new lane files)
- apps/desktop/src/test/kotlin/com/visualworkbench/desktop/capture/ (lane tests)
- apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/CaptureRegionEditor.kt
- apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/CaptureRegionEditorTest.kt

Finish/review the initial a5b683d root wiring at the frozen base, including
current-state admission, independent picker service, pinned helper path/hash,
protection outcome, exact preview save, physical-pixel region mapping, modal/
focus ownership, joined shutdown and visible retry after excluded UI disappears.
Cancellation of exclusion is not an unavailable result. A failed picker close
must retain owners and prevent parent/native-runtime closure. Existing foreground
capture controller remains separately owned; hotkey registration is explicit opt-in.
Do not open a second ordinary file picker around region capture.

Root has already applied the exact small native prerequisite patch from the
accepted handoff; do not edit native host/FFI registration, Cargo/lock, generated
bindings or both apps' EditorControllers. Return any truly missing minimal shared
contract hunk to root. No arbitrary WGC source overlay. Capture's saved project/
PNG is NOT a retained OS selector for verification. Do not populate root's private
McpVerificationTargets without an explicit transferred exact native target owner;
its current recapture route remains the MCP engine. Never edit MCP coordinator,
Desktop EditorController or diagnostics modules. Main's MCP service state access
must use the real existing lifetime owner types, not a guessed DTO.

## C — Durable verification review and failure-package leaves

Reuse verification owner. Exclusive exactly:

- core/crates/vw-store/src/verification_review.rs
- core/crates/vw-store/tests/verification_review.rs
- core/crates/vw-package/src/verification.rs
- core/crates/vw-package/tests/verification.rs

The complete accepted boundary is T5_01_INTERFACE_ADMISSION_20261018.md. Use the
existing settings transaction owner for bounded versioned CAS reviews keyed by
exact immutable original/after package and checked receipt identity/hash. Manual
owner assessments/notes are private, bounded, preserved on failed writes; no agent
verdict setter. All-pass returns no failure package; pending/changed/stale/unknown
marker or pair refuses. Return a bounded failure compiler leaf plus tiny proposed
root exports/manifest/default-None hunks, not a whole root workflow to overlay.

Admitted optional lineage fields: original_package_id, original_target,
original_manifest_sha256, comparison_receipt_id, comparison_receipt_blake3,
original_marker_numbers. New contiguous marker n+1 maps to that original number;
validate uniqueness/range/count, no self-reference/target/hash mismatch, and normal
Package::from_files bounds/proof validation. No selector, path, OS identity, note
or private review data enters lineage. Lineage is descriptive, not alignment or
pixel authority. Image registration remains unresolved. Root owns module exports,
manifest/compiler registration, public FFI/shared APIs, archive privacy removal
and MCP/UI composition. Send an explicit private settings-prefix removal hunk
before the store is wired into project archive export; no extra DB/format.

## D — Whole shared diagnostics privacy enforcement slice

Reuse an idle diagnostics-capable owner (PDF owner can take this independent
software slice; no new PDF/runtime activation). Exclusive:

- apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/DiagnosticsBundle.kt
- apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/DiagnosticsPrivacyAudit.kt (new)
- apps/shared/src/commonTest/kotlin/com/visualworkbench/shared/DiagnosticsBundleTest.kt
- apps/shared/src/commonTest/kotlin/com/visualworkbench/shared/DiagnosticsPrivacyAuditTest.kt (new)

T5.05 already has a closed typed report, exact preview and actual exporters. Finish
an automatic bounded privacy audit at the existing private PreparedDiagnosticsReport
factory before a prepared payload can escape. Preserve current public classes,
constructors, enum codes, schema v1, fields, exact preview bytes and both exporter
contracts. Do not add arbitrary messages, raw logs, trace samples, paths, identifiers,
provider/secret text or a generic report attachment feature. No new dependency:
use existing platform-neutral Kotlin/serialization utilities. Keep the existing
65,536-character ASCII and 256-event bounds and integer/monotonic validation.

The private audit accepts only the exact known report schema/platform/metric/event
keys/types/codes and bounds; refuse unknown members, duplicate keys/ambiguous
representations, arbitrary string contents, nonfinite/overflow/negative counters,
excess events, and synthetic identifier/credential payloads. A detector should
validate actual report structure rather than claim a regex proves all secrets
absent. Malformed audit failure refuses preparation, preserving source state;
no output may bypass it. Show the existing exact closed preview without changing
it into a second independently generated report. Keep no arbitrary free-text
redaction API. Cover normal/rotated reports and bounded malformed/adversarial
fixtures; use only fake data. Return compatibility/source tests plus any real gap
that needs a contract change. Do not edit either platform exporter/lifecycle/UI,
Main, controllers, preferences or other shared files without a new scoped grant.

## Integration dependency order

1. Parent reconciles G1 exact source/receipts and performs/authorizes its coherent
   main promotion; root supplies software evidence without claiming runtime gates.
2. C and D can implement now against frozen interfaces; A and B can implement now
   against the published caller freeze. None depends on held PI1.
3. Root reviews leaf contracts/exports, merges accepted source and runs the one
   coherent native/bindings/platform batch. UI roots are returned by A/B, not
   independently rewritten by root. Review new root/native composition before main.
4. Only after repaired PI1 acceptance does root compose its exact modules/exports
   and prepare public owned-input APIs. Android giant intake/controller changes
   follow A's freeze, avoiding concurrent EditorController ownership.
5. Physical capture/DPI, BLE/S Pen, cross-JNI codec/GPU and performance gates stay
   open; no compilation or synthetic fixture changes them. Existing provider and
   Krita/device deferrals remain in force.
