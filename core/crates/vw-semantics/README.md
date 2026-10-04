# Semantic capture workflow foundation (T2.06/T2.07)

This adapter uses canonical `vw-model` capture records and ordinary
`AddSemanticSnapshot` transactions. It does not collect a platform tree or take a
screenshot. It implements the bounded data/geometry portion of SEM-001–006 and
supplies validated references for the T2.08 package compiler. It is not closure
of T2.06 or T2.07: Windows WGC/UIA, Android explicit-capture AccessibilityService,
FLAG_SECURE, Chromium accessibility, HEIC decoders, own-window exclusion, UI/FFI,
durable application locking and physical A22/A23/A26 checks remain open. No
300 ms collection or 95% accuracy result has been measured here.

## Integration and identity

Create `CaptureView::new(project, revision, document_id)` from the current visible
model and matching full state hash. Retain its `CaptureContext` **before** platform
collection. Feed the observed context and owned `CapturedElement` records into
`prepare`; a late result for a different session, frame, geometry or source fails.
The context also binds the document's canonical primary asset ID. An empty ID
remains an explicitly incomplete live-frame document; this adapter does not
invent original bytes or prove that a source asset exists. Package compilation
must separately require and verify the immutable original before publishing.
`frame_delta_ms` is a signed platform observation; `collection_elapsed_ms` is the
adapter's actual elapsed observation. A UIA result over the original 300 ms
target remains explicitly over target, never reported as a measured pass.

The platform adapter assigns bounded opaque local IDs and parent IDs. The core
namespaces each EID by snapshot UUID, so a marker's canonical `element_eids`
cannot silently resolve to a later frame's similarly named node. It rejects
duplicates, missing parents, cycles/depth overflow and invalid coordinates.
Literal names, roles and text are untrusted. Captured text is limited to 200
Unicode scalar values; larger input returns a typed limit instead of silently
truncating content. Multiple disconnected tree roots and zero-area nodes are
retained; zero-area/disabled nodes are not snapping candidates.

`HostPhysical` bounds subtract the captured physical client origin, including
negative monitor positions. DPI is never applied again. `CapturePixels` input
must already have the platform display rotation mapped into the captured frame;
this crate does not guess Android orientation. Bounds are clipped to the frame
with an explicit `bounds_clipped` flag. No hierarchy nodes are silently dropped.

`CaptureView::plan` produces one canonical op and immutable transaction bytes.
`SnapshotPlan::check_current` must run under the **same** project mutation lock as
the durable `ProjectStore::commit`. `submit` is an in-memory reference path only.
Unaccepted stale plans fail even if the canonical host could rebase their ops;
exact accepted retries still use host journal identity and preserve acknowledgments.
The platform must refresh after refusal. Existing ops supply undo/redo; there is
no second persistence or reconciliation engine in this crate.

`CaptureView::load(explicit_snapshot_id)` decompresses the stored payload and
checks all column/envelope/capture bindings. The versioned compressed envelope
contains immutable project/document/snapshot/capture identity and literal data.
Old arbitrary payloads are refused; no implicit migration or newest-snapshot
selection occurs. Consumers compare the view's project/document/hostSeq/hash to
their current instruction/render binding immediately before publication.

## Snapping and safe output

`Snapshot::snap` accepts the full D-to-physical-screen affine. Point distance is
measured against the actual transformed quadrilateral, including rotation/shear;
box snapping requires all four corresponding corners within 12 physical pixels.
The smallest containing element wins a zero-distance tie, then canonical EID
order breaks remaining ties. Disabled/empty nodes are excluded. The returned
bounds can feed marker placement; the call never mutates a marker or model.
This deterministic choice is provisional UI policy, not a measured target result.

`export_references` resolves an explicit bounded marker EID list and emits the
exact `package.schema.json` element-reference fields (D bounds `[x,y,w,h]`). Missing
or duplicate references fail instead of dropping links. `export_all` emits the
full bounded semantic tree. Both return immutable JSON bytes and quoted prompt
data. `export_all.references()` is empty; marker references are requested explicitly.

The exact required untrusted-text notice is included in JSON and prompt data.
JSON escapes line/control characters; a code-span delimiter longer than every
captured backtick run encloses the literal data. Captured names never become
instruction lines. Export projections omit native window handles, monitor IDs,
device IDs, application/window titles and full local geometry. The package
compiler separately controls any explicit owner opt-in for window-title metadata.

## Resource and cancellation contract

Admission limits: 4,096 elements, 128 parent links, 256-byte platform-local IDs,
4 KiB names, 256-byte roles, 1 KiB automation/resource/HTML IDs, 200-scalar captured
text, 2 MiB aggregate text, 4 MiB JSON, 4 MiB +128 KiB compressed input/output,
64 marker references. IDs count toward the aggregate limit after namespacing.
Caller inputs are borrowed or moved only after count/string/geometry admission;
tree sorting is at most 4,096 elements and parent validation at most 524,288 links.
Canonical project hashing has a 64 MiB serialization admission gate.

The Zstd decoder admits one ordinary frame, a maximum 1 MiB back-reference window,
bounded 16 KiB output reads, and no concatenated/skippable frames or trailing data.
Decompression bombs fail before the next output allocation exceeds 4 MiB. JSON
parsing uses a bounded element visitor before vector growth; unknown/duplicate
fields, noncanonical JSON, future schemas, nonfinite numbers and trust-flag changes
fail. Encoding uses level 3, the same 1 MiB window, a checksum and capped output.
No worker threads or global queues are created. These are concrete input/work
bounds, not a measured RSS/latency guarantee for the native codec or allocator.

Cancellation is checked per element, parent-walk group, snap candidate and codec
block, before/after serialization. Bounded serde/native codec calls are not
interruptible inside a call. Platform collection cancellation and ownership of
late OS callbacks remain the platform adapter's responsibility.

## Dependency provenance (checked 2026-10-03)

Only new direct pin: `zstd =0.13.3`, default features disabled. The upstream
[tagged Cargo manifest](https://raw.githubusercontent.com/gyscos/zstd-rs/v0.13.3/Cargo.toml)
declares MIT. The same tag's
[zstd-safe manifest](https://raw.githubusercontent.com/gyscos/zstd-rs/v0.13.3/zstd-safe/Cargo.toml)
declares 7.2.2, MIT OR Apache-2.0; its
[zstd-sys manifest](https://raw.githubusercontent.com/gyscos/zstd-rs/v0.13.3/zstd-safe/zstd-sys/Cargo.toml)
declares 2.0.14+zstd.1.5.7, MIT/Apache-2.0 wrapper and explicitly includes native
`zstd/LICENSE` and `zstd/COPYING`. The native codec offers a BSD option in
[Zstandard v1.5.7 LICENSE](https://raw.githubusercontent.com/facebook/zstd/v1.5.7/LICENSE);
select that permissive option and retain its notice, alongside wrapper notices.
The separately bundled COPYING is the alternative GPL license, not the selected
license for this integration. Native C/assembly notice inventory must be checked
against the **actually resolved** zstd-sys artifact by the root license gate.
Cargo may resolve newer compatible transitives; this source-only task has not
fetched artifacts or changed the lock. No bindgen, dictionary builder, legacy,
multithread, external system library or experimental codec feature is selected.
The root must run the central native/transitive license and Android packaging gate.

The bounded decoder uses the documented
[single_frame/window_log_max APIs](https://docs.rs/zstd/0.13.3/zstd/stream/read/struct.Decoder.html).
All other direct dependency versions already match project pins. No dependency
was fetched, and no build, test, device or real-capture operation was performed
during source implementation. Written regressions require central execution.
