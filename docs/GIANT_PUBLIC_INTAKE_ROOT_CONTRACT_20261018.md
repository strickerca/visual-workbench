# Next giant-input root boundary

This is the next implementation contract, not an implemented public capability.
It depends on admitted picked-input code `04a913e3`/PI1 repair `ba338adc` and G1
foreign-retirement code `181e4a40`. The current root phase composes capture and
Air Actions callers first. Existing platform file ownership stays with those
lanes until explicit handback. This contract grants no overlapping platform work.

## First native and shared slice

Root will add a facade in `giant_images/intake.rs` and register it from
`giant_images.rs`. Keep the reviewed private `picked.rs`, `input.rs`, producer,
cache and publication owners authoritative. Public methods lower into them;
there is no second admission counter, approval flag or completed-directory factory.

The initial API takes an existing local regular-file path and a checked private
work parent. It reserves `reserve_picked_input(shared_budget())` before opening
the source File or creating a TempDir. It uses the existing non-following
`source::regular` and checked-directory admission. The exact File, TempDir,
existing two-input permit and 4 MiB charge aggregate immediately into the existing
picked owner. File preparation stays off the UI thread; cancellation settles its
actual native work and cleanup before a foreign result is accepted or disposed.
It never reads the full original into a foreign/native byte array or full bitmap.

Planned opaque boundaries:

- `GiantPickedOriginal` wraps the existing private `Arc<PickedOriginal>` and
  exposes one preparation attempt plus joined close. Closing a consumed picked
  facade cannot close its independently delivered prepared owner.
- Existing `PreparedOriginal` exposes bounded metadata, one approved cache build
  and joined close. Export only raw/oriented dimensions, orientation, format,
  encoded byte count and original hashes. Its private source/work paths never
  enter returned records or diagnostic output.
- Existing `GiantBuiltCache` exposes publication into a new project, exact-bound
  rebuild and joined close. Publication consumes `PublishedProject` through
  `adopt_published_project`, preserving store, blobs, SessionPermit and charge.
  A failed/abandoned foreign reply cannot delete an already accepted destination.
  Rebuild uses the real ProjectSession worker/closed flag and WorkflowBinding;
  no caller-supplied project snapshot or unconditional currentness callback.

Build options contain bounded disk quota, timeout and explicit untagged-sRGB
consent. Root selects the producer from the actual compiled platform. Android
uses only the fixed JPEG region runtime; Windows uses only the packaged pinned
runtime. Never export arbitrary runtime pinning/admission, executable paths,
digests-as-approval or a platform enum that can select another platform's runner.
Linux reports Unsupported for production builds. No Windows runtime admission
has been proved by these Linux checks. Existing source/header/color restrictions
remain in force; PNG metadata admission is not Android PNG producer support.

The shared slice adds an optional giant-import capability and opaque suspendable
owners rather than adding required methods to every WorkbenchCore/project fake.
Use the existing `GiantImageFailure` family and native error mapper, same global
session resource pool and cancellation/undelivered-result ownership pattern.
Every close runs under NonCancellable, destroys the foreign handle only after
successful native retirement, and retains the exact failed owner for retry.
Wrapper construction failure and cancellation after native completion must not
lose the returned owner. Do not copy the weaker `finally { destroy() }` pattern
from older unrelated adapters into these new resource-producing calls.

## Android URI dependency remains explicit

A content URI cannot be passed to the local-path API. An adapter that opens or
spools it before native input admission would violate the pre-open invariant.
Do not grant a controller/URI wiring lane on that assumption. The follow-on root
contract must first provide a bounded native-owned staging writer or an equally
strong separately owned stream/staging bundle: admission before resolver open,
incremental size/free-space enforcement, exact file identity, joined foreign
reader retirement, cancellation and retained failed cleanup. Raw integer FDs,
unbounded foreign spools and extra independently credited quota are insufficient.
The path-only slice can be tested and admitted independently while this contract
is completed. No Android import UI is claimed until its real URI path is wired.

## Phase validation and remaining gates

Test pre-open refusal before any OS file/work-root creation, valid/malformed and
symlink inputs, same-permit transfer, repeated preparation, cancellation before
and after result completion, undelivered owner cleanup and failed-close retention.
Use actual native facade calls and the existing bounded fake producer seams;
never expose a production approval bypass for tests. Verify unsupported Linux
builds, explicit consent, exact publication destination/reopen behavior and stale
rebuild bindings. Reuse the accepted G1/PI1 regressions in one serial producer
batch; those global two-input probes cannot run concurrently.

After source review, perform one coherent native/bindings/shared compatibility
batch for the new public ABI. Device URI/JNI/codec/GPU execution, packaged Windows
decoder admission, giant-image target memory/frame-rate measurements and physical
acceptance remain separate. No provider, installation, credential or hardware
authority is added by this plan.
