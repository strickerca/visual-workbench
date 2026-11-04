# MobileSAM local graph reproduction (B2), 2026-11-04

**Result: both graphs were reproduced locally from the pinned public sources.
They are byte-identical to the 2026-10-19 cloud build. Host parity passes. No
GPL/LGPL markers were found in the shipped LiteRT arm64 `.so` files.
Production admission stays open.**

This covers the two D22 B2 conditions recorded on branch
`task/s0-rebaseline-20261103` (`docs/DECISIONS.md` D22 and
`docs/license-proposals/STAGE1_LICENSE_PACKETS_20261104.md` Packet 1). The
cloud archives could not be reached (CONNECT403), so the graphs were rebuilt
in WSL2 Ubuntu with the repo's existing pipeline. No tool file was changed.
The machine-readable receipt is
[evidence/ASSISTED_MODEL_LOCAL_REPRODUCTION_20261104.json](evidence/ASSISTED_MODEL_LOCAL_REPRODUCTION_20261104.json).

## Resulting identities

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `encoder.tflite` | 28,142,464 | `92f0ac2134bb88bb5e555cdb1dd7080bc4620fbe31790a799abcb4c26156f4d3` |
| `prompt_decoder.tflite` | 18,853,624 | `9c0f89743e7056c18f4ee771e299d7db848aa14b500c025b452851fb3e2dbb78` |
| `model-manifest.json` (canonical) | 4,659 | `7ea1a2fe7c979305e65372d85cb3255e49434667cfaa1c636c8d2052518af9f4` |
| `runtime-manifest.json` (canonical, 3 AARs) | 539 | `09b7032db257a8df0e1a41784d8b8064a3b0f0ac8ba6faffdc069d4a7c0ea3c3` |

All three model hashes match the cloud values in
`tools/assisted-selection/HOST_DELIVERY_20261019.md` and Packet 1. The hashes
were produced again from scratch, not copied. Admission can therefore bind the
values Packet 1 already lists. The runtime manifest is new in this run.

The binaries are kept outside Git, in the main checkout's gitignored
`.local/assisted-model-20261104/`. That folder holds the bundle, the
runtime manifest, the reference/host parity JSON, the golden `.npz`, the logs,
and `SHA256SUMS.txt`. A working copy stays in the WSL user's home directory,
which is not part of the repo.

## Environment

- Ubuntu 26.04 LTS on WSL2 (kernel 6.6.87.2, glibc 2.43), 4 vCPU.
  `.wslconfig` caps the VM at 1,536 MiB plus 4 GiB swap. That cap was left
  unchanged because Docker Desktop shares the VM.
- **No apt packages were installed.** Ubuntu 26.04 has no `python3.11`
  package; its system Python is 3.14.
- Python 3.11.17 came from python-build-standalone release `20261003`
  (`cpython-3.11.17+20261003-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz`,
  30,785,899 bytes, SHA-256 `aadcba18…c434`). The archive was checked against
  the release `SHA256SUMS`. Its `python3.11` binary hashes to
  `a2886d92…c076`, exactly the `python_binary_sha256` in
  `host-wheel-lock.json`, so the interpreter is the one the cloud run used.
  It was extracted under the WSL user's home, not system-wide.
- The venv sits outside the repo and uses the bundled pip 24.0.

## What ran

Preparation (with network):

1. `pip download --no-deps --only-binary=:all: --require-hashes -r host-wheel-requirements.txt`
   from PyPI, plus `download.pytorch.org/whl/cpu` for the two `+cpu` wheels.
   All 71 wheels were then checked against `host-wheel-lock.json`: 0 mismatches,
   no extra files, 483,134,587 bytes in total.
2. `pip install --no-index --find-links wheelhouse --require-hashes -r host-wheel-requirements.txt`.
   This succeeded, with 71 distributions installed.
3. `download_pinned_inputs.py --root inputs` verified all 10 source files at
   commit `f706ad9c…` (88,068 bytes) and the checkpoint (40,728,226 bytes,
   `6dbb9052…6c2f`).

Offline steps, run under `offline_exec.py` (seccomp network denial, 10 GiB
address space, 512 MiB file size limit, 2 threads):

