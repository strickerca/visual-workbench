# Android giant producer allocation and retirement trace

The Android producer has a concrete bounded decode implementation and a private
typed result. It does **not** yet have a reviewed native completion bridge:
`PreparedOriginal::build_approved(AndroidRegionJpeg, ...)` currently returns
`Unsupported`. This trace specifies the ownership needed to connect that producer
to native publication and rendering without releasing admission while payloads
remain live. It does not approve an Android runtime or turn metadata hashes into
a producer capability.

This is a source inspection of the working tree based on
`af934061e0b19fb3a128cd40a803dd91cff58d30`. No build, test, Gradle, Kotlin compiler,
device, SDK installation or download was run for this trace. This worker's
local setup checkpoint reports `android: null`; its exported SDK root is absent.
That does not describe the coordinator's separate environment or contradict its
Android37.0 build. Android compilation of these leaves, actual codec behavior and
device evidence remain pending here. All sizes below use MiB = 1,048,576
bytes. Existing admission limits remain unchanged.

## One pool and the owners that retain it

`session_resources::shared_budget()` is the process-wide native pool, capped at
256 MiB on Android and 512 MiB elsewhere. Cloning it shares the same synchronized
pool. `NativeGiantImages.kt` obtains its resource pool through
`sharedSessionResources()`; Android code receives that existing common
`SessionResources`, rather than constructing a new pool for a producer or view.
Opening another project or endpoint must not reset outstanding charges.

| Owner or operation | Existing admission | Retirement boundary |
| --- | ---: | --- |
| `PreparedSource` private input/staging owner | 4 MiB, `Transfer` | Source file, source metadata and exact staging retire; failed directory removal retains its input permit and charge. |
| Android `ProducerOwner` | 80 MiB, `Foreign` | Actual worker joins; tracked platform owners retire; exact Android output cleanup and charge close succeed; only then does the single slot release. |
| Native approved manifest | 24 MiB policy | Manifest allocation is destroyed, or moves with its charge into the private `ApprovedBuild`. Android construction of this owner is still a bridge prerequisite. |
| Native producer/publication staging owner | 1 MiB in existing native staging paths | Exact `OwnedDirectory` cleanup succeeds; uncertain cleanup retains its source/charge owners. |
| Publication workspace | 16 MiB | Synchronous publication/rebuild call actually settles. |
| `PublishedProject` model/store owner | 32 MiB | Must transfer into the actual root `ProjectState` owner and remain through its destruction. |
| Provider `ImageState` disk index resident map | 64 MiB | `DiskCache` and associated allocations retire on the actual image worker. |
| Provider index open/install workspace | 64 MiB transient | Each constructor/index mutation scope ends after real work. This cap is not being lowered. |
| Provider manifest | 24 MiB | `ImageState.manifest` is destroyed before `_manifest_charge`. |
| Same-pool approval registry | 64 KiB per entry, up to64entries =4MiB | Registry replacement/eviction destroys the exact entry and its charge. Process-private approval does not survive restart. |
| Proposed read-only index touch workspace | 1 MiB transient | The bounded streaming touch finishes; this is conditional on the separate root store change described below. |
| Native encoded tile | Exact encoded length + 1 sentinel byte, at most 2 MiB + 1 | Encoded `Vec` is destroyed before its `EncodedTiles` charge. |
| Native tile codec | 32 MiB transient | Synchronous bounded decode actually returns or fails. |
| `CachedTile` pixels | 256 KiB maximum | The last payload/`CacheLease` Arc retires, even after cache eviction or manager close. |
| Outgoing native pixels and ABI reply | Exact pixel copy, plus a separate capacity + 16 KiB ABI reservation | The outgoing allocation and generated reply decode have settled; they are not renderer retirement. |
| Android render surface | 96 MiB, `Foreign` | Joined renderer/session/native close succeeds, then surface charge closes. |
| Each foreign viewport tile | 784 KiB, `Foreign` | Actual renderer retirement, CPU image release and charge close all succeed. |

