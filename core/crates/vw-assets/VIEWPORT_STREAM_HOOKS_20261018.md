# Image viewport and JPEG stream leaf handoff

Base: `5f4f581c84d9215c06c0029f8dd042fc8453f26d`.
These are recovered/adapted source leaves, not a compiled or integrated feature.
The integration owner must adopt the interfaces below before the next combined
build. This file proposes contracts; it does not authorize a wider codec profile,
relax a resource cap, or replace the current controller/app behavior.

## Missing workflow and this boundary

The current ordinary import/background path remains bounded by its existing
encoded/pixel limits. `vw-assets` already has a path-free pyramid, manifest,
leased cache and viewport scheduler, but has no active native giant-image source
handle, approved producer adoption, app viewport owner or giant routing. The
existing remote video path is separate from T3.02 JPEG live annotation.

The smallest new view workflow is: open a source-bound approved pyramid, plan the
latest physical viewport, read only its visible tiles within a foreign-resource
budget, draw only under the same full document/attachment fence, and join native
reads plus renderer retirement before project replacement/close. The recovered
`GiantViewportSession` implements the asynchronous view/retirement owner. Its
provider and actual platform renderer remain integration prerequisites.

The recovered `vw-stream` core supplies the corresponding T3.02 JPEG primitives:
reserve/fill an original ring slot, encode/reconstruct bounded dirty tiles,
acknowledge the frame actually presented, freeze that exact frame on accepted pen
contact, then admit its exact hash-checked lossless PNG for background replacement.
It does not persist a document, grant capture consent, start OS capture, transport
an original BLOB, or acknowledge physical presentation on the app's behalf.

## Coordinator: exact minimal common viewport API

Adopt this view-only subset of the preserved `GiantImagesApi.kt`; PNG export and
producer APIs are deliberately not assumptions of this leaf. `GiantTilePixels`
must not print image bytes through generated data-class debug rendering.

```kotlin
public data class GiantViewport(public val left: Double, public val top: Double,
    public val right: Double, public val bottom: Double,
    public val a: Double, public val b: Double, public val c: Double, public val d: Double)
public data class GiantTileAddress(public val level: UByte, public val x: UInt, public val y: UInt)
public data class GiantViewportPlan(public val assetId: String, public val manifestHash: String,
    public val level: UByte, public val visible: List<GiantTileAddress>,
    public val prefetch: List<GiantTileAddress>)
public class GiantTilePixels(public val assetId: String, public val manifestHash: String,
    public val address: GiantTileAddress, public val documentX: UInt, public val documentY: UInt,
    public val documentWidth: UInt, public val documentHeight: UInt,
    public val width: UInt, public val height: UInt, public val rgba: ByteArray) {
    public override fun toString(): String = "GiantTilePixels(width=$width, height=$height)"
}
public interface GiantImage {
    public suspend fun viewport(view: GiantViewport): GiantViewportPlan
    public suspend fun tile(address: GiantTileAddress): GiantTilePixels
    public suspend fun close(): Unit
}
```

`viewport` supplies oriented D-space rectangles and the D-to-physical linear
transform, with physical DPI applied once. The provider authenticates its original
asset, manifest and tile bytes before returning opaque RGBA8 sRGB derivatives.
The session locks the first source/manifest identity, limits a plan to 4096 keys,
checks tile geometry/opacity, and reserves 802,816 bytes (three 256-square RGBA
copies plus 16 KiB) before each native/foreign tile read. The total owner cap is
192 MiB; native codec/cache and actual drawing surfaces need separate budgets.
The backend must check actual bitmap stride/allocation and texture storage
against the reserved copies before upload, and refuse an unaccounted expansion.
The generic owner does not measure driver allocations or process RSS.
No implicit profile/depth conversion or proxy-as-original behavior is permitted.

The recovered view leaf constructor is concrete:

```kotlin
GiantViewportSession(parent, image, backend,
    binding: WorkflowBinding, attachmentEpoch: Long,
    isCurrent: (WorkflowBinding, Long) -> Boolean,
    maxRetainedBytes: Long = 192L * 1024L * 1024L)
```

There is intentionally no permissive default for `isCurrent`. Root controllers
must compare project ID, document ID, host sequence and state hash, plus the exact
attachment epoch. Each delivered `GiantVisibleFrame` carries those values, the
viewport generation, camera envelope and source/manifest identities. Recheck all
of them against current state at the actual draw boundary; loading or decoding is
not a presentation receipt. Hide/detach the old frame on revision/viewport changes.
Preserve current R164 background/layer/mask freshness and color consent paths.