| Step | Exit | Wall time | Peak RSS |
| --- | --- | --- | --- |
| Synthetic tool tests (`unittest discover`) | 0 (31 OK) | <1 s | - |
| Seccomp probe (`socket()` inside the launcher) | EPERM | - | - |
| `reference_parity.py` (28 upstream cases + golden) | 0 | 56.7 s | 656 MiB |
| `convert_mobile_sam.py` | 0 | 140.9 s | 701 MiB |
| `tflite_parity.py` (ai-edge-litert 2.2.0 host CPU) | 0 | 19.2 s | 322 MiB |
| `verify_artifacts.py catalog` / `model` / `runtime-manifest` / `runtime` | 0 | - | - |

**Deviation from the handoff command form:** on Ubuntu 26.04, `timeout` is
uutils coreutils 0.8.0. Placed inside the launcher (`offline_exec.py timeout N
...`), it exits 126 with "Operation not permitted" under the seccomp filter.
The log of that first attempt is kept; it produced no output. The wall-time
bound was moved outside instead (`timeout N python offline_exec.py python
SCRIPT ...`), so seccomp still covers the whole script process. The conversion
bound was raised from 1200 s to 3600 s as a precaution for the memory-capped
VM. The run finished in 141 s.

`make_delivery_manifest.py` was **not** run. It hard-codes the cloud layout
(13 cloud log names and an `artifacts/host-notices-v3` notice copy), so it
cannot apply to this run without editing the tool. This document and the JSON
receipt bind this run instead.

## Parity

- **Upstream PyTorch vs. the fixed masked wrapper** (`reference_parity.py`,
  atol=rtol=1e-4): all 28 checks pass. Encoder max abs error is 0.0. Across the
  nine decoder prompt cases (positive, negative, point pair, box, mixed, two
  all-padding forms, no prompts, 66-token capacity), the largest mask error is
  2.00e-5 and the largest score error is 2.38e-7. Inverse resize and all eight
  orientation round trips match exactly.
- **Host LiteRT 2.2.0 CPU vs. golden** (`tflite_parity.py`, atol=rtol=1e-3):
  all 19 comparisons pass. The encoder max abs error is 8.64e-7. The overall
  maximum is 6.771e-5, in `box.mask`, the same as the cloud run. Static tensor
  inventory: the encoder has 1,166 tensors, with the largest at 64 MiB and a
  no-reuse sum of 4,939,392,612 B. The decoder has 640 tensors, with the
  largest at 9,306,112 B and a sum of 607,768,577 B. Host RSS after both graphs
  ran was 337,580,032 B.
- Operators (all standard builtins on the verifier allowlist):
  - Encoder (19): ADD, CONV_2D, DEPTHWISE_CONV_2D, FULLY_CONNECTED v5, MUL,
    RESHAPE, SOFTMAX, PAD, TRANSPOSE v6, MEAN, SUB, DIV, SLICE, SUM, SQRT,
    RSQRT, SQUARED_DIFFERENCE, BATCH_MATMUL, GELU.
  - Decoder (25): the shared core plus CONCATENATION, CAST, LESS, SIN,
    TRANSPOSE_CONV v3, EQUAL, LOGICAL_AND, COS, SELECT_V2.

  Full IO tensor bindings are in the receipt.

This is host-only evidence. Android LiteRT 2.3.0 CPU/GPU execution has not been
run, and neither have the device working set, cancellation, quality or latency
checks.

## D22 LiteRT arm64 `.so` scan

The three AARs were freshly downloaded into `.local/litert-aar-scan-20261104/aars/`.
All match their pins: `litert-api` 945,619 B `20f630e9…256b`, `litert`
7,640,140 B `9508c6e9…89dc`, and `litert-gpu` 4,695,499 B `173a4c38…824f`.

**Method.** The scan was read-only. Each `.so` member was read into memory with
`zipfile`. ELF64 sections and `DT_NEEDED` were parsed, and ASCII strings were
regex-searched. Nothing was loaded or executed. The scan output is
`so-scan.json` and `so-component-identifiers.json` in that folder.