The native local `BudgetCache` cap and the Android viewport's 96 MiB tile cap are
additional per-owner limits. Neither reserves its full allowance up front, nor
guarantees it can fill while other owners occupy the shared 256 MiB pool.

## Root input ownership before asynchronous preparation

[input.rs](input.rs) accepts `RootOwnedPickedOriginal { file: File, work: TempDir }`.
Root must already own both values before dispatching asynchronous work. A path,
picker URI, newly opened file inside an abandoned future, or caller-supplied
completion directory is not a substitute for these owners.

`prepare_original` uses the existing bounded startup mechanism, then creates its
serialized input worker. `input::prepare` reserves the 4 MiB owner and acquires
one of two input permits. It streams the selected file into the exact private
`original.input`, computing BLAKE3 and SHA-256 through a 64 KiB buffer. It checks
length, bounded JPEG/PNG header metadata, oriented geometry, explicit
untagged-sRGB consent and conservative work-disk/free-space admission. It does not
allocate an original-sized byte vector or rewrite the source's profile/depth.

`GiantSourceInfo.asset_id` and `.sha256` are each `[u8; 32]`; `encoded_bytes`, raw
dimensions, oriented dimensions, orientation and actual format accompany them.
Root converts the prepared SHA-256 to the Android factory's fixed 64-character
hex representation at its trusted boundary. These hashes bind bytes; they do not
prove which producer ran. The `PreparedSource` file, staging owner, permit and
budget must stay alive throughout Android reads and subsequent native adoption.

`PreparedSource::verify` and provider `CapturedSource::verify_owned` stream-check
the original again. `source::regular` rejects symlinks; Linux/Android opens use
`RDONLY | NOFOLLOW | CLOEXEC`, and the Windows branch checks reparse attributes
and opens with restricted sharing. These file flags do not themselves prove that
an arbitrary supplied path is a root-prepared immutable input. Root's existing
typed owner and the repeated source/binding checks remain required.

## Android source and work directory lowering

The current internal Android seam is:

```kotlin
JpegTileInput.fromRootPrepared(
    source: File, privateWork: File, sha256: String,
    rawWidth: Int, rawHeight: Int, orientation: Int,
    encodedBytes: Long, maxDiskBytes: Long,
    assumeUntaggedSrgb: Boolean,
): JpegTileInput

AndroidJpegPyramid.build(
    input: JpegTileInput, resources: SessionResources,
    progress: (TileBuildProgress) -> Unit,
): AndroidTileBuild // suspend
```

Both input and result constructors are private; these factories are internal.
There is no default resource pool or true default for interpretation consent.
Root must lower the native prepared owner, not arbitrary external paths. The
checked directory layout is a prepared source and sibling private work directory
under the same root-owned staging parent:

```text
<owned prepared staging>/original.input
<owned prepared staging>/android-work/
    vw-jpeg-<operation UUID>/
        .owner
        preview.jpg
        L0/*.webp
        L1/*.jpg ...
```

`requireInput` refuses SDK below 30, absent consent, nonpositive/excessive geometry,
more than one billion pixels, orientation outside 1..8, original length above
1 GiB, disk cap above 16 GiB, invalid SHA text, insufficient conservative disk
envelope, or a two-copy 256-row band above 32 MiB. File identity, exact byte length
and SHA are checked before codec use. Source decoding uses a shared-locked file
descriptor, not a content URI or a later reopened caller-selected source.

The source header census admits three-component 8-bit JPEG only. ICC, XMP, MPF,
JUMBF/HDR and unsupported APP metadata refuse. Current EXIF support is deliberately
limited to an orientation-only IFD. Bounds and actual bitmap admission require
sRGB; no HEIC, PNG, HDR, depth reduction or profile conversion fallback exists.
All eight orientation transforms operate on physical pixel edges. The source
stamp and final streamed SHA checks fence replacement or modification.

## Android 80 MiB operation lifetime

