# Recovery integration contracts — 2026-10-18

These decisions reserve the common boundary before specialist wiring. The source
baseline and lane ownership are in `ROADMAP_EXECUTION_LEDGER.md`. A recovered
postimage supplies a candidate API; registration and acceptance require tests.

## Common rules

Use immutable original bytes identified by the existing BLAKE3 asset ID. Use
SHA-256 separately for publication and encoded-file receipts; never substitute
one hash for the other. Derived previews, tiles, overlays and exported copies
must identify their source and relevant revision. A successful late result is
discarded when project/document/revision or viewport ownership has changed.

Keep the existing `CoreError` and `Cancellation` boundary. Core backpressure is
retryable only for the exact unaccepted batch; do not replay a committed job.
New format/document failures stay typed at their domain boundary. Unsupported
codec/depth/color/isolation and actual cancellation must not become success or
fall back to a preview. Freeform parser/OS errors and private paths must not cross
the public boundary or enter evidence.

Workers admit input/output/memory/time before allocation or native execution.
Cancellation signals stop and then joins the admitted worker and resource owners
before caller scratch, source, project or grants can be released. A durable
commit that already crossed its commit point may complete; cancellation must
retain that distinction. `close` is idempotent and awaits reads/jobs/leases.
Foreign bitmap/texture copies require separate platform reservations; native cache
accounting alone does not prove the app/GPU memory budget.

## Document lane

- Promote the R2 `vw-document-protocol` candidate through integration. Keep its
  fixed `VWDOC01` framing, nonzero nonce, BLAKE3 source binding, bounded metadata/
  data/output, ordered terminal completion and no ambient filenames/endpoints.
  The latest PC-redaction protocol composition must be reconciled before worker
  compilation; its request variant cannot be omitted from exhaustive dispatch.
- Keep PDF/SVG engines independent of UniFFI, Kotlin, project storage and output
  destinations. They receive admitted source/operation/limits and return bounded
  metadata or bytes. PDF password stays in private worker memory/pipe messages,
  excluded from Debug/replies/command lines/logs.
- PDFium rendering, raster redaction and restricted qpdf redaction are distinct
  capabilities. Refuse unsupported content. An overlay or partial-page bitmap
  is not true redaction. Phone-only redaction must rasterize every output page;
  PC output must pass the restricted verification contract before publication.
- SVG uses resvg/usvg with external resources and ambient font discovery denied.
  Preserve explicit fidelity warnings and bounded malformed-input rejection.
- Proposed Kotlin DTO names follow recovered `DocumentsApi.kt` (source identity,
  page/tile geometry, typed operation/reply/failure). Integration owns its final
  adoption, `DocumentJobs`, revision fences, installation and FFI registration.
  Do not make engine crates depend on the Kotlin/FFI request layout.

## Image lane

- `vw-assets` owns path-free manifest/plan/cache/scheduler types. Prefer the
  foundation R2 manifest and lease semantics. Tiles identify original asset,
  manifest and tile address. Invalid/stale/busy/cancelled outcomes remain typed.
- `vw-formats` owns bounded BMP/TIFF admission, decoding and staged encoding;
  it may depend on `vw-raster`. Do not add a reciprocal raster dependency.
  `vw-avif-os` owns header admission and OS-codec capability/refusal. Exact crate
  dependency pins belong in each owned Cargo.toml; integration owns Cargo.lock.
- Original bytes, dimensions, depth, orientation and profile survive admission.
  TIFF's first-page-only behavior requires an explicit notice while all original
  pages stay preserved. Conversions of depth, alpha or color require explicit
  policy, and source/export receipts must expose the resulting representation.
- Platform producers receive private, source-bound input/scratch. Only their
  completed approved pyramid is admitted; a caller-provided arbitrary pyramid
  is not an import source. Publication occurs after hash/manifest verification,
  fsync/atomic adoption and a final cancellation/revision check.
- Proposed shared DTO names follow `GiantImagesApi.kt` plus PNG-export R2 and
  ordinary `OriginalFormatsApi.kt`: prepared source, viewport plan, tile pixels,
  joined close and source/revision-bound receipts. Integration owns shared APIs,
  store/cache plumbing, raster/color changes and all app/controller adoption.

## Build and publication

Specialist owners may edit their crate manifests and dedicated tests/fixtures.
Send dependency requests and necessary outside-lane hunks to the parent for
integration. No root lock/manifest, shared API, protocol/schema, FFI registration,
generated binding, Android manifest, controller or app-root edits by workers.

The parent allocates heavy build slots: one heavy build at a time, at most two
build workers. Run cheap source/parse checks while waiting. Preserve license
gates; do not silently change dependency pins to fit a cloud image. No paid CI,
live provider calls, credentials, signing, deployment or device-identity changes.
Compile/offline tests establish only the behavior actually exercised. Hardware,
Windows-native engines and product gates retain their named pending checks.
