# Offline MobileSAM conversion candidate

These tools prepare and inspect a **pending admission** local MobileSAM `vit_t`
encoder/decoder pair for standalone Android LiteRT 2.3.0. They do not install,
download, enable a backend, grant license/runtime admission, or run inference as
part of verification. `model-runtime-candidate.json` has an empty
`production_admissions` array and no converted artifact hashes. Never use the
upstream `.pt` digest as `AssistedSelectionPins.modelArtifactSha256`.

Only synthetic Python metadata/layout tests have been run in this lane. Actual
conversion, reference comparison, CPU/GPU execution, working-set admission and
device journeys remain OPEN. The fixed graph is an implementation candidate;
conversion/operator compatibility has not been demonstrated.

## Inputs and toolchain

Bring local, reviewed copies of the exact source files and checkpoint listed in
the candidate catalog. The upstream source commit is
`f706ad9c4eb7f219c00d9050e46328518ffb65d2`. The checkpoint is 40,728,226 bytes with
SHA-256 `6dbb90523a35330fedd7f1d3dfc66f995213d81b29a5ca8108dbcdd4e37d6c2f`.
Source file pins were hashed from the exact official raw files, including
`LICENSE`; no checkpoint was downloaded or deserialized in this implementation
lane. Extra local Python files are never imported. The top-level convenience
`mobile_sam/__init__.py` is replaced by an empty namespace; only the hash-checked
model/build source bytes are compiled by a restricted in-memory loader.

The converter requires Linux x86_64, Python 3.11 and these **exact installed versions**:

| Distribution | Version |
| --- | --- |
| torch | 2.12.0+cpu |
| torchvision | 0.27.0+cpu |
| litert-torch | 0.9.4 |
| litert-converter | 0.4.0 |
| ai-edge-litert | 2.2.0 |
| ai-edge-quantizer | 0.9.0 |
| torchao | 0.17.0 |
| timm | 0.9.16 |
| numpy | 2.1.3 |

These primary host pins are candidates, not a complete admitted/reproducible
toolchain lock. All transitive wheels, native dependencies and their notices
still need review. The official `litert-torch` 0.9.4 wheel is 807,293 bytes,
SHA-256 `e338b9ddd71ed5fa6002525a386e22f0cec441da6deb5f9a45e73c97d91b6353`.
Its host dependency is `ai-edge-litert >=2.2.0,<2.3.0`; this differs from the
separately pinned Android runtime 2.3.0. No package installation is performed by
these scripts. Generic PyPI torch/torchvision versions and GPU suffixes are
refused; do not use their CUDA/NVIDIA dependency chain as a conversion shortcut.

Official CPU indices publish the exact CPython 3.11 manylinux_2_28_x86_64 wheels:

| Wheel | Declared bytes (HTTP HEAD) | Official index SHA-256 |
| --- | --- | --- |
| torch-2.12.0+cpu-cp311-cp311-manylinux_2_28_x86_64.whl | 192183537 | d85bdbc271bf22ef1931375a81b0366ab11081509728c58df730cf194a090818 |
| torchvision-0.27.0+cpu-cp311-cp311-manylinux_2_28_x86_64.whl | 1748192 | 376d04dcb7b75069a2b9a1af2c30ff4ac40113309907a54d0b0b26f4ad2ba075 |

Only their PEP 658 metadata was downloaded and matched to the official index
metadata hashes (31,354 bytes SHA-256
`4dea4f71028daacd7f5b67b9992c243a2cefabbadb8566316a8c2e5af0c86228` and
5,480 bytes SHA-256
`cf683281d0addcc624cbe151a0391359864f030638a52840d44063ea4a675fa0`). The CPU
torch metadata declares BSD-3-Clause and no CUDA/NVIDIA dependencies; torchvision
metadata declares BSD. The large wheel bytes were not downloaded or independently
hashed; index pins and HEAD lengths are not full wheel/license admission. Full
wheel/native/transitive notice review remains OPEN. Installed flavor/version and
dependency metadata are checked before imports, then actual `torch.version.cuda`
and `torch.version.hip` must both be `None` before any model construction.