`AndroidTileBuild.build` creates its control owner, acquires the single process
slot, and calls the **supplied** `resources.reserveForeign(80 MiB)` before source
hash buffers, header reads, bounds parsing or platform bitmap/decoder allocation.
`ProducerRuntime.active` retains that same owner strongly, including a successful
result that a caller has not yet closed. The worker runs in the retained
single-thread producer scope; it is not a child job that disappears when the
awaiting caller is cancelled.

| Phase | Actual known owners and checks |
| --- | --- |
| Source admission | One 64 KiB hash buffer, bounded JPEG segment buffers, source descriptor/lock and small metadata. No original-sized byte array. |
| Preview | One sampled `BitmapRegionDecoder.decodeRegion` over the source extent with a power-of-two sample chosen by ceiling arithmetic; actual longest side must be at most 2048. Raw and oriented previews are each capped at 16 MiB. This is a bounded sampled preview, not full-resolution full-frame decode. |
| Full-resolution base | One raw region and its oriented band, each capped at 16 MiB; their combined geometry is admitted before decode. A cropped tile adds at most 256 KiB. `L0` is encoded as bounded lossless WebP so the decoded JPEG original's pixels remain authoritative. |
| Lower levels | At most a 512-square joined bitmap, one 256-square child and a 256-square scaled/copy output, under 2 MiB of known bitmap storage. CPU canvases are explicitly detached. Lossless `R*` files feed later levels; JPEG `L*` outputs do not feed repeated lossy recompression. Old `R*` levels are removed after use. |
| Output writing | No encoded whole-pyramid vector. Each stream is capped at 2 MiB per tile, preview at 16 MiB, and cumulative work at the admitted disk cap. Exact owned names are recorded before encoding. At most 65,536 compact file records and 64 directory records avoid repeated retained copies of the full app-private path. |
| Successful result | Every bitmap and decoder has retired and source verification has finished before the private `AndroidTileBuild` is minted. The 80 MiB charge and process slot nevertheless remain with the returned output owner until close. |

`keep` registers each returned bitmap before post-allocation admission. Admission
checks `ARGB_8888`, sRGB, opaque samples, positive dimensions, row stride,
`allocationByteCount`, per-resource limits, and the combined known live bitmap
cap of 32 MiB + 256 KiB. The bitmap/decoder/canvas registration arrays have fixed
initial capacity before platform allocation. Alias returns from `createBitmap`
or orientation do not create duplicate owners or double-recycle the same bitmap.

The 80 MiB allowance covers these phase-disjoint known owners plus conservative
codec/encoder and bookkeeping workspace. The compact metadata policy allocates
an 8 MiB design allowance within that envelope; Android/JVM object layout and
allocation peaks still need platform validation. Opaque platform decoder/encoder internals,
allocator behavior and driver RSS remain **unmeasured**. Post-allocation stride
checks supplement pre-admission; they cannot retroactively prove a physical peak
or turn an unexpected decoder allocation into a measured 80 MiB RSS guarantee.

Caller cancellation sets the operation flag and awaits the actual retained
worker under `NonCancellable`, including queued work, a blocking codec call,
a blocked progress callback, and a late completed result. Cleanup runs only after
that join. Android does not claim a hard cancellation-latency bound.

`AndroidTileBuild.withRootOutput` holds its adoption mutex and verifies liveness.
Root must hold that gate through the actual joined native reader; close waits on
the same gate. Returning from a callback that merely launched detached native
work would break this ownership requirement.

`closeNow` detaches tracked CPU canvases, recycles tracked bitmaps and decoders,
checks the exact bounded `.owner` bytes, removes only recorded files/directories,
refuses unknown entries, closes the session charge, then releases the slot.
If any retirement, cleanup or charge close fails, the active owner and charge
remain in quarantine and new builds return busy. Android's explicit
`retryQuarantinedCleanup` retries those exact owners. The marker authenticates
cleanup identity only; it never authorizes native cache adoption.

## Required trusted native bridge sequencing

