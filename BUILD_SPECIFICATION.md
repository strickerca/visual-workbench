# Visual Workbench — build specification

**Working name; not a final product name.**

Revision: 2026-09-22 / 1.0  
Audience: owner and implementation agents  
Delivery status: research and specification package, **not an installable application**. No Android APK, Windows executable, driver, or Adobe plugin has been built or hardware-tested in this package.

## 1. Purpose and retained scope

Build a native paired Windows/Android visual workbench for a Samsung Galaxy S23 Ultra with S Pen. The primary workflow is capture/import → precise selection and markup → live two-device review → full-resolution copy/export → handoff to Codex, image-generation tools, or installed creative applications → comparison of returned results.

The complete initial product target contains a shared editable canvas, independent viewports, live intermediate edits, two-way screen sharing and separately authorized remote input, two precision-magnification modes, S Pen Air Actions, USB and local Wi-Fi, large-image handling, PDF/SVG workflows, direct use of installed Photoshop/Illustrator/Paint, and optional true Windows extended-display installation. An optional feature remains retained scope even when it is delivered after the core; do not silently remove it.

The earlier voice-controlled snapshots, timed clips, rolling video buffer, AI observation, and RTS assistant are retained future modules. They are not required to declare the image-and-tunnel release complete. General photo-retouching parity with Photoshop, arbitrary Adobe document round-trip fidelity, and internet-wide remote access are not part of this release.

The owner is right-handed, initially the only user, accepts personal-use installation and recommended defaults, and wants both windowed and fullscreen Windows operation.

## 2. Target hardware and open diagnostics

Owner-supplied screenshot: HP Spectre x360 Convertible 15-eb1xxx; Intel Core i7-1165G7; 16 GB RAM; Intel Iris Xe; Windows 11 Home 25H2, build 26200.9457. Approximately 14 GB free at the time of the screenshot (477 GB total minus 463 GB used). Do not include the screenshot's device ID, product ID, computer name, or other unnecessary identifiers in telemetry or bundles.

Intel documents unified/shared memory and compatibility reporting of 128 MB dedicated graphics memory. Do not interpret that figure as the GPU's actual working-memory ceiling. [S01]

Build around bounded memory and disk use. The application must measure available space before allocating large caches, preserve a configurable reserve, and never delete originals automatically. Photoshop itself recommends substantially more available disk space than its installation minimum; the owner should make room before a substantial Adobe/large-image/build workload. [S02]

First-run diagnostics should record only useful, approved values: app versions, Android/One UI version, display size/DPI/orientation/HDR state, renderer/codec availability, supported pen fields, Air Actions availability, authorized USB bridge state, and free-space limits. Unknown versions are a validation task, not permission to claim compatibility.

## 3. Product modes and input authority

### 3.1 Shared canvas

Both devices address a single versioned document: immutable original assets, editable annotation objects/layers, raster-adjustment parameters, selections/masks, and history. Zoom/pan/rotation, viewport placement, loupe settings, and local tools belong to each device's view state, not shared document state.

Render the local gesture immediately. Transmit provisional updates while the gesture is active. Commit one undoable transaction when it ends. Cancellation removes the provisional operation everywhere; it must not leave an orphan stroke on the second device. Explicitly test palm cancellation, pointer cancellation, disconnection, duplicate delivery, and reordered messages.

The desktop can pin an overview while the phone edits close-up. An optional viewport outline and local pointer identify the phone's working region. Follow-peer and match-view are explicit commands, never automatic consequences of receiving an edit.

### 3.2 Annotate a live capture

The viewer is live until the first valid markup contact. Pin the exact frame most recently presented on the editing device, with frame ID, source ID, dimensions, timestamp, and geometry revision. Do not substitute a newer host frame that the owner never saw.

Create a reversible draft. Pen/touch cancellation discards the canceled operation. A visible Cancel & Resume Live action discards the current accidental draft without overwriting earlier accepted work. After several intentional edits, confirmation may protect a nonempty draft from accidental wholesale deletion. Android Back and Windows Escape should have predictable equivalents.

Only the view freezes. The source application continues running. Clearly display FROZEN and source age. Resume does not claim previous annotations still identify the same controls in a changed interface.

