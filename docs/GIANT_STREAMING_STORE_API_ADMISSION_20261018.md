# Streaming cache-read API admission

The integration owner reviewed the paired proposals in5a7239d1 and the phase
trace in `core/crates/vw-assets/PHONE_READ_BUDGET_HANDOFF_20261018.md` from that
commit. Admit `DiskCache::get_bounded_streaming(&mut self, key: &str,
max_bytes: u64) -> Result<Option<Vec<u8>>, StoreError>` for implementation and
validation. This is source/API admission, not Android producer activation.

The proposed outer16MiB limiting writer, inner fixed64KiB BufWriter and existing
atomic temporary-file/sync/persist/directory-sync sequence address the actual
full-index serialization allocation. The sentinel read preserves exact length,
hash checking and pre-reserved Vec capacity. Reducing a workspace counter while
retaining `serde_json::to_vec` would not be an admissible alternative. Keep durable
LRU, exclusive locks, original bytes and prior index on failed publication. The
new16KiB path limit applies only to the additive read entrypoint; existing caller
signatures and larger constructor/put/remove admission envelopes remain intact.

The parent may assign the image owner these exact previously reserved paths:

- `core/crates/vw-store/src/cache.rs`
- `core/crates/vw-store/tests/blobs_cache.rs`
- `core/crates/vw-ffi/src/giant_images/tiles.rs`
- `core/crates/vw-ffi/src/giant_images/producer/producer_tests.rs`

The image owner's existing dedicated producer leaves/receipts remain its lane.
These four files are unchanged in the concurrent editing/offline integration
batch. Apply both proposed patches together on the producer branch, then commit
real source rather than another unapplied patch. Root retains Cargo/lock, store
module registration, ProjectState/ProjectSession constructors, FFI registration,
shared/app adapters and controller/UI files. Do not overlay newer root files.

Validate exact-limit and failed-write preservation, verified fixed-capacity reads,
LRU reopen/eviction, missing/oversized/corrupt/growing payload refusal and cancellation.
Run existing blob/cache regressions and the proposed full phone-pool fixture after
a heavy-slot handoff. Pin64KiB serialized-index buffer and bounded path/key scratch;
the1MiB read allowance is a conservative source-controlled envelope, not RSS.
Opening/parsing still needs the prior64MiB workspace. Existing pinned cache and
foreign owners cannot be credited to make the decode fit.

Keep the256MiB phone pool unchanged. Android remains Unsupported until a trusted
root `withRootOutput` bridge verifies and copies independently owned native tiles,
then joins the Android80MiB output owner's actual retirement before publication.
The retained32MiB native project constructor, all four PublishedProject ownership
parts, post-publication destination preservation and real renderer retirement
remain root integration obligations. No manifest/boolean/directory can mint
producer approval. Neither these source patches nor the66 earlier host cases pass
Android/Windows runtime, maximum-inventory, driver/RSS, import/display/export or
physical acceptance.

The integration owner is finishing editing/offline validation and currently owns
the heavy slot. This assignment is source-only until the parent grants a build
slot; the separate verified Android environment remains the eventual platform
validation location. No SDK installation or runtime pin change is authorized here.
