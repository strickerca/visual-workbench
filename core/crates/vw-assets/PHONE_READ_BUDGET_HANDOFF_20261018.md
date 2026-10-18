# Phone tile-read overlap: proposed source repair and bridge requirements

Follow-up to producer checkpoint `af934061e0b19fb3a128cd40a803dd91cff58d30`.
This checkpoint contains reviewable patch proposals and allocation/retirement
traces only. No production file, root-owned contract, manifest or lock changed.
The coordinator holds the heavy-build slot: no Cargo, Gradle, Kotlin compilation,
tests, runtime, device, SDK install or downloads ran for this follow-up. The
previous checkpoint's 66 passing host cases do not validate these proposals.

## Concrete read allocation and proposed root contract

`DiskCache::get_bounded` does not reconstruct the index on every read. Its
`BTreeMap` is resident. It clones one entry, reads and hashes the payload, then
`touch` writes durable LRU metadata. That write currently serializes the entire
index into a `Vec` before atomic persistence. The native conservative64MiB index
workspace covers this allocation; merely reducing the reservation would leave
an uncharged allocation and is not a repair.

The reviewable root patch is
`core/crates/vw-ffi/src/giant_images/producer/STORE_STREAMING_READ_PROPOSAL_20261018.patch`.
It proposes the existing-signature additive API:

```rust,ignore
DiskCache::get_bounded_streaming(&mut self, key: &str, max_bytes: u64)
    -> Result<Option<Vec<u8>>, StoreError>
```

It retains authenticated length/hash reads, the exclusive manager lock, durable
LRU and atomic index replacement. Index serialization goes through an outer
16MiB limiting writer into a fixed64KiB buffer and the existing temporary file,
with the existing flush/sync/persist/directory-sync sequence. It never materializes
a serialized full-index Vec. The new API bounds root/cache paths to16KiB; this
includes admitted canonical giant roots<=8192 bytes plus fixed cache suffixes.
Existing general APIs gain no new path restriction. Exact payload allocation is
bounded by declared length+one sentinel; a stack sentinel detects growth without
`read_to_end` reallocating past the admitted capacity. The root proof file explains
the proposed1MiB per-read scratch contract and unmeasured allocator/OS limits.

This is a proposal for the coordinator-owned store. It has not been applied.
The second patch,
`core/crates/vw-ffi/src/giant_images/producer/PHONE_STREAMING_READ_ADAPTER_PROPOSAL_20261018.patch`,
is ready for application after root admits that API. It changes only the owned
native manifest/tile readers to the new API and1MiB read workspace. Constructor,
publication, cache-install and remove/put64MiB envelopes remain unchanged. The
second patch also adds a real local publication/provider regression fixture with
the256MiB phone pool, a96MiB surface, a pre-call Android-sized foreign tile owner,
original-byte verification, cancellation and retained-owner teardown checks.
Synthetic producer approval is confined to tests; this is not Android execution.
The fixture uses the existing test-only facade owner for32MiB and does not complete
root's pending production `from_store_budgeted` constructor.

Both patches must be reviewed/applied together and typechecked/tested by the
coordinator before integration. Keeping the adapter in a patch prevents this
worker branch from introducing calls to an unapproved/absent store method.
A later schema/FFI/controller contract is not hidden in either patch.

## Allocation phases and remaining proof

All figures below are admission envelopes in MiB, not measured process RSS.
The256MiB limit is unchanged. Base live owners after production composition are
32 project +64 disk-map resident +24 manifest +96 surface =216MiB.

