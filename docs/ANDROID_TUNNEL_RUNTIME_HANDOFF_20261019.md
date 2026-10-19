# Pinned scrcpy 4.1 runtime prerequisite (source and byte evidence only)

This handoff prepares T5.03's Windows scrcpy input without enabling the tunnel,
shipping adb, changing root runtime manifests, or admitting the DLL licenses.
It is based on `cloud/roadmap-source-integration-20261018` at
`6efc8b8cba9daa4e4ff598e6419c4db41deb6a8b` and the F grant in
`INDEPENDENT_RUNTIME_GRANTS_20261019.md`.

## Verified release and exact census

[Genymobile's v4.1 release](https://github.com/Genymobile/scrcpy/releases/tag/v4.1)
exists. Its tagged [Windows instructions](https://github.com/Genymobile/scrcpy/blob/v4.1/doc/windows.md)
pin `scrcpy-win64-v4.1.zip` to SHA-256
`5b12172b3264b2889f4583ee64752ce832e29bc8b1089dca81093459697165db`.
The tagged [build instructions](https://github.com/Genymobile/scrcpy/blob/v4.1/doc/build.md)
pin `scrcpy-server-v4.1` to SHA-256
`deacb991ed2509715160ffdc7907e47b4160eb30d1566217e9047fd5b8850cae`.
The Git tag resolves to source commit `2926c06c5dc3064ae6d8db706f1a98a37cfcf3f0`.

The tool downloaded the official ZIP to a disposable owned directory outside
Git. Its observed size was **11,305,298 bytes** and its SHA-256 matched the
official pin. It read all 16 archive files, verified the embedded server hash,
recorded every file's size and SHA-256, and checked all seven retained PE images
as PE32+ AMD64. The 11 retained files total **17,576,573 bytes**. Five other
archive files were hashed but excluded: `adb.exe`, `AdbWinApi.dll`,
`AdbWinUsbApi.dll`, `open_a_terminal_here.bat`, and `scrcpy-noconsole.vbs`.
The checked [inventory](../tools/android-tunnel/pinned-runtime-inventory.json)
contains the exact per-file hashes, including FFmpeg, libusb and SDL DLLs, the
client, server, images, bundled license text, and excluded adb files. The
bundled `LICENSE.txt` SHA-256 exactly matches the tagged scrcpy source license.

The prepared bytes currently reside at
`/tmp/vw-scrcpy-v4.1-prepared-20261019`. No executable, server, adb command,
device, or DLL was launched or loaded. No binary or downloaded archive was
committed. The exact text-only
[root manifest proposal](../tools/android-tunnel/root-manifest-proposal.json)
gives the 11 `third_party/scrcpy/` paths, their observed hashes/sizes, and six
source-notice paths. These are **proposed entries, not root manifest edits or
license approvals**.

## Reproducible preparation and verification

`tools/android-tunnel/scrcpy_runtime.py` uses only the Python standard library.
It accepts only the pinned official release URL and full archive hash/size;
checks an exact member set and member sizes; refuses traversal, symlinks,
unknown/duplicate/case-colliding names, encrypted/unsupported ZIP entries,
truncation, changed bytes, wrong PE architecture, and a wrong embedded server.
The output directory must be new, absolute and outside Git. Preparation creates
an owned staging directory, verifies it, then creates the destination without
replacing any existing file. An interrupted or altered destination fails
`verify`. The download has a 16 MiB byte ceiling, a 30-second socket timeout
and a 180-second transfer budget. It follows HTTPS redirects only to GitHub's
release-asset host. It never extracts the bundled adb or wrapper scripts.

```text
python3 tools/android-tunnel/scrcpy_runtime.py download --destination /tmp/new-scrcpy-v4.1
python3 tools/android-tunnel/scrcpy_runtime.py verify --destination /tmp/new-scrcpy-v4.1
python3 tools/android-tunnel/scrcpy_runtime.py prepare --archive /tmp/scrcpy-win64-v4.1.zip --destination /tmp/new-scrcpy-v4.1
```

`prepare` and `download` return `vw.scrcpy-prepared.v1`: `version`, the
URL/size/SHA-256 of the archive, official server SHA-256, a sorted `files`
array with `name`, `bytes`, `sha256`, `role`, and `architecture`, a sorted
`excluded` array with hash and disposition, and an explicit pending license
state. The checked-in inventory pins the exact observed file hashes so
`verify` can rehash a prepared directory even after the temporary ZIP has
been deleted. Supplying `--archive` additionally rechecks the full source ZIP.
The tool does not execute downloaded code.

Synthetic ZIP tests cover valid preparation, absence of delivered adb,
changed archive/prepared bytes, missing or unexpected files, traversal,
duplicates, symlink entries, wrong server/PE, existing destination, and an
incomplete or altered output directory. They do not run a device or DLL.

## License and build provenance boundary

The [scrcpy source](https://github.com/Genymobile/scrcpy/blob/v4.1/LICENSE)
is Apache-2.0. This does **not** license all bundled DLLs. The tagged
[Windows release recipe](https://github.com/Genymobile/scrcpy/blob/v4.1/release/build_windows.sh)
names FFmpeg, libusb, SDL and dav1d dependency builds, copies their generated
DLLs, and also copies adb. The tagged FFmpeg recipe selects **8.1.2** with
shared Windows libraries and dav1d **1.5.3** statically; libusb is **1.0.30**
and SDL is **3.4.12**. These are source/release-recipe claims. They do not by
themselves prove the configuration or license of the **observed hashed DLLs**.

Public tagged-source license texts and a byte-hash/source table are under
[`third_party/notices/scrcpy`](../third_party/notices/scrcpy/SOURCES.json):
scrcpy Apache-2.0; FFmpeg's LGPL-2.1 text plus its license/external-library
summary; libusb LGPL-2.1; SDL's zlib text; and dav1d's BSD-2-Clause text.
[FFmpeg's tagged license summary](https://github.com/FFmpeg/FFmpeg/blob/n8.1.2/LICENSE.md)
explicitly says optional GPL or external-library options can change the
aggregate terms. The published ZIP contains scrcpy's `LICENSE.txt`, but no
individual FFmpeg/libusb/SDL/dav1d notice files. Their source notices have
therefore been retained separately, without treating them as proof of the DLLs.

The Windows LGPL exception in `docs/BUILD_SPECIFICATION.md` requires the
actual FFmpeg/libusb DLLs to remain dynamically linked and replaceable. Before
root admits or distributes this runtime, a separately authorized Windows
diagnostic must rehash these same DLLs and query `avcodec_license()`,
`avformat_license()`, `avutil_license()`, `swresample_license()` and their
`*_configuration()` strings. GPL/nonfree/unknown responses must refuse. The
authored `tools/android-tunnel/dll_license_probe.py` is an explicit future
Windows-only diagnostic and **was not run**. libusb has no equivalent FFmpeg
license query; its actual DLL build provenance, PE imports/dependencies,
aggregate obligations, dynamic replaceability and required notices still need
review. The same actual-byte/notice check applies to SDL and statically
incorporated dav1d/zlib components. A source recipe, file name or SHA alone
does not close this policy gate.

Root still owns `third_party/LICENSES`, packaging/runtime manifests, owner-adb
selection, process lifetime, lossless screencap/uiautomator ownership and
feedback-loop refusal. T5.03 tunnel/runtime execution and any device result
remain unimplemented or unrun; Krita and physical acceptance remain deferred.