Before importing ML packages, conversion checks source/checkpoint bytes and
installed versions. The checkpoint stays on the same held, non-symlink regular
file descriptor for hash verification and
`torch.load(..., map_location="cpu", weights_only=True, mmap=False)`. There is no
unsafe-pickle fallback, custom globals allowlist, `torch.hub`, credential access,
download, remote compiler or cloud invocation. The source builder receives
`checkpoint=None`; the verified tensor state is loaded with `strict=True`.
TinyViT's evaluation attention-bias cache is rebuilt after trained weights load.

Conversion is heavy and is only for an explicitly allocated offline host slot:

```sh
python3.11 tools/assisted-selection/convert_mobile_sam.py \
  --source-root /private/pinned-MobileSAM \
  --checkpoint /private/pinned-mobile_sam.pt \
  --output /private/new-mobilesam-bundle
```

Run in a private, network-isolated environment with reviewed installed packages.
The script additionally denies Python socket connections/name lookup and sets
Hugging Face offline/telemetry flags during imports/conversion; these controls
are not a sandbox for arbitrary native dependency code. No user image is needed.
`--help` and importing the scripts do not import torch/LiteRT or construct a model.

The output parent must exist without symlink components; the output directory
must not exist. Each output file is created exclusively in a private staging
directory. After both graphs and the manifest verify, Linux
`renameat2(RENAME_NOREPLACE)` atomically publishes the complete directory. Existing
files/directories are never overwritten. Failed staging directories may remain
for inspection; they must not be loaded or admitted and may be removed after
failure review. Each graph is capped at 128 MiB, combined graphs at 256 MiB,
manifest JSON at 1 MiB, one static subgraph, 32,768 tensors, and 8,192 operators.
FlatBuffer metadata inspection also has a two-million-read work cap, including
vector visits, to refuse adversarial alias amplification.
These are artifact-inspection limits, **not** a claim that execution fits the
unchanged 256 MiB application pool. Actual weights, native/delegate allocations,
all intermediate tensors, source/foreign copies, embeddings, coverage and import
reservations need a conservative reviewed working-set bound before runtime use.

## Exact fixed graph contract

Graph contract: `mobilesam-vit-t-fixed66-masked-v1`. Both artifacts have the
`serving_default` signature. Manifest input/output arrays use the logical order
below; actual raw tensor order/indices/names are inspected, never assumed.

| Graph | Direction | Signature name | Type | Shape |
| --- | --- | --- | --- | --- |
| encoder | input | image | float32 | 1,3,1024,1024 |
| encoder | output | image_embeddings | float32 | 1,256,64,64 |
| prompt_decoder | input | image_embeddings | float32 | 1,256,64,64 |
| prompt_decoder | input | point_coords | float32 | 1,66,2 |
| prompt_decoder | input | point_labels | int32 | 1,66 |
| prompt_decoder | input | point_valid | int32 | 1,66 |
| prompt_decoder | input | token_count | int32 | 1 |
| prompt_decoder | output | mask_logits | float32 | 1,1,256,256 |
| prompt_decoder | output | score | float32 | 1,1 |

There is no mask-input refinement tensor. The decoder uses the upstream
`no_mask_embed` and produces raw mask0 logits plus the corresponding raw IoU
score, matching `multimask_output=False`; it does not choose whichever mask has
the highest score. The other four output tokens still participate in attention.
All learned prompt, transformer, upscaling and hypernetwork modules/weights are
upstream modules. The wrapper computes mask0's hypernetwork output only.

Pack up to 64 positive/negative points as labels 1/0. With a box, append exactly
two active corners, labels 2 and 3, and no active `-1`. Without a box, append
exactly one active `-1` sentinel after the points. A box-only prompt is supported;
an entirely empty prompt is refused. The remaining slots have coordinates zero,
label `-1`, valid zero. `token_count` is the number of contiguous active slots,
1..66; `point_valid` must contain exactly that prefix of ones followed by zeros.
Coordinates have already been resized into SAM's unpadded image coordinate grid.
The model adds the upstream `+0.5` shift once, including for box corners.

