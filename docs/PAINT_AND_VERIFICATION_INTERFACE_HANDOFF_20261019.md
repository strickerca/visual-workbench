# Remaining observation and coordinate interfaces

Parent activates the existing P and V owners against this publication. Main and
capture callbacks are implemented here; F/R/Android grants remain exclusive.
These contracts remove a need to wait for root registration decisions. Expensive
ABI/apps/Windows/hardware checks remain deferred; no observation is executed by
this grant. Root will integrate reviewed leaves and regenerate bindings once.

## P: the genuine joined Paint observation

Existing P owner01a15145-22a1-76b1-ba3e-d2bbe0326006 retains its importer,
NativeRemoteEdit/NativeSessions factory hook and the native runner/frame/new
compatibility leaves granted in PAINT_NATIVE_OBSERVATION_GRANT_20261018.md.
Root inspected published1b53eadd read-only: the current importer correctly
requires a NativeCompatibilityObservationAccess, but the active root has no
native/session implementation that supplies that owned observation. Parsing a
receipt path does not fill this gap.

Freeze the existing JVM boundary without a new trust parameter:

```kotlin
internal interface NativeCompatibilityObservationAccess {
    suspend fun withCurrentReceipt(
        expected: RemoteTargetState, receiptSha256: String,
        body: suspend (NativeOwnedCompatibilityReceipt?) -> RemoteCompatibilityAdmission?,
    ): RemoteCompatibilityAdmission?
}
```

The factory must pass the real implementation to NativeRemoteEdit. Its body
retains the actual completed process/Job/IO result until the callback settles,
including cancellation. Bytes, independent records, tool/settings and native
source identity are supplied from that owner, not the imported request. No new
ByteArray record may hide a late opaque retirement handle. If crossing UniFFI,
return a prefetched opaque source owner before lifting bulk bytes; hold its
resource/receipt lease through callback/retirement and return only plain data
from the bulk call. Enforce retained and serialized capacities, not lengths alone.

P may implement the following crate-private, read-only native source interface
in its NEW remote_compatibility module. Root grants only the accompanying
minimal adapter block in session/remote_edit.rs, plus module/export declarations
in session.rs necessary for that leaf; do not change the rest of those roots.
No other worker owns these small blocks and root will not concurrently edit them:

```rust
// Actual identity from this RemoteHub; not a serialized caller snapshot.
pub(super) struct CompatibilitySource {
    local_epoch: u64,
    binding: vw_remote::Binding,
    target: vw_remote::Target,
    identity: vw_remote::profile::ImageIdentity,
    profile_digest: String,
}
impl RemoteHub {
    pub(super) fn compatibility_source(&self, expected: &vw_remote::Binding)
        -> SessionResult<CompatibilitySource>;
    pub(super) fn check_compatibility_source(&self, expected: &CompatibilitySource)
        -> SessionResult<()>;
}
```

Use bounded clones under the existing short state lock, then release it before
any process/Job/IO join, callback or cancellation wait. Compare full local epoch,
binding/input session, scope/capture generation/geometry, target identity and
native image/profile on recheck. Current/closed/retiring/source-replacement
predicates remain authoritative. A read-only view must not mint an input grant
or change action eligibility. Snapshot fields are observations of current
identity only: profile tool/settings and Injected/ACK are never evidence of the
editor's actual selected tool or effect. The joined owner must independently
derive actual observed tool/settings and each claimed aspect, and check them
before/after its borrow.

P chooses internal record/visibility details to fit its source; the public
contract remains the existing withCurrentReceipt above and ordinary SessionError
families. Native non-Windows paths return unavailable, not synthetic support.
No transport/protocol/helper process launch/input-control roots are delegated.
If retaining completed runner evidence needs an additional exact completion
accessor in remote_host.rs, propose that accessor promptly to the parent; do not
turn arbitrary file bytes or a new global receipt registry into owner evidence.

The real producer still matters: an input-free frame with input_sent=false or
effect receipt with physical_or_editor_pixel_effect_proven=false cannot produce
Supported. Wire retained completed results from the existing runner/frame owner.
If the normal runtime lacks a causally joined effect producer, return its exact
missing source/trigger/lifetime interface as implementation work, separately from
the unperformed Windows/owner experiment. Import never runs an experiment,
injects input or creates new helper-control authority. Physical/S Pen aspects
stay Not tested without actual correctly bound observations. Krita is deferred.