The following is required integration work, not a registered or executed bridge.
The current Android platform branch must continue returning `Unsupported` until
the trusted boundary and its validation are reviewed.

1. Root retains the exact native `PreparedOriginal`/`PreparedSource` and supplies
   its `GiantSourceInfo`, root-created sibling work directory and explicit consent
   to `fromRootPrepared`. The `SessionResources` value wraps the same native pool.
   Root must join cancellation and retain an unadopted typed Android result if its
   cleanup fails; a marker, hash, directory string or caller boolean cannot mint
   a native approval capability.
2. Only the actual Android runner's private `AndroidTileBuild` enters the trusted
   bridge. Hold `build.withRootOutput { live -> ... }` and the original native
   owner through all native copying, inventory creation and verification. Compare
   full prepared SHA/BLAKE identities, exact bytes, raw/oriented dimensions and
   orientation before and after the accepted native work.
3. Root creates an independently owned native output `TempDir` before asynchronous
   adoption. Its `OwnedDirectory` must retain the source and its existing cleanup
   admission. Copy only the exact expected `L*/tile` inventory into that owner's
   `pyramid` child, using bounded streaming or individually charged tile reads.
   The Android directory also contains `.owner` and `preview.jpg`, whereas
   `cache::verify_output` accepts `L*` and optional `pyramid.vwt`. Validate those
   Android auxiliary entries separately; do not pass the entire Android directory
   off as a native completed cache.
4. Admit the native manifest under the existing 24 MiB policy **before** allocating
   its inventory. Derive it from actual checked output, with exact encoded lengths,
   hashes and geometry; Android does not supply a native manifest. Retain it with
   the new output/source owner. `cache::verify_output` rechecks the original,
   bounded inventory, every encoded record, strict tile codec/opaque admission and
   source again. Each encoded file has its own length + 1 charge and each native
   decode its 32 MiB workspace. Only this actually completed private runner path
   may mint `ApprovedBuild` and the `GiantBuiltCache` worker.
5. After native adoption has actually settled and owns independent output,
   successfully close the Android result. Its files cannot be aliased by the new
   `ApprovedBuild`: Android close always deletes them. Releasing the 80 MiB here
   follows retirement; it does not create a gap in ownership of any copied native
   manifest, output, source or pixel allocation. A failed close stops progression
   and retains the same owner and admission.
6. Publish or rebuild from the private `ApprovedBuild`. Publication reserves its
   existing 16 MiB work and 32 MiB new-project charges before model/store creation,
   streams the immutable original through `BlobStore::put_reader`, installs all
   derived tiles, installs the manifest last, and checks project integrity/full
   binding. No-clobber publication is the commit point: a later delivery error
   preserves the accepted final project for explicit reopen. Rebuild runs inside
   one accepted project-worker job with full binding/source checks before and
   after; the built-cache worker joins that job without awaiting the project
   worker from inside its own closure.
7. Transfer `PublishedProject` directly into one root payload owner. Its
   `into_parts()` returns store, blobs, session permit and the **same** 32 MiB
   charge. Bundle them before any fallible work or async dispatch, preserve drop
   order, and retain the charge through actual `ProjectState` destruction. At this
   inspection, `ProjectState` and the three-argument `ProjectSession::from_store`
   have no such charge field/parameter: the budgeted root constructor is still
   required. Calling that existing constructor and discarding the charge would
   make the proposed 32 MiB retention false.
8. After accepted publication and root facade adoption, close the built-cache and
   prepared-input owners before opening the renderer. Actual native manifest
   destruction releases its 24 MiB; output cleanup and the last source Arc release
   their cleanup/input reservations. `cleanup::OwnedDirectory` retains failed
   exact directory removals and associated source/permit/charges in native
   quarantine, with no automatic retry. A persisted cache or provenance string
   does not recreate the process-private approval on restart.