### 3.3 Remote-edit mode

Forward authorized pen/mouse/keyboard input to the actual selected Windows application or supported Android target. Keep its live feedback running; **do not freeze the screen on pen contact in this mode**.

Capture permission, remote-control permission, recording/export, and sending to an AI destination are separate capabilities. An observed window is not automatically an input destination. Stop forwarding if focus/foreground ownership or source geometry becomes invalid. Do not inject into a different foreground window merely because a captured window remains visible in the preview.

### 3.4 Extended-display mode

An optional virtual monitor is a real Windows display endpoint, separate from shared canvas and ordinary screen mirroring. Support selecting resolution, orientation, and scaling. Do not require this driver for normal canvas use. Restore windows to a physical display on intentional removal when possible; provide recovery if a session ends unexpectedly.

Windows documents an indirect display driver model using user-mode UMDF/IddCx and DirectX surfaces. Driver packaging/signing is an independent delivery gate. [S09, S10] No normal install flow may ask the owner to disable Secure Boot, memory integrity, or equivalent safeguards. An unsigned demonstration driver is not an acceptable finished feature.

## 4. Precision input and magnification

### 4.1 Pen foundation

Use Android native input for tool type, pressure, tilt/orientation, hover, and cancellation. Android's documentation describes these inputs and explicit cancellation handling. [S03] Evaluate stable Android Ink 1.0.0 as the initial authoring dependency; official release notes also list 1.1.0-alpha08 as preview on the verification date. Do not accidentally ship preview APIs merely because the documentation example uses them. [S04]

The owner draws with the pen and navigates with fingers by default. Support configurable pressure response, brush cursor, adjustable stabilization, keyboard shortcuts, handedness-aware controls, and pen-accessible tool switching. Preserve raw samples separately from normalized application values where useful for diagnostics. Never assume all reported pressure values or devices share one scale.

### 4.2 Contact-following inspection loupe

An optional offset magnifier appears while the pen touches the image. It shows the target region and a precise reticle at configurable magnification. Default placement is above-left for a right-handed owner, with edge avoidance and manual placement. It must not obscure the intended mark or change the phone's normal viewport.

The loupe is an inspection aid; enlarging a display alone does not change the physical input-to-document gain. Offer nearest-neighbor/pixel-grid viewing and smooth display as explicit alternatives. Do not use generated detail to fake source resolution.

### 4.3 Interactive precision lens

A separate pen-button toggle opens an editable inset around the hovered/last selected document point. Suggested multipliers are 2×, 4×, and 8× relative to the existing phone viewport. Keep the rest of the phone at the identical zoom, pan, and rotation. A miniature region indicator provides context.

The inset has its own coordinate transform, so the same physical pen movement represents a smaller document displacement. Pin the target and transform during an active stroke. Moving a target-following editing coordinate system under a moving pen creates unstable input; the inspection loupe may follow, the editable lens must latch appropriately.

Toggling an interactive mapping during pen-down must not connect unrelated coordinates with an accidental stroke. Queue transform-changing requests until pen-up, or use a separately proved continuous mapping strategy. Default: queue until pen-up with immediate visual acknowledgement. A hold-to-peek visual option may appear during contact because it does not remap input.

Closing the lens restores the unchanged base viewport. Remote host-editor magnification is constrained by the host-rendered pixels; source-detail precision requires an editor zoomed view or document integration.

### 4.4 Air Actions and in-range button routing

Implement ordinary in-range stylus button handling independently of Bluetooth Air Actions. Samsung documents declarative Air Actions that become Android KeyEvents and recommends this route for predefined swipe/circle gestures rather than implementing raw motion recognition. Manual app enablement is part of personal setup. The foreground and capability restrictions are real; do not promise global background interception. [S05, S06]

Proposed defaults, fully remappable:
- In-range button: toggle interactive precision lens; optional hold-to-inspect.
- Air single click: toggle precision lens at last valid target.
- Air double click: fit view / restore view (non-destructive).
- Air swipe left/right: undo/redo in the active authorized editing context.
- Air swipe up/down: previous/next tool or document/page, configurable.
- Air circle clockwise/counterclockwise: increase/decrease lens magnification.

