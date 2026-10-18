# Giant viewport and stream implementation ownership

This is a contract/source checkpoint, not a usable giant-image or stream workflow.
T3.05 is independently reviewed and published to main at exact
`83663107d864b21d22d2e746b9ca6eb80d47beac` by a normal fast-forward push.
Its Windows C/archive toolchain, mixed-peer compatibility, measured calibration,
Android instrumentation and physical gates remain as recorded. PDF stays gated.

The next candidate includes reviewed leaf source `ca773ed` as `f6bc5ce`, the
minimal common `GiantImagesApi.kt`, native `giant_images/dto.rs`, and these store
helpers: `ProjectStore::root_path()` is read-only; `DiskCache::get_bounded(key,
max_bytes)` refuses an oversized declared entry before payload access/allocation.
BlobStore already has streaming `put_reader`, streaming hash `verify` and checked
`path`; do not copy recovered store postimages. No new third-party versions are
admitted. Cargo.lock adds only vw-stream and its existing dependency edges.

## Root ownership and concrete API

Root owns Cargo/lock, all shared Kotlin contracts, native DTO/error definitions,
`vw-ffi/src/giant_images.rs`, root `lib.rs` and ProjectSession method registration,
store APIs, NativeCore adapters, both controllers, Main/WorkbenchApp and actual
renderer wiring. The common capability is `WorkbenchGiantImagesProject`:

    openGiantImage(binding: WorkflowBinding, diskCacheBytes: ULong,
        ramCacheBytes: ULong): GiantImage

GiantImage has suspend viewport(view), tile(address), close. The separate
SessionResources.reserveForeign(bytes) -> SessionResourceCharge contract wraps
the SAME native SessionBudget and keeps each idempotent close token beside its
actual foreign resource. Root must compose that required pool into the viewport
owner and backend before production wiring; the recovered local192MiB counter
alone is not session-wide accounting. Root owns the native foreign-budget object
and its UniFFI adapter, so the image owner need not invent a new public FFI type.

 The concrete DTOs in
GiantImagesApi and giant_images/dto.rs are authoritative. There is no permissive
isCurrent default. Full project/document/hostSeq/stateHash and attachment epoch
are checked at request, result adoption AND draw. Plan/tile source asset and
manifest hashes must agree with the immutable handle. No byte-bearing Debug or
unbounded fallback source read is allowed.

Root will register this native ProjectSession method after leaf review:

    open_giant_image(options: GiantOpenOptions, cancellation: Arc<Cancellation>)
        -> GiantImageResult<Arc<GiantImage>>

The native object owns asynchronous viewport(view, cancellation), tile(address,
cancellation) and close(). Its private factory contract is:

    pub(crate) async fn open_image(
        project: Worker<ProjectState>, project_closed: Arc<AtomicBool>,
        options: GiantOpenOptions, budget: vw_stream::SessionBudget,
        cancellation: Arc<Cancellation>
    ) -> GiantImageResult<Arc<GiantImage>>

Root supplies one shared budget for the active endpoint session; the provider
MUST NOT create a fresh budget for each handle/read/codec. Workers can clone the
existing Worker<ProjectState> and closed gate. Check the full binding on that
serialized project worker before source capture and every operation. Closure or
stale revision refuses results; cancellation joins actual work before returning.
Never await the project worker while already inside that worker's closure.
Use a nested Result when preserving GiantImageError through Worker::call.

## Delegated image-owner lane A: native source/provider

Exclusive new files under core/crates/vw-ffi/src/giant_images/:
`provider.rs`, `source.rs`, `tiles.rs`, `producer.rs`, `tests.rs`.
Do not edit root giant_images.rs, dto.rs, project.rs, store, manifests, lib.rs,
shared files or apps. Root adds module declarations/exports on return. Dedicated
module tests may live in these owned files and be registered by root for the
coherent native phase. No speculative public DTO or export signature change.

Implement the factory/object above by deliberately adapting preserved giant_import
source/tiles and reviewed foundations. Source capture uses actual document asset
metadata and streaming hash verification, never a caller-selected tile directory.
Bound manifest reads by MAX_MANIFEST_BYTES and tile reads by that record's exact
encoded length AND MAX_ENCODED_TILE_BYTES using get_bounded. Verify manifest source,
identity, tile hash, dimensions, orientation/depth/profile admission and opaque
RGBA8 before adoption. Budget reservations precede allocations/codec/native reads;
charge retained cache leases as well as transient decode workspace. No whole
original Vec read or implicit profile/depth conversion. Source IDs/full hashes,
exact page/source semantics and original bytes remain authoritative.

