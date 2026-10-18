# Native giant provider and session accounting handoff

Contract base: `32b60a5a0c9db925039d359d9b4f73704e51833b`, then the published
delivery contract `8d95427fa169fe0d68816f1c7375dbac0d65a7ef` by fast-forward.
This branch changes only delegated native leaves, pure stream ownership and
lane-local evidence. No root registration, manifest/lock, shared/app/controller,
renderer or store implementation was edited.

## Returned source and validation

`vw-stream` now exports one shared `SessionBudget`, seven `MemoryKind` categories
and non-Clone RAII `MemoryCharge`. Ring/capture reservations, encoded packets,
codec workspaces, decoded and actually presented/frozen frames, verified originals,
wire/BLOB copies and foreign admission compose that supplied pool. Shared Arc
owners retain charges through their last reader; physical copies reserve again.
Production budgeted entrypoints refuse legacy uncharged or different-pool inputs.
The old isolated utilities remain explicitly documented as such. DTOs are
transport receipts, not metadata-complete exports. `try_update` replaces the
recovered deprecated atomic API. No R164 or HEVC replacement is made.

Current focused evidence: **57 stream +59 portable +5 R164 display +16 narrow JVM
viewport cases =137 passing cases**, zero failures/ignored. Strict all-target
Clippy passes for stream and the three portable crates. Cargo-deny passes.
The original 13 viewport cases also passed at the earlier contract; they are a
subset of the current16 and are not counted twice. The JVM evidence uses exact
production viewport/API source, the full WorkflowApi, exact extracted ProjectInfo
and SessionResources interface declarations, and a small annotation/assertion
adapter with coroutines1.9.0. Production compilation uses explicit API strict
and `-Werror`. This is not Gradle, generated UniFFI, NativeCore, app or device proof.

The five new native files are syntax/formatted source only: `provider.rs`,
`source.rs`, `tiles.rs`, `producer.rs`, `tests.rs` under `vw-ffi/src/giant_images`.
There are **20 unrun native cases**, registered from provider.rs once root admits
that module. They include private reader fixtures; those fixtures deliberately
do not claim an approved production producer. Root's explicit ownership rule
prohibited editing registration merely to make an isolated test command pass.
Native type checking, lint and these cases belong to the next coherent phase.

## Exact registration and delivery hooks

Root adds the four owned modules, exports `provider::GiantImage`, and calls:

```rust
provider::open_image(project_worker, project_closed, options,
    session_resources::shared_budget()?, cancellation).await
// -> GiantImageResult<Arc<GiantImage>>

image.read_viewport(view, cancellation).await
// -> GiantImageResult<GiantViewportPlan>
image.read_tile(address, cancellation).await
// -> GiantImageResult<provider::ChargedTile>
image.close_worker().await
// -> GiantImageResult<()>
```

Root retains its public/native viewport/tile/close method names and annotations.
`ChargedTile::into_parts(self) -> (GiantTilePixels, vw_stream::MemoryCharge)` is
the exact consuming hook for `marshal_delivery(pixels, outgoing)` published at
8d95427. Never clone/project the pixels while discarding that outgoing charge.
The resulting root `GiantTileDelivery { pixels, native_lease }` retains outgoing
and separately pre-admitted ABI storage until complete generated-reply decoding.
The already reserved common foreign lease must continue through actual renderer
retirement; failed retirement/failed charge release must remain quarantined.

Two root-side corrections are needed before using the existing marshal helper:

1. Put both function parameters into a struct owner **before any fallible work**,
   with pixels declared before outgoing. Function parameters drop in reverse
   order on an early error; the bare `(pixels, outgoing)` parameters can credit
   outgoing just before destroying the actual pixel allocation. Keep this owner
   intact through validation, shared-budget lookup and ABI reservation, then move
   both fields only into a successfully constructed delivery.
2. Check `outgoing.belongs_to(&budget)` and `pixels.rgba.capacity() as u64 <=
   outgoing.bytes()` before admission. Both outgoing and ABI charges must belong
   to the root process pool; do not silently mix a test/private or new pool.

