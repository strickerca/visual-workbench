# Desktop borrowed stream draft — 2026-10-20

This independent T3.02 implementation partition starts from main
`90c0ee22d5e7d94a56c54068f85bf63ffa481b68`. It prepares the private Desktop
annotation editor and transparent canvas. It does not activate the normal
Desktop stream receiver or compose a held presentation candidate.

## Execution ledger

| Partition | Implemented behavior | Remaining dependency |
| --- | --- | --- |
| Temporary editor ownership | A retained child adapter borrows the supplied native annotation project and document. It initializes in its own joined scope; caller cancellation and late initialization cannot lose the borrow. Disposal failures remain explicitly retryable. | Parent session must retain the adapter, close it from outside the child scope, and close the native draft only after successful editor retirement. |
| Source pixels and camera | Draft installation avoids ordinary background, image-format, document-worker, image-workflow and provider attachment. The annotation canvas keeps its page and surface transparent. The supplied initial camera is used exactly. | The external frozen-source renderer must follow `adapter.state.value.view.camera`, including viewport/pan/zoom changes, with the same transform as the overlay. Actual AWT/Compose composition remains unverified. |
| Immutable Save boundary | Save refuses an unfinished contact, independent eraser/selection/rotation work, selection publication, busy instruction/semantic work, or dirty instruction draft. Once admitted it retires idle eraser/selection capabilities, fences other mutation paths, prevents refresh from reattaching them, and drains accepted ordinary edits. | Parent must reserve the operation-counter range, prepare one immutable native Save, preserve exact retry identity, and request degraded-save consent where required. |
| Ordinary editor | The default constructor retains ordinary project ownership and opaque page/surface rendering. | Focused regression results are recorded with this source snapshot; Windows packaging and physical acceptance remain separate. |
| Desktop receiver and lifecycle | Not activated by this partition. | Promote only the original owner's independently accepted presentation leaves, then add the root session/panel, source-to-overlay camera composition, first-contact freeze, lossless adoption, Save/resume and joined retirement. |

The adapter never closes its borrowed `WorkbenchProject`. A successful close
means real controller disposal and child-scope join both completed before the
parent's retirement callback. A throwing disposal or callback preserves the
completed stages and the exact retained adapter for retry.

Sealing is terminal for this child. Resume must retire it and its renderer before
the native annotation session resumes; a fresh child/view is required for a later
freeze. Cancelling a Save waiter does not reopen mutation admission.

## Verification boundary

Focused tests use the real editor and adapter with in-memory project fixtures,
and an 8-by-8 in-memory Skia canvas. They acquire no window, device, provider,
capture, stream module or user image. Linux Desktop tests exclude
`:desktop:packageConnectionRuntime`, whose Windows runtime artifacts are not
available here. No generated binding or native interface changes are part of
this partition.

The receipt in `docs/evidence/DESKTOP_STREAM_BORROWED_DRAFT_20261020.json` binds
the final source hashes, actual commands, test results and independent source
review. Compilation or synthetic pixel checks do not pass presentation receipts,
real stream/freeze/Save/resume, frame rate, latency, memory, S Pen, Bluetooth,
thermal or owner acceptance gates.