producer.rs owns the private approved-producer seam and deliberate candidate
promotion; an external arbitrary manifest is not approval. If an actual platform
producer needs additional root input DTOs, first return exact required signatures
and the inspected candidate, rather than installing binaries or inventing caller
paths. Missing/unsupported producer/cache must refuse truthfully; do not claim
that an authenticated cache reader alone completes import/rebuild. Preserve the
approved-platform boundary and return a bounded staged producer/import plan
alongside the provider source so root can compose genuine production callers.

Test source replacement, stale full hashes, closure/cancellation races, malformed
and oversized manifests/tiles, absent cache, codec expansion, exact tile geometry,
opaque/color refusal, cache lease lifetime and original preservation. Real WIC/
Android codec behavior is separate and unpassed on this Linux host.

## Delegated image-owner lane B: one stream session budget

Exclusive core/crates/vw-stream/src/*.rs and tests/*.rs (including root lib.rs
module/export declarations within this pure crate); no Cargo manifest/lock edits.
Add `SessionBudget` as a Clone wrapper sharing one synchronized pool, explicit
construction `SessionBudget::new(limit_bytes: u64) -> Result<Self>`, never an
unlimited default. Limits above 512 MiB or zero refuse. Export PHONE_SESSION_BYTES
256 MiB and DESKTOP_SESSION_BYTES 512 MiB; those are admission caps, not measured
RSS, and callers still enforce stricter per-resource/surface limits.

Export `MemoryKind` (RingOriginal, CodecWorkspace, EncodedTiles, DisplayFrame,
VerifiedOriginal, Transfer, Foreign) and non-Clone RAII `MemoryCharge`. The concrete
admission API is `budget.reserve(kind, bytes) -> Result<MemoryCharge>` plus a
read-only `budget.used_bytes()` and `budget.limit_bytes()`. Exhaustion/checked-arithmetic failure refuses
before work. A charge cannot be manually credited while payload ownership remains.
If shared pixels need multiple owners, retain the charge inside the shared payload
owner until its last Arc drops. A physical copy requires another reservation.

Compose budgeted ring, codec, presentation/freeze, encoded/verified-original and
transfer ownership in this crate; do not stop at an unused generic counter.
Original ring/evicted/pinned frames, queued/drawing/displayed/frozen frames,
old/new codec workspace, lossless verified originals, wire/BLOB and foreign copies
must all consume that supplied same pool. Preserve existing exact presentation
and freeze identities. Ownership may transfer with a payload without crediting
its bytes in between. Failed renderer retirement remains charged/quarantined by
root; root will retain corresponding foreign reservations through real retirement.

Keep DTOs transport-only: EncodedOriginal is not a metadata-complete user export.
No default capture consent, fake presentation ACK, source switch, timeout quality
upgrade, live provider call or R164/HEVC replacement. Existing unbudgeted primitives
may remain for isolated tests only if production budgeted entrypoints cannot
silently route through them without the supplied pool and all payload charges.
Replace the recovered ring.rs AtomicU64.fetch_update deprecation with the current
try_update API while composing the pool; contract compilation reported that one
warning, and strict Clippy is still pending.
Test aggregate pressure across all categories, allocation refusal, pinned Arc
retention, transfers without an admission gap, cancellation/late settlement,
concurrent reservation/release, overflow and failed retirement simulations.

## Validation/build slot

Root is publishing this source contract with format/import/locked-graph/policy
checks and a targeted bounded-cache regression. The prior51 written leaf tests
remain unrun until the image owner's native phase/shared root hooks are ready.
No bindings are regenerated at this checkpoint.

After parent forwards this handoff, the image owner may take the single heavy
slot for cargo-deny then focused pure native stream/assets tests (max2 workers).
Return exact tested source, commands/results, remaining provider registration
prerequisites and explicitly release the slot. Root will work on reserved
Kotlin/native/renderer seams in parallel without a heavy build. Provider tests
requiring root module registration join the next combined phase; do not edit
reserved registration merely to make an isolated worker test command pass.