| arm64 member | Bytes | SHA-256 | DT_NEEDED |
| --- | ---: | --- | --- |
| `liblitert_jni.so` | 529,296 | `b4bc082f…71dc` | libandroid, libdl, libm, liblog, libc |
| `libLiteRt.so` | 5,629,696 | `e5b7e341…97d8` | libdl, libGLESv3, libEGL, libm, liblog, libc |
| `libLiteRtClGlAccelerator.so` | 3,253,032 | `c2003692…6c6d` | libandroid, libdl, libm, libGLESv3, libEGL, liblog, libc |

These hashes match the children recorded in `tools/assisted-selection/README.md`.

**Findings:**

- **No strong markers in any of the three files.** None contains a GPL, LGPL or
  AGPL license phrase or SPDX ID, FSF text, a gnu.org/licenses URL, a Classpath
  or GCC runtime exception, or the bare word GPL/LGPL.
- The only `GNU` hits are standard ELF section names: `.gnu.version*`,
  `.gnu.hash` and `.note.gnu.build-id`. Two more matched only as substrings:
  the bionic import `__gnu_strerror_r`, and a DWARF opcode name inside LLVM
  libunwind.
- There are no libgcc, libstdc++, glibc/GLIBC_, libgomp, FFmpeg, readline, GMP,
  MPFR, iconv or gettext identifiers.
- All three files were built with Android clang 21 and linked with LLD. They
  link only Android system libraries and carry LLVM libc++/libc++abi/libunwind.
- Identifiable embedded components: abseil, XNNPACK, ruy, fft2d, cpuinfo,
  flatbuffers, a protobuf hardening flag, and a tcmalloc malloc hook. No Eigen
  identifier was seen.

**Limitation.** A string scan cannot prove that no copyleft-derived code is
present if that code carries no text marker. The files have no `.symtab`. Root
license review should confirm the embedded components' permissive licenses
against the AAR `THIRD_PARTY_NOTICE.txt`. On the evidence available, the D22
scan condition is met: nothing found blocks admission under the Android
no-LGPL policy.

## Catalog entry (not added)

`LiteRtReviewedCatalog` is unchanged. The entry that would eventually be
injected by root composition is:

```kotlin
LiteRtReviewedAdmission(
    pins = AssistedSelectionPins(
        modelArtifactSha256 = "7ea1a2fe7c979305e65372d85cb3255e49434667cfaa1c636c8d2052518af9f4",
        runtimeArtifactSha256 = "09b7032db257a8df0e1a41784d8b8064a3b0f0ac8ba6faffdc069d4a7c0ea3c3",
        modelLicenseAdmissionId = /* D22 B2 model admission ID, to be recorded */,
        runtimeLicenseAdmissionId = /* D22 B2 runtime admission ID, to be recorded */,
        runtimeAdmissionId = /* OPEN: needs Android CPU/GPU execution evidence */,
    ),
    modelManifest = "mobile-sam-fixed66/model-manifest.json",
    runtimeManifest = "runtime-manifest.json",
    installedApks = /* OPEN: post-build reviewed APK/split identities */,
    memory = /* OPEN: LiteRtReviewedMemory working-set evidence */,
)
```

## Next steps

1. Record the D22 B2 model and runtime license admission IDs, binding the
   model manifest `7ea1a2fe…f9f4` and the runtime manifest `09b7032d…a3c3`. Add
   the MobileSAM/SAM/TinyViT/Swin/LeViT attribution notices described in
   Packet 1.
2. Package the two graphs and the three AARs in a real APK build. Run Android
   LiteRT 2.3.0 CPU (and optionally GPU) sessions on the selected device. These
   produce `runtimeAdmissionId` and the reviewed installed-APK identities.
3. Establish a reviewed working-set bound (`LiteRtReviewedMemory`). Host
   no-reuse static tensor sums are about 5.5 GB, far above the 256 MiB pool, so
   the actual LiteRT arena behaviour must be measured.
4. Only after steps 1–3, inject the catalog entry above. Production
   `production_admissions` in `model-runtime-candidate.json` stays empty until
   then.