Route commands through one context-aware command dispatcher. Avoid interpreting one gesture as a lens toggle plus a remote right-click/eraser. Debounce/resolve single-versus-double actions and duplicate event paths. A gesture never submits a chat message or applies a generated replacement by default. Visible controls remain available when Air Actions is off or unavailable.

## 5. Existing Windows editors

### 5.1 Universal remote surface

The phone can operate an existing Windows application through captured video and forwarded input. Implement native pen injection where supported, not merely mouse movement with a pressure number attached. Windows exposes synthetic pen input and pen pressure/tilt fields. [S07] Validate the target application, tool, input API, driver, DPI, and current document state as a combination. Ordinary Windows input injection is subject to integrity restrictions; do not bypass them or silently elevate the whole product. [S08]

Photoshop/Illustrator/Paint are explicit compatibility targets, not already-tested certifications. Report pointer, drawing, pressure, tilt, eraser, shortcuts, and independent host-view capability separately for each installed version. Adobe's tablet documentation does not certify our custom bridge. [S11]

### 5.2 Independent native editor views

Photoshop supports another window of the same image and separate zoom behavior. Illustrator explicitly supports multiple simultaneous windows of one document with independent view settings. [S12, S13]

Use the physical display for overview and a virtual phone display for a detail window of that same document. Do not open a second file copy. Keep synchronized-view settings off unless deliberately requested. Capture both views as needed. Paint's corresponding native multiview behavior is not verified; provide the ordinary remote surface and inspection lens rather than inventing parity.

### 5.3 Photoshop document bridge

An optional local UXP plugin can improve precision export and selection handoff. Photoshop's Imaging API exposes document/layer pixels, rectangular source regions, selection and mask operations, and controlled pixel insertion. [S14]

Proposed workflow: retrieve clean source pixels and the selection; let the phone refine a mask; return the selection or create a new approved layer; copy a source-resolution crop to the transfer shelf. Retain original pixels and history identity. Explicitly select document/layer IDs; never apply a response to whichever document became active later.

Respect returned bounds, pyramid levels, component depth, profile, alpha semantics, and Photoshop modal/history requirements. The sourceBounds returned for a cached pyramid are not automatically full-resolution coordinates. A screenshot is not a substitute for these metadata. This bridge is not an excuse to implement a duplicate Photoshop engine, and Illustrator must not be assumed to support the same UXP API.

## 6. Image, PDF, and SVG handling

### 6.1 Large-image design

Target full-resolution S23 Ultra photo workflows, including a 200-megapixel-class test case. One 200,000,000-pixel RGBA8 buffer is 800,000,000 bytes before layers, staging buffers, history, or application overhead. Do not create full-image copies per gesture or upload a giant source texture merely to show an overview.

Use demand-driven tiled decode, a multiresolution pyramid, bounded CPU/GPU tile caches, and region-first transfer to the phone. Keep a readable lower-resolution preview while true source tiles arrive. Distinguish loading detail from a permanently downsampled source. Maintain coordinates in original image space through EXIF orientation and display rotations. Candidate rendering/decoding foundations must undergo format, performance, security, and license evaluation. Android region decoding and libvips provide relevant foundations, not a guarantee that every codec supports cheap arbitrary-region decode. [S15, S16]

No literal unlimited-size promise. Apply explicit resource and format bounds, predictable errors, cancellation, and an alternative export. Never silently reduce the saved source resolution.

### 6.2 Format fidelity policy

| Format group | Target behavior | Boundary |
|---|---|---|
| PNG, JPEG, WebP, BMP, TIFF | Import; annotate/select; explicit applicable raster exports | Encoder dimension, bit-depth, transparency and compression limits remain visible. |
| PDF | Page navigation, clipped high-resolution rendering, page-indexed overlays, original-plus-markup or explicit flattened-copy export | Not an Acrobat-equivalent semantic text/layout editor; preserve originals. Password/protection and unsupported content produce clear messages. |
| SVG | Safe vector rendering, scalable overlays, appropriate SVG export | No arbitrary scripts/external network content; fonts, effects, and unsupported constructs require fidelity warnings. |
| HEIC/HEIF, AVIF | Add through validated codecs when present or distributable | Detect codec availability; do not label missing-codec failures corrupt images. |
| RAW/DNG | Useful rendered preview/import where feasible | Not a complete nondestructive raw-development engine. |
| PSD/PSB/AI | Native Adobe remote editing and approved integration; composite import where supported | Do not claim complete layered native-file round trips inside this app. |