9. Open the provider first, allowing its 64 MiB index-open workspace to retire,
   then create the 96 MiB Android surface owner. Keep the 64 MiB resident map,
   24 MiB provider manifest and every actual tile owner charged. Retire a previous
   recorded renderer/native owner before attempting a successor; retain failed
   creation/retirement in the existing attachment/controller quarantine.

Retiring the Android owner before publication is material to admission. A
publication/install example retaining prepared input4MiB, approved manifest24MiB,
output owner1MiB, publication work16MiB, new project32MiB, publication staging1MiB,
resident index64MiB, index workspace64MiB and encoded tile2MiB totals about208MiB.
Keeping the old Android80MiB owner through that phase would make the same set
about288MiB, before other process owners. This is sequencing under the unchanged
256MiB cap, not permission to release a still-live Android output early.

The trusted coordinator helper can conceptually accept the original native
prepared handle and `AndroidTileBuild`, using `withRootOutput` internally. There
must be no public `complete(directory, approved=true)` or equivalent external
directory receipt. The private native capability and process approval registry
remain the authority. `cache::register` binds the final canonical project root,
source hash, manifest identity and same pool after final checks; descriptive
asset provenance and full hashes are not replacement approval.

## Provider read and foreign delivery pressure

The required steady owner floor for a newly published Android project is:

```text
32 MiB project + 64 MiB resident index + 24 MiB manifest + 96 MiB surface
    = 216 MiB, leaving 40 MiB of the Android256MiB pool
```

This floor assumes the missing root 32 MiB adoption is implemented and the
producer/input/publication owners have retired. It excludes other projects,
approval registry entries, cache tiles, foreign tiles, transfer/ABI charges and
temporary canonical binding work; those remain additional real admissions.
The same-pool registry retains at most64approval entries of64KiB each, so its
maximum alone adds4MiB to that floor.

Today's `DiskCache::get_bounded` reads the already bounded payload, then calls
`touch`, which calls `save_manifest`; that function uses
`serde_json::to_vec(&self.manifest)` for the **entire** index on every hit. Bounding
the tile length does not bound that temporary index representation to a tile.
The existing giant read wrapper therefore reserves 64 MiB around the touch.
Beside the 216 MiB owner floor, that scope alone needs at least 280 MiB before
new encoded bytes, native/foreign tiles and other live owners. It truthfully
refuses in the Android256MiB pool; silently dropping the index reservation would
hide the allocation.

The separately owned root proposal is `get_bounded_streaming`: keep bounded
payload integrity checks and durable LRU semantics, but serialize the index to
the exact temporary file through a fixed 64 KiB buffer with a 16 MiB JSON output
cap, sync and atomic replacement, without a full JSON `Vec`. Parent then swaps
the owned giant manifest/tile reads to that API with a 1 MiB read workspace.
This does not shrink the 64 MiB resident index, constructor/install workspace,
24 MiB manifest, 32 MiB decode, 32 MiB project or 96 MiB surface budgets. It is
conditional on source review and focused store/read validation, not an already
measured memory improvement or an Android runtime approval.

For one cold maximum-size tile, the proposed scopes are disjoint:

| Scope beside216MiB floor | Additional admitted owners | Illustrative subtotal |
| --- | --- | ---: |
| Bounded file read/LRU touch | 784 KiB foreign tile + at most2MiB encoded + sentinel +1MiB streaming workspace | About219.77MiB |
| Decode after touch returns | Same foreign/encoded owners +32MiB codec +256KiB new cached pixels | About251.02MiB |
| Native delivery after decode | Cached pixels +784KiB foreign tile + at most256KiB outgoing pixels + at most272KiB ABI allocation | About217.53MiB |

