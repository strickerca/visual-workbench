# Shared mask math

`vw-mask` is the software foundation of T2.01. It provides document-resolution
selection coverage and data exports. It does not implement editor tools, project
transactions, undo storage, object/vector-stroke erasers, platform bindings or
acceptance UI. Mask erasure is `subtract`. Those integrations, both-device
rendering, G1/G2 and target performance remain separate work.

The implementation uses existing approved exact pins only: thiserror 2.0.21,
libm 0.2.16, BLAKE3 1.8.7 with `pure`, png 0.18.1, and proptest 1.11.0 for tests.
There are no OS, network, filesystem, GPU or worker-runtime calls in this crate.
Callers must run these bounded CPU operations on their application workers.

## Coordinates and coverage version 1

`Size::new(width,height)` is the **oriented document pixel grid**. A pixel `(i,j)`
occupies `[i,i+1) × [j,j+1)` and its center is `(i+0.5,j+0.5)`. D-space x points
right and y points down. Apply source EXIF orientation before constructing this
grid. Camera, zoom, rotation, display scaling and DPI do not enter mask math.
Region crops use integer pixel coordinates without resampling.

Continuous input coordinates are finite and bounded to ±8,388,608 D pixels.
They round to the nearest 1/256 pixel using `libm` (half ties away from zero).
Off-document geometry is clipped. Off-document coverage is always zero.
Coverage 0 means unselected, 255 fully selected. Alpha is straight, never
premultiplied. Unsupported values return errors before changing an input.

- Rectangles integrate exact covered area after edge quantization.
- Lasso paths close implicitly and use even-odd fill, including self crossings.
  Reversed winding and a repeated closing vertex have the same meaning.
- Painted selections union circles at path vertices and connecting rectangles,
  producing round capsules with constant radius. Rectangle normals round to
  1/256 pixel. Overlaps are filled once; opacity multiplies the final area once.
  One point is a dab. Repeated points are removed. Variable-pressure brush policy
  is an application/ink integration choice; this API does not invent a pressure
  curve for mask painting.
- Lasso/paint area is integrated on 256 horizontal subpixel centers per pixel.
  Interval endpoints round to 1/65536 x pixels; sums and final 8-bit rounding are
  integer. Polygon intersections use i128; circle intersections use pinned libm.
  Work uses active edges and span differences rather than a 256×256 pixel loop.

The tests include exact fractional rectangles, triangle and self-crossing lasso
areas, analytic circle/capsule answers, and randomized triangles compared with a
separate polygon-clipping/shoelace reference within one 8-bit level. These are
software reference checks; no unrun Windows/Android comparison is claimed.

## Immutable versions and sparse tiles

`Mask` has private storage and immutable, shared tile bytes. Each operation
returns a new mask, even when its pixels equal the input. Unary operations advance
`version` by one; a binary operation advances `max(parent_versions)` by one.
Counter overflow is an error. Constructors start at version 0 (`empty`, dense or
explicit imported tiles); selection constructors create version 1.

The counter records ancestry depth, not a globally unique project operation ID.
The application must assign its transaction/mask ID and retain prior masks in
history. The library never deletes previous versions or retains an unbounded
history internally. `content_hash` binds algorithm, size and canonical tile bytes,
and intentionally excludes the history counter so identical coverage can share
an immutable blob.

Tiles occupy a 256×256 grid. Only nonzero tiles are stored. Edge tiles contain
their actual in-document width and height, row major, avoiding enormous padding
for a thin document. `TileView` supplies those dimensions. Tiles are ordered by
tile y, then x. All-zero, duplicate, out-of-grid and malformed imported tiles are
rejected. `encode_lossless`/`decode_lossless` provide strict canonical little-endian
data with no compression, ambiguity or trailing bytes:

```
8 bytes "VWMASK01"
u32 algorithm_version, u32 width, u32 height, u64 version, u32 tile_count
repeated: u32 tile_x, u32 tile_y, u32 byte_count, exact coverage bytes
```

A container may apply lossless compression. Masks never use lossy color codecs.
`bounds` reports the smallest integer rectangle containing nonzero coverage;
empty masks return `None`. Crop export preserves the requested document origin.

## Math and finite support

For two coverage values `a` and `b`, add is `max(a,b)`, subtract is
`max(a-b,0)`, intersect is `min(a,b)`, and invert is `255-a`. These are grayscale
set operations; repeated add/intersect are idempotent. Binary dimensions must
match exactly. Mask painting opacity is not repeated source-over compositing.

Expand/shrink take an integer radius and use maximum/minimum over offsets
`dx²+dy² ≤ radius²`. The disk is Euclidean, not a square. The radius-1 neighborhood
has five pixels. Shrink includes off-document zero pixels, so it erodes the
document border. Horizontal monotone deques implement the disk extrema; results
are tested against a separate direct disk enumeration.

Feather is a **normalized uniform disk average**, using the same finite disk.
It is not an unbounded Gaussian. The denominator is the full number of disk
samples even at document edges; sums divide once, with nearest integer rounding.
A radius-1 impulse yields five values of 51. A constant full mask at a corner
becomes 153 because two of the five samples are outside the document.

This explicit provisional kernel makes support predictable: no feathered pixel
lies outside `expand(radius)`'s support. The future composite proof's extra pixel
margin can conservatively contain coverage. This library does not itself compose
AI results or establish their outside-mask proof. Kernel changes require a new
algorithm version and reviewed fixture changes.

## Exports and limits

`encode_mask_png(region)` writes grayscale PNG8 with exactly the exported mask
values and no color transform. A provider's alpha-mask polarity is a separate
adapter contract and must be mapped explicitly.

`cutout_rgba8` / `cutout_rgba16` accept the complete oriented document's straight
RGBA pixels and an integer crop. They preserve every RGB channel, including RGB
under zero alpha, and multiply alpha by mask coverage using `(a*m+127)/255`.
There is no resampling, matte, color conversion or bit-depth reduction. Returned
`Cutout8` / `Cutout16` contain `{region,size,rgba}`. Carry the source ICC profile
into the raster PNG exporter; this crate does not silently tag arbitrary RGB as
sRGB. Encoding/profile-preserving cutout PNGs remains raster/UI integration.

The grid is limited to 50,000,000 pixels and 1,000,000 pixels per side. Paths have
at most 16,384 points. Radius is at most 256 pixels. Active scan conversion and
filters have a 1,000,000,000-unit work budget; filters charge three linear passes
per disk row before allocation. Individual output/scratch buffers and estimated
filter working buffers have a 256 MiB ceiling. A full 16-bit cutout may exceed
that bound and must be exported in bounded regions. Allocation and bound errors
are explicit, and incomplete operations cannot mutate the source mask.

The library has no implicit resize, coordinate fallback or unbounded radius.
Tests must be run through the parent's serialized build/validation lane. The
`mask-golden` example prints only deterministic synthetic fixture hashes for a
later Windows/Android comparison; it contains no user/device data.
