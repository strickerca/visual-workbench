# Visual Workbench — Decision Log

Revision 2026-10-01. Each record: context, decision, alternatives considered, consequences, confidence, status. "Provisional" means a Phase 0 spike can overturn it; the spike task is named. To change a decision, add a new dated entry below the old one rather than editing history.

---

## D1 — One Rust core shared by both apps
- **Context:** v1 warned against "two incompatible definitions of coordinates, transactions, or revision identity" but proposed no mechanism. Separately written Kotlin and .NET apps would drift.
- **Decision:** a single Rust workspace (`vw-core`) owns coordinates, the document model, ops/transactions, revision identity, stroke geometry, hit testing, masks, export rasterization, protocol, transport, pairing and storage. Apps feed it input and draw what it returns.
- **Alternatives:** Kotlin Multiplatform for all logic (weaker for codecs, PDFium, resvg and Windows APIs); duplicated implementations with shared tests (drift risk remains).
- **Consequences:** one source of truth and identical exports from either device. The cost is an FFI boundary (UniFFI) and Rust build tooling on the laptop. Proven pattern: Firefox, 1Password, Signal and Bitwarden ship Rust cores to Android and Windows.
- **Confidence:** 90% · **Status:** accepted.

## D2 — Kotlin Multiplatform + Compose for both UIs; Windows services in Rust
- **Context:** v1 defaulted to a .NET Windows app. With D1, a .NET UI adds a third language and needs `uniffi-bindgen-cs`, which supports UniFFI 0.31 while UniFFI is at 0.32.2. The owner wants an iPad app later.
- **Decision:** Android app in Jetpack Compose; Windows app in Compose Multiplatform desktop (JVM). Shared view models, state machines, the command dispatcher and the design system live in a KMP module whose common code avoids Android-only APIs. Windows platform work (capture, encode, injection, UI Automation, virtual display, clipboard, hotkeys) lives in Rust (`vw-host-win`).
- **Alternatives:** .NET 10 + WinUI 3 (Windows App SDK 2.5) with the core pinned to UniFFI 0.31; Avalonia 12; Tauri with a web UI; a Rust-native UI (Slint, egui).
- **Consequences:** two languages instead of three; one binding generator; most of the code builds on Linux CI; an iOS target stays reachable. Costs: Compose desktop has no pen pressure on Windows (issue CMP-1609), which only matters for drawing on the laptop itself (fixable later with a Rust pointer hook); the JVM adds about 200–300 MB of RAM; packaging uses jpackage (MSI/EXE), and the bundled Java runtime is a documented license exception (D17).
- **Confidence:** 80% · **Status:** accepted.

## D3 — Host-authoritative operation log, offline phone rebase
- **Context:** two devices, one user, the PC as master; the owner wants the phone usable alone.
- **Decision:** Figma-style host sequencing with property-level last-writer-wins. Op ID = (device, Lamport counter); transaction ID = UUIDv7 (idempotent retries); revision = host sequence + state hash. Gesture previews ride an ephemeral channel and never enter history. Offline phone transactions queue with their base revision and rebase on reconnect; same-property conflicts resolve by later wall-clock write and land in a conflict record (written from Phase 1; the review UI arrives in Phase 3). This is the one place wall clocks are compared across devices.
- **Alternatives:** CRDT (Loro 1.16, MIT) for fully symmetric editing; Automerge 3; Yrs.
- **Consequences:** linear, human-readable revisions; simple validation; undo per device. The offline rebase and conflict review must be written and simulation-tested (`vw-sim`). Switch to Loro only if symmetric multi-master editing becomes a requirement.
- **Confidence:** 85% · **Status:** accepted.

## D4 — Coordinate spaces
- **Decision:** document space = oriented original source pixels (or PDF points per page, or SVG user units), f64, origin top-left, y down. Views are similarity transforms; capture frames carry geometry that maps to Windows virtual-screen physical pixels; the precision lens has its own latched transform. Full definitions in BUILD_SPECIFICATION §4.2.
- **Consequences:** every component shares one module (`vw-geom`) with property-tested round trips; 4K/high-DPI correctness depends on physical-pixel discipline in Windows code.
- **Confidence:** 95% · **Status:** accepted.