| Phase | Additional admission | Required lifetime/limit |
| --- | --- | --- |
| Open provider |64 constructor +24 manifest; surface absent | Old surface and producer/output/input owners must actually retire first. Opening beside32 project +96 old surface +64 resident +64 constructor reaches256 before approval metadata and correctly refuses. |
| Read/touch proposed | <=2 encoded +1 index scratch | Durable LRU still writes; encoded and raw manifest allowances are separately retained. |
| Decode | <=2 encoded +32 codec +0.25 new tile | Index scratch has retired; existing cached and foreign owners remain charged. |
| Native delivery | <=0.25 outgoing +<=0.25 ABI +0.015625 ABI metadata | Codec/encoded owners have retired. Root retains actual native lease through foreign decode. |
| Foreign tile |0.765625 each | Three256px RGBA copies plus16KiB; pre-reserved before native tile call and retained until actual renderer retirement. |
| Binding/source fence |4 +4x bounded project/replica estimate, plus bounded source hashing | Repeated before/after operations. Actual estimate is dynamic; it may refuse under pressure before the local tile-cache retry. |
| Approval metadata |0.0625 each, max64=4 total | Process-private exact root/source/manifest/same-pool registry. It survives prepared-owner close until eviction/process exit. |

With one foreign tile, all64 registry entries and no other retained tile, worst
admitted decode is216+4+2+32+0.25+0.765625 =255.015625MiB, before the encoded
sentinel byte. Further cached/foreign owners can force refusal. The existing
cache-miss retry clears only unpinned actual native payloads before one retry;
pinned/foreign owners remain charged. The patch is intended to remove the
unconditional280MiB read overlap, not guarantee arbitrary viewport concurrency.
The regression fixture is a small real local payload; maximum inventory/codec
peaks, backpressure with pinned renderers, full ABI/JNI lifetimes and platform
RSS require separate scheduled validation.

## Android bridge and staged retirement

The detailed source trace is
`core/crates/vw-ffi/src/giant_images/producer/ANDROID_BRIDGE_AND_RETIREMENT_TRACE_20261018.md`.
The bridge is still absent; native Android builds still return Unsupported.
A trusted root adapter must call the actual Kotlin runner with the root-owned
source and same session pool, keep its typed `AndroidTileBuild` alive and adopt
only under `withRootOutput`. The Android output includes `.owner` and preview
files; native expects a separately owned `<output>/pyramid/L*` inventory. Closing
Android output deletes it. The adapter must therefore verify and copy normalized
tiles into a separately admitted native `OwnedDirectory` while adoption holds,
then close/join Android's80MiB owner before publication/cache installation. The
copy needs exact disk allowance, bounded copy memory, source/manifest/tile fences,
cancellation and quarantine ownership; no raw directory/marker approval factory
is acceptable. This copy/bridge is not implemented by the read repair.

Keep the old canvas retired before publication. After publication, close/join
actual prepared/build output/manifest owners (4MiB+24MiB+directory allowance),
open the native provider without a surface, then admit the new96MiB surface.
Root's retained32MiB constructor must aggregate store/blob/permit/charge before
fallible work and retain the charge through actual ProjectState destruction.
Framework renderer retirement, native-source handoff and this entire staged
sequence still need end-to-end platform receipts.

## Local Android SDK path recheck

`android-sdk-local-path-recheck-20261018.json` records the exact environment after
sourcing `/workspace/.vw-tools/cloud-setup/start.sh`. Both SDK variables resolve
to `/workspace/.vw-tools/android-sdk`; that root itself is absent (not a symlink),
and therefore `platforms/android-37.0/android.jar` is absent here. Local setup
verification explicitly records `android: null`; local pins do name the correct
`platforms;android-37.0`. Startup/pins/verification hashes are retained. This is
an actual missing local installation, not an outdated SDK variable or suffix.
It does not contradict the coordinator's successful Android37.0 build in its
separate environment. No setup investigation was duplicated beyond this requested
path verification, and no new component was downloaded or installed.

Next action is coordinator store-contract review, paired patch application and
focused host validation when its heavy slot is available; then trusted Android
bridge/retirement composition and platform validation. Scope: medium for the
read repair, large for the platform workflow. Keep Sol6.1/xhigh; this worker
cannot inspect its hidden actual model/effort. Existing explicitly selected
child agents performed source-only patch/trace work in disjoint new files.
