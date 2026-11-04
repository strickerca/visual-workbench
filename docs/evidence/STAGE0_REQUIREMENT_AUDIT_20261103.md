# Stage 0 requirement source audit — 2026-11-03

Read-only source audit of the 51 requirements that were `not_started` at `2b6b2ea`.
This is a source-presence audit, **not a verification run**: "source" means the
implementing source and test files exist and are wired into the app roots. It
does not mean they compiled or passed, and no acceptance clause is closed by it.
An independent reviewer spot-checked 8 rows against the tree.

Six measurement-only requirements stay `not_started` (PERF-001, PERF-005,
PERF-006, PERF-007, PERF-008, QUALITY-008). PERF-007 also has an unconfirmed
code question: whether the client moves back to a higher-priority carrier that
returns while a lower one is live (`vw-ffi/src/session/link.rs`). INSTR-005
has no source by design and needs a dated owner decision (T5.08).

Some cited handoff/ledger docs carry a 2026-11-03 reconciliation banner; the
rows cite them only for where the source was integrated.

| ID | Audit result | Key source | Integration doc | Open (not exhaustive) |
| --- | --- | --- | --- | --- |
| CORE-009 | source | vw-store/src/local_views.rs; shared DocumentNavigator.kt | T3_07_SAVED_VIEWS_HANDOFF_20261018.md | ≥20-document device journey |
| PEN-004 | source | shared PrecisionController.kt | evidence/T3.03-T3.04-precision-adapters-20261018.md | one-frame latency on hardware |
| PEN-005 | source | PrecisionController.kt; vw-ffi/src/precision.rs | same | hardware property run |
| PEN-006 | source | vw-geom/src/lens.rs; vw-ffi/src/precision.rs | evidence/T1.01.md; T3.03-T3.04 docs | build/test result only |
| PEN-007 | source | PrecisionController.kt; PrecisionGrid.kt | T3.03-T3.04 docs | manual check |
| PEN-008 | source | PrecisionController.kt (toggle queued to pen-up) | T3.03-T3.04 docs | 200-trace instrumented replay |
| PEN-013 | source | android CanvasGestureRouter.kt/CanvasSurface.kt hover; desktop EditorCanvas.kt | evidence/T0.04.md | S23 hover rate |
| PEN-014 | source | vw-geom/src/stylus_tilt.rs; shared RemotePenAxes.kt | evidence/T3.05-DEDICATED-LEAVES-20261018.json | T0.04 calibration traces |
| PEN-016 | source | PrecisionController.kt (hover aim) | T3.03-T3.04 docs | ≤1 px first-contact replay |
| EDIT-003 | source | vw-ffi/src/document_controls/dto.rs; LayersPanel.kt | evidence/T3.07-LAYER-CONTROL-LEAVES-20261018.json | cross-device sync/undo journey |
| EDIT-009 | source | vw-ffi/src/image_frames/*, tone_controls/* | ROADMAP_CURRENT_EXECUTION_LEDGER_20261019.md (T3.07) | golden/export journeys |
| EDIT-011 | source | shared AssistedSelectionInteraction.kt | EDITING_OUTPUT_ASSISTED_HANDOFF_20261018.md | manual check |
| EDIT-012 | source | vw-geom/src/object_rotation.rs; shared ObjectRotationHandle.kt | evidence/T3.07-ROTATION-CONTACT-LEAVES-20261018.json | goldens/hardware |
| EDIT-013 | source | vw-mask/src/shape_selection.rs; shared SelectionPolygon.kt | evidence/T3.05-DEDICATED-LEAVES-20261018.json | cross-device golden |
| SEL-001 | source; runtime blocked | shared androidMain assistedselection/backend/LiteRt*.kt | ASSISTED_RUNTIME_REGISTRATION_20261019.md | empty production LiteRT catalog (B2); 100 ms |
| CAPTURE-007 | source | vw-capture/src/picker.rs; desktop capture/CapturePicker*.kt | T3_01_DESKTOP_CALLER_HANDOFF_20261018.md | Windows/DPI/USB hardware run |
| FORMAT-002 | source | vw-assets cache.rs; vw-ffi/src/giant_images/* | NATIVE_GIANT_PRODUCER_REPAIR_20261020.md | device performance/GPU |
| FORMAT-006 | source | vw-pdf/src/ffi.rs, overlay.rs | ROADMAP_CURRENT_EXECUTION_LEDGER_20261019.md (T3.09) | PDFium runtime admission and journeys |
| FORMAT-007 | source | vw-svg (deny-all engine, export.rs) | same (T3.09) | golden runs |
| FORMAT-013 | source | vw-formats/src/bmp.rs, tiff.rs | FORMAT_ROOT_COMPLETION_20261019.md | build/test result only |
| FORMAT-014 | source | vw-avif-os; AndroidAvifImages.kt | FORMAT_ROOT_COMPLETION_20261019.md | OS codec run |
| QUALITY-001 | source | vw-document-protocol; host-win vw-document-win; vw-pdf/vw-svg limits.rs | ROADMAP_CURRENT_EXECUTION_LEDGER_20261019.md (T3.09) | runtime/LPAC admission; malformed corpus is generated |
| QUALITY-002 | source | vw-pdf/src/pc_redaction.rs, raster_redaction.rs | same (T3.09) | qpdf runtime admission |
| AIEDIT-007 | source | AndroidProviderRuntime.kt; android AiServiceController.kt | ROADMAP_CURRENT_EXECUTION_LEDGER_20261019.md (T3.11) | phone-only W1 journey (live spend not authorized) |
| SYNC-002 | source | vw-ops/src/replica.rs, host.rs | T3_11_OFFLINE_REVIEW_HANDOFF_20261018.md | hardware run |
| SYNC-003 | source | vw-store/src/conflict_review.rs; OfflineReviewPanel | T3_11_OFFLINE_REVIEW_CALLER_HANDOFF_20261018.md | manual check |
| PEN-009 | source | android airactions/AirActionBarrelCorrelation.kt, AirActionKeys.kt | T5_04_AIR_ACTIONS_HANDOFF_20261018.md | S23 Bluetooth run |
| PEN-010 | source | shared AirActions.kt, AirActionPreferences.kt | T5_04_AIR_ACTIONS_HANDOFF_20261018.md | build/test result only |
| PEN-011 | source | android AirActionsPanel.kt (onboarding), AirActionControls.kt | T5_04_AIR_ACTIONS_HANDOFF_20261018.md | manual check |
| CAPTURE-008 | source; runtime blocked | vw-remote-host android_tunnel/mirror.rs; desktop AndroidMirrorCaptureGuard.kt | ANDROID_TUNNEL_RUNTIME_HANDOFF_20261019.md | tunnel license admission (B3); device run |
| TUNNEL-002 | source; runtime blocked | android_tunnel/{mirror,session}.rs; DesktopAndroidTunnel.kt | ANDROID_TUNNEL_SESSION_HANDOFF_20261019.md | COMPILED_APPROVED_CATALOG is None until B3 |
| TUNNEL-005 | source; runtime blocked | android_tunnel/control.rs, session.rs | ANDROID_TUNNEL_CONTROL_HANDOFF_20261019.md | B3; device run |
| QUALITY-003 | source | shared DiagnosticsBundle.kt, DiagnosticsPrivacyAudit.kt | DIAGNOSTICS_PRIVACY_COMPOSED_20261018.md | real export journey |
| SEM-007 | source; runtime blocked | android_tunnel/session.rs, capture.rs | ANDROID_TUNNEL_SESSION_HANDOFF_20261019.md | B3; device run |
| AGENT-006 | source | mcp/src/server.mjs submit_result; desktop McpComparePanel.kt | T5_01_VERIFICATION_HANDOFF_20261018.md | hardware run |
| AIEDIT-006 | source | vw-ai-provider/src/gemini.rs, flux.rs; AiCandidateGrid | AI_EDITOR_RESERVED_BOUNDARIES_20261018.md | live provider journeys |
| VERIFY-002 | source | McpVerificationPanel.kt; vw-ffi/src/verification/geometry.rs | VERIFICATION_NATIVE_COMPOSED_20261018.md | A30 |
| VERIFY-003 | source | McpMarkerAssessments.kt; vw-store/src/verification_review.rs; vw-package/src/verification.rs | VERIFICATION_NATIVE_COMPOSED_20261018.md | A30 |
| FORMAT-009 | partial | vw-formats/src/psd.rs | FORMAT_ROOT_COMPLETION_20261019.md | present: PSD/PSB labelled composite; missing: PDF-compatible AI opens as a PDF document, not a labelled flattened composite (T5.08) |
| FORMAT-012 | partial | vw-formats/src/raw_preview.rs | FORMAT_ROOT_COMPLETION_20261019.md | present: embedded-JPEG preview for TIFF-based DNG/CR2; missing: Android platform DNG and Windows WIC Raw Image Extension routes (T5.08) |
| VERIFY-001 | partial | desktop mcp/DesktopMcpCoordinator, McpVerificationTargets.kt | CAPTURE_VERIFICATION_CALLERS_20261019.md | present: Desktop window recapture and alignment; missing: phone-screen and tunnel captures never register a verification target (T5.01) |
| VERIFY-004 | partial | mcp/src/verification.mjs; DesktopMcpHost.kt | VERIFICATION_NATIVE_COMPOSED_20261018.md | present: grant-gated recapture_package and get_marker_comparison; missing: phone recapture inherits the VERIFY-001 gap (T5.01) |
| QUALITY-006 | partial | tools/release/assemble.py, msi.py, build-msi.ps1 | MSI_RELEASE_CLOSEOUT_20261020.md | present: TOOLCHAIN/SHA256SUMS/notices assembly and MSI hook; missing: reproducibility comparison tool, generated notices (LIC-003), WiX/jpackage admission and Windows run (T5.06) |
| LIC-003 | partial | tools/release/assemble.py; third_party/LICENSES | RELEASE_ROOT_COMPLETION_20261020.md | present: curated hash-bound notices; missing: generation from the resolved Cargo/Gradle/npm tree plus bundled binaries; 125 candidates unadmitted (T5.06) |