These are exact integration proposals, not edits to root-owned code. Pixel
records, owners and diagnostic output do not print raw bytes. Root's global
process pool also prevents opening a successor session from resetting old live
charges. The stream transport parser must enforce its bounded wire/field census
before allocating an inbound DTO; `TilePacketReservation::adopt` validates an
already allocated packet and does not replace that upstream parser admission.

## Provider invariants and actual limits

The factory captures the full project/document/hostSeq/stateHash on the serialized
project worker. Every operation repeats capture before work and after result;
nested Result preserves GiantImageError through Worker::call. No project worker
await occurs from inside its own closure. Image close seals admission and joins
actual worker/state destruction. Dropped delivery futures leave ownership in the
worker/result until actual settlement. Cancellation and project/image closure
are checked during streaming hash chunks, not merely at adoption.

Only the selected non-paged image document's immutable project blob is read.
The source has no user-selected producer directory. Its original BLAKE3 and
exact byte length are streamed/checked, and bounded headers check actual format,
all8 EXIF orientations and oriented dimensions. Admission is restricted opaque
8-bit untagged PNG/JPEG with no implicit profile/depth conversion; known HDR,
MPF/JUMBF, alpha and profile headers refuse. Header inspection alone is not full
codec validation. Original bytes are never rewritten or loaded into a full Vec.
Hashing currently repeats before and after each viewport/tile read (up to4GiB
per pass); latency and throughput are unmeasured and may require a separately
proved immutable-source/worker stage before production use.

The manifest is read with `get_bounded(MAX_MANIFEST_BYTES)` and bound to that
exact source. Tile reads use the record's exact encoded length and the2MiB cap,
verify content hash, reject auxiliary WebP chunks/metadata or unsupported JPEG
headers, bound codec expansion, and require exact dimensions/orientation/no ICC/
opaque RGBA8. D-space rectangles clip odd edges without scope expansion. Singular
physical transforms refuse; maximum singular scale preserves detail under shear.
There is no whole-original decoder fallback or proxy-as-original substitution.

The session pool admits all allocations first. The validated cache map holds a
64MiB resident envelope; a separate64MiB transient envelope covers opening and
each JSON/index read/write and drops before decode. This retains the original
128MiB index peak while allowing the Android96MiB surface plus24MiB manifest to
coexist with32MiB decode work in256MiB. Metadata and cache limits still apply.
Encoded reads include DiskCache's sentinel capacity. Every cached tile owns its
session charge through actual Arc leases. Shared pressure clears only unpinned
cache owners before one retry. Outgoing copies get a second transfer reservation.
These are conservative admission envelopes, not measured RSS/driver/codec peaks.

## Approved producer and import/rebuild stage

The inspected candidates are the archived native `giant_import/{source,tiles,
tests}.rs`, Android `shared/giant/AndroidJpegPyramid.kt`, Windows
`GiantWindowsProducer.kt`, host `pyramid.rs`, and `vw-assets-win/runtime-pins.json`.
Seven archived SHA/byte checks are retained in the JSON evidence. Source header
code was recovered/adapted; no old store/root/controller postimage or generator
was overlaid. The native candidate's raw caller-directory publisher was not
promoted as an approval boundary.

`producer::stage(&CapturedSource, disk_cache_bytes)` returns an internal bounded
`StagedProducerPlan`: source ID/bytes/oriented dimensions/orientation/format,
256 tile side/count, disk cap<=16GiB,2MiB tile/8MiB manifest caps,32MiB band cap
and whether that band fits. This is planning data, not runtime approval. Android's
inspected producer is JPEG-only; a PNG band fitting does not make PNG supported
on Android. Untagged-sRGB consent is still required before any production build/
display path. The plan has no credential/provider call or claimed installed codec.

**No approved producer is currently admitted.** Missing cache returns Missing.
An otherwise valid inventory returns Unsupported at `require_approved_producer`.
No manifest, caller boolean or self-authored persisted approval marker can bypass
this. Consequently the public factory cannot yet open an image for real rendering.
It does not complete import, cache rebuild, giant PNG export or original transfer.

