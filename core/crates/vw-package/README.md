# VIP compiler foundation (T2.08)

This new crate compiles one immutable image/capture document snapshot into an
in-memory `vip-1` package: `manifest.json`, `prompt.md`, `semantic.json`, a clean
source derivative, a marked overview and one crop per instruction marker. It
performs no capture, network request, credential lookup, filesystem publication
or project mutation. Application preview/send/publish authority remains local to
the caller. This crate alone does not close T2.08 or PKG-001–004.

## Inputs and source authority

`compile(CompileInput, CompileOptions, cancellation)` borrows an immutable
`Project`, its full matching `Revision`, one document ID, and a map of exact
encoded originals. The caller must obtain that snapshot under its project lock;
all input borrows remain fixed throughout compilation. All provided asset bytes
are size-admitted and BLAKE3-checked against canonical metadata. Decoded primary
source dimensions, EXIF orientation, bit depth and ICC bytes must match metadata.
Missing originals fail. No preview or thumbnail can stand in for an original.

Instruction projection uses reviewed `vw-instructions` numbering/link/role
validation. An explicitly selected semantic snapshot uses reviewed
`vw-semantics` loading, matching project/document/full revision hash, capture
session/frame/geometry and primary asset. Every requested EID must resolve in
that snapshot. No automatic selection of a newer accessibility tree occurs.

The manifest stores the full source hash/host sequence in `extensions` as well as
the required abbreviated revision label; equal visible hashes at later host
sequences remain different package receipts. Package ID and creation time are
explicit inputs, so repeated identical calls produce identical bytes. No device
identity, native window handle, monitor identifier or private project title is
projected. Captured application name is literal data; window title is omitted
unless the caller sets the explicit owner preview choice.

## Pixels and coordinates

The authoritative ROOT `vw-raster` implementation supplies source decoding,
canonical full-document rendering and ICC conversion. The old placeholder
raster crate in this isolated worktree is not an implementation dependency to
copy into ROOT. The package creates explicit sRGB, RGBA8 derivatives; untagged
source assumption and 16-to-8-bit conversion each require caller permission.
Encoded originals and model state remain unchanged. This does not claim that
visual derivatives preserve high-depth source samples.

Integer rational area resampling uses premultiplied alpha and a single uniform
scale for both axes. Source rectangle, numerator/denominator, content size and
final canvas size are retained in the manifest. Fractional final rows/columns
and minimum-side padding use white context on the bottom/right; document
coordinates are never shifted. The overview has at least a 64-pixel canvas side
for readable badges. Crops expand each box by 1.75 with at least 256 document
pixels per side, clamping to a sufficiently large original and padding smaller
originals. Crop outputs have at least 256 pixels per side. All output images stay
within the explicitly selected model edge.

Package marker overlays use role colors, three compiled pixels of outline and
a 24-pixel-high badge with fixed bitmap digits. Both overview and crops contain
the same marker identity. Canonical annotations are rendered first; role overlays
do not change stored appearance. Point markers at source edges get an in-bounds
one-pixel D box. This version refuses markers outside the source image. Hidden
marks are still described in prompt data, and every numbered instruction marker
receives an explicit package overlay.

Profiles are explicit, not guessed from model names:

- Claude: configured modern 2576 or legacy 1568 long edge; coordinates describe
  actual image dimensions, not API padding to multiples of 28.
- OpenAI Responses: configured edge and the `original` detail instruction.
- Codex localImage: configured, caller-verified edge and installed-schema SHA256;
  the compiler does not inspect an installed client or invent a detail option.
- Gemini: `[ymin,xmin,ymax,xmax]` normalized to 0–1000 using final compiled image
  dimensions; point coordinates are `[y,x]`.
- Generic: absolute compiled `[x,y,w,h]`, with original D geometry alongside it.

These are deterministic output configurations. No current provider capability,
installed-client behavior, cost, acceptance or live send is inferred by this
crate. The caller owns current capability checks.

## Literal text, manifest and verification

Owner instructions, object geometry/text and captured semantic names are kept as
literal JSON strings in code spans whose delimiter is longer than every input
backtick run. Control characters/newlines cannot create new prompt structure.
Roles are typed separately; unlabeled marks are context only. Each document
object is described, including hidden marks; stroke descriptions retain brush,
sample count and endpoints without dumping an unbounded sample log. The required
untrusted-text notice is included in JSON and at the end of the prompt, together
with the explicit preserve-outside-change constraint. Markdown files are data;
an app must not execute embedded links or treat imported package prose as local
authority.

