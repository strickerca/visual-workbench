# Krita/Paint milestone compatibility

Observed on 2026-10-04. These rows describe the actual query-only checks; pen,
shortcut and latency results remain pending. The earlier
[T0.05 injection inventory](injection-smoke.md) is retained as historical evidence.

| Editor | Exact observed version | Tool scope | Pointer / drawing | Pressure | Tilt | Eraser | Shortcuts / size / navigation | Measured latency |
|---|---|---|---|---|---|---|---|---|
| Krita | File version 5.3.4.0; portable Krita 5.3.4 | Owned blank startup window; numeric controls queried without tool changes | Not tested | Not tested | Not tested | Not tested | Not tested | Not measured |
| Paint | File and installed-package version 11.2605.81.0 | Owned blank window; canvas reports Brush tool; no input sent | Not tested | Not tested; support unconfirmed | Not tested | Not tested | Not tested | Not measured |

The [source-bound observation receipt](../evidence/M4-KP-editor-observations-r18.json)
records the exact executable BLAKE3/length, probe and runner SHA256, complete
selected-root queries and actual owned-process/IO retirement. No screenshots were
created. Both editors closed after their query-only checks; no owner document or
editor preference was reset.

Krita's query completed both bounded control groups. The two unnamed toolbar
range controls have neither an automation ID nor a label witness. Their values,
ranges and positions do not establish size or opacity semantics. Paint's query
completed 224 nodes without traversal errors and found named brush, eraser, size
and zoom candidates. Candidate discovery grants no input/profile authority;
actual accepted input, editor effect and guarded restoration are still required.

Paint source trust uses its explicit, exact installed-package policy, the
retained process/final image and OS package namespace checks. It does not claim
the ordinary ancestor leases used for portable Krita. Unknown versions or
unverified command profiles must remain unavailable.

For the milestone, the editor owns its document and save operation. Visual
Workbench does not provide a layered PSD round trip, a Krita pixel/layer bridge
or Paint multiview. The broader editor and bridge work remains deferred. This
matrix does not pass editor compatibility, physical S Pen fidelity, the practical
owner drawing session or target-device performance.

Later automated checkpoint: the rows above retain the original query-only
baseline. Subsequent owned-window drawing/history attempts did not pass complete
pixel restoration; their failures remain in
[the milestone evidence](../evidence/M4-KP.md). Krita's selected-Brush operation
moved keyboard focus from the canvas to the toolbox. Native Paint diagnostics
proved that the Brush point hits a passive Image immediately inside the exact
Brush button, explaining the existing exact-element guard refusal. Reviewed
corrections remain unbuilt local candidates at the owner-requested pause.
Phone-to-editor drawing/Undo/Redo/restoration, essential control calibration,
physical pen behavior and performance acceptance remain pending.
