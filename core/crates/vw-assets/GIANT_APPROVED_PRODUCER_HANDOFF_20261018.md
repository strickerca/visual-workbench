# Bounded producer recovery and focused host validation

Based on composed candidate `6f3e2f3b9ed7a760d4dd7f4c6f26fa44dc5ad88d`.
This implements private producer/preparation/publication leaves. It does **not**
complete an approved, usable Windows/Android import workflow or pass a product gate.
No runtime was installed, no device or provider was contacted, and no new license
was accepted. Root manifests/lock, public DTOs/registration, controllers and apps
were preserved. R164 display conversion and restricted still-image AVIF are unchanged.

## Recovery and actual behavior

The recovered Windows R3 `vw-assets-win` source and fixed runtime pins, Android
region JPEG producer, and native giant_import publication/rebuild sequence were
verified against preserved source-candidates byte/SHA manifests before adaptation.
The earlier raw caller-result-directory approval factory was not promoted.
Recovery receipts are retained under `/tmp/vw-assets-validation/giant-producer-next/`.
The unified `giant-approved-producer-20261018.json` binds final source and test logs.

The admitted private input contract is implemented as
`RootOwnedPickedOriginal { file: File, work: TempDir }`, `GiantInputLimits`,
`GiantSourceInfo` (BLAKE3 and SHA256 are `[u8;32]`), `prepare_original`,
`PreparedOriginal::describe`, and `build_approved`. The file/private directory
already belong to root before asynchronous dispatch. No arbitrary picker path is
reopened by preparation. A 64KiB streaming copy creates `original.input` inside
that exact owned directory, verifies length/hashes and bounded PNG/JPEG metadata,
and requires explicit untagged-sRGB interpretation. No whole-original Vec exists.
The source File remains pinned, and subsequent actual runner reads verify that
private original. Only opaque, untagged 8-bit PNG/JPEG are admitted natively;
Android's runner additionally requires JPEG and orientation-only EXIF metadata.

Preparation uses the existing four-startup admission before creating its worker,
two live input permits, and a retained 4MiB source envelope. Abandoned preparation
futures seal the actual copy at its next checked chunk. Work admission is checked
against actual available disk after reserve and a conservative bound of
`original*4 + oriented_pixels*24 +128MiB`, capped by the supplied <=16GiB limit.
The largest nominal pyramid geometry therefore need not fit this work policy.

Only the concrete Windows runner can construct a production `ApprovedBuild`.
The Windows native adapter uses the supplied session budget, a 256MiB whole-child
Job/process cap, <=2 libvips threads/cache64MiB, fixed8.18.7 DLL pins, locked input/
helper/runtime ancestors/files and joined actual termination. Parent manifest,
encoded and decoded allocations reserve first. The actual inventory retains its
24MiB charge. Runtime setup is a separate root-only packaged capability; ordinary
`build_approved` has no helper/runtime/hash override. The process-wide packaged
capability starts absent: source recovery alone never approves it.

The Android producer keeps its actual band/preview/tile algorithm, all8
orientations, lossless base/working levels and JPEG upper levels. Its mandatory
80MiB supplied `SessionResources` charge is retained with the single producer slot
through actual joined work, bitmap/decoder/canvas retirement, typed result use,
adoption and exact cleanup. `AndroidTileBuild` has a private constructor and
`withRootOutput` serializes adoption against close. SDK<30, unsupported metadata,
profile/depth/alpha, excessive band/stride/disk/metadata census refuse. This is an
admission envelope, not a measured Android heap/codec/driver peak.

Native verification checks exact source before/after, complete canonical level/
tile inventory, no links, manifest/hash/length, exact tile codec/dimensions/opaque
pixels and shared limits before cache mutation. Installation removes the old
manifest first and writes a complete manifest last. A bounded process-private
64-entry approval registry admits the exact final root + original + manifest +
same session pool only after final binding/source fences. No persisted marker or
external manifest grants approval. Restart/eviction requires an approved rebuild;
already-open immutable image handles retain their own admitted inventory.

Publication streams the original through BlobStore, creates a complete sibling
project, verifies exact model/binding/source/cache, then publishes without clobber.
Accepted durable projects survive later cancellation/reopen/sync failures. Bounded
provenance uses the existing flat `software`/`color_profile` privacy allowlist;
original bytes/profile/depth stay unchanged. Rebuild installs only derived cache
inside one serialized project worker job with full project/document/hostSeq/
stateHash fences before and after. The separate built worker joins actual project
work even if its awaiting foreign future disappears; no self-await occurs.

At most one build and one publication attempt are admitted per prepared/built
owner. Failed cleanup quarantines its exact directory and retained source/permit/
charge, preventing repeated failures from admitting unbounded new work. Explicit
close waits for actual worker/state destruction and reports cleanup failure.
Quarantine recovery is not claimed. Windows teardown responsiveness is unmeasured.

## Root integration required

1. Add this **proposed only** target edge to `core/crates/vw-ffi/Cargo.toml`:

   ```toml
   [target.'cfg(windows)'.dependencies]
   vw-assets-win = { path = "../../../host-win/crates/vw-assets-win", version = "=0.1.0" }
   ```

   Existing workspace glob includes the new crate. The isolated validation lock
   delta adds only its workspace package and the FFI dependency edge; no external
   version changed. Exact delta: `/tmp/vw-assets-validation/giant-producer-next/native/proposed-lock.diff`.

2. Register/lower private input/producer owners from root. Do not expose a raw
   result-directory, runtime path/hash or approval boolean to callers. The native
   factory expects an owned File and TempDir, not a path DTO. Root picker/blob/FD
   admission before dispatch remains necessary; no public input callback was added.

