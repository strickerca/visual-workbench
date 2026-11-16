# License-review packets: B2 assisted selection, B3 WiX, B3 Android tunnel

Prepared 2026-11-04. Repo: `C:\Users\redacted\projects\Design Workbench\visual-workbench` (read only, nothing edited).
Policy (BUILD_SPECIFICATION.md section 2.5 / D17): no GPL/AGPL/SSPL/non-commercial/unknown licenses in the app or anything linked into it. LGPL is allowed only on Windows, as dynamically linked, replaceable DLLs, and **never on Android**.
Scope: a personal build that is not distributed. Most Apache/MIT/LGPL/MS-RL obligations apply only on *distribution*, and installing on your own PC or phone is generally not distribution. Each packet still lists the notices so the build is ready for distribution later.
"Unverified" means I could not confirm it this session.

---

## Packet 1: B2 assisted selection model (SEL-001, T3.06)

### What would be admitted
**Ships in the APK:**
| Artifact | Identity |
|---|---|
| `encoder.tflite` (cloud build) | 28,142,464 B, SHA-256 `92f0ac2134bb88bb5e555cdb1dd7080bc4620fbe31790a799abcb4c26156f4d3` |
| `prompt_decoder.tflite` (cloud build) | 18,853,624 B, SHA-256 `9c0f89743e7056c18f4ee771e299d7db848aa14b500c025b452851fb3e2dbb78` |
| `model-manifest.json` | 4,659 B, SHA-256 `7ea1a2fe7c979305e65372d85cb3255e49434667cfaa1c636c8d2052518af9f4` |
| `com.google.ai.edge.litert:litert:2.3.0` AAR | 7,640,140 B, `9508c6e9142646f0f2bc1fe68b3925fb470ee7ddd29114540339a04a8ab189dc` |
| `litert-api:2.3.0` AAR | 945,619 B, `20f630e9ed0b4f0564da616fd3d6406f3013cacad7d492cbe2963330f519256b` |
| `litert-gpu:2.3.0` AAR | 4,695,499 B, `173a4c3813601f7e9542f4e6a49e237775e2f9ce1e0948f35407098bffb1824f` |

If the graphs are reproduced locally (see "Practical route" below), they get **new** hashes. The tools refuse to reuse the cloud hashes for a fresh conversion, so admission must bind whichever hashes are actually produced.

**Host only, not shipped:**
- Checkpoint `weights/mobile_sam.pt`: 40,728,226 B, SHA-256 `6dbb90523a35330fedd7f1d3dfc66f995213d81b29a5ca8108dbcdd4e37d6c2f`, at MobileSAM commit `f706ad9c4eb7f219c00d9050e46328518ffb65d2`.
- 10 pinned MobileSAM source files.
- The 71 converter wheels, each hash-pinned in `tools/assisted-selection/host-wheel-requirements.txt`. Key ones:
  - `litert-torch` 0.9.4: `e338b9dd…6353`
  - `torch` 2.12.0+cpu: `d85bdbc2…0818`
  - `torchvision` 0.27.0+cpu: `376d04dc…a075`
  - also `ai-edge-litert` 2.2.0, `litert-converter` 0.4.0, `timm` 0.9.16, `numpy` 2.1.3

  These wheels carry MIT, BSD, Apache and MPL-2.0 terms, plus 10 wheels with no declared license. `torch` bundles a `libgomp` (GCC runtime). None of this reaches the APK.

**Explicitly excluded:** `com.google.android.play:ai-delivery:0.1.1-alpha01`, which comes in through the LiteRT API POM under Play Core terms. Gradle already excludes it (ASSISTED_RUNTIME_REGISTRATION_20261019.md).