Padding is **not** represented by repeated active `-1` tokens: that changes SAM
attention. The wrapper intersects `point_valid == 1` with slot index below
`token_count`; its five output tokens are always valid. It masks inactive **keys**
to negative infinity in both sparse-token self-attention branches and every
image-to-token cross-attention block. Inactive queries cannot influence valid
queries/images through these paths. Layer norms/MLPs act per token. This explains
the intended equivalence; real upstream-versus-wrapper parity for point counts,
negative points, box-only and combined prompts remains required and unrun.

Preprocessing schema `sam-rgb-resize-longest1024-v1` means: acquire full oriented
original D pixels through the trusted source owner, resolve explicit color/alpha
choices to RGB, resize RGB uint8 with upstream Pillow-compatible antialiased
bilinear sampling, and calculate resized H/W by
`floor(original_dimension * 1024/max(H,W) + 0.5)`. Refuse a rounded zero short
side. Convert to float, normalize RGB by means `[123.675,116.28,103.53]` and stds
`[58.395,57.12,57.375]`, then right/bottom pad with normalized zero to 1024 square.
This already-normalized NCHW tensor is the encoder input. Scale prompt x/y by
the actual rounded resized W/H divided by original W/H, respectively.

Postprocessing schema `sam-mask0-logits-inverse-v1` means: bilinearly upsample
raw mask0 logits to 1024 square with `align_corners=False`, remove right/bottom
padding using the actual resized H/W, then bilinearly resize to full oriented D
with `align_corners=False`. Reject nonfinite logits/scores. Coverage is 255 where
the final logit is strictly greater than zero and 0 otherwise. Do not threshold
before resizing, resize a binary mask, infer source orientation from dimensions,
or pass display/output-frame pixels as originals.

## Manifest identity and verification

All manifest identities are SHA-256 of exact canonical bytes: recursively sorted
ASCII object keys, ASCII strings only, compact JSON separators `,` and `:`, and
exactly one trailing LF. Only dictionaries, lists, strings, booleans and signed
64-bit integers are allowed. Floats, null, duplicate keys, unknown schema fields,
whitespace variants and noncanonical bytes refuse. Paths are relative portable
ASCII components with no traversal, symlinks, hardlinks or special files.

The model manifest has exactly these top-level fields:

```text
schema: "vw-assisted-model-v1"
status: "pending-admission"
model: "MobileSAM-vit_t"
source_commit: exact upstream commit
checkpoint: {bytes, sha256}
graph_contract: "mobilesam-vit-t-fixed66-masked-v1"
artifacts: [encoder artifact, prompt_decoder artifact]
preprocessing: {schema: "sam-rgb-resize-longest1024-v1"}
postprocessing: {schema: "sam-mask0-logits-inverse-v1"}
converter: {packages: exact catalog version map, files: three provenance children}
```

Each model artifact is exactly
`{role,path,bytes,sha256,signature_key,inputs,outputs,operators}`. Paths are
`encoder.tflite` and `prompt_decoder.tflite`. Each IO tensor is exactly
`{name,signature_name,tensor_name,tensor_index,dtype,shape}`; `tensor_name` is
the raw FlatBuffer name and `signature_name` is the actual named signature
binding used by Android. Operators are unique entries `{code,name,version}`,
sorted by code/version; the full opcode table must be used. Only the finite
standard builtin allowlist in `verify_artifacts.py` is allowed. Custom, Flex,
DELEGATE, vendor/unknown operators, dynamic tensors, external buffers and mutable
resource/variant tensors refuse. Metadata inspection is not proof of CPU/GPU
operator support or successful execution.
Graph IO index vectors and signature binding indices must each be unique.
In particular, `point_labels` and `point_valid` cannot alias one tensor even
though their int32 shapes match. Matching sets alone are insufficient.

The three converter provenance children are
`convert_mobile_sam.py`, `verify_artifacts.py`, `model-runtime-candidate.json`,
each `{path,bytes,sha256}`. Verification checks these against the actual local
tool files as well. The converter captures immutable bytes of all three files at
module startup, before any ML import or conversion, derives provenance from that
snapshot, and reads the candidate configuration from the same retained bytes.
It refuses changes before conversion starts and revalidates all three files
immediately before atomic publication. A file edited during conversion cannot
be recorded as though the changed tool produced the artifacts. Keep the exact
tool revision with a conversion receipt.