WebP has a documented maximum of 16,383 pixels per dimension. Export preflight must detect this instead of silently scaling a longer screenshot or panorama. [S17]

PDF pages retain page identity and crop/rotation coordinate transforms. Use extracted native text for selection where available; do not default to OCR. A black overlay is not secure PDF redaction: originals/hidden data must not be included in a purportedly redacted result. SVG/PDF/image parsing needs resource/time limits, disabled scripting/network retrieval, and isolated processes where practical. PDFium and resvg are candidates for evaluation, not selected versions or unqualified fidelity promises. [S18, S19]

### 6.3 Color and source preservation

Retain original profile and depth in project assets. Distinguish screenshot/rendered capture from source pixels. Offer a predictable sRGB export profile for chats while preserving the original separately. Document conversions and alpha handling. Pixel-identical preservation is asserted only for a verified lossless path with unchanged geometry/color representation outside the permitted blend region. Compressed remote preview is not a color-proofing guarantee; desktop/phone display appearance must be evaluated separately.

## 7. Complete editing and handoff toolset

Include editable pen/highlighter strokes; stroke/object and mask erasing; arrows/lines/rectangles/ellipses; labels/text; sequential numbered markers; layer visibility/order/opacity/locking; grouping; move/resize/rotate; keyboard editing; undo/redo; native project save/recovery; crop/rotate/resize and basic nondestructive tonal adjustments. Full professional retouching remains delegated to the native editor.

Selections: rectangle, ellipse, polygon, freehand lasso, painted mask; add/subtract/intersect/invert; expand/shrink/feather; handles; selection bookmarks; optional assisted selection with editable manual correction. Store change/preserve/reference/explain intent independently of appearance.

Copy/export choices: clean whole image; clean selected region; marked whole image; marked selected region; irregular transparent cutout; separate mask; context overview plus detailed crop; instructions only; complete edit package. Always say which source revision and resolution are being exported. Never accidentally include app toolbars in a clean export.

The shared transfer shelf contains owner-selected exports, not an unrestricted background clipboard recorder. Windows clipboard/image/file-drag and Android sharing are complementary handoff routes. Large exports can offer file-based transfer instead of exhausting clipboard memory. Stage attachments and text separately when the receiving app cannot accept a compound paste.

An edit package contains original asset reference/hash, document and revision IDs, capture timestamp when relevant, dimensions/profile, region coordinates and masks, annotation objects, clean and marked previews, requested change, preservation constraints, and destination/project/conversation identity where supported. Do not expose unnecessary system identifiers or credentials.

Codex support begins with reliable explicit handoff; add a narrowly scoped MCP interface for retrieving staged packages or an embedded App Server session where appropriate. These are supported interfaces, not evidence of a universal method for pushing into any existing visible chat. [S20, S21]

Returned results are alternatives linked to the originating request. Provide wipe/blink/split/difference views and partial acceptance. A screenshot of a generated UI is a visual proposal, not proof the actual application was changed. Source-code changes remain owned and approved through the coding tool.

## 8. Transport and local operation

No mandatory account or public cloud backend. Core viewing/editing/export must work without internet. Pair only with explicit owner approval; use authenticated encrypted channels and revocable device trust. Discovery should not broadcast private document names. Use vetted cryptographic implementations, never a custom cipher.

Local Wi-Fi media may use a suitable WebRTC/native media stack, with local signaling and negotiated capabilities. Keep durable document operations separate from replaceable cursor/media updates. A USB developer bridge is an acceptable personal-use path. ADB forwarding/reversing exposes TCP/socket mechanisms; it does not magically tunnel WebRTC's UDP media. [S22]

