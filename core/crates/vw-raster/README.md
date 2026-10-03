# vw-raster (T1.05 software)

This crate exports source pixels and CPU annotations. Its optional streaming
adapters read/write a caller-owned private scratch file. It does not choose
paths, contact providers, consult system fonts, replace originals, or use previews.
The committed nine-entry PNG/layout golden manifest matches Windows x64 and
physical OnePlus IN2019 arm64. Sixty-two ordinary tests pass on each platform;
Windows additionally passes the explicit 50MP PNG16 scratch/marked-export test.
See `docs/evidence/T1.05.md` for exact bindings, resource measurements and open
application/S23 acceptance. These native tests do not establish app performance.

## APIs and exact semantics

- `decode(bytes, DecodeLimits)` accepts PNG/JPEG/WebP, hashes the original encoded
  bytes, extracts ICC and consumes EXIF orientation exactly once into D space.
  RGBA8/16 are straight, unpremultiplied samples. Gray channels are expanded
  losslessly in memory; a gray ICC causes PNG/JPEG to use gray channels again.
  RGB/gray ICC are supported; other profiles fail explicitly. Until tiled import
  arrives, default maximum is 50 million pixels, with an independent memory guard.
  An explicit PNG sRGB declaration is normalized to an sRGB ICC without changing
  samples. Gamma/chromaticity-only PNG and cICP/HDR declarations are refused until
  their color semantics can be retained; they never silently become untagged.
