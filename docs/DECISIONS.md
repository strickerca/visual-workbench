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

## D7 — Transport
- **Context:** adb forward/reverse carries TCP only (no UDP), so QUIC can't run over adb; USB tethering gives a real IP link but may take over the PC's internet route; the default path must not require developer mode (possible future product).
- **Decision (provisional):** one framed protobuf protocol over two carriers: QUIC (quinn: reliable streams plus datagrams) on Wi-Fi or USB tethering, and a TCP multiplexer over adb as a developer/power option. Default order: USB tethering → Wi-Fi → adb. Route safety check on the PC.
- **Spike (T0.06):** measure RTT, jitter and throughput for tethered QUIC, Wi-Fi QUIC and adb TCP on the real devices; reorder defaults if the numbers say so.
- **Confidence:** 85% · **Status:** provisional (T0.06).

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

## D10 — Pen input into Windows apps
- **Decision:** `InjectSyntheticPointerInput` with PT_PEN: pressure, tilt, barrel, eraser; hover as in-range without distance; ≥ 20 Hz keepalive while in contact; per-batch guards on foreground window, geometry, DPI, input session and integrity level.
- **Known limits:** no hover distance, one barrel button, contacts cancel after about 1 s without refresh; injection cannot reach higher-integrity (admin) windows; WinTab-only apps get no pressure.
- **Spike (T0.05):** verify pressure in a test harness window, Paint, Krita, GIMP and Photopea in a browser (Photoshop once installed).
- **Confidence:** 80% · **Status:** provisional (T0.05).

## D11 — Virtual monitor
- **Decision (provisional):** fork SudoVDA, build it from source with the WDK from NuGet, sign it with a project-only self-signed code-signing certificate, and install the certificate into LocalMachine Root and TrustedPublisher. After signing, the private key moves offline (a password-protected `.pfx` on removable media) and is deleted from the certificate store. Control by IOCTL with a watchdog. Secure Boot and Memory Integrity stay on (Memory Integrity is verified on).
- **Licenses:** SudoVDA's own changes are MIT or CC0 per its README (the repository has no LICENSE file); the Microsoft IddCx sample code it builds on is MS-PL. MS-PL allows binary distribution under compatible terms, so the separately installed driver package is a documented exception to the D17 allow-list.
- **Evidence gap:** no Microsoft document confirms that a self-signed user-mode (UMDF) display driver installs with Secure Boot and Memory Integrity on; T0.08 is the proof.
- **Alternatives:** VirtualDrivers VDD (MIT; SignPath-signed; reloads the driver on every change); Amyuni usbmmidd (Microsoft-signed, ad prompt, old); Parsec VDD (proprietary binary); writing an IddCx driver from scratch.
- **Spike (T0.08):** install, add/remove a 3088×1440 display, kill the app to test the watchdog, uninstall — all with Memory Integrity on.
- **Consequences:** no EV certificate (about €330+/yr) until distribution; then Microsoft attestation signing is required.
- **Confidence:** 80% · **Status:** provisional (T0.08).

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