Engineering default for a USB bridge: separate framed control/document/media streams over authorized forwarded TCP sockets, with bounded latest-frame queues and reconnect framing. An alternative USB-network connection can reuse IP transports if validated. Select one implemented, tested route rather than describing these alternatives as if both already exist.

For Android cross-app mirroring/control, evaluate reuse of the official scrcpy implementation or its well-established architecture with proper license notices. Shared canvas operation itself should not require cross-app debugging. No root, no unattended public ADB port, no authorization bypass. Android MediaProjection requires proper consent and lifecycle handling; protected content is not forcibly captured. [S23, S24]

Switching Wi-Fi/USB must suspend invalid remote input, retain pending document edits, renegotiate video, and verify peer/source identity. Do not replay old clicks or strokes into a live external application after reconnection. Durable native-canvas operations and ephemeral remote-input events have different recovery policies.

## 9. Performance, interaction, and quality

Initial performance goals are engineering targets, not measurements: immediate local pen feedback; under 100 ms typical local-network peer-preview age when achievable; responsive 30–60 fps screen-view profiles after hardware/codec negotiation. Prefer stable operation and clear quality choices over unproved fixed latency claims.

Use hardware capture/encode/decode where tested. Text and fine-line detail require a quality-focused profile, not just game-video settings. Optional still/ROI refinement can improve static screen detail but must be revision-matched; it cannot recover source pixels absent from the host-rendered image. Thermal and battery modes lower video work before harming document correctness.

Measure pen-to-local-preview, pen-to-peer-preview, host-rendered response, capture age, dropped/stale frames, document backlog, cache size, memory peak, and export latency separately. Cross-device one-way times need calibrated clocks/uncertainty; otherwise use suitable round-trip or hardware/video measurements, not invalid timestamp subtraction.

Windowed, maximized and borderless fullscreen modes preserve layout and work. Portrait/landscape phone layouts support right-handed ergonomics, safe insets, readable text, adjustable lens/tool placement, high-contrast controls, reduced motion, and keyboard accessibility. Persistent status distinguishes LIVE/FROZEN, source, target app, control grant, synchronization, and export readiness.

## 10. Safety and reliability invariants

1. Local pen feedback never waits on networking or an AI service.
2. Viewport changes are local unless explicitly shared.
3. A provisional canceled operation leaves no committed remote edit.
4. Remote source/layout changes invalidate input mapping before further input.
5. A frozen annotation references the displayed frame, not an unseen newer frame.
6. Remote-edit pen contact does not trigger annotation freezing.
7. Precision-lens toggles never alter the saved base viewport or create a stroke jump.
8. A lost connection never replays old remote clicks into a different live state.
9. Copy/save/export never silently uses a lower-resolution preview.
10. An AI return cannot overwrite a different document revision without review.
11. Control, capture, file access, and transmission are independently authorized.
12. Driver readiness, format support, and app compatibility are reported per tested configuration, not inferred from API existence.
13. No public executable/driver is labelled complete solely because a source project compiles.

## 11. Architecture and delivery gates

Default architecture: native Kotlin/Compose Android application; native .NET Windows companion with appropriate rendering/native interop; portable protocol and tested geometry contracts; separate optional virtual-display driver; optional Photoshop UXP plugin; provider-neutral handoff adapters. Rendering-engine and transport-library final choices require a focused spike and dependency/license/security review. Do not create two incompatible definitions of coordinates, transactions, or revision identity.

**Gate A — highest-risk experiments:** verify Android button/Air Action event delivery; native pen pressure into the actual installed editors; same-document dual views; USB and LAN video paths; signed-driver distribution path; representative giant-image decoding. Record failures rather than hiding them behind fallback screenshots.

**Gate B — end-to-end core:** source import/capture, phone pen input, desktop live preview, independent zoom, both loupes, selection, clean/marked copy, save/reopen. Test actual hardware, not only simulator input.

**Gate C — robust editing/format support:** full toolset, page/vector workflows, bounded memory, transaction reconciliation and recovery, color/export policy, transfer shelf and real receiving-app tests.

**Gate D — tunnels and host editors:** both capture directions, authorized remote input, Windows input compatibility, USB/LAN switch/reconnect, geometry/focus safeguards, Android permissions, remote contextual shortcuts.

