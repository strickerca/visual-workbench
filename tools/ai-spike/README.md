# Image edit software spike

This Rust CLI prepares a real OpenAI image edit request and implements the local
crop, resize, color conversion, composite and proof pipeline. Its default
configuration has no cost forecast, so live sending remains disabled until an
operator reviews an estimate and confirms the exact request. No credential or
paid request is needed for `prepare`, `mock`, the Rust tests or the Python verifier.
Five offline mock cases establish local containment only. The owner-photo edits,
API account access, semantic quality, measured model latency and actual spend
remain unverified until the separately authorized live run.

## Local contract

- Source inputs are bounded 8-bit PNG/JPEG, at most 20 million decoded pixels and
  16,384 pixels per edge. Higher bit depths and non-RGB ICC profiles fail clearly;
  the original file is never written. Untagged sources require `--assume-srgb`.
  EXIF orientation is applied before document-coordinate masks are interpreted.
- The change mask is the maximum-coverage union at document resolution. In mask
  PNG inputs, alpha 0 means full change; alpha 255 means preserve. Rectangle masks
  select their exact half-open integer bounds. Empty masks are rejected.
- The crop expands the mask bounds by 1.75 times, with at least 64 pixels on each
  side, rounding outward and clamping to source bounds. Aspect correction grows
  the crop without shrinking it. When the whole source is too narrow for a valid
  aspect, the crop can extend outside it: source edge pixels are replicated for
  model context and the padded mask is preserve-only. Output dimensions never grow.
- Model size is the valid grid size minimizing the sum of squared logarithmic
  width/height scale changes. Ties use width, then height. This makes the spec's
  “closest” rule deterministic, including minimum-pixel upscaling. Grid rounding
  may produce slight anisotropic scaling, which is inverted during stitching.
- Pixels resize with normalized premultiplied-alpha Lanczos3 in their source encoding, then
  convert through the original RGB ICC profile to sRGB. The returned image is
  normalized to sRGB, resized back with Lanczos3 and converted to the source ICC
  encoding. The composite PNG retains that profile. Original encoded bytes and
  decoded exterior RGBA bytes remain unchanged.
- Feather is a normalized circular box kernel with compact radius support and
  zero mask extension outside the image. Radius defaults to 8 and is limited to
  64 pixels. The composite uses the specification's channel-wise blend formula;
  every zero-weight pixel and every pixel outside the crop is copied exactly.
- Proof compares all RGBA channels outside a Euclidean disk dilation of the
  original mask by `feather + 1` pixels. Nonzero exterior changes are errors.
  SHA-256 binds source/result pixels, mask, original encoded bytes and the two
  indexed exterior streams. The independent verifier uses row bitsets rather
  than the Rust prefix-sum dilation implementation.
- The prepared request also binds the exact normalized PNG ICC profile, even
  when the original explicitly assumed sRGB. Independent verification requires
  the source and composite profiles to match that binding; profile-only changes
  cannot pass merely because the RGBA channel bytes remain equal. Generated
  standard sRGB profiles use a fixed ICC header date, so identical preparations
  produce stable request IDs across runs. Supplied ICC bytes are preserved.
- Metrics describe the selected pixels after source-profile conversion to
  8-bit sRGB: CIEDE2000 in Lab D65 and global masked-luminance SSIM (population
  variance, K1=.01, K2=.03, L=1). They are descriptive change metrics, not quality
  acceptance. RGB-only metrics omit alpha; the exact proof includes alpha.