The arithmetic is an admission example, not a guarantee that every miss fits.
Existing cache/foreign tiles and process approvals add to it. The source verifier
reserves 1 MiB around streaming hash/header work, while `source::capture` reserves
the dynamic `workflow::canonical_workspace(..., false)` estimate for full binding
checks; the latter includes a 4 MiB base plus serialized-state copies. Those
checks precede/follow the actual tile work and can still refuse under pressure.
At the full registry bound, the cold maximum-size decode example is
`216 + 4 + 2 + 32 + 0.25 + 0.765625 = 255.015625MiB`, plus the encoded sentinel,
before any other cached or foreign owners. Root's canonical check needs
`4MiB + 4 × model workspace estimate` before/after the read and can refuse even
before the cache-clear retry is reached. Streaming touch removes the64MiB
read-touch requirement; it does not remove these reservations or establish
blanket phone usability.
Cache clearing releases only unpinned payloads and retries once; a pinned Arc,
failed retirement or another session cannot be credited to make the calculation
fit. Local64MiB native-cache and96MiB foreign-tile caps cannot both be assumed full.

`CachedTile::decode` admits encoded length + sentinel before the bounded read,
then admits codec and retained RGBA before decode. It verifies encoded content,
strict lossless WebP at level0 or strict baseline JPEG above it, exact dimensions,
orientation1, no ICC and opaque RGBA8. Source/profile/depth/geometry flags that
change must refuse; no whole-original or implicit conversion read is a fallback.
`read_tile` repeats full source/binding checks before and after work. Source
streaming buffers and native decode temporaries remain real owners, rather than
being erased by an unchanged source hash.

`ImageState::tile` makes a separately charged outgoing pixel copy.
`marshal_delivery` immediately bundles pixels and that charge, checks same pool
and actual capacity, then reserves a separate ABI allowance. The returned native
lease holds outgoing and ABI charges through generated-reply decoding. Kotlin
already holds its independent784KiB tile charge before calling native; releasing
the native lease after reply decode therefore does not credit the Kotlin
ByteArray/bitmap/texture copy. Unknown native lease release failures stay held.

`GiantViewportSession` owns that foreign charge beside each actual backend image.
Android bitmap admission checks row stride/allocation, opaque8sRGB and known
ByteArray/CPU/texture/row copies before any upload. A cancelled creation retains
its acquired bitmap in the backend handoff. Retirement hides the frame, joins the
recorded renderer callback, releases failed/published images, then closes each
charge. `GiantAndroidCanvas.close` credits the96MiB surface only after session and
native close succeed; failed creation retains its concrete owner. Clearing a
frame, changing generation, cancelling a coroutine, recycling a wrapper or a
timeout is not a renderer retirement acknowledgment.

## Conditions before Android production admission

- Review and implement the trusted typed Android bridge, independent native
  output ownership and actual completion capability. Keep the Android native
  platform branch unsupported until that boundary exists.
- Implement the root32MiB project facade adoption and preserve it through every
  success, error, abandoned reply and actual worker shutdown path.
- Review the streaming index-touch store path and1MiB giant read integration;
  retain existing64MiB open/install admission and run the source-affected checks
  in the coordinator's serialized build slot.
- Establish SDK30+ compilation, bounds `outColorSpace` behavior, all-eight
  orientation/band-edge pixel results, JPEG/WebP output subset compatibility,
  cancellation/cleanup/charge-failure cases and actual renderer retirement on the
  named authorized device. The19 Android producer cases are written, not run by
  this trace.
- Measure platform codec/encoder/driver and graphics peaks, cancellation latency
  and relevant performance separately. Session caps, actual allocation checks,
  source hashes and this arithmetic do not provide those measurements or owner
  acceptance.

Primary inspected sources are [input.rs](input.rs), [cache.rs](cache.rs),
[publish.rs](publish.rs), [cleanup.rs](cleanup.rs), [producer.rs](../producer.rs),
[provider.rs](../provider.rs), [tiles.rs](../tiles.rs), [source.rs](../source.rs),
native session resources and delivery lowering, `AndroidJpegPyramid.kt`,
`GiantViewportSession.kt`, `NativeGiantImages.kt`, `GiantAttachmentOwner.kt`,
`GiantAndroidBitmap.kt` and `GiantAndroidCanvas.kt`. Only this trace file is owned
by this documentation task; production source and admission constants were not
modified.