### Licenses (with sources)
| Component | License | Evidence |
|---|---|---|
| MobileSAM code | **Apache-2.0** | Repo `LICENSE` at f706ad9c, SHA `c71d239d…0ab4`. The hash is identical to other verbatim Apache-2.0 copies in the census. https://github.com/ChaoningZhang/MobileSAM/blob/f706ad9c4eb7f219c00d9050e46328518ffb65d2/LICENSE. **No NOTICE file** at that commit (checked against the GitHub contents API). |
| MobileSAM checkpoint weights | **No explicit license** (unverified; most likely Apache-2.0) | The README has no license section and says nothing about the weights. The checkpoint ZIP has no license entry (repo audit). The file is committed inside the Apache-2.0 repo, so Apache-2.0 is the most natural reading, but there is no explicit grant. |
| TinyViT (image encoder lineage) | **MIT**, "Copyright (c) Microsoft Corporation." | https://github.com/microsoft/Cream/blob/main/TinyViT/LICENSE. The header of `tiny_vit_sam.py` reads "Copyright (c) 2022 Microsoft / Adapted from LeViT and Swin Transformer" and has no license line. |
| LeViT, Swin (TinyViT's sources) | LeViT **Apache-2.0**; Swin **MIT** (Copyright Microsoft Corporation) | https://github.com/facebookresearch/LeViT/blob/main/LICENSE, https://github.com/microsoft/Swin-Transformer/blob/main/LICENSE |
| SAM (pipeline, mask decoder, prompt encoder, distillation teacher) | **Apache-2.0** for code and model | https://github.com/facebookresearch/segment-anything. The README says "The model is licensed under the Apache 2.0 license." |
| LiteRT 2.3.0 Android runtime | **Apache-2.0**, plus an aggregate `THIRD_PARTY_NOTICE.txt` | https://github.com/google-ai-edge/LiteRT/blob/main/LICENSE. Each AAR contains the same 13,575 B LICENSE and 1,960,672 B THIRD_PARTY_NOTICE. Both are already in `third_party/notices/LiteRT-2.3.0-*` and indexed in `third_party/LICENSES`. |

### What must ship in the APK as notices
Nothing is legally required for a build that is never distributed. For distribution readiness, ship:
1. The LiteRT LICENSE and THIRD_PARTY_NOTICE. These are already registered.
2. The MobileSAM Apache-2.0 text, plus a short attribution:
   - MobileSAM (Zhang et al.), Apache-2.0
   - SAM (Meta), Apache-2.0
   - TinyViT and Swin: MIT notice "Copyright (c) Microsoft Corporation" with the permission text
   - LeViT, Apache-2.0
3. A statement that the `.tflite` files are *converted/modified* from the original checkpoint (Apache-2.0 section 4(b)).

Apache-2.0 section 4(d) does not apply, because MobileSAM has no NOTICE file. The repo's candidate MobileSAM text is `docs/license-proposals/notices/assisted-selection/upstream/MobileSAM-f706ad9c-LICENSE`. It is not yet registered.

### Risks / unknowns
- **Weights rights are implied, not stated.** This is the main legal gap the repo itself flags.
- **Training data.** According to the MobileSAM paper (not re-checked here), MobileSAM was distilled on a subset of SA-1B, and SA-1B has its own research-only dataset license. Whether that restricts the derived weights is legally unsettled. The risk is low for personal use.
- **GPL texts in the LiteRT aggregate notice.** `THIRD_PARTY_NOTICE.txt` (35,718 lines) contains about 35 GNU license headers: GPLv2 + Classpath exception, a "Google Runtime Environment" GPL text, and autoconf/bison exception texts. These look like build-environment notices. It is unverified whether any GPL/LGPL code is compiled into the arm64 `.so` files that ship. Because Android forbids LGPL, the repo's still-open per-native-component review must close this before release.
- The real Android CPU/GPU execution, memory bound and APK census are unrun. Those are separate gates, not license gates.

### Practical route (the cloud archives are unreachable)
- The 3 private Library archives are in the cloud workspace. Every download fails with **HTTPS proxy CONNECT403 before any payload bytes**: authorization succeeds and all 3 partial files are 0 B (DEFERRED_VALIDATION_LEDGER_20261018.md around line 417).
- **They can be reproduced on the laptop from public, pinned sources:**
  - `download_pinned_inputs.py` fetches the checkpoint (raw.githubusercontent.com at f706ad9c) and the source files.
  - The 71 wheels come from PyPI / download.pytorch.org with `--require-hashes` against `host-wheel-requirements.txt`.
  - Then run `reference_parity.py`, `convert_mobile_sam.py` and `tflite_parity.py` under `offline_exec.py`.
- **The catch:** the wheels are cp311 manylinux_2_17/2_27/2_28 **x86_64**, and `offline_exec.py` is a **Linux seccomp** launcher. Neither runs on native Windows Python.
- **`wsl.exe -l -v` result (run read-only, exit 0):**
  ```
  * Ubuntu          Stopped   2
    docker-desktop  Running   2
  Default Distribution: Ubuntu, Default Version: 2
  ```
  So a WSL2 Ubuntu exists and can serve as the Linux environment. `docker-desktop` is Docker Desktop's internal distro and is not meant as a work environment; a Docker Linux container is an alternative route.
- **Unverified (I did not start the distro):**
  - the Ubuntu release, and whether it meets glibc ≥ 2.28 (Ubuntu 20.04 or later does)
  - whether Python **3.11** is available (the original run used 3.11.17)
  - whether seccomp works under the WSL2 kernel
  - free RAM and disk for conversion. The host peak RSS of 338 MB was measured only for the parity interpreters; conversion is heavier, amount unknown.
- Expect new graph hashes; byte-identical output to the cloud build is not established.

**Recommendation: approve with conditions.**
- Admit the MobileSAM code (Apache-2.0) and LiteRT 2.3.0 (Apache-2.0).
- Admit the checkpoint and the converted graphs only if you accept that the weights' rights are implied by the Apache-2.0 repo rather than stated.
- Bundle the attribution list above.
- Before release, close the per-`.so` LiteRT native review (Android no-LGPL rule).
- Reproduce the archives through WSL2 Ubuntu and bind the new hashes.

---

## Packet 2: B3 Windows installer, WiX Toolset 3.14.1

### What would be admitted
- **A host-only build tool:** WiX 3.14.1 (build **3.14.1.8722**), specifically `candle.exe` and `light.exe` plus the rest of its inventory. jpackage 21.0.12.1 drives it through Candle/Light, and the WiX v4+ `wix.exe` does not work for this profile (MSI_INPUTS.md).
- **Official artifact:** https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip, **41,297,555 bytes**, published 2024-03-23. The release page is https://github.com/wixtoolset/wix3/releases/tag/wix3141rtm.
- **No published SHA-256.** The release page lists no hashes, and the GitHub API `digest` field is `null` for every asset. You will have to record your own hash at download time.

**Local copies (observed read-only; neither has been admitted):**
| Path | What it is |
|---|---|
| `C:\Users\redacted\.gradle\compose-jb\wix311.zip` | **WiX 3.11.2, not 3.14.1.** `candle.exe` and `wix.dll` FileVersion 3.11.2.4516. 34,670,186 B, SHA-256 `2C1888D5D1DBA377FC7FA14444CF556963747FF9A0A289A3599CF09DA03B9E2E`, 220 entries. Do not use it for this profile. |
| `C:\Users\redacted\AppData\Local\tauri\WixTools314\` | **WiX 3.14.1.8722**, an extracted directory of 68 files including WixUIExtension, WixUtilExtension and others. `candle.exe` SHA-256 `A464676E2A2191D58E6CA643E89505BFB9628D3E721873A14994D3F4667987E5`; `light.exe` `8EE3D7FA5592511152A4D1FC605780EAE1B27AC7B41D75D518C958515DE39529`; `wix.dll` `A2C897A03F9F4795426EE058EA386A6891ACB9A8179B77060515A6BA6525E242`. The Tauri CLI downloaded it and its provenance is unverified. Compare it file by file with the official zip before reusing it. |

Both copies contain the same `LICENSE.TXT` (SHA `4004370957CF…0000`), which states MS-RL.

### License
- **Microsoft Reciprocal License (MS-RL):** https://opensource.org/license/ms-rl-html. FireGiant's page, cited by the repo: https://docs.firegiant.com/wix3/main/license/.
- **What MS-RL requires.** Reciprocity is per file. Section 3(A): if you *distribute* a file that contains WiX code, in source or binary form, you must provide the source code for that file and include the MS-RL text. Section 3(D): keep the copyright and attribution notices. Files that contain no WiX code can be under any license.
- **What gets embedded in this MSI.** I checked the JDK 21 sources (`WinMsiBundler`, `WixUiFragmentBuilder`):
  - jpackage adds `-ext WixUIExtension`, and with it the WixUI dialogs and bitmaps, **only** when `--win-dir-chooser`, `--license-file` or `--win-shortcut-prompt` is passed.
  - `tools/release/msi.py` passes only `--win-per-user-install --win-menu`, so **no WixUI resources are embedded**.
  - `WixUtilExtension` is not added.
  - jpackage **always** embeds its own custom-action DLL, `wixhelper.dll`. That DLL comes from the JDK (Oracle, GPL-2.0 WITH Classpath-exception-2.0), not from WiX. Whether it statically links any WiX SDK library is unverified (I believe it does not).
- **Effect on a personal unsigned MSI.** Nothing is distributed, so MS-RL imposes no obligations. Even if it were distributed, no WiX-licensed file appears to be embedded, so the reciprocity clause would not be triggered. If any WixUI flag is ever added, ship the MS-RL text and offer the source of the embedded WixUI files.

### Risks / unknowns
- **WiX v3 is out of community support.** https://docs.firegiant.com/wix/wix3/ says it has been out of community service since **2025-02-06**: "no additional fixes, even for security issues". This tool runs locally on trusted inputs, so the risk is low. The known 3.x CVEs concern Burn bundles (from memory, unverified), and jpackage MSIs do not use Burn.
- **`wixhelper.dll` and the project's JDK exception.** The documented exception covers "the unmodified Java runtime bundled by jpackage". `wixhelper.dll` is a jpackage installer component rather than part of the runtime, so you may want to widen the exception wording explicitly.
- **MSI contents are not inspected yet.** Inspect the MSI's `Binary` and `CustomAction` tables after the first build to confirm only `wixhelper` is present.
- WiX 3.x needs the .NET Framework at run time. Which version is required is unverified.

**Recommendation: approve with conditions.**
- Admit only WiX **3.14.1.8722** as a host-only tool.
- Take it from the official `wix314-binaries.zip`, or from the Tauri copy once it matches that zip, and record your own SHA-256.
- Never use the 3.11 cache.
- Re-review if any jpackage UI or license flag is added.

---

## Packet 3: B3 Android tunnel runtime (TUNNEL-002/005, SEM-007, CAPTURE-008)

### What would be admitted
- **Release:** scrcpy **v4.1**, source commit `2926c06c5dc3064ae6d8db706f1a98a37cfcf3f0`.
- **Archive:** `scrcpy-win64-v4.1.zip`, **11,305,298 B**, SHA-256 `5b12172b3264b2889f4583ee64752ce832e29bc8b1089dca81093459697165db`. This matches the hash pinned in upstream `doc/windows.md`.
- **Repo pins:** `tools/android-tunnel/pinned-runtime-inventory.json`. In `android_tunnel_root.rs`, the `ApprovedCatalog` requires `scrcpy.exe`, `scrcpy-server` and exactly these six `DLL_NAMES`, in order:
  **`SDL3.dll`, `avcodec-62.dll`, `avformat-62.dll`, `avutil-60.dll`, `libusb-1.0.dll`, `swresample-6.dll`**.
  `COMPILED_APPROVED_CATALOG` is currently `None`, so the tunnel is closed.
- **Correction to the brief:** v4.1 bundles **SDL3** (3.4.12), not SDL2.

| File | Bytes | SHA-256 | Component / license |
|---|---:|---|---|
| scrcpy.exe | 720,524 | `575ca1284345c7b3975585bc61b66d564a9a4f1ecb28fbb4c599c92a124054a9` | scrcpy 4.1, Apache-2.0 |
| scrcpy-server | 733,706 | `deacb991ed2509715160ffdc7907e47b4160eb30d1566217e9047fd5b8850cae` | scrcpy 4.1, Apache-2.0. Pushed to the phone at run time and runs there via app_process. It is *not* inside the APK. |
| avcodec-62.dll | 8,603,648 | `7179de2b132e78eb0a76458a0a3859dfe1edcbb6d2eeb4a456f03f7ae96d5b66` | FFmpeg 8.1.2, LGPL-2.1-or-later (claimed). Includes dav1d 1.5.3, statically linked, BSD-2-Clause. |
| avformat-62.dll | 723,456 | `7232316acce00371d89f589748b570d95885ea6bbfc1972a0a9d3b884903eee1` | FFmpeg 8.1.2, LGPL-2.1+ (claimed) |
| avutil-60.dll | 1,039,360 | `3d6170dd68549c6f39b8d8710a37f79d9678905df705a8b0a6bc7ea9037daddf` | FFmpeg 8.1.2, LGPL-2.1+ (claimed) |
| swresample-6.dll | 139,776 | `4cc809d2cd822e186906fbc9d8a0acffa937e35de1282b2e2ab7346cfed96fed` | FFmpeg 8.1.2, LGPL-2.1+ (claimed) |
| libusb-1.0.dll | 227,005 | `8ec130918a476b0dbd114c803e71314360608ceabdd2b6f38c83f6f208c608e0` | libusb 1.0.30, LGPL-2.1-or-later |
| SDL3.dll | 5,366,478 | `0619eb2da6032984dc6e2098897aeacdbd66b0415bb87bc03e628159ba60b15d` | SDL 3.4.12, zlib |
| LICENSE.txt | 11,387 | `01c12035bf35af37241298dc7ad538eb2a07e5c940437bc6876feeaa9d1951d0` | scrcpy Apache-2.0 text (matches the tagged source) |
| scrcpy.png / disconnected.png | 6,530 / 4,703 | see inventory | scrcpy resources |

**Not bundled:** `adb.exe`, `AdbWinApi.dll`, `AdbWinUsbApi.dll`, `open_a_terminal_here.bat` and `scrcpy-noconsole.vbs` are hashed but never extracted. The runtime uses only an owner-selected `platform-tools\adb.exe` and refuses the ZIP's adb digest. adb is therefore outside this packet.

### Licenses and build configuration (with sources)
- **scrcpy:** Apache-2.0. https://github.com/Genymobile/scrcpy/blob/v4.1/LICENSE
- **FFmpeg build flags.** I fetched upstream `app/deps/ffmpeg.sh` at v4.1 (https://github.com/Genymobile/scrcpy/blob/v4.1/app/deps/ffmpeg.sh):
  - FFmpeg **8.1.2**, built with `--disable-everything`, a small set of decoders (h264, hevc, av1, vp8, vp9, libdav1d, opus, aac, flac, pcm, png), `--enable-libdav1d`, `--enable-zlib`, and for win64: `--enable-shared --disable-static`.
  - **`--enable-gpl`, `--enable-version3` and `--enable-nonfree` appear nowhere in the script.**
  - Per FFmpeg's LICENSE.md (https://github.com/FFmpeg/FFmpeg/blob/n8.1.2/LICENSE.md), that recipe yields **LGPL-2.1-or-later**, not GPL or LGPLv3.
  - Statically linked dav1d is BSD-2; zlib is under the zlib license.
- **SDL 3.4.12:** zlib. https://github.com/libsdl-org/SDL/blob/f87239e71e42da91ca317a12eefb82cfbf3393eb/LICENSE.txt
- **libusb 1.0.30:** LGPL-2.1-or-later. https://github.com/libusb/libusb/blob/master/COPYING
- **Recipe vs. actual DLLs (unverified).** The recipe is not proof of what the shipped DLLs contain. `avcodec_license()`, `avutil_license()` and the `*_configuration()` strings of these exact hashed DLLs have **not** been queried. `tools/android-tunnel/dll_license_probe.py` would do that, but it was never run, and running it loads the DLLs. It requires exactly "LGPL version 2.1 or later" and the flag set above, and refuses anything GPL, nonfree or unknown.
- **A lower-risk alternative:** search the DLL bytes read-only for the embedded configuration string and for `--enable-gpl`.

### Obligations
- **Personal build, not distributed:** LGPL, Apache and zlib impose nothing until you convey copies.
- **Project policy (already met):**
  - The LGPL DLLs are dynamically loaded by `scrcpy.exe`, a separate process from the app; nothing is linked into Visual Workbench.
  - Nothing LGPL goes to Android. `scrcpy-server` is Apache Java.
  - Notices are prepared at `docs/license-proposals/notices/scrcpy/` but not yet registered in `third_party/LICENSES`. They cover:
    - scrcpy Apache-2.0
    - FFmpeg 8.1.2 LGPL-2.1 and LICENSE.md
    - libusb 1.0.30 LGPL-2.1
    - SDL 3.4.12 zlib
    - dav1d 1.5.3 BSD-2
- **If ever distributed:**
  - Include the LGPL text.
  - Provide or offer the exact FFmpeg 8.1.2 and libusb 1.0.30 source, plus the build script.
  - Allow users to replace the DLLs.
- **Replaceability vs. the hash pin.** `ApprovedCatalog` hash-pins every DLL and refuses changed bytes, which conflicts with the "replaceable" intent of LGPL-2.1 section 6. That is acceptable for personal use, because you control the catalog and can re-pin. Distribution would need a documented re-pin/replace route.

### Risks / unknowns
- The actual DLL license and configuration strings are unverified (above).
- SDL3's internally bundled sub-components (for example hidapi) are unreviewed.
- Static MinGW runtime pieces (libgcc and winpthreads, normally under GCC runtime exception / MIT-style terms) and PE imports are unchecked.
- libusb build provenance is unverified.
- **No conflict with the no-GPL policy has been found.** The only way one appears is if the probe reports GPL, in which case refuse.

**Recommendation: approve with conditions.**
- Admit scrcpy v4.1 exactly as hashed above, with adb excluded.
- Before enabling `COMPILED_APPROVED_CATALOG`, require a passing LGPL-2.1 check on the four FFmpeg DLLs (the probe or a read-only string scan) with no `--enable-gpl`/`nonfree`.
- Register the six notice texts in `third_party/LICENSES`.