The provider must settle each native read before returning cancellation. A native
open hook should be a separate capability on the project facade, initially
`openGiantImage(binding: WorkflowBinding, diskCacheBytes: ULong,
ramCacheBytes: ULong): GiantImage`, admitted on the real project worker. Its handle
must retain the original asset/document binding and reject a changed/deleted
source; the outer full revision/attachment guard remains mandatory for adoption.
The integration owner retains this API, NativeCore changes, UniFFI registration,
controllers, Main, WorkbenchApp and actual backend wiring.

`GiantRenderBackend.retire()` must destroy/detach the renderer's recorded pipeline
before acknowledging completion. `release()` then destroys only its own resource.
A failed retirement retains every image and byte charge. Keep that failed owner in
the existing controller retirement/quarantine registry; it cannot be discarded as
success or used to credit a replacement renderer. Close may report that failure
after joining the actual native read and closing its native image source.

## Coordinator: workspace and JPEG stream boundary

The root workspace wildcard already discovers `core/crates/vw-stream`. Its owned
manifest pins only existing versions: vw-model/vw-proto/vw-geom `=0.1.0`, image
`=0.25.10` with JPEG only, png `=0.18.1`, thiserror `=2.0.21`. Update Cargo.lock in
root ownership to admit the new package. This lane did not edit the lock or any
root manifest. No current protocol schema change is required by these pure leaves:
they use the existing CaptureGeometry, FrameTiles, TileImage, RectI and UUID shapes.

Expose no native stream API until the coordinator has composed the preserved
`transport/t3-stream-finish/new/core/crates/vw-ffi/src/session/stream/` adapters
against current pairing/grant/focus/retirement paths. Pass only authenticated
connection epochs and explicit local capture approval into `RegionConsent`.
Reserve ring bytes before CPU readback; pinned/evicted originals continue charging
that same pool. Codec workspace and actual platform/GPU resources are separate.
The per-ring and per-codec-call limits are not a global session limit. Root must
reserve queued/drawing/displayed/frozen DisplayFrame owners, verified originals,
wire/BLOB buffers and all foreign copies in one bounded session ledger before
new work. Reuse the preserved stream memory/retirement adapters; retaining an Arc
does not make its pixels free, and these policy caps are not measured process RSS.

Use `PresentedFrames::begin_presentation`'s exact token through actual draw and
`presented`/`not_presented`. A newer decode never changes the sole in-flight draw.
Only accepted physical pen DOWN calls `FreezeMachine::first_contact`; hover,
prediction, rejected tools/palms and remote-edit mode do not freeze. On source
replacement stop/retire old drawing before invalidating it, and explicitly reset
the decoder/presentation owners. A same-session keyframe now rejects changed
source geometry or a non-increasing capture clock; no silent source switch.

`EncodedOriginal` is an exact transport BLOB, not a standalone user export. Bind
its content address and FrameIdentity to the authenticated outstanding request;
`VerifiedOriginal` alone may settle lossless replacement. A timeout explicitly
retains degraded quality. Durable capture publication still requires the existing
canonical revision/source/capture provenance and CORE-006 export path. Do not
publish raw stream PNGs as metadata-complete exported files.

## Preserved next-stage sources and acceptance gaps

The Android producer/viewport/intake R2, Windows producer R3, desktop viewport R2,
native giant import and PNG export R2 are preserved and were inspected. They are
not overlaid onto current registration/store/app files. Native import needs
bounded DiskCache reads and project-root access; verify existing store streaming
blob behavior rather than copying the old store postimages. Platform producers
must prove their approved output derives from the retained original before cache
adoption; arbitrary external tile directories are not image import formats.

Unimplemented/unverified at this checkpoint: native giant open/import/rebuild,
real producer/decoder operation and approved runtime packaging, original BLOB
transfer, giant JPEG/HEIC routing, actual Android/Desktop renderer leases and app
adoption, generated bindings, device/GPU/RSS/cancellation latency, A07/A10/A11 and
frame-rate/latency acceptance. Restricted AVIF remains unchanged. No HEVC or Krita
expansion is included. Source formatting/census/secret scans do not pass these
runtime or product gates; focused module/JVM tests await the coordinator's next
heavy-slot grant and API/lock admission.