Runtime manifest fields are exactly
`{schema,status,runtime,version,artifacts}`, with schema
`vw-assisted-runtime-v1`, status `pending-admission`, runtime `LiteRT`, version
`2.3.0`. Each runtime child is exactly `{role,path,bytes,sha256}`, in catalog
order `litert-api`, `litert`, `litert-gpu`. Their exact AAR sizes/hashes/URLs are
in the catalog and inherited from the accepted research record. Verification
requires every actual local child; it does not synthesize or fetch an AAR.

```sh
python3 tools/assisted-selection/verify_artifacts.py catalog
python3 tools/assisted-selection/verify_artifacts.py model \
  --root /private/new-mobilesam-bundle --manifest model-manifest.json \
  --sha256 EXPECTED_CANONICAL_MODEL_MANIFEST_SHA256
python3 tools/assisted-selection/verify_artifacts.py runtime-manifest \
  --root /private/pinned-runtime-aars --manifest runtime-manifest.json
python3 tools/assisted-selection/verify_artifacts.py runtime \
  --root /private/pinned-runtime-aars --manifest runtime-manifest.json \
  --sha256 EXPECTED_CANONICAL_RUNTIME_MANIFEST_SHA256
```

Creating a runtime manifest is also exclusive and never overwrites an existing
file. Verification emits `production_admitted:false`. Neither a matching digest
nor a pending manifest closes the five-field shared admission contract. Root
must separately review/record the model license admission, runtime license
admission and runtime admission IDs and bind both exact canonical manifest hashes
in a closed catalog. Production catalog entries stay empty until that occurs.
Native split/APK and `.so` extraction identities, bundled notices, actual CPU
support and optional GPU support are further Android admission responsibilities.
Installed APK/split hashes must come from a CLOSED detached, installation-local
trusted post-build review record injected into the typed backend catalog. An APK
cannot embed its own final hash as a constant, and hashing the currently installed
APK is not permission to approve it. That detached record must bind actual source,
license and memory evidence; it must not become a new approval registry. The
backend separately checks OS-provided APK/splits and actual packaged arm64 `.so`
identities against the exact reviewed AAR child identities. No such production
entry is provided in this lane.

