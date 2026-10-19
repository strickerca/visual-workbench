# Root platform adapters after e32c046

Current adapter update after reading Android085cca7f (not yet composed): the
stream module owns all original/preview/surface pixels and the native draft
owns its temporary project. This deliberately supersedes the earlier774c048
bitmap-owning child adapter, preserved in Git and its evidence.

- `EditorController(application, core, streamDraftOnly = true)` creates the
  existing private annotation editor with no capture service, project/capture
  enumeration, AI/assisted or document/layer/tone workflow admission.
- `installStreamDraft(project, documentId, initialCamera)` retains the exact
  native borrow before suspension, awaits initialization, verifies the adopted
  document, and preserves the surface's exact camera without a second fit or
  device-local camera restore. Ordinary/giant/Result background loaders are
  skipped. `streamScenes` publishes actual ordinary annotation scenes with
  background == null for the surface's separately budgeted pixels.
- `settleStreamEdits()` seals new contact/mutations synchronously, joins the
  accepted edit/stroke/preview queue, then checks `streamEditsIdle()` under the
  existing mutation mutex. It is idempotent for the same retained Save retry.
  The module must first fence/cancel input and serialize the actual draw post.
- `disposeStreamEditor()` seals admission, joins actual editor/canvas work,
  retires metadata/workflow/native-child owners and joins the child scope. It
  never calls project.close on the native draft borrow. Only after this returns
  may the annotation owner close its native draft or resume the paired stream.
  Cleanup failure keeps the exact child owner for retry.

Root will adapt AndroidStreamDraftEditor.scenes/gestures/viewport/sealEdits/close
using these actual controller methods after bounded module review. The earlier
installStreamOriginal bitmap method is removed from this still-unwired adapter:
the module's retained pixel owner now controls lossless swap without a duplicate
root bitmap or preview decoder. Parent project/link and ordinary editor flows
remain owned by their current controllers.

Private children cannot navigate to another screen/project, export, open AI or
assisted selection, open layer/tone/document workflows or create raster/vector
eraser dependencies unsupported by the native Save compiler. The current
annotation toolbar subset is sufficient; do not activate the full app menus in
the child. The parent project, pairing and current stronger image-frame/giant/
document/AI retirement flows stay intact.

`ProjectRepository.streamLamport(minimum)` reserves the full disjoint 4,096-op
range before preparing a native Save plan. Its actual CAS helper is tested for
remote floors, simultaneous reservations, exhaustion and ordinary-edit tails.
The normal instruction allocator keeps its 1,024-op bound and uses that same
range allocator. A retained Save retry reuses its existing native plan and must
not reserve a second range or create a new transaction.

DesktopLiveLink has a separate optional `beforeRetire` suspension. Root will
pass the actual producer/packet retirement there during platform composition.
A refusal seals and retains the current link for retry, before native link
destruction. The existing `beforeClose` focus cleanup retains its previous
close-on-failure behavior, including remote input release; it was not weakened
to implement stream cleanup. This new hook is not wired to a producer yet.

Actual Android shared/app and Desktop production sources compile against the
fresh e32c046 core API. This checks the authored methods and the existing app
roots, not a new normal stream entry point. Android counter tests pass 6 cases.
Desktop focused retirement results and exact scope are recorded in the matching
ROOT_APP_ADAPTERS evidence. The Windows runtime packaging task is excluded only
for those synthetic Desktop unit tests; no packaged-runtime or delivery pass
follows. No JNI/device/private-editor or source/render/freeze/swap/save journey
has executed.

## Producer composition decisions still owned by root

Parent now accepts G edf2883041f4346c2bf91463ac543e9657584c06, including the
complete failure-path JSON workspace bound, negative control and legitimate
maximum shapes. G is now registered by root as c0a13e5 with actual clock/wire,
open_budgeted injection and exact ninth runtime-helper registration. The actual
foreign budget/lifetime facade and normal callers remain code work.

The publisher must open before the OS producer so its actual native
`CapturePublicationSignal.connectionEpoch/sourceGeneration` can bind the G
request. Do not use a synthetic 1, Java connection sequence, or source label as
that authority. Recovered HostCaptureStreamOptions/StreamCancellation/Heartbeat
and two-number Original APIs predate G's current exact request/identity contract;
they are not valid generated bindings.

Root now uses `EncodedTiles.with_captured_at_ms(actual clock)` before
serialization. The registered local protobuf preflight accepts current fields
10–12 and checks captured UTC against its reply header; local delivery sequence,
source generation and input authority in the payload remain zero. Kotlin must
not rewrite serialized protobuf or synthesize capture time.

G's StreamProcess currently creates a SessionBudget internally and exposes its
exact clone through `budget()`. That same pool must govern its parser, packets,
native/foreign copies and validation. The authenticated publisher also owns an
actual parented pool; separate uncoordinated full process limits cannot be
claimed as one shared allowance. Root still owes either direct pool injection
within one native owner or a pre-reserved bounded host partition held through
actual helper/packet/foreign retirement. No such cross-library capacity API is
published or implemented by this adapter candidate. Neither a nominal Vec
length nor destruction of a packet before the foreign copy settles closes that
gap. Preserve G's shared-budget hooks when adapting the final facade.

Normal Main/WorkbenchApp entry points, source selection/status, both controller
attachments and the existing R/Android stream module composition remain root
work after their reviewed handbacks. This document does not dispatch a new
worker or authorize another source owner.

Current worker review: R b94fa553 remains held on six source findings (final-blit
receipt, canceled raster, ACK/contact race, concurrent close, sealed-epoch lossless
adoption, HiDPI budget/lifetime), assigned to the existing R owner. Android085cca7f
and correct F control2adb2fe are fetched and await bounded review. None is composed.
The new child methods are real compiled source; their AndroidStreamDraftEditor
adapter and normal app acquisition/mount/lifecycle gates still require root wiring.