CIEDE2000 reference numeric pairs come from the authors'
[supplementary test data](https://hajim.rochester.edu/ece/sites/gsharma/ciede2000/)
for Sharma, Wu and Dalal (2005), checked 2026-10-02. The formula is implemented
directly; no MATLAB or spreadsheet implementation is copied or shipped.

## Offline commands

The crate is in the root workspace. Build with `build.ps1 build-ai` (including
license gates), then run `build.ps1 test-ai` for Rust/Python tests, strict Clippy
and formatting. `build.ps1 hil-test rust vw-ai-spike` runs the platform-neutral
kernels on the explicitly authorized physical Android device; the live HTTP
adapter remains Windows-only. Serialize these commands with other heavy builds.
Independent verifier tests use the already configured Pillow tool dependency:

```powershell
python -m unittest discover -s tools/ai-spike/tests -p 'test_*.py'
```

After building, create each fixture in a fresh task-owned temporary directory
outside Git. Repeat for `color`, `object`, `text`, `border` and `small`:

```powershell
target/debug/ai-spike.exe --mode mock --synthetic-case small --out C:/owned-temporary-run/small
python tools/ai-spike/verify_exterior.py C:/owned-temporary-run/small
```

The mock intentionally replaces every returned pixel. Its filenames are scenario
labels, not evidence that an AI removed an object or edited text. The fixture
pixels are generated locally from arithmetic; no external or private photo is
included in source. Dispose task verification images after inspection and retain
only text counts, hashes and cleanup receipts.

For an owner-selected image, first prepare without sending:

```powershell
target/debug/ai-spike.exe --mode prepare --source C:/private/photo.png --rect 100,100,80,60 --prompt-file C:/private/instruction.txt --out C:/private/prepared-edit
```

The output is a new private directory with `original.input`, normalized
`source.png`, the document `mask.png`, request image/mask PNGs, request description
and a final `result.json` marker. `mock` and confirmed `send` also produce the
provider PNG, composite PNG and proof JSON. An existing directory is never
overwritten. A failed write may leave a partial private directory; without the
completion marker it must not be treated as a completed result. The independent
verifier checks the encoded original's hash and uses the normalized PNG as the
exact pixel baseline, avoiding differences between independent JPEG decoders.

## Live request and cost boundary

`config/openai-2026-10-02.json` records the verified endpoint, model IDs,
capabilities, quality, prices, sources and unresolved documentation differences.
Select a model and an explicit quality through configuration. Supply a reviewed
`token_estimate` containing `text_input`, `image_input`, and `image_output` token
counts. The cost calculation uses integer micro-USD; mock results have no spend.
The provider docs do not establish a hard per-request token ceiling, so this is
an estimate, never a promise that actual billing cannot exceed it.

Only a subsequent `--mode send --confirm REQUEST_ID` with exactly the reviewed
inputs/configuration can enter the live adapter. It reads the generic Windows
credential `VisualWorkbench/openai`, username `openai`, directly with `CredReadW`.
There is no key argument, key file or environment-key fallback. The owner enters
the key in Windows Credential Manager; never enter it into chat. Key and header
buffers are wiped before release and never included in errors, output or evidence.

Before reading credentials, a persistent exclusively locked ledger reserves the
estimated charge. Total spike reservations/settled charges must stay within
$2; the configurable daily soft budget defaults to $5. There is one HTTP attempt,
no application retry, no redirects and no automatic authentication/cookies. TLS
uses Windows validation. Native operations have finite timeouts, total receive
time and body size are bounded, and the outer process runner should retain its
own deadline. This is a synchronous Windows-only transport prototype.

The live ledger has one fixed location under
`LOCALAPPDATA/VisualWorkbench/ai-spike-budget.json`; there is no live path override.
Each reservation binds the exact request ID and provider configuration. Before
the adapter can attempt a POST, it synchronizes an `attempted` marker to disk.
The same reservation cannot enter the adapter again. Process loss, uncertain
network outcome and missing usage retain the charge and block further sends.
Old or malformed ledger records without the required bindings fail closed and
require review; never clear them to force a retry.

Successful valid usage reconciles the reservation to its actual token cost. A
missing or inconsistent usage record, network error or interrupted run leaves an
unresolved reservation and blocks further sends until reviewed. A confirmed
pre-send credential failure releases the reservation. Never delete, replace or
rotate ledgers to bypass the guard. A provider-side billing cap and account access
remain owner configuration; this local estimate guard cannot guarantee them.

## Official provider facts checked 2026-10-02

[The edit reference](https://developers.openai.com/api/reference/cli/resources/images/methods/edit)
documents `POST /v1/images/edits`, PNG alpha masks with fully transparent editable
areas, a matching first-image size, one or more image inputs, PNG output and a
32,000-character prompt limit. This spike sends one PNG and one mask with `n=1`,
explicit size/quality, and omits unsupported legacy `response_format` and ambiguous
`input_fidelity` settings.

[The image guide](https://developers.openai.com/api/docs/guides/image-generation)
documents Sunburst/Flare, prompt-guided masks, side multiples of 16, edges up to
3,840, areas 655,360–8,294,400 pixels, aspect ratios 1:3–3:1, and experimental
outputs above 3,686,400 pixels. Organization verification may be required. Its
50 MB mask wording differs from the reference's 4 MB cap; the configuration uses
the stricter decimal 4 MB cap. Direct Images requests receive no cached-input
discount. The reference's usage description still names an older model; the guide
recommends usage for current billing, so missing usage fails closed locally.

The [Sunburst](https://developers.openai.com/api/docs/models/gpt-image-2.5-sunburst)
and [Flare](https://developers.openai.com/api/docs/models/gpt-image-2.5-flare)
pages identify the undated names and `2026-09-08` snapshots, with
`low`, `medium`, `high`, `xhigh`, `max` and `auto` quality settings. Configuration
uses explicit quality, omitting `auto` for repeatability. Standard rates per million
tokens are $5 text input, $8 image input and $30 image output, also listed on the
[pricing page](https://developers.openai.com/api/docs/pricing). Free-tier access is
unsupported; the published Tier 1 limit is 100,000 tokens/minute and 5 images/minute.
The owner's actual account tier, permissions and available models are unverified.

Dependencies reuse pinned image 0.25.10 (MIT/Apache-2.0), moxcms 0.8.1
(BSD-3-Clause/Apache-2.0), serde, serde_json, thiserror, fs2, tempfile and windows.
Official crates.io metadata checked on the same date identifies sha2 0.11.0 and
base64 0.23.1 as stable releases under MIT OR Apache-2.0. Root's lockfile/license
checks are required before every dependency-changing build; exact results and
remaining acceptance limits are recorded in `docs/evidence/T0.11.md`.