## D5 — Project storage
- **Decision:** a project folder with SQLite in WAL mode (op log, snapshots, current state, metadata) and content-addressed blobs (BLAKE3) for originals and AI outputs; derived pyramids in a deletable cache.
- **Alternatives:** zip of JSON files (Sketch-style), a single SQLite file holding blobs (bloats, slow), a CRDT document store.
- **Consequences:** crash-safe writes, easy inspection, cheap dedup. Backups export as a single `.vwbz` file.
- **Confidence:** 90% · **Status:** accepted.

## D6 — Stroke representation and engine
- **Context:** strokes must look identical on both devices and in exports. Jetpack Ink 1.0.0 is stable and fast on Android, but its JVM native loader ships a Linux x86-64 library only (1.1.0-alpha09 adds macOS arm64); neither version ships a Windows library.
- **Decision (provisional):** the canonical stroke is raw samples plus a versioned brush spec; geometry is generated by `vw-ink` in the core and used for wet ink, dry ink and exports on both devices. Wet ink renders through androidx.graphics front-buffered rendering with androidx.input motion prediction.
- **Spike (T0.10):** compare the core engine against Jetpack Ink's `InProgressStrokes` on the S23 Ultra for latency and feel. "Clearly better" means Christian prefers Ink in the blind A/B for at least 2 of the 3 stroke types (handwriting, circles, fast lines) AND Ink's p95 wet-ink latency is not worse than `vw-ink`'s. Adopt Ink only if it is clearly better AND google/ink's C++ core builds for Windows within the T0.10 time box (4 hours of agent effort). If Ink is clearly better but the 4-hour Windows build fails, ask Christian whether to spend up to 2 more days on it; otherwise keep `vw-ink`.
- **Consequences:** a deterministic stroke algorithm to write and golden-test; no "pop" between wet and dry ink (acceptance ≤ 0.5 px difference).
- **Confidence:** 70% · **Status:** provisional (T0.10).

### D6 software checkpoint — 2026-10-02

Continue the Rust algorithm-1 candidate for software integration under the owner's
full-continuation instruction. Sixteen synthetic geometry goldens are identical
on Windows x64 and the physical OnePlus IN2019 (Android 11/API 30); 21 engine tests
and one JNI test pass on each. Three real Android comparison tests pass, including
32,768 identical wet/dry pixels in memory. Native append CPU p95 is 5.300 us on
Windows and 1.354 us on IN2019, excluding JNI/rendering/display. Callback timing
from the two authoring paths measures different stages and is not comparable
contact-to-present latency. Android 11 pre-layout and reattachment failures were
fixed and retained in the evidence.

No owner handwriting/circle/fast-line preference has been recorded, and no S23
presentation-latency comparison or owner-recorded trace parity was performed.
Google Ink Windows feasibility attempts are recorded in `docs/evidence/T0.10.md`.
The original two-of-three preference/latency/build rule is unchanged. D6 remains
provisional; software integration, a Windows core build, or IN2019 timing alone
cannot select either engine or pass PERF-001. See `docs/evidence/T1.04.md`.

## D7 — Transport
- **Context:** adb forward/reverse carries TCP only (no UDP), so QUIC can't run over adb; USB tethering gives a real IP link but may take over the PC's internet route; the default path must not require developer mode (possible future product).
- **Decision (provisional):** one framed protobuf protocol over two carriers: QUIC (quinn: reliable streams plus datagrams) on Wi-Fi or USB tethering, and a TCP multiplexer over adb as a developer/power option. Default order: USB tethering → Wi-Fi → adb. Route safety check on the PC.
- **Spike (T0.06):** measure RTT, jitter and throughput for tethered QUIC, Wi-Fi QUIC and adb TCP on the real devices; reorder defaults if the numbers say so.
- **Confidence:** 85% · **Status:** provisional (T0.06).
- **2026-10-02 measurement checkpoint:** S23 Ultra SM-S918U / Android 16, adb TCP: 1,000 echoes per size; 64 B p50/p95/p99 9.703/40.273/64.194 ms, 4 KiB 8.016/32.896/74.416 ms, 1 MiB 70.394/133.265/222.764 ms; 256 MiB verified upload 32.41 MiB/s. No USB-tether interface was present; phone Wi-Fi was disabled. Physical recovery and Windows inbound firewall checks remain open. Preserve USB tethering → Wi-Fi → adb provisionally; one measured carrier cannot justify reordering. Echo RTT does not establish PERF-002. See docs/evidence/T0.06.md.

