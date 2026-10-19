# Root native drawing adapter preparation

Current root integration supersedes the earlier pending-factory/barrier sections
below. `remote_drawing.rs`, `remote_drawing_gate.rs`, the actual serial host input
worker and `NativeDrawingRuntimeAccess.kt` now implement that source path. P's
scoped c2098fe handle is composed as9103cf5; F55505c6 repair as32befbd. See
`evidence/PAINT_CONCRETE_NATIVE_INTEGRATION_20261019.json` for exact source hashes,
phase tests and remaining Windows/binding/app/runtime gates. This new root source
is a final-review candidate, not a passed Windows or hardware implementation.

Root's current source adds `NativeRemoteDrawingOwner.kt` and wires the real
`NativeRemoteEdit` capability to its session consumer registry. The actual
foreign access is deliberately absent from the normal factory until the
producer/input barrier is integrated. No UI positive result is enabled by this
preparation. These new changes await review; acceptance of6efc8b8 does not cover
them.

The private `NativeDrawingPreparationAccess.prepare(expected)` supplies an
actual opaque `NativeDrawingPreparation`. Its allocation is registered with the
session before it starts. Its Kotlin owner remains in that registry after the
acquiring coroutine returns and while confirmation waits. Close invalidates the
owner, signals native cancellation, joins late allocation and any running task
outside its monitor, then retires the foreign owner. An already acquired cancel
signal completes before foreign destruction. Cleanup failure retains that same
owner for retry; concurrent UI/session close shares one attempt. The normal
common acquisition bracket remains cancellation-safe through owner delivery.

The foreign adapter must implement:

- `target`: actual prepared source projection, read before destruction.
- `cancel()`: nonblocking signal through the actual native cancellation token;
  no provider query, join or capture. The token must already be registered with
  the native session before query/helper creation can outlive close.
- `runConfirmed()`: one-shot actual native producer, returning only its owned
  receipt/record projection after completed process/Job/IO/pixel evidence.
- `retire()`: actual joined retirement and private-file disposal; read actual
  `Unchanged/Restored/RecoveryNeeded/Unknown` state before foreign destruction.
  A failure retains the same native owner. Never infer restored pixels from
  resource retirement. Native must seal uncertain canvas/input authority before
  releasing its real peer/host exclusion; the UI fence alone cannot protect
  another endpoint.

Root's positive decoder additionally requires exact owned bytes and records,
the same active binding/identity/tool/settings, `paint.brush`, exactly seven
aspects, mouse-only Drawing/Supported/EditorObserved with evidence, and six
NotTested/NotPerformed/Unknown aspects without evidence or device claims.
Receipt import supplies none of the foreign/source/input ownership.

## Pab477729 handback inspected and composed

Root inspected `ab477729ecf1e30bab210c59422fd5013ad9b91d` from
`task/paint-positive-producer-20261019` and composed it as107b829..2da57ee under
the parent integration direction.
Its branch contains no independent review receipt, so root does not count the
worker review claim as independently verified evidence. P's public preparation
and completed getters can supply the successful branch; root will keep completed evidence in the actual native
session/source owner before disposing transient preparation resources.

One required failure hook remains to reconcile before concrete FFI wiring:
`PaintDrawingPreparation::run_confirmed(self)` currently consumes the preparation
and returns only `Result<CompletedPaintDrawingObservation>`. `retain()` moves its
actual failed Owner into the global quiet-retirement queue and returns generic
RetirementPending. Root needs an opaque per-run retained owner/outcome handle,
or an equivalent keyed accessor to that actual owner, to retry and observe its
own process/Job/IO/private-file retirement and actual canvas/input uncertainty.
It must remain source-held, not be reconstructed from a receipt or a caller
flag. The library-wide retirement count is an unload fence; zero/nonzero alone
does not provide this operation's canvas state or actual cleanup receipt.

Parent should route this precise accessor requirement to the existing P owner;
root has no callable outgoing cloud sender. P retains ownership of its helper
and retained-owner hooks; root retains FFI/Hub/protocol/registration and Kotlin.
Do not overlay root/shared/controller files or add another worker.

Root also still must implement the actual sequenced peer admission and
quiescent host input-helper barrier, scoped authorization callback, pinned helper
registration/Cargo inventory, normal foreign factory and one coherent ABI build.
No generic ordinary command may launch the experiment. No profile/ACK/parse
result becomes positive effect evidence. Physical input, real Windows capture
and owner/hardware acceptance remain deferred.

## Bounded validation

Twelve focused owner cases pass against synthetic foreign/session boundaries
with the actual production preparation/registry/owner/decoder source. Existing
14 common lifecycle cases,16 importer groups and5 Desktop file guards pass in
the same probe. This does not compile the full NativeRemoteEdit or app.

`run_compatibility_probe.py --check-adapter` attempted the complete actual
NativeRemoteEdit source against cached older shared/binding jars. It fails on
the pre-existing `remoteConfigureObservationHelper` absent from the cached core
binding jar. The failure is retained; no generated API stub, binding regeneration
or broad build was used. The next coherent native/ABI batch must resolve it.

Root now registers the exact `vw-paint-drawing-helper` Cargo target and its eighth
Desktop runtime inventory entry. The33 synthetic runtime/catalog and33 package
report cases pass; no helper executable was built, launched or assigned a hash
without an actual packaged artifact. The real foreign/input integration above
remains required before the normal factory advertises the capability.


Parent now independently accepts exact Pab477729 with14 focused checks. This
supersedes the earlier unverified worker-review claim. Those checks do not
validate root's foreign integration or actual Windows/GUI/native behavior.
Root must acquire real exclusive peer/host input admission; its fixed callback
must recheck actual session/source epoch, grant, scoped reservation and the
shared cancellation token before sending authorization. On native return, root
must revalidate the retained native source and actual session/grant/cancellation
before cache or UI publication; completed getters alone have no currentness.
Retain actual unresolved canvas/history/source/Job/permit ownership and block
another probe. Only Drawing/Supported/EditorObserved/Mouse may be produced;
all six other aspects stay NotTested. None of these native hooks is supplied by
the Kotlin lifetime owner, cached UI target or helper filename registration.