## V: canonical D/O verification coordinates and real comparison

Existing V owner01a1507a-2f92-71e6-b312-c985eb631012 already owns FFI
verification/**, shared Verification APIs/NativeVerification, Desktop mcp/** and
dedicated tests. Extend to NEW verification/geometry.rs and dedicated tests.
Root module/export declarations already expose verification DTOs; no root Cargo,
protocol/schema, NativeCore, controller or Main edits are needed. Parent may
activate this immediately in that existing lane.

Add the following method and plain metadata records to the existing borrowed
WorkbenchVerification/native owner. Keep source/canonical/catalog/inbox/close
checks under its existing joined original/after owners; no new project/file
owner is needed. Preserve old fake implementations with an explicit Unsupported
default if necessary, rather than enabling geometry accidentally:

```kotlin
public suspend fun geometry(pair: VerificationPair,
    memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): VerificationCoordinatePair

public data class VerificationImageCoordinates(
    public val binding: WorkflowBinding,
    public val sourceAssetId: String,
    public val sourceWidth: UInt, public val sourceHeight: UInt,
    public val outputWidth: UInt, public val outputHeight: UInt,
    public val imageWidth: UInt, public val imageHeight: UInt,
    public val cropDocument: Rect,
    public val documentToImage: Transform,
    public val imageToDocument: Transform,
    public val hasOutputFrame: Boolean,
)
public data class VerificationCoordinatePair(
    public val pair: VerificationPair,
    public val original: VerificationImageCoordinates,
    public val after: VerificationImageCoordinates,
)
```

Native verification_geometry follows the existing verification_review arguments:
original receiver, exact after session/catalog/inbox/pair, budget, cancellation.
Produce both matrices only from checked immutable packages: validate each
image_output through ImageOutputMetadata.frame()/vw-geom::ImageOutputFrame,
or identity for its absence, then compose the admitted compiled package scale.
Check actual clean PNG dimensions/retained hashes/receipt/catalog identity.
Use the canonical inverse and all four corners for bounds; never reconstruct a
second crop/rotation/resize engine in Kotlin. Preserve source-oriented D and
actual full O/packaged-image extents independently. Enforce canonical and
serialized metadata budgets; geometry contains no opaque handles or bulk pixels.

This geometry is descriptive. It is not image registration, editor identity,
capture permission or proof that two images align. Desktop must use it for
marker/semantic projection and real paired tile extraction/normalization before
removing the current blanket frame refusal. For a measured D translation d and
before/after document-to-image matrices B/A, corresponding package coordinates
obey q_after = A(B^-1(q_before) + d). Rotation, unequal output resize, crop offsets
and differing package scales must therefore be considered explicitly. A scalar
scale or equal PNG dimensions is insufficient, including equal-size90-degree
frames. Real registration/semantic provenance and thresholds still determine
alignment; unsupported phase/extent/normalization stays explicitly unresolved.

Keep compileFailures translations in original/after D coordinates. Validate the
actual whole marker/ROI after translation and the after frame's crop before
native pixel instructions. Preserve the exact after-frame metadata on compiled
failures and the six V lineage fields. Geometry/registration must never manufacture
manual verdicts, saved notes, capture grants or replace the exact source/CAS
checks. Historical notes can display with checked immutable O images after the
current original revision changes; compiling new failures still requires its
current exact source. Ordinary unframed package bytes/behavior remain unchanged.

Return positive asymmetric crop/quarter-turn/nonuniform-resize cases through
actual retained PNG pixels, plus mismatched scales/phases, omitted ROIs,
equal-size quarter-turn, stale source/receipt/cancellation, native D translation
and failure-package frame preservation tests. A metadata DTO alone is not this
task's completion. New bounded pixel helpers, if necessary, belong in V's owned
module and existing inbox/source lifetimes; return precise root raster-export
requests rather than altering the compositor or inventing a second owner.

## Evidence and handback

Source review/publish logical commits with exact hashes and executed/deferred
tests. Root's capture callbacks do not enable D/O comparisons or observations;
keep refusals until the corresponding real implementation is admitted. No GUI
experiments, live provider calls, credentials, signing, paid Actions, deployments,
device/security/driver changes or worktree cleanup are authorized by these grants.