## D8 — Pairing and trust
- **Decision:** QR code with the PC certificate fingerprint and a 128-bit one-time secret; TLS 1.3 with mutual certificate pinning; secret proven by HMAC over the TLS exporter; revocable trust list kept at app level (never in projects); mDNS advertises only a random ID regenerated at each start. The 8-digit code fallback runs through a PAKE (SPAKE2 or CPace, bound to the TLS exporter; T1.06b picks a stable, permissively licensed crate and records why); codes are single-use, expire after 5 minutes, and pairing locks for 10 minutes after 5 failures.
- **Alternatives:** an HMAC over the short code (rejected: a spoofed PC could brute-force 10^8 codes offline); OPAQUE (no password database here, so unnecessary); Noise protocol (snow, unaudited).
- **Confidence:** 90% · **Status:** accepted.

## D9 — Getting PC pixels to the phone
- **Context:** the Snapdragon 8 Gen 2 exposes no hardware 4:4:4 HEVC/AV1 decode, so plain video smears UI text; markup needs exact frames; remote editing needs low latency.
- **Decision:** two paths. (a) Live annotate uses a frame stream: JPEG tiles over dirty regions at 10–30 fps, each frame individually addressable by ID. (b) Remote edit uses Quick Sync HEVC low-latency video, with optional lossless dirty tiles for static text. Both are backed by a bounded lossless ring buffer on the PC so a freeze anchors to the exact frame in lossless quality.
- **Spike (T0.07):** Quick Sync HEVC encode latency on the Iris Xe and MediaCodec decode latency on the phone.
- **Consequences:** Phase 3 needs no video codec; Phase 4 owns the HEVC pipeline. No GPL streaming code (D17), so the pipeline is written from OS APIs.
- **Confidence:** 75% · **Status:** provisional (T0.07).

### D9 measurement update — 2026-10-02 (T0.07)

Retain the two-path design with implementation conditions, not product performance
acceptance. Native Intel hardware HEVC at 24 Mbps, no-B/low-latency controls and
one frame outstanding produced portrait/4K submit-to-output p95 of 43.576/40.910 ms.
S23 hardware decode p95 was 8.831/13.039 ms. Capture interval tails and an observed
IntelControlLib.dll_unloaded crash leave long-running host stability and 30 fps
unproven. Separate diagnostic workers passed; a future streaming worker needs
explicit restart/recovery validation. No driver or security setting was changed.

WGC DirtyRegions reported the entire GDI source. Add measured pixel difference
when region hints are coarse; do not assume they represent actual changed area.
Tight derived regions matched 10%/25%, with JPEG encode p95 20.470/41.764 ms.
Recorded tiles reached 17.872/18.695 fps over adb including phone decode/posting,
but concurrent capture/encode is excluded. Live annotate still needs its complete
STREAM-001 test, frame identity/geometry and an optimized readback/encoding path.

For this generated 4K UI, PNG fast/Sub p95 11.947 ms was faster than QOI 25.494 ms;
QOI was smaller (226,524 versus 411,337 bytes). Keep lossless freeze support and
prefer measured PNG for this workload; other content and full freeze replacement
remain unverified. The S23 advertises the QTI picture-order parameter but neither
FEATURE_LowLatency nor the QTI low-latency parameter. Record requests separately
from supported/applied behavior and use explicit monotonic Surface timestamps.

See docs/evidence/T0.07.md and T0.07-results.json. STREAM/PERF statuses and targets
remain unchanged. D9's architecture is retained with these measured conditions;
integrated performance and the G0 owner decision remain open.

## D10 — Pen input into Windows apps
- **Decision:** `InjectSyntheticPointerInput` with PT_PEN: pressure, tilt, barrel, eraser; hover as in-range without distance; ≥ 20 Hz keepalive while in contact; per-batch guards on foreground window, geometry, DPI, input session and integrity level.
- **Known limits:** no hover distance, one barrel button, contacts cancel after about 1 s without refresh; injection cannot reach higher-integrity (admin) windows; WinTab-only apps get no pressure.
- **Spike (T0.05):** verify pressure in a test harness window, Paint, Krita, GIMP and Photopea in a browser (Photoshop once installed).
- **Confidence:** 80% · **Status:** provisional (T0.05).

