# T3.02 native/shared integration candidate

Base: `6ae93d495fdfffd511f1e882facb80aaa0ab2dd4`. This is implemented source,
not a task-only grant. G/F leaves remain separate pending their parent reviews.
No Windows build, WGC/GUI/input/device/provider execution, runtime admission,
deployment or signing occurred in this slice. Fresh Linux core bindings and
the actual shared JVM facade compilation passed. Krita remains
deferred. Linux checks do not pass Windows, apps, frame rate or owner gates.

## Implemented native/session and shared consumer behavior

- Negotiated `capture_stream_v1` dispatch in the actual authenticated LiveSession
  loop, with connection-entry epoch fencing and independent publisher/receiver
  ownership. Remote announcements cannot allocate a local receiver, select an
  OS source or create a publisher. Region requests are bounded expiring prompts.
- A bounded codec worker, source/delivery ordering and actual presentation-ticket
  receipts. First contact pins the last positively presented frame; pending draw
  receipts refuse contact, remote-edit never freezes, and source loss preserves
  shown/frozen data. Source availability and owner-selected display name are
  separate from freeze stage so a frozen source-close banner is representable.
- Exact scoped transient BLOB transfer and PNG/hash/frame/time validation before
  lossless readiness. Timeout keeps explicitly degraded preview pixels. Old
  background handles remain honest after a lossless upgrade. No arbitrary
  project asset read or canonical installation follows a peer's content hash.
- Private annotation project/draft ownership, joined close, source/target CAS,
  mutation seal and exact retained Save plan. One ordinary durable transaction
  creates a new capture document, immutable original and supported annotations.
  Unknown commit outcomes retry that same transaction; no post-commit cancel
  check erases a durable receipt. Degraded Save is an explicit distinct choice.
- Stream Save provenance persists with the document and through normal/image-
  frame/file export and package receipts. Proof validation precedes file
  publication. Legacy captures omit the new optional canonical field and keep
  prior metadata/title behavior. New optional package schema fields are bounded.
- Core endpoint budgets are children of an actual shared process pool. Codec,
  display and verified-original allocations use current budgeted primitives;
  retained frames/originals keep their own charges. Queued outgoing DTO/wire
  reservations survive replacement and actual carrier-write settlement. Scoped
  original request/ACK priority preserves existing BLOB scheduling behavior.
- Actual JVM facades and native session child registration are authored.
  Acquisition registers before allocation, joins canceled/late results and
  retains failed cleanup. Generated destructor failure is sticky. Source/draft/
  plan/row owners close before the parent foreign session is destroyed. Common
  annotation state supports exact-frame retry, original readiness, explicit
  degraded Save, durable retry and draft-only discard/resume.

The native draft intentionally accepts its documented annotation subset, not
Results/raster masks/semantic dependencies that its Save compiler cannot retain.
Platform stream tools must match that subset and retire their child renderer
before releasing/resuming the draft. The paired project/link stays attached.

## Concrete API for subsequent platform wiring

Common files are `CaptureStreamApi.kt`, `CaptureStreamExtensions.kt`,
`CaptureDraftApi.kt`, `CaptureDraftExtensions.kt` and
`CaptureAnnotationSession.kt`. JVM implementations are NativeCaptureStream,
NativeCaptureDraft, CaptureStreamOwnership and CaptureChildren. The normal
platform convenience is `ProjectLink.captureAnnotationSession(...)`.

`ProjectLink.openCaptureStream(memoryBudgetBytes)` returns the owned receiver.
Call `beginDraw()` for the sole permission to draw, admit actual bitmap/GPU
copies before reading <=256 KiB RGBA row batches, then settle `presented(true)`
only after that exact platform draw actually happened. A signal/decode/bitmap
allocation is insufficient. Serialize this receipt and accepted real pen DOWN;
stop new live draws before `CaptureAnnotationSession.freeze()`. Retire all child
draw/pixel owners before stream/link replacement. Unknown presentation seals the
view; do not invent a false receipt on cancellation.

Use `CaptureStreamSignal.sourceAvailable` for source-loss banners even while
stage remains Freezing/FrozenLossless/FrozenDegraded. `sourceName` is bounded
display data, not permission. Publisher selection supplies the actual chosen
source label. `captureFrameAgeMs` uses the existing peer-minus-local clock
offset and returns null if synchronization is unknown; it is not photon timing.

The platform calls `refreshOriginal()` for readiness and separately acquires
`stream.frozenBackground()` to adopt complete lossless rows while preserving
provisional annotations/camera. The private draft's preview asset is not silently
rewritten. Durable Save uses the verified original rather than that preview;
explicit degraded Save permanently flags its provenance. A retained exact plan
can inspect an already-durable retry after disconnect. New writes still require
the original live native lease and unchanged target binding.

`ProjectLink.openCapturePublisher(CapturePublicationSource, budget)` is explicit
local source approval. It returns connection epoch/source generation for the
actual platform producer. Frame payloads are raw encoded FrameTiles protobuf,
not an Envelope or JSON: local delivery_sequence, source_generation and
last_input_seq_applied MUST be zero. Native publication assigns delivery/source
ordering and does not turn a queue receipt into a rendered ACK. Original supply
must match every requested source/frame/time field before transient transfer.

