# AI/provider and external-editor reserved lanes

Parent assigns existing workers and activates these grants; no owner is assigned
by this document. Both lanes start from exact accepted main
6d505416963fb42009921bd3fb8639994e73752a. These are implementation-first source
grants, with aggregate checks in DEFERRED_VALIDATION_LEDGER_20261018.md. Krita
implementation/reactivation is deferred. No runtime/hardware/provider acceptance
follows from source. The four active grants and Air/capture file ownership remain
intact. No worker edits both EditorControllers, Main, WorkbenchApp, Activity,
CanvasSurface, RemoteEditPanel, manifests, Gradle/Cargo roots, lockfiles, protocol/
schema, generated bindings, NativeCore or any giant-input module.

## A — Gemini/FLUX adapters, candidate grid and AI editor integration

On parent activation, this exact list supersedes the broader reserved A entry in
ROADMAP_PARALLEL_IMPLEMENTATION_GRANTS_20261018.md:

- core/crates/vw-ai-provider/src/{lib,http,worker,tests}.rs, src/tests/**,
  NEW src/{gemini,flux}.rs and dedicated adapter tests. secret.rs/trust.rs and
  protected credential implementations remain read-only; request a narrowly
  described capability addition if the existing abstraction is insufficient.
- core/crates/vw-ai/src/{config,request}.rs and dedicated provider request/config
  tests. Budget/proof/memory/pixels modules and crate lib registration are read-only.
- core/crates/vw-ffi/src/ai_edit.rs and ai_edit/** plus AI-specific FFI tests.
- shared commonMain/{AiInteraction,AiEditsApi}.kt, jvmMain/NativeAiEdits.kt and
  AI-specific commonTest/jvmTest files. No MaskApi, ImageFrames, verification,
  offline, remote palette or giant files.
- Android editor/{AiWorkspace,AiServiceController,AiResultController,AiLatestTask,
  AiPixels,AiEstimateInput,AiViewGeometry,AiResultPainter,AiEditorOverlay,
  AiEditorController,AiCompareCanvas}.kt and their AI-specific tests.
- Desktop/{AiWorkspace,AiServiceController,AiResultController,AiLatestTask,AiPixels,
  AiEstimateInput,AiViewGeometry,AiResultPainter,AiEditorOverlay,AiEditorController,
  AiCompareCanvas}.kt and their AI-specific tests. Platform key dialogs, ProviderIo
  and secure key-store implementations remain read-only.

Reuse AuthorizedRequest/Prepared, immutable source/mask/outside-pixel proof,
BudgetLedger/Confirmation, configuration fingerprints, current credential-provider
abstraction, candidate owners and canonical edit/undo application. No second AI
service or new quota ledger. Existing OpenAI/GPT behavior stays compatible; add
explicit provider/model capabilities rather than reinterpret a GPT model string.
Return additive enum/config/native/shared signatures and tiny root registration/
controller hooks with the first coherent batch. Root will compose required
out-of-grant budget/proof/schema or credential-capability changes after review.

No network request without explicit owner Send and admitted cost/profile
confirmation. Each attempted request remains unresolved until actual settlement;
no automatic retries, missing-key fallback, environment/chat key lookup, free-cost
assumption or secret/provider-body log. Model/API details need primary official
specification verification. Synthetic transport fixtures must cover real bounded
wire formats, failure/usage/deadline/cancel cases, candidate-grid binding and
outside-pixel refusal. Implement offline with credentials unavailable; do not make
live calls, store keys, change accounts or fabricate provider quality evidence.

## P — Existing Paint controls, compatibility and external-editor palette

On parent activation, exclusive:

- host-win/crates/vw-remote-host/src/platform/shortcuts/paint_controls.rs and
  paint_controls/{production,experiment}.rs, NEW paint_controls/compatibility.rs
  and dedicated Paint-control tests. Existing shortcuts.rs/global dispatcher,
  observer.rs and all krita_controls/** stay read-only.
- host-win/crates/vw-remote-host/src/editor_probe/paint.rs,
  editor_effect/paint.rs, editor_probe_runner/frame/paint.rs and their dedicated
  Paint fixtures/tests. Do not edit WGC/capture, HEVC, input injection/guards,
  helper/job/process roots or package-identity/signature authority.
- shared commonMain/{RemoteEditorPalette,RemoteEditorCompatibility,
  RemoteEditorProfileEvidence}.kt, commonTest/RemoteEditorPaletteTest.kt,
  NEW RemotePaint*.kt and dedicated tests. RemoteEditApi.kt,
  RemoteCommandDispatcher and NativeRemoteEdit.kt remain read-only.
- Android remote/RemoteEditorPalettePanel.kt and Desktop
  remote/RemoteEditorPalettePanel.kt plus dedicated palette tests. Android
  RemoteEditPanel.kt remains with the Air owner until explicit handback.

Finish actual Paint/source-derived control and compatibility gaps found during
preparation using existing exact target/process/editor/profile/runtime evidence,
native control grant, foreground/pen/contact/sequence guards, serial dispatch,
balanced input and joined retirement. Keep visible command destination, current
tool eligibility, no local undo fallback and no queued/retried remote commands.
Air Actions depends on current RemoteEditorPalette/RemoteEditorAction declarations
and dispatch semantics: preserve them. Add only optional evidence/capabilities;
return any public/action/native registration changes for root review before use.
An Injected receipt proves input injection, not visible editor success. Do not
promote screenshots, coordinates, guessed UIA ordinals or a synthetic ACK to
authority. Missing verified routes must remain visibly unavailable.

This does not activate an arbitrary-key bridge, new editor profile, Adobe paid
installation or Krita plugin. Photoshop/UXP, GIMP/Affinity/web-editor, virtual-display
and tunnel additions need their own source-backed file proposals before expansion;
they cannot silently broaden this Paint grant. Existing admitted generic remote
editing is reused. External Windows/editor/S Pen checks are final authorized
journeys; Linux source/synthetic checks do not pass them.

## Return and resource rules

Parent activates one owner per list. Return exact candidate/base, changed paths,
stable additive interfaces, minimal out-of-grant hooks, actual cheap-check results
and deferred ledger rows. One heavy build globally/max two workers; request parent
allocation before a non-cheap run. No broad builds/binding generation, package
assembly execution, installation, signing, paid Actions, driver/security/device
identity change or live provider/editor action during these source lanes. Review
each coherent source batch with Astra and retain rejected/older source and logs.
