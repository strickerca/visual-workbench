# Giant delivery repair and native region composition handback

Parent activates the following exact nonoverlapping jobs on this publication.
Root retains NativeCore, NativeGiantImports, project factory helpers, manifests/
lock, protocol/schema, generated bindings, giant URI staging and platform roots.
Root will not edit the newly delegated files concurrently after delivery. No
worker spawning, heavyweight build/bindings/package run or device/editor action
is authorized here. Cheap focused source/ownership checks and narrow review are
required; all final native/ABI/app/runtime gates remain separate.

## E: remove unread owned handles from giant bulk delivery

Suggested existing backend owner01a15009-f394-70eb-9e10-e873087f1003, after its
accepted54b42f20 repair. Additional exclusive scope:

- core/crates/vw-ffi/src/giant_images/dto.rs
- core/crates/vw-ffi/src/giant_images.rs, only delivery module registration,
  tile facade, marshal_delivery and delivery_tests; preserve intake/provider and
  all other public registrations
- NEW core/crates/vw-ffi/src/giant_images/delivery.rs and dedicated tests
- apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeGiantImages.kt
  and NEW dedicated JVM giant-delivery ownership tests

Known defect: GiantTileDelivery serializes pixels (including rgba) before an
owned Arc native_lease. If ByteArray lifting throws first, RustBuffer destruction
does not lift/drop the unread Arc. Closing the independently prefetched foreign
reservation cannot reclaim that leaked handle. The existing native test explicitly
closes a decoded lease and does not exercise this failure. Do not mask it with
an explicit test-only Arc drop, field reordering or eventual GC assumption.

Use the accepted assisted-source approach: return a small actual prefetched
owner before bulk lifting, fetch its known publication lease separately, require
that preparation before a single take, retain the native charge through the
outstanding bulk return, and put only plain metadata/bytes in the bulk record.
Retirement failures retain the exact owner/charges and remain retryable. Keep
the existing foreign SessionResourceCharge live before native read and through
renderer retirement. No new pool/counter, caller retirement assertion or raw FD.

Stable native interface for the following G job:

```text
PreparedGiantTile (opaque UniFFI object)
PreparedGiantTile::admit(pixels: GiantTilePixels, outgoing: MemoryCharge)
  -> GiantImageResult<Arc<PreparedGiantTile>>                 // crate-private
GiantImage::prepare_tile(address, cancellation)
  -> GiantImageResult<Arc<PreparedGiantTile>>                 // exported async
PreparedGiantTile::publication_lease() -> Arc<SessionResourceCharge>
PreparedGiantTile::take_pixels() -> GiantTilePixels           // bytes/metadata only
PreparedGiantTile::release_source() -> GiantImageResult<()>
```

The crate-private constructor validates the actual provider allocation, same
endpoint pool and bounded ABI copy before adopting. Root/G cannot mint approval
from external pixels. NativeGiantImages keeps the existing shared GiantImage.tile
signature and lowers through this prefetched owner. Preserve cancellation,
late-result cleanup and failed-retirement retention. If retaining the old native
tile entry would leave an unsafe unprefetched route, return an explicit refusal
there until it can satisfy the same ownership contract; never leave the known
unread-Arc path reachable. Return any exact compatibility/registration hook.

Add actual production Record -> RustBuffer lowering tests: simulate lift failure
before pixels finish, destroy the buffer, retire known source/prefetched owners,
and assert final Weak destruction and quota. Reproduce failure against the old
record. Cover concurrent close, cancellation/undelivered replies and failed
source/lease close. Publish this stable interface promptly so G can integrate.

## G: actual giant D-space composition through existing region renderer

Suggested idle O owner01a1500a-13fe-77e4-835e-201034a9901c after its accepted
package output return; parent chooses/activates one owner. Exclusive scope:

- core/crates/vw-ffi/src/giant_images/provider.rs and tiles.rs
- NEW core/crates/vw-ffi/src/giant_images/editing.rs and dedicated tests
- NEW core/crates/vw-raster/src/source_region.rs and dedicated region tests
- apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/GiantImagesApi.kt,
  only the optional capability below and dedicated tests

Root applies tiny raster/giant module and shared JVM registration hunks after E/G
handback. NativeGiantImages is E-owned meanwhile. Existing RasterSource,
render_buffer_mode, R's canonical compositor, approved lossless level0 cache
and the same global memory/cache/worker owners remain authoritative. No new
compositor, approval, raw path factory, provider/cache manager or full-image
decode. Current image_frames.editingView fully decodes under50MP and is NOT a
giant implementation; do not promote a downsampled cache level to original pixels.

Implement bounded current-project D-space tonal/annotation/result composition
using exact source/cache identity, an admitted immutable revision snapshot and
existing asset resolver policy. Read actual original-resolution regions from
approved level0 tiles through the existing cache, then the one compositor with
the correct full-D origin. Admit every source/result/snapshot/scratch/output
allocation in the same endpoint pool before allocating. Preserve revision/closed/
cancel checks before and after worker work, actual native/foreign ownership and
truthful full-D source coordinates. Output-frame O crop never hides editing pixels.

For coarser viewport levels, compose D before any downsampling. Never apply a
nonlinear tone to a lossy/downsampled source and label it canonical. Provide
bounded actual streaming/resampling where supported; explicitly report/refuse
unsupported extents/quality rather than pretending the original raw tile is the
composed result. Keep exact unsupported cases in the ledger. Budget/quality
refusal is not measured frame-rate or completion of an unimplemented viewport.

Stable additive interfaces (root supplies registration/adapter hooks):

```text
GiantImage::prepare_editing_tile(address, cancellation)
  -> GiantImageResult<Arc<PreparedGiantTile>>
public interface GiantEditingImage {
  public suspend fun editingTile(address: GiantTileAddress,
      foreign: SessionResourceCharge): GiantTilePixels
}
```

The current native/shared source-owner and tuple identities remain unchanged.
Use E's admit function to preserve native/ABI charges for composed pixels. The
optional shared interface adds no required method to existing GiantImage fakes.
Root's NativeGiantImage will implement it after E returns. Platform callers
choose committed-composition mode explicitly and suppress duplicate canonical
draws; root/G returns exact adapter and UI hooks to the existing Desktop R and
Android owner. Do not edit their controller/canvas/Main files. Preserve ordinary
raw tiled mode and transient vector/selection ownership. No synthetic frame can
claim actual GPU, renderer retirement, giant performance or physical acceptance.

If a precise existing visibility/resolver helper is needed outside these lists,
return the minimal hunk before editing it. Keep E's delivery paths separate.
Return exact source SHA, interface hooks, actual source-bound tests, deferred
commands and remaining implementation cases for Astra before promotion.
