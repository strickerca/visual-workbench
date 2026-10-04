# Shared AI edit foundation

This is the provisional software foundation for T2.02/T2.03 and BUILD_SPECIFICATION
§4.9.2. It performs local image work and durable spend accounting. It contains no
HTTP client, credential access, upload, purchase, UI, or simulated success for a
platform operation. Product gates, live-provider checks, target-device tests, UI
integration and the 12 MP / 2-second requirement remain unverified.

## Request lifecycle

1. Construct a validated `ProviderConfig` and `PrepareOptions`. Model IDs,
   capability limits, verification date and micro-USD price rates are configuration.
   `estimated_tokens` comes from a provider-specific, versioned estimator; this
   library does not guess a universal token formula. An absent or zero estimate
   allows local preparation but cannot authorize a paid request.
2. `Prepared::new(original_encoded, change_masks, options, cancellation)` consumes
   no credentials. The immutable review includes source/revision binding, crop,
   dimensions, depth policy and estimate. `request_image_png` and
   `request_mask_png` show the exact bytes proposed for transmission. Instructions
   are role-tagged data; callers supply only Change-role selections to the mask
   union. Preserve, Reference and Explain selections are not edit permission.
   Before returning that review, preparation admits the exact feather and
   proof-dilation work/buffer limits and the complete result/proof memory peak,
   using actual retained request PNG lengths and a full 64 MiB encoded-response
   allowance. It also constructs the return-to-source ICC transform on one pixel.
   These checks prevent known impossible local completions from reaching Send;
   they do not guarantee provider validity or future allocator availability.
3. On the owner's explicit Send action, call `Confirmation::explicit_send` for
   the displayed review and current UTC day. Reserve it in the application-owned
   `SqliteBudgetLedger`. A separate acknowledgement is required when the default
   $5 daily soft budget (configurable) would be exceeded.
4. `begin_attempt` commits the Attempted state before returning a non-cloneable
   permit. `Prepared::authorize` consumes that permit and produces an owned
   `AuthorizedRequest`. The platform `ProviderAdapter::send_once` implementation
   then obtains an OS-stored credential and performs exactly one request with
   automatic retries disabled, a deadline and a bounded response read. Header
   credentials are never part of this library's payload or error types.
5. Parse the response through that `AuthorizedRequest`, which binds it to the
   request ID and checks image count, PNG encoding and token totals. Settle a
   valid provider usage receipt even if later local rendering fails. Missing or
   uncertain usage leaves Attempted unresolved and blocks new requests.
6. `Prepared::finish` returns an immutable `Completed` candidate and proof. The
   app stores the result as a new result-layer asset with the source, mask,
   request and revision bindings; it must check the current project revision
   before applying it. This crate does not write originals or commit model ops.

`ProviderPayload` identifies the parsing path, not evidence that a network call
occurred. `OfflineMock` is explicit and never produces actual spend usage. No
test fixture establishes a real API, credential-store, or billing integration.

## Durable ledger contract

The sealed `BudgetLedger` interface currently has one durable SQLite backend.
Run it on a blocking worker. `create` is an exclusive first-install operation;
`open` fails closed on missing, corrupt or incompatible history. A platform must
retain a separate provisioned-ledger marker in app-private storage and must not
call `create` as recovery for a missing history. The fixed path is application
state, never a project/archive/export path. Protect its parent directory against
untrusted writers; containment checks cannot prevent a hostile concurrent path
replacement without an OS directory-handle API.

Transactions use WAL, FULL synchronous mode, a 500 ms busy bound, a 4096-entry
history cap and monotonic UTC days. The stored data contains hashes, counters and
charges, never image bytes, prompts, keys or endpoints. Repeated request IDs are
refused even after cancellation. A fresh owner intent requires a fresh UUIDv7.
Only Reserved can transition to NotSent. Attempted cannot be refunded, abandoned
by Drop, or retried after restart. Settlement is idempotent for the same charge
and rejects a different second charge. A receipt reconciliation UI, history
migration/archival, protected rollback prevention and OS credential adapters are
future integration work; no recovery silently erases an uncertain charge.

The exact schema is checked at open and again under every Immediate mutation
transaction; extra triggers, indexes, views or altered table definitions fail
closed. Each write checks its affected row count and reads back the complete
expected entry before committing. Typed SQL preflight and an 8 KiB SQLite value
limit precede any persisted string conversion. The 4096-byte page size and
2048-page / 8 MiB database cap apply to existing databases as well as new writes.
File lengths are admitted before opening SQLite and before starting transactions;
the fixed WAL recovery allowance is 1 GiB, shared-memory sidecars allow 16 MiB,
and rollback journals allow 16 MiB. The WAL cap bounds disk recovery work, not
RAM: repeated writes with delayed checkpoints can exceed the database's size.
SQLite uses a 256 KiB page-cache target with mmap disabled. Nonempty valid WAL
recovery is preserved. Oversized files are refused before SQLite opens them;
malformed history is never reset or deleted. Normal SQLite recovery/checkpointing
may still occur for admitted files. Reconciliation or migration requires an
explicit future integration path. These checks are not protected
rollback detection against an attacker who controls the application's directory.