### D10 measurement update — 2026-10-02 (T0.05)

Retain guarded PT_PEN injection as the provisional approach. On this host at
168 DPI (175%), the normal harness run submitted and recorded 472 samples;
all 65 pressure-ramp pairs matched (Pearson r = 1.000). Tilt on both axes,
rotation and barrel fields matched; the maximum contact interval was 26.480 ms.
The tested combined INVERTED | ERASER value (6) did not pass through: 66 matched
samples had different pen flags, with three unmatched lifecycle samples and one
pressure mismatch near that transition. Do not promise eraser compatibility yet.

Revise the earlier approximate one-second timeout assumption for this host:
with refresh withheld for 1500 ms, Windows emitted an automatic UP at 500 ms
after DOWN, without POINTER_FLAG_CANCELED, and the next update began a new
contact. Keep the >=20 Hz policy; the probe schedules 50 Hz and stops a normal
run when a contact gap exceeds 50 ms. This is measured behavior, not a universal
Windows timeout guarantee.

Editor compatibility and elevated-window behavior remain unmeasured. The native
guard experiment observes only its two owned harness windows and cannot prove
absence of input to every other desktop window. D10 remains provisional and G0
remains open. See [T0.05 evidence](evidence/T0.05.md) and the
[compatibility matrix](compat/injection-smoke.md) for source bindings, guard
results, retained failures and untested clauses. No TUNNEL requirement is passed.

## D11 — Virtual monitor
- **Decision (provisional):** fork SudoVDA, build it from source with the WDK from NuGet, sign it with a project-only self-signed code-signing certificate, and install the certificate into LocalMachine Root and TrustedPublisher. After signing, the private key moves offline (a password-protected `.pfx` on removable media) and is deleted from the certificate store. Control by IOCTL with a watchdog. Secure Boot and Memory Integrity stay on (Memory Integrity is verified on).
- **Licenses:** SudoVDA's own changes are MIT or CC0 per its README (the repository has no LICENSE file); the Microsoft IddCx sample code it builds on is MS-PL. MS-PL allows binary distribution under compatible terms, so the separately installed driver package is a documented exception to the D17 allow-list.
- **Evidence gap:** no Microsoft document confirms that a self-signed user-mode (UMDF) display driver installs with Secure Boot and Memory Integrity on; T0.08 is the proof.
- **Alternatives:** VirtualDrivers VDD (MIT; SignPath-signed; reloads the driver on every change); Amyuni usbmmidd (Microsoft-signed, ad prompt, old); Parsec VDD (proprietary binary); writing an IddCx driver from scratch.
- **Spike (T0.08):** install, add/remove a 3088×1440 display, kill the app to test the watchdog, uninstall — all with Memory Integrity on.
- **Consequences:** no EV certificate (about €330+/yr) until distribution; then Microsoft attestation signing is required.
- **Confidence:** 80% · **Status:** provisional (T0.08).

- **2026-10-02 D11 preparation checkpoint (T0.08):** SudoVDA source is pinned at `a4b09fa2aa731a964d0cb5d139cb1e6240e4da12`; eleven reviewed files are retained, while two EDID files remain withheld because their output licensing is unresolved. The source build gate refuses the incomplete vendor. MSVC 14.50.35717 lacks x64 Spectre libraries; all three 10.0.28000.2526 NuGet hashes match. Running HVCI is confirmed; a fresh Secure Boot read and restore-point enumeration were unavailable. Independent Rust probe code handles the exact ABI and shared-watchdog constraints, but no driver was built, signed, installed or accepted. D11 and fallback selection remain provisional; no security setting or requirement was relaxed. See `docs/evidence/T0.08.md` and the driver source review/rollback plan.

