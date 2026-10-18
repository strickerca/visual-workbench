# Desktop output caller handback

Root hands the following formerly reserved files to one parent-activated owner,
suggested existing R owner01a15161-4bf5-71d5-bfe6-f25904e4b024 after its compositor
return. Root will not edit them concurrently after this publication:

- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/EditorController.kt
- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/EditorWorkflowControls.kt
- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/EditorCanvas.kt
- NEW dedicated Desktop output/controller/canvas tests

Start from this document's publication, which includes root source-borrow hooks
from22736c7 and R compositor7fe5b235. Retain the exact existing signatures and
ownership of `ownsVerificationOriginal`, `withVerificationOriginal` and
`withCurrentVerificationOriginal`; V/Main will consume those methods. Both
AiServiceController/AiEditorController and AI leaves remain exclusively with A.
NativeCore, shared APIs, FFI, platform Main/capture and Android roots are excluded.
ImageFramesPanel and the existing ImageFrames API/interaction/native leaves are
read-only admitted dependencies unless parent extends their ownership explicitly.

Implement the actual Desktop output-frame panel/controller activation and
ordinary-image full-D tonal viewport composition described in
EDITING_OUTPUT_ASSISTED_HANDOFF_20261018.md, using the existing ImageFramesPanel
and existing mutation mutex. Register `ImageFramesInteraction` with full captured
binding/epoch/page tokens and immediate ordered OPS observation, exact mayMutate,
dirty-form/retry departure guards and state.blocksInput. Check departure before
beginDeparture, seal callbacks before waiting for edits, and join detach/close
before replacing the project or parent scope. The committed callback already
holds edits: refresh without reacquiring it or joining its own action. Failed
retirement keeps its precise owner/project and must remain retryable, without
recommitting a saved command.

The main editing canvas stays in immutable D coordinates; O crop/rotation/resize
belongs to the output frame and its preview/export. For supported ordinary image
sizes, use the existing background owner and `editingBackground` with complete
binding and existing bitmap conversion. Invalidate before async OPS refresh,
refresh while tones are active, restore ordinary source display after the last
tone is removed, and paint canonical content once. A composed background already
includes annotations/results. Keep selection/transient overlays, and retire
composited display during geometry moves rather than drawing a moved object over
its old baked position. Existing background, renderer and cache budgets remain
authoritative; do not make a second image/cache/export owner.

GiantCanvasHost/native giant provider remain root-owned. The current
image_frames editingView implementation still decodes a full source with a
50-million-pixel admission cap; its bounded returned tile does NOT prove giant
source-region composition. Do not claim that calling it completes giant tonal
support or bypass the original-pixel/budget rules. Preserve honest refusal and
return the exact optional viewport hook needed for the root's actual giant-region
implementation. Root tracks that as missing code, separately from deferred tests.

Formats/export callers remain F-owned and package mapping O-owned. Return exact
additional root hooks for supported PSD/RAW notices or output metadata only when
needed; do not overlay an older controller or silently enable unavailable
formats. Shared/native APIs already carry the admitted source/output contracts.
No broad Gradle/native/binding/package run. Use cheap meaningful focused checks,
retain failure receipts, obtain narrow source review and return exact SHA/paths
and missing-code versus deferred-validation rows to the parent.

Separately, the existing capture/Main owner must wire V's exact capture-origin
callback and attach `McpVerificationOriginalAccess`, including V's forthcoming
current-source history method. Instructions are in
core/crates/vw-ffi/src/verification/INTEGRATION.md. No Main/capture ownership is
changed by this document; parent can send that job to its current owner or
explicitly arrange handback. Root's URI source path and Android UI remain separate.
