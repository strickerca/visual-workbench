# Next giant input ownership lane

This is a private interface/file admission for the next source lane after the
root producer batch freezes. It does not export or activate a user import API.
The parent assigns the owner and build slot. Keep the accepted 256 MiB phone
pool, existing two-input limit, bounded startup/worker queues and current pins.

## Pending input and prepared input are one admission

Admit a private `PendingInputAdmission` and `PickedOriginal` owner under
`giant_images::producer`, with these operations:

- `reserve_picked_input(budget: SessionBudget) -> Result<PendingInputAdmission>`
  reserves the existing 4 MiB input envelope and acquires the SAME existing input
  permit before root opens a File or creates its TempDir. No new counter/pool.
- `PendingInputAdmission::attach(self, original: RootOwnedPickedOriginal,
  limits: GiantInputLimits) -> Result<Arc<PickedOriginal>>` immediately bundles
  File, TempDir, permit and charge in one owner before fallible admission. The
  native File and actual TempDir already belong to root before async dispatch.
- `PickedOriginal::prepare(cancel: Arc<Cancellation>) ->
  Result<Arc<PreparedOriginal>>` admits exactly one preparation attempt, performs
  the existing bounded streaming copy/header/hash/disk validation, and transfers
  the SAME permit/4 MiB charge into the actual PreparedSource staging owner.
- `PickedOriginal::close_worker() -> Result<()>` joins actual queued/running
  preparation and cleanup. Drop signals cancellation but cannot credit a still
  live job. After a successful transfer, picker close/drop must not close the
  independent returned PreparedOriginal. Failed/abandoned replies still own and
  retire every actual input/prepared resource before admission returns. Do not
  leave an empty per-picked OS worker alive after successful consumption; use
  existing bounded startup ownership, or join that worker before delivery.

These are private Rust interfaces, not public callbacks. Keep producer approval
separate: neither selecting/preparing an input nor a path, consent, manifest or
completion directory can mint ApprovedBuild. Root will lower its selected input
through separately reviewed synchronous native File/TempDir creation. Android's
future URI adapter may use a separately owned, bounded app-private staging file;
no raw integer FD or arbitrary foreign completed-directory API is admitted here.
No complete original byte vector or full-resolution bitmap is permitted.

Refactor input::prepare into a shared admitted implementation if necessary.
Existing prepare_original callers/signatures remain valid. A picked source must
not acquire a second input permit or another input envelope while transferring
its existing admission: two selected inputs must still be able to prepare without
self-created backpressure, and three must refuse. Keep charges with actual file/
directory ownership through every fallible constructor, cancelled startup,
undelivered result and quarantined directory cleanup. Original bytes and accepted
published destinations remain preserved.

## Exact source ownership

The parent may assign the image owner:

- core/crates/vw-ffi/src/giant_images/producer/picked.rs (new)
- core/crates/vw-ffi/src/giant_images/producer/picked_tests.rs (new)
- core/crates/vw-ffi/src/giant_images/producer/input.rs (existing)

Root keeps producer.rs module/export composition, Cargo/lock, public DTOs/FFI
registration, shared APIs/bindings, JNI bridge, publication facade and both app
controllers. Return the tiny exact module/export hunk for root; do not overlay
producer.rs or older roots. Existing granted streaming-read files remain the
accepted 9b05bea implementation. The native private producer platform and source
records are already the common contract; do not invent a duplicate DTO family.

Tests must cover pre-open memory/slot refusal without invoking an opening hook,
exact two-input admission and third refusal, successful same-charge transfer,
close during blocked preparation, cancellation before the worker body starts,
constructor failure, abandoned reply, close after consumed success, and actual
cleanup failure retaining the same permit/charge. Keep tests synthetic and bound
all waits. Do not label a cancellation signal or counter subtraction retirement.
The next root phase composes reviewed exports and public intake/caller contracts,
then runs one coherent native/bindings/app batch. Source-only work may run in
parallel; builds require the parent's heavy-slot handoff. No SDK/tooling research,
paid services, signing, runtime substitution, device identity or physical gate
work belongs to this lane. This continuation is large; retain the owner's chosen
Sol6.1/xhigh settings without changing executor configuration.
