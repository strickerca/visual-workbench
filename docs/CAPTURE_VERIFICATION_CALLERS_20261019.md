# Desktop capture and verification callers

Main now observes every MCP owner replacement and attaches the existing editor's
ownsVerificationOriginal, withVerificationOriginal and current-original history
borrow. Those methods retain their editor-owned mutex/current-source checks.
No new project, capture target, grant or controller lifetime is introduced.

Foreground and full selected-window picker capture delivery now pass the actual
native target/receipt and the captured project's final semantic revision to the
already-running MCP owner's editorOrigin/rememberEditorCapture path. A monitor
or cropped region is not offered as a full-window origin. The callback never
starts MCP or substitutes another target, and native retention refreshes exact
window/process/creation identity. The final ProjectInfo is read before the
destination owns close; the observer receives only metadata after successful
destination return. Semantic collection failure still delivers its capture and
allows origin retention. Missing metadata or an optional callback refusal reports
verification unavailable without another project close or failed-owner retention.
Destination failures preserve the existing exact-owner/cancellation contract.
Post-delivery observation settles under NonCancellable before capture shutdown.

Quit previously called controller.close before joining MCP and called it again
from nested finally cleanup. Main now joins capture callers, Codex, MCP UI tasks
and the service before closing the editor once, then its link/native parents.
An MCP request/service retirement refusal leaves the editor/parents alive for
the existing visible Quit retry. Parent close is sequential, so an uncertain
child retirement cannot credit native teardown through a finally block.

Executed focused evidence:49 JUnit methods pass; current Foreground/Picker
controllers and their actual delivery/admission/workspace/retirement leaves
compile with Kotlin2.4.20 -Werror against the cached unchanged shared/core/host
ABI signatures. Eight new methods cover post-close metadata-only observation,
callback and metadata failures, exact failed destination ownership, joined
cancellation and service/request shutdown failures. No native factory is
substituted in the compiled capture controllers. Text receipt:
evidence/CAPTURE_VERIFICATION_CALLERS_20261019.json.

Main/Compose assembly, real editor/MCP callback execution, regenerated ABI,
WGC/UIA/JNI, whole-app shutdown and platform journeys remain unrun. These hooks
do not enable transformed D/O verification and do not prove Paint observations
or physical gates. The existing blanket transformed-comparison refusal remains
until V implements the real checked geometry/pixel alignment handoff in
PAINT_AND_VERIFICATION_INTERFACE_HANDOFF_20261019.md.

Repeat the focused phase check with the approved setup/cached jars:

```bash
python3 tools/tests/check_capture_verification.py \
  --tools-root /workspace/.vw-tools \
  --receipt-dir /tmp/vw-capture-new-receipt
```

Parent reports Astra narrow source acceptance of the preceding524e647 batch,
including independent three-case and40,000-oracle geometry checks. Its seven
close boundary tests/Linux check retain their exact limits. E b2c8746's edge
reservation defect is under repair9511c379; G4821ed18 includes portable hook
patches. Neither pending review candidate is composed here. All prior failures,
retained owners and old worktrees are preserved; broad validation is batched.