The next root-owned input/producer API should preserve this exact ownership seam:

```rust
// Proposed private interfaces; not newly registered public DTOs.
struct RootOwnedPickedOriginal {
    file: std::fs::File,          // root-selected blob or private picker copy
    work: tempfile::TempDir,      // exact operation-owned private staging
}
struct GiantInputLimits {
    max_original_bytes: u64,      // 1..=4GiB; Android intake<=1GiB
    max_work_disk_bytes: u64,     // 1..=16GiB
    assume_untagged_srgb: bool,   // explicit original/display interpretation
}
struct GiantSourceInfo {
    source_path: std::path::PathBuf, // runner-only path inside owned staging
    asset_id: [u8; 32],
    sha256: [u8; 32],
    encoded_bytes: u64,
    raw_width: u32, raw_height: u32,
    width: u32, height: u32,
    orientation: u8,
    format: String,               // admitted "jpeg" or "png" only
}
enum GiantProducerPlatform { AndroidRegionJpeg, WindowsPinnedVips }
struct GiantProducerOptions {
    platform: GiantProducerPlatform,
    disk_cache_bytes: u64,        // 1..=16GiB
    timeout_ms: u64,              // 1..=600000; not a cancellation-latency claim
    assume_untagged_srgb: bool,
}
struct GiantPublishOptions {
    path: String,                // root-admitted new project, no clobber
    project_id: String, document_id: String, layer_id: String,
    device_id: String, title: String, now_ms: i64,
    disk_cache_bytes: u64,
}

prepare_original(input: RootOwnedPickedOriginal, limits: GiantInputLimits,
    budget: SessionBudget, cancellation: Arc<Cancellation>)
    -> GiantImageResult<Arc<PreparedOriginal>>;
PreparedOriginal::describe() -> GiantSourceInfo;
// PreparedOriginal owns source/work/budget through every actual runner read.

PreparedOriginal::build_approved(options: GiantProducerOptions,
    cancellation: Arc<Cancellation>) -> GiantImageResult<Arc<GiantBuiltCache>>;
// Uses the PreparedOriginal's supplied shared pool. No caller-selected result
// directory, helper/runtime path, runtime hash override or approval bool.
// Native owns exact staging; only the actual pinned platform runner can mint
// GiantBuiltCache's private completed witness after real worker settlement.

GiantBuiltCache::rebuild(project_worker, project_closed,
    expected: WorkflowBinding, cancellation: Arc<Cancellation>) -> GiantImageResult<()>;
GiantBuiltCache::publish(options: GiantPublishOptions, cancellation: Arc<Cancellation>)
    -> GiantImageResult<Arc<ProjectSession>>;
// Recheck full source/binding, inventory/codec/hash and current budget before
// no-clobber publication; close joins workers before exact owned cleanup.
```

Android needs the real BitmapRegionDecoder callback/private input owner and
all8-orientation band tests. Windows needs the preserved vw-assets-win internal
crate/helper/runtime-pin owner and approved target dependency registration, plus
actual worker/job shutdown and packaging evidence. Existing package pins must
be reviewed; no binary installation or new third-party version was performed.
Platform owners must not replace this seam with an arbitrary directory receipt.

Restricted still-image AVIF remains unchanged: no HDR, multiframe, encoder or
new HEIC/giant routing claim. Actual Android/WIC/libvips/GPU, target memory,
performance, BLOB transfer and physical acceptance remain unpassed. No device,
paid service, live provider, credentials, deployment or license expansion occurred.

## Next coherent phase

Root registers the native modules/wrappers and fixes the marshal error ownership,
then runs the20 native cases and strict FFI lint before one binding generation.
Re-run only source-affected checks. Platform renderer owners retain their assigned
files and retirement evidence. The producer/import/rebuild stage above remains a
separate concrete implementation prerequisite, not a passed build milestone.
The worker heavy slot is released. Recommended next executor remains the owner's
requested gpt-6.1-sol/xhigh; this worker cannot inspect its own hidden model setting.
