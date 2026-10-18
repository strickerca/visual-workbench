# Streaming store read proposal — source only

The sibling `STORE_STREAMING_READ_PROPOSAL_20261018.patch` is **unapplied**.
It changes only `core/crates/vw-store/src/cache.rs` and its existing
`tests/blobs_cache.rs`. `git apply --check` passed against root HEAD
`af934061e0b19fb3a128cd40a803dd91cff58d30`; source hashes are recorded in
`/tmp/vw-assets-validation/giant-producer-next/windows/store-streaming-proposal/bindings.json`.
No typecheck, build, test, device action or dependency operation was run.

## API and preserved behavior

```rust,ignore
DiskCache::get_bounded_streaming(&mut self, key: &str, max_bytes: u64)
    -> Result<Option<Vec<u8>>, StoreError>
```

Before calling, the native owner reserves the returned payload's exact declared
length plus one byte, and a separate **1 MiB transient read reservation**, from
the existing session. The cache manager retains its existing resident index
reservation. The new API creates no pool and does not assume that max_bytes is
itself a memory reservation. Its returned Vec must remain inside the existing
native payload/charge owner through destruction, including early errors.

The new wrapper limits root and internal cache-directory encoded paths to
**16 KiB**, before path allocations. This accommodates the native 8192-byte
canonical-root admission plus fixed cache and temporary suffixes. Existing APIs
receive no new path restriction. The wrapper shares get_bounded's sequence:
path checks, lookup, length admission, readonly regular-file open, exact length
and BLAKE3 verification, then durable LRU touch. The same manager retains its
exclusive lock throughout. Unknown files and immutable original blobs are not
opened for writing or evicted by a read.

The proposal refactors the shared read_entry and save_manifest helpers, so ordinary
get/get_bounded and mutation persistence also receive the bounded implementations.
Their public signatures, limits, JSON representation, locking and LRU policy stay
the same. Applying this shared-helper refactor is a root decision; the patch is
not an authorization to edit the reserved store file.

## Payload and persistence bounds

read_entry checks declared length, computes checked `len + 1`, calls
try_reserve_exact, and refuses reported capacity above that allowance. It resizes
only to len and read_exact fills that already allocated slice. Unexpected EOF is
still classified as corrupt content. A one-byte stack sentinel detects growth;
Interrupted is retried. Hash verification runs only after exact EOF. No payload
read_to_end, append or second payload Vec occurs on this path, including len=0.

The index writer uses serde_json::to_writer over a borrowed manifest, through an
outer counter/limiter and an inner **64 KiB BufWriter**. Buffer capacity is checked.
The limiter rejects an entire over-limit write before forwarding any of it, counts
accepted buffered bytes as well as already written bytes, and caps output at the
existing **16 MiB** manifest limit. Production always uses that constant; a private
smaller-limit helper permits small failure fixtures. Checked addition prevents
counter overflow. Partial underlying writes count only their actual accepted size.

The success sequence remains serialize, explicit buffer flush, temporary-file
sync, atomic NamedTempFile persist, published-file sync, directory sync. No full
serialized index Vec is created. Serialization, cap or flush failure exits before
persist; the previous index remains installed. The temporary may contain only a
capped prefix and is dropped. Directory-sync behavior on Windows is unchanged
from the existing implementation. A failure after atomic persist may leave the
new durable index, as before; this proposal does not claim transactional rollback
after that commit point.

touch still advances its in-memory clock and entry before persistence. If the
write fails, memory may therefore be ahead of the old durable index, matching the
existing semantics. The proposal does not silently introduce an LRU rollback.

## Per-read scratch allowance

The resident <=100k-entry map is borrowed, never cloned or rebuilt during a read.
Validated keys are <=128 ASCII bytes, filenames exactly 70 bytes, and hashes 64
bytes. Only one DiskEntry is cloned. Serde serializes the map one entry at a time;
AssetId's serde conversion may clone one 64-byte hash at a time, rather than all
hashes. The schema has fixed shallow nesting and no attacker-selected recursion.

This conservative source allocation envelope fits within the caller's 1 MiB:

| Component | Allowance |
| --- | ---: |
| Checked BufWriter capacity | 64 KiB |
| Eight temporary byte-path slots, <=33 KiB each | 264 KiB |
| Four Windows UTF-16 conversion slots, <=66 KiB each | 264 KiB |
| One cloned entry, per-entry serde hash, serializer/error objects | 16 KiB |
| Remaining fixed I/O/tempfile bookkeeping envelope | 64 KiB |
| Total admission envelope | **672 KiB** |
| Unconsumed portion of caller's 1 MiB | **352 KiB** |

The path slots conservatively sum stages which are often not live together:
check_paths' cache join, entry path, index path, tempfile candidate and retained
TempPath, persist/error paths and directory-sync path. The maximum variable byte
path is 16 KiB plus a separator and the 70-byte filename. With geometric capacity
slack it fits 33 KiB. UTF-16 conversion allows four times encoded input length,
including element size and growth slack, fitting 66 KiB; persist has source and
destination conversions, and four slots allow additional overlap. The retained
root/directory themselves remain part of the resident manager envelope.

These are source-controlled admission allowances under the pinned standard
library/tempfile/serde implementations. The fixed writer and returned Vec bounds
are enforced by code; the OS/path/bookkeeping subdivisions are conservative
engineering allowances, **not measured allocator/RSS or kernel-memory bounds**.
An absolute claim covering undocumented runtime internals would require actual
target measurement and allocation instrumentation. That remains a verification
gap and is not replaced by this source proof.

With the retained 32 MiB project, 96 MiB surface, 64 MiB index and 24 MiB manifest,
the phone baseline is 216 MiB. Adding 32 MiB decode, a 2 MiB payload plus sentinel,
and 1 MiB read scratch gives **251 MiB +1 byte**, leaving 5 MiB minus 1 byte in the
256 MiB pool for other live charges. Any additional live owner still composes
against that same pool and may cause refusal. This arithmetic does not justify
discarding old owners, shrinking their charges or treating input/build memory as
already retired.

## Proposed tests and remaining work

Five source cases are included and **all are UNRUN**. Three integration cases
cover durable LRU after reopen, capacity/lock/original/foreign-file preservation,
missing/oversize/hash-corrupt reads leaving the index untouched, and failed index
publication preserving the prior index and payload. Two small private cases cover
the writer's exact byte cap and serialization-cap failure preserving the old
index and removing its temporary file; no large fixture was generated.

The store currently declares `[lib] test = false`. Those two private cfg(test)
cases are not ordinary default-library test evidence; root must explicitly decide
how to enable/select their library test target. The three appended integration
cases live in the existing blobs_cache target. No test selection or Cargo manifest
change is included in this proposal.

Root still owns application of this patch, strict type/lint/test checks, the native
tiles call-site change and the complete phone-budget regression. Opening the cache
still reads/parses the bounded 16 MiB JSON index using the existing larger workspace
reservation; this read optimization does not justify reducing that opening budget.
Full index serialization and durable filesystem sync still occur on each touch;
their latency and storage cost are unmeasured. Existing temporary-unlink failure
handling and OS durability limitations are unchanged.