## G integration dependency, distinct from its held source review

G a6268af6 is fetched/read but not composed. The parent reviewed its replacement
9e970b5 and holds it for JSON workspace undercharging: a bounded 16 KiB header
can allocate about 279 KiB against its 32 KiB reservation. The existing G owner
pushed90d02999f0f2d6c70c9d82359468e811c784de05 for that repair; root has fetched
it for inspection and must await parent review closure before composition.
Its base predates these wire fields. The current root library supplies
`EncodedTiles::with_captured_at_ms(actual_clock_ms)`, preserving its allocation
charge and frame identity. At composition the producer must call it after
encode_tiles_budgeted and before to_transfer_bytes, using its actual FrameClock.
Otherwise its payload UTC remains zero and the current publisher correctly
refuses it. Header/payload validation must also require that same capture UTC
and zero local delivery/source authority. Do not patch bytes in Kotlin or use
receipt time. G's reply header can retain its separate authenticated request
epoch/source correlation; that is different from the zero local payload fields.

Root still owns approved G module/bin/Cargo/ToolHelp registration, exact helper
runtime inventory, host FFI facade and actual selected-source producer wiring.
Do not substitute the one-shot helper, self-hash an arbitrary executable or
activate the held G source before its reviewed repair arrives.

## Exact remaining code versus validation

Remaining CODE: integrate reviewed G producer/registration/clock adapter and
host facade; implement actual Desktop/Android stream canvases and controllers;
wire Main/WorkbenchApp and both EditorControllers through their current modal,
mutation, attachment and retirement gates. Adopt real lossless platform pixels,
preserve provisional input and show both-device LIVE/source or FROZEN/age.
These normal UI flows are not implemented by passing the native tests.

Parent can extend existing R/Android owners ONLY to new platform `stream/**`
modules and their matching tests against this committed API. Root retains
Main, WorkbenchApp, both EditorControllers, contracts/FFI/registrations and all
runtime inventories. Recovered platform modules are candidates; they predate
current contracts and cannot replace current app roots. No new executor needed.

Executed VALIDATION: Linux core library build, fresh core binding generation
and the whole shared JVM compilation pass. The host generated API is unchanged
and was not regenerated in this phase. Focused native stream/draft tests pass
(84), net tests pass (87), process-budget tests pass (3), tile tests pass (10)
and the full package suite passes (54). Actual Gradle shared tests pass the
20 new ownership/state cases against those fresh bindings. The earlier isolated
Kotlin probe repeats those cases and is not a second set of distinct tests.

Separate pending VALIDATION: actual Desktop/Android/Compose/JNI application
compilation and execution, complete native/policy suites;
authorized Windows/packaging and actual source/render/freeze/swap/save/resume
journeys; 15fps/eight-frame memory/timing and physical/owner acceptance. No app,
Windows or product pass follows from isolated Kotlin or native-library tests.

The strict stream/net library Clippy scope passes with warnings denied. Strict complete FFI
remains blocked by unchanged baseline diagnostics in provider owner, giant
editing, verification prepare and remote drawing gate; dependency-wide strict
Clippy also reports unchanged raster source_region diagnostics. New stream
diagnostics were repaired or narrowly documented where the bounded actor uses
an intentional value enum/native parameter list. No policy gate was waived.

## Subsequent actual app compiler and root adapter closure

Root's next adapter slice provides the Android private-child methods needed by
its stream owner, lossless retention over ordinary/in-flight preview refresh,
full4,096-op Save counter reservation, and a Desktop link child-retirement seam.
See T3_STREAM_PLATFORM_ROOT_ADAPTERS_20261019.md and
[evidence/T3_ROOT_APP_ADAPTERS_20261019.json](evidence/T3_ROOT_APP_ADAPTERS_20261019.json).
Actual Android shared/app and Desktop production/test sources now compile
against this API. Six Android counter and3 Desktop lifecycle cases pass.
Desktop focused tests exclude Windows runtime inventory packaging. No new
native binding generation occurred; this uses the actual e32c046 core API.
Normal app entry points and reviewed worker composition remain CODE work;
fresh ARM64/Windows binaries and actual JNI journeys remain unverified.

## F compile feedback (not active composition or runtime admission)

Parent supplied current F `7533f7c0b6934fd01aae281f4261508d48d9aa13` for cheap
compilation. An isolated harness compiled its exact seven module sources with
locked roxmltree0.21.1/blake3 pins; the unmodified source fails E0063 at
android_tunnel/capture.rs:181: ParsingOptions lacks entity_resolver. Proposed
repair is explicitly `entity_resolver: None`, preserving allow_dtd:false and
the node bound. Only a temporary copy with that one-line repair passes its four
synthetic tests. That is not an as-published F compile, full host-crate check or
Windows/adb/scrcpy result. Its proposed patch and exact evidence are retained.
TUNNEL-005 control also remains CODE work: current Windows launch always uses
--no-control. DLL/runtime/license admission stays closed. Parent retains F
review/repair dispatch; root has not overwritten or activated that lane.