Cheap checks, with no installed ML package requirement:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/assisted-selection/tests -v
```

Current tool-only result: **30 synthetic tests passing**. Official source pins
matched all ten model/build/license files, 88,068 bytes total. No torch,
LiteRT, model constructor, conversion or learned-model inference was imported/run
by these checks. Python syntax and owned-file whitespace checks also passed.

Fixtures are synthetic FlatBuffer metadata and byte files, not runnable model or
runtime artifacts. AST checks establish the intended boundaries/mask call sites;
they are not learned-model parity, conversion or inference evidence.

## Attribution, official sources and remaining gates

The adapter in `convert_mobile_sam.py` derives from the SAM decoder/transformer
architecture: Copyright (c) Meta Platforms, Inc. and affiliates, Apache-2.0 as
identified by the pinned MobileSAM root LICENSE. MobileSAM is maintained by
Chaoning Zhang and collaborators. Its TinyViT implementation retains Microsoft
2022 attribution and references LeViT/Swin; their source/transitive obligations
must remain in the root redistribution review. No root license inventory or
binary bundling change is made here. The exact MobileSAM LICENSE is 11,357 bytes,
SHA-256 `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4`.
Before bundling derived models/tooling, root must retain the upstream Apache-2.0
license text, relevant notices/attribution and reviewed TinyViT/transitive notices,
plus converter and actual Android native/runtime notices. A declared license
string or POM is not admission.

Primary sources inspected for this implementation:

- [Pinned MobileSAM source and LICENSE](https://github.com/ChaoningZhang/MobileSAM/tree/f706ad9c4eb7f219c00d9050e46328518ffb65d2).
- [SAM transformer](https://raw.githubusercontent.com/ChaoningZhang/MobileSAM/f706ad9c4eb7f219c00d9050e46328518ffb65d2/mobile_sam/modeling/transformer.py), [prompt encoder](https://raw.githubusercontent.com/ChaoningZhang/MobileSAM/f706ad9c4eb7f219c00d9050e46328518ffb65d2/mobile_sam/modeling/prompt_encoder.py), [mask decoder](https://raw.githubusercontent.com/ChaoningZhang/MobileSAM/f706ad9c4eb7f219c00d9050e46328518ffb65d2/mobile_sam/modeling/mask_decoder.py).
- [Pinned resize transforms](https://raw.githubusercontent.com/ChaoningZhang/MobileSAM/f706ad9c4eb7f219c00d9050e46328518ffb65d2/mobile_sam/utils/transforms.py).
- [LiteRT Torch v0.9.4](https://github.com/google-ai-edge/litert-torch/tree/v0.9.4), [converter API](https://raw.githubusercontent.com/google-ai-edge/litert-torch/v0.9.4/litert_torch/_convert/interface.py), [published distribution metadata](https://pypi.org/pypi/litert-torch/0.9.4/json).
- [PyTorch weights-only loading](https://docs.pytorch.org/docs/2.12/generated/torch.load.html).
- [Official torch CPU index](https://download.pytorch.org/whl/cpu/torch/) and [torchvision CPU index](https://download.pytorch.org/whl/cpu/torchvision/).
- [Official FlatBuffer schema](https://raw.githubusercontent.com/tensorflow/tensorflow/v2.20.0/tensorflow/compiler/mlir/lite/schema/schema.fbs), SHA-256 `20685cadade475c2a5cccf29339ecd2bf27755da50dc3d2c092f1ee2e21ff7d3`.
- [Android LiteRT documentation](https://ai.google.dev/edge/litert/android).

Remaining gates: execute this conversion under parent resource allocation;
resolve any unsupported lowering without enabling custom/Flex/vendor shortcuts;
compare real upstream and fixed-wrapper outputs across the full prompt contract;
inspect and admit exact resulting artifacts/toolchain/notice inventories; establish
a conservative total execution working set inside the existing pool; run actual
CPU and optional GPU sessions with joined cancellation/retirement; validate source,
orientation, prompt inverse mapping, correction/import/undo and S23 quality/latency.
No NPU, <=100 ms, segmentation accuracy or hardware acceptance claim is made.

## Cross-lane integration handoff

Source base `9e41ed1d779b0c91a7c60d55a829c1f3e1defb28`, branch
`feat/assisted-backend-20261018`, boundary grant
`2f018f296a4333f44ae9c5d02e1a4c885998434d`. Existing protected source files,
module roots, dependency manifests, locks and license inventories were not edited
by the delegated leaf lanes. Root retains integration and final admission.

The source/native checkpoint is
`4f26dfff9b6def0ca0bdaf7d19de8738c7bdcb68`, pushed by root on that branch. It
contains the six common/JVM/native source leaves. The initial tools checkpoint
is `f9ff4a329c9df7c230725999c62b6efe0af4e7d8`; subsequent review repairs reject
aliased IO indices and bind provenance before conversion. These source records
are not evidence that generated bindings were integrated.

The native source lane reports three new Rust files plus 14 unrun Rust tests
(10 integration, four unit). Root registration is:

```rust
mod assisted_source;
pub use assisted_source::*;
```

Its `ProjectSession.assisted_source_pixels(binding, width, height, depth, page,
assume, budget, cancellation)` returns the owned source `Arc`. Call `describe`
and acquire `publication_lease` BEFORE `take_pixels`/`release_source`, retaining
actual delivered-byte ownership through retirement. These names avoid adding a
UniFFI method named `close`, which collides with generated ownership APIs.

The common/JVM factory `assistedSourcePixels(project)` depends on root's future
`withNativeGiantProject(project, suspend (ProjectSession) -> T)` lifetime borrow
from candidate `79d7d7fc20b8bc99c4b5990507a7cf65570f12f7`. Reuse that close-mutex
borrow, including ordinary projects; never copy an unpinned native handle across
suspension or add another close gate. The acquisition port returns an exact
source/RGBA/close lease; `retireUndelivered` retains failed cleanup ownership.
The source lane reports five common contract tests and strict common/JVM source
checks passing against ABI/project-borrow fixtures. These are not actual
regenerated-binding or full application integration evidence. Parent performed
Sol source checks of native/facade publication ownership repairs.

Root must register Android standalone `litert` and `litert-gpu` 2.3.0 with the
same transitive `litert-api` pin, retain all native/transitive notices, build the
actual APK, obtain the detached closed catalog/memory/source review described
above, and wire the actual caller's thread-safe project/currentness/page/epoch
providers. The Python tool tests are synthetic only; native integration, actual
conversion/reference inference, device CPU/GPU/cancellation/retirement and
correction/undo journeys remain separate unrun gates. Parent Astra review follows
the coherent source candidate; no product gate or production admission is closed.

The Android source checkpoint is
`3d1d6328a3710a13fb889126b813c7513c32df8a`. Its public composition hook is:

```kotlin
val backend = liteRtAssistedSelectionBackend(
    context, catalog, actualProject, currentSource, assumeUntaggedSrgb = false,
)
// Supply backend and the matching reviewed pins to the EXISTING
// AssistedSelectionInteraction constructor; keep pins null while unadmitted.
```

`currentSource` must be a thread-safe read-only callback returning null when the
actual project reference changes, during disposal/reopen, or when the full source
binding/page/attachment epoch changes. Reuse the existing interaction's ordered
invalidate/close, shared resource reservation, proposal acceptance, mask importer
and ordinary correction/undo paths. No second document/session manager is added.
`LiteRtReviewedCatalog.empty(appPrivateRoot)` is the supplied safe configuration;
only trusted root composition may inject detached `LiteRtReviewedAdmission`
records with exact manifest/APK/license/runtime/memory evidence.

Actual LiteRT 2.3.0 AAR/Android 37 API jars passed focused Kotlin compilation with
`-Werror` and explicit API checks. The native source factory was linked through a
temporary signature-only fixture: regenerated ABI and actual caller builds remain
unrun. No `Environment`, `CompiledModel`, learned-model inference, Android service
or device execution occurred. Sampled resize goldens came from installed Pillow
12.3.0, independently of any ML package. Unknown LiteRT initialization failures
propagate because the public 2.3.0 exception API exposes no typed status getter;
only observed provider absence can permit the existing single CPU fallback.
Prompt execution never retries. Encoder-only handles retire before decoder load.
Each model loads from an exclusive private unlinked snapshot, with all writable
descriptors closed before compilation and read-only descriptors retained to close.

Inspected upstream AAR arm64 children (not packaged APK admissions):

| Child | Bytes | SHA-256 |
| --- | --- | --- |
| `liblitert_jni.so` | 529296 | `b4bc082f740076f09d566b2deaa8a1af6b1ec1252d20c3688acb8405d69871dc` |
| `libLiteRt.so` | 5629696 | `e5b7e3413a8b428b1a1b669f368cf613058868bc7b63e10d0b9c2e6ba55c97d8` |
| `libLiteRtClGlAccelerator.so` | 3253032 | `c200369269a3994b646a8d0ac56ff29127c6ce742abcfd345a7d478eab3b6c6d` |

The adapter's conservative arithmetic bound is 84,672,512 fixed bytes plus
`5 * maximumSourcePixels`, reviewed model residency, reviewed activation bytes
and reviewed runtime bytes; the sum must fit 256 MiB. This is not a native peak
measurement. Original decode/publication and later mask-import reservations also
retain their existing shared pool charges. Closed memory evidence must include
snapshots, mappings, compiled weights, native/delegate allocations and intermediates
for the exact admitted model/runtime/APK identities. An artifact-size check alone
cannot close that review.

## Retained upstream license text

The following is the complete pinned MobileSAM root LICENSE, reproduced for the
derivative tooling adapter. Its original file identity is recorded above; root
still owns full bundled source/native/transitive notice admission.

```text
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```