## Pixel and coordinate rules

All masks use full-resolution, orientation-applied document pixels D. Sparse
`vw-mask` coverage is unioned with maximum coverage. The crop expands the mask
bounds by 0.375 on every side, with a 64 px minimum margin; it clamps to the image
then grows to meet aspect limits. Extreme aspect ratios use edge-replicated
pixel padding and zero-change mask padding. The closest provider grid size is
chosen by squared logarithmic width/height distance, with deterministic size
tie-breaking. Pixels use premultiplied-alpha Lanczos3 with normalized floating
channels; masks use exact overlap-area resampling. Provider alpha is `255-mask`.

The source ICC is preserved. Untagged input requires an explicit sRGB assumption;
generated sRGB ICC headers use a fixed creation timestamp. Unsupported profiles
fail. A 16-bit original requires explicit permission to create an 8-bit provider
copy. Its output remains 16-bit and all unchanged original samples, including
low bits and alpha, are copied exactly. The generated region has the provider's
8-bit precision. Requests resize in source encoding then convert to sRGB;
responses resize in sRGB and return to the source profile.

Compositing uses `vw-mask` v1's finite Euclidean disk feather, default selected
by the caller (the product default is 8 px), rounded to deterministic 8-bit
coverage. This shares the spike's support/kernel but quantizes coverage before
blending; fractional spike feather weights are not claimed bit-identical.
Straight RGBA channels blend with `(source*(255-w)+result*w+127)/255`. Pixels
outside the crop are copied, and no nonzero feather support extends beyond the
finite disk. Every successful candidate proves `changed_outside == 0` outside
`dilate(change_mask, feather+1)`. The proof hashes indexed exterior RGBA samples,
dimensions, depth, profile, source file and mask. It measures CIEDE2000 mean/max
and global masked sRGB luminance SSIM; exact proof includes alpha while perceptual
metrics exclude it. `verify_candidate` independently checks recovered full images.

Partial acceptance blends against the immutable source. Zero selection coverage
copies source RGBA exactly, and a second proof confirms every unaccepted pixel.
`acceptance_mask` retains the most recent selection as provenance; its result
pixels are already composited, so callers must not apply that mask a second time.
Wipe, blink, split and difference return 8-bit sRGB previews; differences include
alpha changes as visible intensity. Export uses the candidate's original-profile
8/16-bit PNG path, never a preview.

## Bounds and validation

The implementation caps images at 20 MP / 16384 px edge, encoded images at 64 MiB,
feather at 64 px, provider grids at 262144 candidates, union work at 250 million
pixel visits, and configurations/instructions/response bodies independently.
Working-memory admission defaults to 512 MiB with a hard 2 GiB ceiling and counts
retained sources, intermediate float buffers, images and encoded payload copies.
Preparation also charges every caller-supplied mask tile and conservative map/Arc
overhead before source decode, even when masks share backing allocations. The
caller owns those inputs after preparation and must account for any unrelated
retained memory when scheduling later operations.
Operations may reject large inputs and require a future tiled adapter. The same
nonallocating `vw-mask` admission used by morphology checks both the feather
radius and the proof's radius-plus-one before review. A sparse selection still
charges dense document work; it cannot bypass the 1-billion-work bound.
Cancellation is checked between codecs/morphology and within AI pixel/metric
loops; codecs and bounded morphology are not interrupted inside a call.
Wall-clock performance and cross-platform resampling hashes still
require measured validation.

The source contains independent crop/price known answers, CIEDE2000 reference
pairs, constant-color/transparent Lanczos regressions, exact exterior and 16-bit
proofs, partial acceptance, cancellation, malformed-provider responses, configuration
limits, ledger contention/schema-rejection/reopen cases, two real killed-child recovery
cases and four property suites. At this source handoff these tests are written,
not executed; the parent task owns serialized validation and retained receipts.
Additional review regressions cover state-reset/ignored-update triggers installed
before or after open, oversized persisted strings and file/page counts, all
4096 history entries retained in a nonempty WAL, and allocation-free admission
arithmetic for many independent masks. The existing SQLite pin now enables its
`limits` feature; no dependency version or new package was added.
Pre-send regression sources also cover feather and proof-only work refusal,
completion peaks that exceed otherwise affordable preparation, exact encoded
payload boundary admission, and zero ledger entries/provider attempts on refusal.

All dependencies reuse project-approved exact pins. Pixel/crop/metric functions
were extracted from the T0.11 spike, then share `vw-raster` decoding and `vw-mask`
feather/dilation. The corrected root spike's normalized Lanczos formula is used.
