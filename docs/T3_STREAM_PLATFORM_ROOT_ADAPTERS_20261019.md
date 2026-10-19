# Root platform adapters after e32c046

Root added the following Android editor methods against the actual current
controller, rather than replacing it with the recovered controller:

- `EditorController(application, core, streamDraftOnly = true)` creates a
  private annotation child. Its initialization does not install a capture
  runtime, enumerate saved projects/captures or admit AI/assisted workflows.
- `installStreamDraft(project)` retains the exact acquired borrow before any
  suspension, awaits initialization, opens the existing native editor/render
  path and checks the actual adopted project/background. Failed initialization
  remains owned for explicit joined disposal.
- `streamEditsIdle()` and `settleStreamEdits()` include queued operations,
  contact/strokes, preview work, selection/eraser and rotation work. Save must
  also hold the platform's input seal and actual draw/contact retirement.
- `installStreamOriginal(bitmap, documentId)` consumes the bitmap even on
  refusal. The original remains retained across ordinary background refresh,
  including a preview decode already suspended at arrival. It leaves camera,
  annotations and provisional input in place. It does not recycle a bitmap
  still reachable by an older render snapshot. The stream module must retain
  its actual platform memory reservation through joined child disposal.
- `disposeStreamEditor()` seals admission, joins accepted work and actual
  canvas retirement, closes editor/document/workflow owners and the retained
  project borrow, joins the child scope, then retires the retained original.
  A failure retains the exact owner for retry. This is called from the owning
  parent stream scope, not from the child's own scope.

Private children cannot navigate to another screen/project, export, open AI or
assisted selection, open layer/tone/document workflows or create raster/vector
eraser dependencies unsupported by the native Save compiler. The current
annotation toolbar subset is sufficient; do not activate the full app menus in
the child. The parent project, pairing and current stronger image-frame/giant/
document/AI retirement flows stay intact.

`ProjectRepository.streamLamport(minimum)` reserves the full disjoint4,096-op
range before preparing a native Save plan. Its actual CAS helper is tested for
remote floors, simultaneous reservations, exhaustion and ordinary-edit tails.
The normal instruction allocator keeps its1,024-op bound and uses that same
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
roots, not a new normal stream entry point. Android counter tests pass6 cases.
Desktop focused retirement results and exact scope are recorded in the matching
ROOT_APP_ADAPTERS evidence. The Windows runtime packaging task is excluded only
for those synthetic Desktop unit tests; no packaged-runtime or delivery pass
follows. No JNI/device/private-editor or source/render/freeze/swap/save journey
has executed.

## Producer composition decisions still owned by root

Parent now accepts G edf2883041f4346c2bf91463ac543e9657584c06, including the
complete failure-path JSON workspace bound, negative control and legitimate
maximum shapes. No G leaf is composed in this adapter commit. Root still owns its registration, actual clock
adapter, host FFI and the exact packaged runtime inventory.

The publisher must open before the OS producer so its actual native
`CapturePublicationSignal.connectionEpoch/sourceGeneration` can bind the G
request. Do not use a synthetic1, Java connection sequence, or source label as
that authority. Recovered HostCaptureStreamOptions/StreamCancellation/Heartbeat
and two-number Original APIs predate G's current exact request/identity contract;
they are not valid generated bindings.

At reviewed composition, use `EncodedTiles.with_captured_at_ms(actual clock)`
before serialization. G's local protobuf preflight must accept current fields
10–12 and check captured UTC against its reply header; local delivery sequence,
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