- `export(source, request)` supplies full or integer-bounded regions at one output
  pixel per source pixel. It emits PNG8/16, JPEG with quality 1..100, WebP lossless,
  or native WebP lossy with quality 1..100. There is no resize parameter.
  Depth reduction needs explicit permission. JPEG alpha needs an explicit matte
  (values are in the output profile's encoded color space). Clean PNG samples,
  including hidden RGB at alpha zero, are preserved. Lossy formats inherently
  change color samples; their choice is explicit in metadata.
- Preflight rejects WebP edges >16383, JPEG >65535, bad quality/depth/alpha,
  regions outside the source, missing revision/capture identity and working-set
  estimates over the caller's budget. The estimate includes a 32 MiB reserve and
  64 bytes/output-pixel (160 for native lossy WebP), not a measured RSS guarantee.
  Caller-owned source memory is additional. Errors suggest PNG, split or tiles.
- Clean exports retain ICC bytes. `ConvertToSrgb` uses moxcms at source precision,
  preserves alpha and records conversion/profile hashes. Untagged input requires
  an explicit assumption. Gray ICC cannot label WebP's RGB channels: select sRGB
  conversion or PNG/JPEG. Unusable/CMYK profiles return an error, never a silent
  relabeling. No untrusted source metadata is copied besides ICC/orientation.
- Every PNG contains `VisualWorkbench` iTXt JSON. JPEG APP1 XMP and extended WebP
  `XMP ` carry the same JSON: full revision tag, original asset BLAKE3, source and
  output dimensions, depth, consumed orientation, profile hashes, all settings,
  and optional paired capture-session/frame identity. Old EXIF is not copied.
- `render_document` validates project/source identity, stable layer/object order
  (fractional key then ID), visibility, isolated layer opacity and normal/multiply
  blending. It renders vw-ink compound NONZERO contours once, lines, arrow heads,
  rectangles, ellipses, polygons, text, numbered markers and resolved result
  images. Source samples outside coverage are never round-tripped through Skia.
  Shared ink geometry and source pixels remain immutable.
- Annotation coverage is tiny-skia RGBA8, converted from sRGB into source ICC,
  then composed into retained 8/16-bit source precision. Textures/results retain
  source precision, use deterministic nearest-neighbor inverse affine sampling,
  and require original/hash plus registered dimension/orientation/depth/ICC binding.
  Ready/accepted result previews are supported. Other states and unmaterialized
  acceptance masks are refused; partial acceptance compositing belongs to T2.03.
  Screen-constant line width is interpreted
  at export's explicit 1 D-pixel/output-pixel view. This is not viewport scaling.
- Selection vector/raster bounds and crop outlines are editor guides, excluded
  from normal marked exports. `include_guides` explicitly draws their outlines;
  it does not render sparse mask tile contents as source imagery. Raster mask
  processing/selection boolean operations belong to the later mask module.
- Brightness/contrast/levels operate nondestructively on their own normal-blend
  adjustment layer; mixed adjustment/annotation layers and multiply adjustment
  layers are refused until their backdrop semantics are specified. Alpha and
  fully transparent samples are retained. The formula is in `apply_adjustment`.
- Persisted `vector_eraser` strokes return a typed refusal. T1.08b must materialize
  whole-object erasing as deletion operations; export must not invent pixel erasure.
  PDF/SVG page rasterization belongs to their source modules and is refused here.

## Bounded PNG source and marked exports

The additive streaming API removes the second full rendered image and encoded
output Vec from the library's working set. Existing buffered APIs retain their
documented guards. No existing refusal has been changed to silently downscale.

- `RasterSource` exposes immutable full-source metadata, retained byte accounting,
  row workspace and an exact integer region read. `BorrowedSource` borrows an
  existing RGBA8/16 original; its estimate includes that buffer's full capacity.
- `PngSpool::decode(encoded, &mut File, SpoolLimits, cancelled)` accepts a fresh,
  empty read/write scratch file. PNG is decoded by row; Adam7 pass rows merge into
  scratch without a full in-memory image. Palette/gray/transparent RGB and 16-bit
  samples are preserved. Scratch is raw RGBA in original scan order; region reads
  apply all eight EXIF orientations exactly once without a whole-image rotation.
  APNG is refused rather than silently selecting a frame. Default scratch cap is
  400,000,000 bytes, enough for 50MP RGBA16; RGBA8 needs 200,000,000 bytes.
- `JpegSpool::decode` uses the same scratch/lifetime contract for baseline JPEG.
  Before constructing the decoder it admits only one complete interleaved scan,
  8-bit 1/3-component data and standard 1x/2x component sampling. The decoder's
  compact Luma/RGB frame is converted directly into scratch by row, avoiding a
  second full RGBA allocation. Budget is decoded channels/pixel plus four encoded
  lengths, 4096 bytes/source-column for bounded MCU workspaces and 32 MiB reserve.
  This is conservative source inspection of pinned image 0.25.10/zune-jpeg 0.5.15,
  not a measured process-memory claim. Progressive and unusual/multi-scan JPEG
  return `Unsupported`: the upstream progressive decoder retains full i16 DCT
  component planes, which cannot generally coexist with 50MP pixels in 256 MiB.
- `plan_png` exposes the clean-export region, chosen strip rows and accounted peak
  before output. `export_png_to` streams clean PNG8/16; `export_document_png_to`
  accepts `MarkedDocument { project, document, assets, options }` and uses the
  same stable layer, shape, ink, text, color, opacity and adjustment compositor.
  Coordinates stay in full D space. `StreamLimits` bounds pixels (default 50MP),
  strip rows and encoded bytes independently. `StreamReport` returns exact
  output-byte/strip counts, the plan and the same revision/source metadata.
- Vector coverage is rasterized on a fixed document-anchored 256px grid in both
  buffered and streaming paths. Regions and strip heights only select exact
  samples from those tiles; they never alter curve clipping or antialias coverage.
  A single 256 KiB tile fits within the geometry reserve. Geometry is currently
  rebuilt for each intersecting tile; very complex scenes need later caching.
- The stream estimate includes retained source bytes, input scratch row, three
  PNG encoder rows plus serialization row, 16 MiB codec reserve, ICC copies,
  conservative per-object geometry workspace (at least 16 MiB), and 64 bytes per
  strip pixel. Result objects still use the existing full-frame `AssetResolver`;
  their declared decoder workspace is reserved before allocation. A large result,
  very complex single shape, excessively wide PNG row or large retained encoded
  source can still fail explicitly. These are open capacity boundaries, not
  passed acceptance. Caller-owned project data, sink allocations, unused capacity
  beyond an encoded input slice and unrelated application memory are additional.
- A file sink keeps encoded bytes out of RAM. A `Vec` sink is supported for small
  fixtures, but its caller-owned allocation must be budgeted separately. The
  writer checks every write against `max_encoded_bytes`; it never publishes a
  destination, overwrites an original, or retries a failed sink. Fixed PNG filters,
  compression and output chunk size are independent of strip height. Generated
  standard-sRGB ICC headers use 2000-01-01 for stable hashes; supplied ICC bytes
  are unchanged, including when they are explicit color-conversion destinations.
- Cancellation is checked before allocation/output, between decoder/encoder rows,
  layers and objects. JPEG's single synchronous upstream decode call cannot be
  interrupted midway; cancellation is checked immediately before and after it.
  Caller owns scratch/output handles and paths, must keep partial artifacts
  private, and must close/remove them after success, failure or cancellation.
  Publish only after a successful report and the caller's flush/durable-save rule.

Ordinary already-decoded 50MP RGBA8 can fit a 256 MiB streaming plan without a second
full render buffer. PNG8/16 can avoid that full source allocation through scratch;
admitted baseline JPEG uses its smaller temporary RGB/Luma source. WebP decoding,
progressive JPEG, large materialized result assets and JPEG/WebP output still use
the buffered boundary. Android/FFI adoption and measured target-device memory are
separate unfinished work; this crate does not claim every <=50MP file can now be
imported/exported by the app. `tests/streaming.rs` includes strip/whole parity,
Adam7, orientation, palette, profiles, cancellation, I/O and budgets. Its explicit
ignored 50MP PNG16 marked-export test exercises the complete scratch path under a
256 MiB accounted budget and reuses scratch in separate 400MB phases. Its
completed Windows run and confirmed scratch disposal are recorded in T1.05
evidence; it is deliberately separate from ordinary native tests.

## Shared text layout

`layout_text` exposes glyph IDs, original UTF-8 clusters, font identity, advances,
and complete quadratic/cubic outlines, quantized to 1/256 D pixel. Position (0,0)
is the first baseline. Explicit newlines advance 1.25 em; wrapping is a caller
decision. Default variable-font axes are frozen by the bundled font bytes.
Unicode bidi visual runs and script/font runs are shaped with Rustybuzz OpenType.
Fallback selects one font for the complete script run, including inherited marks
and joining controls. Runs no bundled font fully covers fail explicitly.
There is no host-font fallback or network loading. The three required fonts are
bundled with individual OFL licenses and pinned commit/SHA256 provenance.
Missing glyphs/scripts fail explicitly; the bundle does not cover all Unicode.
Tabs, CRLF and other controls require explicit normalization. Limits: 65536 UTF-8
bytes, 32768 glyphs, one million outline commands, font size 0.25..4096 D pixels.

## Dependency review (2026-10-02)

Exact new pins were checked against official crates.io release metadata:

| Dependency | License | Official source |
| --- | --- | --- |
| tiny-skia 0.12.0 | BSD-3-Clause | https://crates.io/crates/tiny-skia/0.12.0 |
| rustybuzz 0.20.1 | MIT | https://crates.io/crates/rustybuzz/0.20.1 |
| unicode-bidi 0.3.18 | MIT OR Apache-2.0 | https://crates.io/crates/unicode-bidi/0.3.18 |
| unicode-script 0.5.8 | MIT OR Apache-2.0 | https://crates.io/crates/unicode-script/0.5.8 |
| libwebp-sys 0.14.4 | MIT + bundled BSD-3-Clause libwebp | https://crates.io/crates/libwebp-sys/0.14.4 |

Existing exact image0.25.10/png0.18.1/moxcms0.8.1/libm0.2.16/serde/thiserror pins
are reused. moxcms is BSD-3-Clause OR Apache-2.0, image/png MIT OR Apache-2.0.
Rustybuzz's ttf-parser (MIT OR Apache-2.0) supplies glyph outlines. License gate
must review the final locked transitive graph before central build acceptance.
`licenses/` retains the bundled **libwebp 1.6.0** notices/patent grant and source
binding. Only `ffi.rs` contains unsafe code; native output uses a RAII guard.

tiny-skia uses `default-features=false, features=["std"]`; SIMD and PNG helpers
are disabled. The direct PNG encoder has fixed Paeth filtering and compression.
The native lossy encoder can select CPU-specific kernels; only PNG and shared
layout outputs are the committed byte-exact cross-platform golden boundary.

## Reproducing central validation

1. Use the locked dependency graph and run the license gate before a new build.
2. The manifest is committed. `cargo run -p vw-raster --example generate_goldens`
   prints nine procedural PNG/layout BLAKE3 entries without image files. Do not
   replace the manifest merely to make a failing comparison pass.
3. Run scoped Rust tests, Clippy `-D warnings`, format checks and Rustdoc centrally.
4. Run the same integration tests against the same manifest on the authorized
   Android device. S23 acceptance and application UI remain separate gates.

Tests include exact 8/16-bit alpha/hidden-RGB round trips, clean regions,
all eight EXIF orientations, ICC/color-conversion provenance, all formats'
metadata, limit/preview/depth failures, hostile decode inputs, text limits/bidi,
all drawable annotations, guide exclusion, layer opacity/multiply, overlapping
ink opacity, untouched 16-bit samples, adjustments and result asset binding.

## Color profile admission

Untrusted profiles are capped at 4 MiB encoded and a conservative 16 MiB parser
expansion estimate before cloning or semantic parsing. The structural pass bounds
tag counts, localized-text record counts, channels and offsets, charging repeated
or overlapping references for every parser allocation. Caller budgets additionally
reserve simultaneous profiles and conversions before resolving result assets or
reading/writing rows. Declared but malformed PNG iCCP never becomes untagged/sRGB.
Hidden/transparent objects and excluded guides do not reserve geometry or invoke
the asset resolver. These checks protect the library budgets; caller-owned model,
sink and unrelated application memory remain separate.