**Gate E — retained advanced integrations:** signed virtual monitor, dual native Adobe views, Photoshop selection/pixel bridge, structured Codex access, result comparison and project packaging.

**Gate F — release:** actual APK and Windows installer, optional separately identified driver/plugin packages, checksums, reproducible build instructions, dependency notices, versioned compatibility results, uninstall/recovery guide, acceptance-test evidence. No unimplemented placeholder control may masquerade as working functionality.

These gates are implementation order, not permission to silently narrow the agreed product.

## 12. Acceptance scenarios

A01: Open one source on both devices, pin desktop overview, zoom/rotate phone, draw and manipulate annotations, verify shared content and unchanged peer view.

A02: Move sliders/objects/selection handles; desktop shows intermediate motion, not just the final pen-up state.

A03: Show inspection loupe only during contact; exercise boundaries and handedness; no input gain change or displaced stroke.

A04: Toggle precision lens at multiple zooms/rotations; edit within it; close; assert exact base-view restoration and correct source coordinates.

A05: Request precision toggle during a stroke; no coordinate jump, connector segment or duplicate commit.

A06: Receive palm cancellation, pointer cancellation and app backgrounding; remove provisional canceled ink from both devices.

A07: Change the live interface between host capture and phone contact; pin the actual phone-presented frame; cancel accidental draft and resume.

A08: Draw remotely in Photoshop; live result remains visible; no freeze-on-contact; no input when the selected target becomes invalid.

A09: Exercise Air Actions enabled/disabled/disconnected; remap commands; no duplicate single/double or barrel/remote interpretation; no unauthorized undo or chat send.

A10: Import representative 200-MP-class photo and long screenshot; navigate without unbounded memory; detailed tiles arrive; export original-resolution selection.

A11: Export an image exceeding WebP dimension bounds; clear error/alternative, no silent scale. Preserve alpha and metadata according to chosen policy.

A12: Open multipage PDF and SVG; annotate, zoom, select and export; verify documented fidelity and no active/external content execution.

A13: Copy/drag into the owner's actual Codex/chat/editor interfaces; verify original/selection/marked/context variants and text attachment behavior.

A14: USB and LAN session transitions preserve native document transactions but discard stale host input. Disconnect mid-edit and reconcile without double application.

A15: Photoshop and Illustrator same-document dual views show genuine edits at independent native zooms. Paint behavior is tested and accurately recorded.

A16: Photoshop bridge exchanges selection masks and image regions with explicit IDs, returned bounds/pyramid/profile handling, and non-destructive insertion.

A17: Install/enable/remove the signed virtual display without disabling system safeguards; restore visible windows and correct DPI/orientation mapping.

A18: Minimize/maximize/fullscreen/rotate/sleep/lock/permission revoke/source close; predictable state and no lost project data.

A19: Run bounded-cache, low-disk, decode-failure, stale-frame, malformed-file, unauthorized-peer, and export-redaction tests.

A20: Generate release artifacts and record exact toolchains, hashes, installed application versions, verified features, limitations and test results. Untested is not passed.

## 13. Evidence and provenance

Official technical documentation was inspected on 2026-09-22. Source availability supports feasibility and constraints, not measured performance or a claim of completed implementation. `SOURCES.json` contains source URLs, publisher, narrow verified claim, and validation boundaries. Owner choices and the hardware screenshot are separate inputs. No inaccessible previous project/repository is asserted to have been read.

## 14. Implementation-agent instructions

Begin by reading this specification and REQUIREMENTS.json. Preserve every requirement ID; update implementation and verification status separately. When a platform restriction changes a proposed approach, report the evidence and an explicit alternative. Never silently redefine independent zoom as screen duplication, pressure-sensitive input as mouse events, original pixels as a video screenshot, USB as an unimplemented network toggle, or a virtual-display driver as a borderless app window.

Implement vertically, build on each target platform, and record real evidence. Use pinned dependencies with notices and check official documentation before selecting changing versions. Do not ask the owner to solve avoidable engineering questions. Ask only for genuinely unresolved owner choices or access needed to run an actual hardware test. Avoid a giant initial all-features rewrite; retain the ultimate target while proving the riskier paths early.
