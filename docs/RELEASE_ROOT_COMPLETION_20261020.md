# Release source completion and remaining cloud scope — 2026-10-20

Historical bounded-grant receipt. The later overnight grant activates
[MSI source closeout](MSI_RELEASE_CLOSEOUT_20261020.md); its implemented producer
supersedes the missing-hook/source-work statements below. Tool/runtime/license
and actual installer acceptance still require their own evidence.

Base is published main `6a263dd5b330a3769203bba6cd17156c47f114f0`.
The accepted Desktop18158c7 renderer and its root integration are unchanged.
The previously reviewed release branch `2a70c7c04ddd5a67a250eb891dc44501ea78097f`
is reconciled by exact cherry-picks `4841faade7eed53118b04e94324d124fca7870df`
and `558aa0307419ca86bf5a9a7de837abfdd41c60ba`. Its original branch and
historical receipt remain preserved; the old Desktop partition is marked as
superseded by the actual integrated receiver.

## Completed independent cloud work

The explicit `build.ps1 assemble-release` entry forwards five supplied,
hash-bound manifest/Python/destination inputs to the existing bounded Windows
wrapper. It requires PowerShell7, refuses missing or blank inputs and release
flags on other commands, and bounds execution to1–3600seconds. It discovers,
downloads, builds, signs and installs nothing. Existing source, inventory,
notice and input validators remain authoritative; `release_ready` remains false.

Reconciliation found an actual packaging mismatch: Desktop Gradle already
required both Java2D exports and `compose.interop.blending=true`, but the
launcher validator admitted only the older three properties. It now requires
all six exact options. The two exports have distinct module/package identities;
repeated exports, property overrides, unknown JVM switches and external resource
paths remain refused. Six focused methods exercise the actual parser, including
every omitted required option and conflicts. The complete synthetic app-image
fixture now represents the current configuration. No application or Gradle
source changed in this partition.

[Back up, update, recover and remove](UNINSTALL_RECOVERY.md) records current
project locations, complete editable backup boundaries, safe development app
updates/removal and recovery checks. Quick Start and the release manifest
example link it. Its existence is source documentation, not installation,
restore, Android private-storage access or A20 acceptance.

| Fresh check | Result and scope |
| --- | --- |
| `python3 -m unittest discover -s tools/desktop-test -p test_reports.py -v` |39 passed; exact launcher parser and synthetic distribution/report validation |
| `python3 -m unittest discover -s tools/desktop-test -p test_personal_package.py -v` |16 passed; synthetic personal package validation |
| `python3 -m unittest discover -s tools/release -p test_assemble.py -v` |41 passed; synthetic assembler and reused validators |
| `python3 tools/check_setup.py` |IDs, third-party and workspace policy passed |

The three test commands set `PYTHONDONTWRITEBYTECODE=1`. These96 cases are
separate Python test methods, not actual Windows packaging or new application
runtime tests. No heavy build, binding regeneration or repeated historical
native/Desktop batch was needed. Independent Astra read-only review reports no
actionable findings in the hook, parser/test changes, guide or MSI handoff.
The [machine-readable receipt](evidence/RELEASE_ROOT_COMPLETION_20261020.json)
binds final sources and the fresh raw log hashes.

## Exact remaining work and admission inputs

| Remaining category | What is still needed |
| --- | --- |
| Windows root/wrapper and package runtime | PowerShell7 is absent here. Parsing/execution, Windows NTFS/reparse/sharing/process cleanup, actual app image and launch remain unrun. MSVC `lib.exe` remains unavailable. Linux tests do not establish Windows behavior. |
| MSI implementation and package admission | No MSI build hook exists. Review the exact Windows JDK/jpackage and compatible WiX versions, hashes, license/notice inventory and task graph. Supply a finalized app-image receipt. `package-desktop-app` currently creates the image, binds/copies the final MCP bridge, then records it; an installer must consume that final image. A fresh output receipt must bind it and admitted producer inputs to actual MSI bytes. No guessed WiX pin, download or disabled skeleton replaces this work. |
| Release content and reproducibility | Actual APK/MSI and applicable optional packages, complete native/runtime resource closure, final producer toolchain record, checksums, notices/licenses and reproducibility are not established. The assembler can copy explicit already-built MSI bytes but does not prove MSI structure, installability or signatures. Signing/deployment require their own authority. |
| Model/decoder/tunnel admission | E source110d4ec remains accepted, but its artifact transfer failed CONNECT403 before bytes; checkpoint/weights/TinyViT/native aggregate/Android memory/runtime/catalog remain closed. PDFium/qpdf/libvips, OS codecs/color and giant-image fidelity/security/performance require admitted runtimes and actual journeys. Configured scrcpy DLL licenses/native aggregate/replaceability/notices and compiled catalog remain open. No alternate blocked-artifact route or tool execution occurred. |
| Real platform/provider journeys | Actual Windows/GPU Swing/Compose/D3D presentation, packaged launcher, stream/freeze/lossless/Save/retry/resume/source loss, remote Paint controls/input retirement, format/security, diagnostics redaction/export, recapture/alignment/A30 and provider quality/billing are unverified. Live provider calls remain outside this grant. |
| Device and owner acceptance | G0–G5 and release A01–A30, S23 physical S Pen, Bluetooth/Air Actions/onboarding, frame rate, latency, memory/thermal, carrier/offline/phone-AI journeys, install/uninstall/restore A20, compatibility and daily-use acceptance need their actual device and owner evidence. Compilation and fixture results do not pass them. |
| Deferred scope and decisions | Krita and broader Phase4 text refinement, virtual monitor/driver, Photoshop/UXP, editor/plugin/palette work and full G4 remain deferred. INSTR-005 still needs a dated next-release plan or owner-approved drop; neither an owner decision nor a release date is invented here. |

No further independently executable implementation item was identified after
reconciling the currently authorized cloud roadmap. This closes this bounded
source partition only. It does not mark a whole phase complete, admit a runtime,
waive a gate, erase the remaining MSI implementation or authorize external work.
The full phase breakdown remains in
[Remaining roadmap phases](ROADMAP_REMAINING_PHASES_20261020.md).