3. **One deliberate return-type refinement needs root adoption.** The admitted
   `GiantBuiltCache::publish` was proposed to return `Arc<ProjectSession>`. The
   implementation currently returns `PublishedProject`, owning actual
   `ProjectStore`, `BlobStore`, `SessionPermit` and a retained32MiB shared charge.
   Existing `ProjectSession::from_store` cannot retain that charge. Root must add:

   ```rust,ignore
   ProjectSession::from_store_budgeted(store: ProjectStore, blobs: BlobStore,
       permit: SessionPermit, charge: vw_stream::MemoryCharge)
       -> crate::Result<Arc<ProjectSession>>;
   ```

   Put all four incoming arguments in one owner **before any fallible work**.
   Keep charge last in actual ProjectState, through actual worker/state teardown.
   Root's facade then immediately moves `PublishedProject::into_parts()` into this
   constructor and returns the admitted Arc type. Do not drop the charge after
   ordinary `from_store`, or in the foreign reply wrapper. No undefined root hook
   or temporary uncharged public session was added in these leaves.

4. Windows root packaging must review unchanged pins/notices, build/pin the exact
   helper and obtain a real Windows execution/Job teardown receipt before calling
   the private `pin_windows_runtime` / `admit_packaged_windows_runtime` seam. These
   are trusted packaging inputs, never producer request fields. The latter stores
   the already-pinned same-pool capability once. Windows timeout minimum currently
   remains1000ms from the recovered worker; requests below that refuse.

5. Android needs a trusted JNI/platform bridge that actually calls the Kotlin
   runner with the owned input and the same native session pool, retains the typed
   `AndroidTileBuild` through `withRootOutput`, binds source hashes/length/geometry,
   and mints native completion only from that actual runner's settled ownership.
   Existing JNI code has no general JavaVM/runner bridge to reuse. A public path/
   manifest completion factory is not an acceptable substitute. Consequently
   `AndroidRegionJpeg` currently returns Unsupported in native `build_approved`.
   Root must also retire the Android80MiB result before overlapping publication/
   a96MiB surface when the aggregate would exceed256MiB. Publication also needs
   the64MiB resident +64MiB transient cache envelope beside its retained project/
   inventory/workspace owners; it can correctly refuse beside an old96MiB surface
   even after the80MiB producer retires. Retire the old canvas before publication
   and open the new surface after publication resources have settled. A safe bridge may copy
   verified tiles into native-owned output while typed adoption is locked, then
   close/join the Kotlin result before native publication. That copy needs its own
   exact disk/memory admission and source/inventory fences; it is not implemented.

   A further phone rendering blocker follows from the current proposed retained
   project allowance:32 project +96 surface +64 resident cache +24 manifest +64
   transient index =280MiB before encoded tiles, above256MiB. Native host success
   uses512MiB and does not pass phone rendering. Root must prove a smaller project
   allowance or a bounded immutable/read-only cache path that avoids full transient
   index reconstruction per read. Do not shrink envelopes or drop live credits
   merely to force admission. This source correctly refuses the overlap.

Root wrappers can use `PreparedOriginal::close_worker` and
`GiantBuiltCache::close_worker`. Platform runtime approval and the public workflow
remain pending after these source/test leaves; ordinary Missing/Unsupported must
continue to refuse truthfully until composition is concrete.

## Verification scope

Final focused Linux native batch passes52:29 new producer/publication cases and23
existing affected provider/delivery cases. It covers real local BlobStore/project
publication, no-clobber, retained resources, joined full-binding rebuild and an
approved fixture tile read. Synthetic completion construction is test-only; those
cases are not OS producer approval. The recovered Windows crate's14 portable
admission/cancellation/budget cases pass. Strict all-target Clippy passes for both
`vw-ffi` and `vw-assets-win` on Linux. These66 current cases are not additive to
all491 native or816 JVM results reported by the integration owner.

Validation ran in an isolated worktree at the composed candidate, with exact owned
source copied in and the proposed target edge/lock there only. Coordinator-owned
manifest/lock were never edited. Resource limits were two Cargo workers/two test
threads and bounded process-tree commands; heavy builds remained serial.

Offline licenses, dependency bans and sources pass. Full advisory checking cannot
run because this environment lacks its local advisory database; no fetch was used.
Windows-target checking was attempted and stops in native C tool setup at missing
`lib.exe`; Windows-specific code/execution and12 Windows-only cases (including2
opt-in packaged runtime cases) remain unverified. This environment has no Android
SDK platform `android.jar`; the new Kotlin leaves and19 instrumented cases remain
uncompiled/unrun here. No device, runtime DLL, helper binary or SDK was installed.
Earlier recovery hashes/lexical checks are source evidence only.

Native failures were retained: a generic fixture cache type mismatch (including an
isolated-copy synchronization replay), nested provenance refused by the actual
model privacy allowlist, and two strict Clippy corrections. The final receipts
bind repaired source and passing logs, not the failed versions.

No claim is made for actual Android/WIC/libvips/GPU behavior, target performance,
codec/RSS peaks, whole giant image usability, HEIC/HDR/multiframe/AVIF encoding,
network original BLOB transfer, giant PNG export wiring, or hardware/owner gates.
The temporary image fixtures are synthetic and never committed. Existing portable
format/color/PNG foundations and R164 conversion were preserved without rerunning
unaffected checks.

Next task is root retained-session/picker/platform bridge composition and actual
platform validation (large). Keep the owner's Sol6.1/xhigh setting; this worker
cannot inspect its own hidden model/effort. The two child executors explicitly
used the requested settings. Heavy-slot release is reported with the final commit.