## D12 — Android screen on the PC
- **Decision:** pinned scrcpy 4.1 (Apache-2.0) for live view and control (bundled client window or own client); markup captures use lossless `screencap` and the accessibility tree, never the video.
- **Consequences:** requires USB debugging (a developer/power feature). Uses the owner's installed adb; `adb.exe` is never bundled. scrcpy's LGPL DLLs (FFmpeg, libusb) are documented exceptions listed in `third_party/LICENSES`; their licenses are checked at build time (for example `avcodec_license()`).
- **Confidence:** 90% · **Status:** accepted.

## D13 — Photoshop bridge
- **Decision:** a UXP plugin acting as a WebSocket client to the PC app (UXP cannot listen on sockets); Imaging API for raw pixels, selections and pixel masks with explicit document/layer IDs; changes inside `executeAsModal`; develop on Photoshop 27.9.1 (27.10 has acknowledged regressions); install with the UPIA command line.
- **Consequences:** depends on the owner installing Photoshop (not installed as of 2026-10-01). Generative Fill has no official API; it is not used.
- **Confidence:** 80% · **Status:** accepted.

## D14 — AI handoff
- **Decision:** a local MCP server (stdio bridge + localhost Streamable HTTP; protocols 2025-11-25 and 2026-07-28) for pull; push through a Claude Code channel (the stdio bridge is the channel server; research preview, enabled with `--dangerously-load-development-channels`) and the Codex App Server `turn/start` with `localImage` (experimental, checked against the installed version's schema); both isolated in adapters. Clipboard and drag are permanent fallbacks. Direct image-model adapters start with GPT Image. The MCP token lives in Credential Manager; clients use the stdio bridge so no token lands in their configs.
- **Consequences:** push paths may break with client updates; the fallbacks keep the loop working. MCP Apps (UI inside chat clients) is out of scope for this release.
- **Confidence:** 75% · **Status:** accepted.

## D15 — Giant images
- **Decision:** the PC builds tile pyramids with libvips (LGPL, dynamically linked, Windows only behind `cfg(windows)`); the phone fetches lossless tiles for its viewport; phone-originated photos get a one-time background tiling pass with platform decoders (no libvips on Android).
- **Evidence:** libvips 8.18.7 built a DeepZoom pyramid from a synthetic 16320×12240 JPEG in 2.2–3.0 s at 120–206 MB peak on a 2-vCPU VM (2026-09-30). Android's HEIF region decode decodes the whole image (AOSP Skia source).
- **Spike (T0.09):** timings and memory on the Spectre and the S23 Ultra.
- **Confidence:** 85% · **Status:** provisional (T0.09).

### D15 measurement revision — 2026-10-02

T0.09 replaces the historical VM timing assumption with measurements on the
actual laptop and S23. For the fixed 102.47 MB synthetic 16320x12240 JPEG,
libvips 8.18.7 full decode took 2.82–3.38 s, 2040px thumbnail 2.26–2.79 s, and
256px JPEG-Q90 DeepZoom pyramid 22.38–38.24 s (three observations each, two
workers, OS caches unflushed). Peak CLI working set was 196.9 MiB. This simple
all-JPEG spike does not implement the product's mixed WebP/JPEG pyramid.

On SM-S918U, opening/decoding a 1024px JPEG region took 139–239 ms at the top,
473–557 ms in the middle and 808–838 ms at the bottom; sample-size-eight full
JPEG decoding took 845–862 ms. A 200 MP-dimension HEIF generated from a scaled
small RGB pattern took 2.89–3.00 s to decode to 2040x1530, peaking at sampled
PSS 702713856 bytes (670.16 MiB). Full-sized RGBA was skipped by the
384 MiB allocation guard; the first full YUV-buffer generation strategy actually failed
with OutOfMemoryError. The real owner camera fixture is absent.

**Revision:** retain background pretiling and cached previews; direct repeated
JPEG region decoding is not sufficient evidence for a 250 ms viewport target.
Reduced-output HEIF decoding still has substantial internal memory cost. Keep
full-resolution HEIF work on the paired PC pending bounded mobile evidence.
The existing 1.5 s preview, 6 s pyramid and 700 MB phone targets remain unchanged
and unpassed; no performance relaxation or gate acceptance is implied. T3.08
must test the real mixed-format pipeline, full app/GPU memory and viewport fill.
See `docs/evidence/T0.09.md` and its raw text-only measurements/failure receipts.

## D16 — PDF and SVG
- **Decision:** PDFium (bundled per platform, BSD/Apache) via pdfium-render on both devices; true redaction via object removal, rasterization of affected areas, full rewrite and qpdf (Apache-2.0) cleanup, run in the PC worker process; on the phone without the PC, a redacted PDF export rasterizes every page; resvg with a deny-all resource resolver and bundled fonts.
- **Alternatives:** MuPDF (AGPL — forbidden by D17); Android's platform PdfRenderer and Windows.Data.Pdf (two different engines → inconsistent rendering).
- **Confidence:** 85% · **Status:** accepted.

## D17 — Licensing
- **Context:** personal tool first, possible monetization later; the owner chose a no-GPL rule after reviewing the cost (about 1–2 extra weeks for an own streaming pipeline; rasterized text in redacted PDF areas).
- **Decision:** no GPL, AGPL, SSPL, non-commercial or unknown licenses in the app or anything linked into it. LGPL only dynamically linked on Windows. `cargo-deny` and a Gradle license check fail the build; bundled binaries are listed in a checked `third_party/LICENSES` manifest. GPL editor plugins live separately and talk over sockets. Documented exceptions: the unmodified Java runtime bundled by jpackage (GPL-2.0 WITH Classpath-exception-2.0), the separately installed driver package (MS-PL parts, D11), and LGPL DLLs loaded by the bundled scrcpy executable. rustls and quinn use the `ring` provider (the default aws-lc-rs provider's license includes the OpenSSL license). Workspace crates are `LicenseRef-VisualWorkbench-Proprietary` and unpublished until Christian decides otherwise.
- **Confidence:** 95% · **Status:** accepted.

## D18 — Document kinds (future video)
- **Decision:** every document carries a kind and schema version; `timeline` is reserved; the asset store accepts video files; transport has a bulk BLOB channel suitable for video proxies.
- **Consequences:** the future phone video editor reuses sync, storage and transport.
- **Confidence:** 85% · **Status:** accepted.

## D19 — Pen devices
- **Decision:** all input goes through a stylus abstraction built on standard Android `MotionEvent` axes with per-device capability flags. Samsung specifics (Air Actions, declared through manifest meta-data so they arrive as ordinary KeyEvents) are optional modules; the proprietary S Pen Remote SDK is not used. Tuned and tested on the S23 Ultra; any Android stylus works; the iPad app later reuses the core.
- **Confidence:** 85% · **Status:** accepted.

## PERF targets after Phase 0 — 2026-10-02 proposal, awaiting owner confirmation

The T0.12 reconciliation recommends retaining all nine original targets. Component
measurements expose engineering work; they do not establish replacement product
targets. No target or requirement acceptance text is relaxed. G0 remains BLOCKED
and this proposal is not owner approval. See [T0.12](evidence/T0.12.md) for the
original methods, source-bound comparisons and remaining physical baselines.

| Requirement | Target retained |
|---|---|
| PERF-001 | Local ink at most 25 ms p95 using the prescribed presentation method. |
| PERF-002 | Remote markup at most 50 ms p95 over USB and 80 ms p95 over 5 GHz Wi-Fi. |
| PERF-003 | Pen-to-host-to-phone at most 80 ms p95 over USB, with immediate ghost ink. |
| PERF-004 | Freeze a 4K window within 300 ms over USB. |
| PERF-005 | PC preview within 1.5 s, full pyramid within 6 s, phone screen fill within 250 ms. |
| PERF-006 | PC working set at most 1,200,000,000 B and phone PSS at most 700,000,000 B. |
| PERF-007 | Resume sync within 2 s after the new carrier is available, with zero duplicate commits. |
| PERF-008 | Selection crop within 1 s; full 200 MP PNG within 20 s and cancellable. |
| PERF-009 | Zero changed exterior pixels on every composite; local 12 MP processing within 2 s excluding model time. |

The owner authorizes provisional software progression while S23 checks are
deferred and applicable OnePlus testing continues. Final D6/D7/D9/D10/D11/D15
confirmation, physical/manual acceptance and gate approval retain their recorded
boundaries. The missing thermal method reconciliation is work to complete before
collecting a baseline, not permission to relabel 30-second samples as 1 Hz data.