Each payload has a SHA256 inventory entry; images also carry SHA256 and physical
dimensions. `manifest.json` is deliberately excluded from its own inventory;
`manifest_sha256()` supplies its separate receipt. `Package::from_files` rejects
duplicate/unknown/path-traversal/case-alias names, unknown/duplicate JSON fields,
hash and inventory disagreement, malformed PNGs, dimensional mismatches, invalid
coordinate mappings, unresolved/mismatched semantic references and unsupported
compiler schema. All file names are validated data, never filesystem paths.
Every image's single uncompressed `VisualWorkbenchPackage` receipt must exactly
match its manifest project/document/full revision/source/mapping. Duplicate,
compressed or foreign text receipts and unexpected ancillary chunks fail before
pixel decode. Decoded ICC bytes must equal the authoritative raster engine's
deterministic sRGB profile, including its fixed header. A foreign valid profile
cannot be relabeled by changing hashes and the manifest's color-space string.
The loader is intentionally specific to this compiler's extension version; it
does not claim general forward compatibility with arbitrary external VIPs.
Hashes establish byte integrity, not the authenticity or truth of an imported
author's source/coordinate/color claims. A trusted compile path establishes those
bindings against its borrowed project and originals.

The typed manifest matches the current `contracts/package.schema.json` draft.
No shared schema was edited. Central validation must additionally run the real
JSON Schema validator against generated manifests; no schema-validation result
is claimed by source review.

## Resource admission and cancellation

Default limits are 1 GiB logical memory, 64 MiB total encoded originals, 50 MP
source, 128 MiB total package, 32 MiB per PNG and 64 markers. Configurable ceilings
are 4 GiB memory, 256 MiB originals, 512 MiB package and 512 markers. Canonical
project serialization is admitted at 2 MiB before model hashing; this deliberately
refuses larger models instead of allocating a hidden unbounded projection.

Before any source decode/clone, memory admission reserves twice the complete
configured package cap (including bounded geometric Vec capacity), 96 MiB for
canonical/semantic/manifest/prompt projections, and retained original capacities.
Before serde deserialization, a nonallocating JSON walk bounds depth, field and
array counts, raw string lengths and a conservative typed projection estimate.
Manifest/semantic parsing reserve at most 32/16 MiB of that 96 MiB allowance.
Constraints and redactions are capped at 64 each; markers/images/files use the
caller marker limit, and instruction/reference/tree arrays use their producer
limits. Escaped/non-ASCII field names are refused in this own-compiler format;
escaped and Unicode string values remain supported. Full JSON type, duplicate
field and syntax validation follows this admission pass.
Subsequent codec budgets subtract every retained decoded image and intermediate
encoded image. Resampling/PNG staging additionally reserves a 32 MiB codec margin,
and work is bounded at 220 million area-cell visits per image. PNG output writers
and JSON serialization are capped during writes. Admission is conservative and
may refuse a 50 MP document at the default budget; no measured 50 MP phone memory
or latency pass is claimed. Full streaming/tiled package compilation is open.

Cancellation is checked between assets, rows, markers, prompt objects and
verification images. Bounded model hashing, canonical full render, ICC conversion
and individual native codec calls are not interruptible internally. The function
returns no partial `Package`; caller-owned input bytes are never deleted or
modified. No global worker, temporary file or process is created.

## Central integration and open work

Only this new crate is owned here. Parent integration must add its workspace
member and use reviewed `vw-instructions`, reviewed `vw-semantics`, current ROOT
`vw-raster` and existing dependency pins, then format, compile, lint, run Windows
and Android deterministic fixtures, validate generated schema, and bind results
to the exact source manifest. The written regressions exercise actual core
source rendering as well as exact integer pixel/mapping goldens. No tests were
run during this source-only implementation.

Open: atomic folder/`.vipz` filesystem publication, Send preview/application FFI,
provider adapters and current capability inventory, review/cost/secret gates,
PDF-page/SVG import rasterization, very large streaming packages, redaction UI,
package read-back UI, and physical/cross-language/device acceptance. Unsupported
PDF/SVG inputs return an explicit error. This foundation cannot imply permission
to capture a window, access an accessibility tree or send private content.
