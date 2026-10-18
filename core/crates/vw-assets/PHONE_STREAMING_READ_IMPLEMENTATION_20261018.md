# Bounded phone cache-read implementation

The four-file grant was read directly from coordinator candidate
`707e940841373b076ebbe750c01319df2d992eed`,
`docs/GIANT_STREAMING_STORE_API_ADMISSION_20261018.md`.
Both proposals from5a7239d are now applied as real source on the producer branch.
The historical proposal receipts still describe their earlier unapplied state;
this implementation receipt supersedes that status. No coordinator Cargo/lock,
store registration, root facade, shared/app/controller or runtime pins changed.
The editing/offline candidate was inspected for admission, not overlaid onto this
producer branch. Coordinator composition remains a separate integration step.

## Implemented allocation boundary

`DiskCache::get_bounded_streaming` is additive and preserves the existing manager
lock, verified length/hash reads, durable LRU and atomic index replacement.
Only that new entrypoint adds the16KiB encoded root/cache-directory path bound.
Existing public cache signatures and constructor/put/remove admission stay intact.

The shared payload reader reserves exactly declared length+one byte, checks
reported Vec capacity, fills only the declared slice and probes one stack
sentinel. It cannot append/reallocate on post-metadata file growth. Truncation,
growth or hash mismatch refuses. Metadata remains checked on the opened regular
file before the reader; the private helper enables deterministic race tests.

Shared index persistence borrows the validated map and serializes through an
outer16MiB limiting writer and an inner fixed64KiB BufWriter. Actual buffer
capacity is checked. The full serialized-index Vec is gone. The existing temporary
file flush, sync, atomic persist, published-file sync and directory sync order is
preserved. Serialization/size/write failure before persist leaves the previous
index installed; sync failure after persist may leave the new index, as before.
The in-memory LRU clock can advance on failed persistence, matching prior behavior.

Giant manifest/tile reads reserve1MiB around that streaming read, using the same
session pool. The original64MiB constructor/install workspace,64MiB resident map,
24MiB manifest,32MiB codec,32MiB retained project policy,96MiB surface and256MiB
phone limit are unchanged. The bounded-path/key scratch allowance is documented
in the earlier `STORE_STREAMING_READ_PROOF_20261018.md`: fixed Vec/buffer/output
bounds are code-enforced; allocator/OS/path bookkeeping remains a conservative
engineering envelope, not measured process RSS.

## Lifetime evidence

The native regression uses actual local original BlobStore publication and
provider opening/decoding, with a test-only synthetic producer witness. It:

- Drops actual build/output/manifest and prepared-input owners before opening.
- Holds an old96MiB surface and observes real opening refusal; then drops that
  owner, opens the provider and admits the new96MiB surface.
- Verifies the exact live floor:216MiB project/provider/surface +784KiB foreign
  tile +64KiB same-pool approval. A further64MiB old index reservation refuses.
- Keeps a further16MiB foreign owner live. A real tile miss refuses under codec
  pressure and returns to the exact prior charged total. Dropping that owner
  permits the same real tile read, without raising256MiB.
- Checks after the read that only one256KiB cached payload and the exact outgoing
  payload charge remain above that floor; encoded/index/codec temporaries retired.
- Cancels another read with no charge drift; closes the provider and verifies
  exact resident/map/manifest/cached-payload retirement while outgoing, surface and
  foreign owners remain charged. Payload destruction precedes outgoing credit.
- Reads the original project blob after close and verifies its exact bytes.

This is admission/lifetime evidence for the real host path, not a codec/GPU/RSS
measurement or JNI/Android execution. With all64 same-pool approval entries and
one maximum encoded tile, cold decode still admits216+4+2+32+0.25+0.765625
=255.015625MiB plus sentinel before other owners. Pinned native/foreign tiles can
force refusal; the existing retry releases only unpinned actual cache payloads.
Dynamic4MiB+4x canonical model checks can refuse before that retry. The repair
removes the unconditional280MiB read overlap, not every possible pressure limit.

## Focused validation

Validation uses the existing isolated worktree at6f3e2f3 with the exact producer
checkpoint plus these four granted sources, two Cargo workers/test threads and
bounded process-tree commands. Its already-proposed Windows dependency/lock edge
exists only there; task-branch manifests and lock remain untouched. No runtime,
provider, device, SDK install/download or new license acceptance occurred.

All87 focused host cases pass:53 native giant-image cases,30 existing/new
blob/cache integration cases and4 explicitly selected private store cases.
Strict all-target Linux Clippy passes for store and FFI; offline licenses, bans
and sources pass (warnings only). These counts are not additive to coordinator
native/JVM totals, and no unaffected Windows/Android tests were rerun.
Final source/log hashes are in
`phone-streaming-read-implementation-20261018.json`. The commands are:

```text
cargo deny --offline check licenses bans sources
cargo test --offline --locked -p vw-store --lib --test blobs_cache -- --test-threads=2
cargo test --offline --locked -p vw-ffi --lib giant_images -- --test-threads=2
cargo clippy --offline --locked -p vw-store -p vw-ffi --all-targets -- -D warnings
```

Explicit `--lib` runs the store private cases despite its default `test=false`;
their execution is recorded, rather than inferred from ordinary integration tests.
Host logs remain in `/tmp/vw-assets-validation/phone-streaming-read/`.
No test/typecheck/lint failures occurred in this implementation batch. The initial
passing runs are retained beside final runs after exact lifetime/limit assertions
were strengthened. The heavy-build slot was released after these checks; further
build ownership belongs to the coordinator.

## Remaining coordinator/platform obligations

Root separately composes the retained32MiB ProjectState constructor, atomically
bundling all four PublishedProject parts before fallible work. The fixture's
retained owner exercises the charge policy but is not that production constructor.
Root also owns the trusted Android runner bridge: copy verified exact tiles from
`withRootOutput` into independent native ownership, close/join the Android80MiB
output before publication, then retire native prepared/build owners before opening
provider and new surface. Existing Android native builds remain Unsupported.
No raw directory, marker, manifest or boolean grants producer approval.

The Android SDK root remains absent only in this worker environment; the
coordinator's separate Android37.0 build is unaffected. Maximum inventory,
platform codec/driver/RSS peaks, JNI/renderer retirement, Windows dynamic runtime,
import/display/export and physical acceptance remain unverified. R164 conversion
and restricted still-image AVIF capability are unchanged.

Next step: coordinator composition and full platform workflow validation (large),
using Sol6.1/xhigh. This worker cannot inspect its hidden actual model/effort.
