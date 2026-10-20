# Desktop stream receiver integration — 2026-10-20

This coherent T3.02 source integration builds on main
`90c0ee22d5e7d94a56c54068f85bf63ffa481b68` and borrowed-draft implementation
`8396012668aae8a507cb8d6ab2584be04f313472`. Parent independently accepted the
bounded presentation closure in `18158c7cd7b26210da1e0b1ff40bba26bb1f1d60`.
Only its six production stream modules and eight test/witness files are
promoted, byte-for-byte. Older Main/controllers/shared/native/Gradle files are
not overlaid. The separate release-hook branch `2a70c7c` remains preserved.

| Slice | Integrated behavior | Verification boundary |
| --- | --- | --- |
| Entry and parent fence | Workbench → Receive paired live capture is an explicit local receiver action on an authenticated attachment. Admission pins project/document/page/generation, LiveAttachment/preview transition and native connection epoch, refuses unfinished independent edits, and joins existing parent mutex work. Ordinary commands, mouse menus, shortcuts, dialogs, verification borrows and late capture delivery are fenced while the receiver is owned. | Actual DesktopController acquisition/cancellation, same-link reconnect and independent-parent-erase fixtures pass; no real link or OS capture ran. |
| Acquired ownership | Receiver, annotation, session, renderer and borrowed child are retained before cancellation checks. Departure joins acquisition and entered operations before renderer, child, annotation/stream, link, project and native runtime retirement. Failed stages remain retained and native unload refuses outstanding ownership. | Late receiver/view/editor results, cancelled/concurrent close, failed cleanup and exact retry are covered by root and real annotation-session fixtures. A permanently failed accepted renderer remains fail-closed; retry is not claimed to repair a broken platform surface. |
| Contact and camera | Valid stylus or primary-mouse DOWN settles actual live draws before native freeze, validates exact frame identity and opens the private child. Its camera fits the actual settled contact frame. Up to 4,096 real samples can wait for actual editor geometry readiness. Fixed header/issue/tool slots preserve canvas origin and viewport through freeze. Size/DPI changes, secondary contact, focus/modal interruption or overflow cancel pending replay. | Actual controller/adapter and retained late-freeze fixtures plus an off-screen ImageComposeScene layout test pass. Physical input, Windows DPI and optical presentation remain unverified. |
| Frozen drawing | The accepted retained raster supplies pixels beneath the transparent ordinary annotation renderer. Physical Compose camera coordinates convert once to logical AWT coordinates. Original arrival adopts the same frame without replacing the child or its camera/annotations. Compose preparation jobs have an explicit joined lifetime before child disposal. | Synthetic pixels, camera conversion, real preparation-job cancellation, exact original adoption and wrong-lease refusal pass. Actual heavyweight Canvas/SwingPanel/Compose composition is unverified. |
| Save and resume | Save seals the child's complete mutation boundary and drains accepted edits. Each new native stream Save plan reserves a monotonic 4,096-counter range before preparation; unknown outcomes retain the exact plan and permit receipt retry after attachment loss. Degraded Save requires a separate explicit choice. Discard/resume retires old render/child owners before native resume, then creates a fresh renderer. | Real CaptureAnnotationSession fixtures cover unknown retry, degraded consent, source loss, fresh resume and retained cleanup stages. This stream-only allocator does not claim that ordinary edits skip its unused reserved tail; every plan retires before parent edits resume. |
| User decisions | LIVE/WAITING/FROZEN/source-closed status, source name, estimated frame age, original/display readiness, durable Save result and cleanup state are visible. Unsaved close/resume requires explicit discard. Parent Quit refuses until this draft is saved/closed through its owner. | Modal, tool and dispatch source is independently reviewed. Actual Windows mouse/pen/window journeys remain pending. |

The application and Test tasks share both required JDK exports and enable
`compose.interop.blending` before Compose startup. The entry point also sets the
blending property before UI work. This uses the documented experimental
[Swing interop blending](https://kotlinlang.org/docs/multiplatform/compose-desktop-swing-interoperability.html#experimental-interop-blending)
mechanism; default SwingPanel layering would cover Compose annotations. Its
actual behavior with this heavyweight Canvas and Direct3D pipeline is a Windows
runtime acceptance gate, not established by the off-screen layout test.

The final bounded combined Desktop batch passes **125 methods across 15 suites**:
31 accepted stream-module, 22 session, 6 actual root, 15 borrowed adapter,
3 transparent canvas, 5 render lifetime/counter/input, 1 actual Compose layout,
6 publication, 9 sessions, 20 Android-tunnel and 7 ordinary-background methods.
The repository setup's three offline policy checks and `git diff --check` pass.
The independent Astra root review reports no remaining actionable findings after
closing fixed-layout, primary-pointer, parent-menu and resize/DPI replay issues.

The final batch runs offline, with one heavy build and at most two workers, and
explicitly excludes `:desktop:packageConnectionRuntime` on Linux. The Linux JDK
warns that `sun.java2d.d3d` is absent, and Fontconfig reports an unwritable cache;
all named tests pass. These warnings do not establish Windows D3D availability.
No public DTO, protocol, native/FFI registration, lockfile, generated binding or
Android source changed in this partition. No redundant binding regeneration or
historical native/full-suite rerun was performed.

The machine-readable source/log/JUnit receipt is
`evidence/DESKTOP_STREAM_RECEIVER_INTEGRATION_20261020.json`. Earlier 121-case
failure and 123-case passing logs are retained as intermediate receipts; only
the final 125-case run describes the final tested implementation.

Open gates include actual Windows/GPU composition and presentation, packaged
launcher configuration, real stream/freeze/lossless/Save/resume/source-loss,
frame rate, memory, latency, physical pen, Bluetooth, thermal and owner
acceptance. MSVC `lib.exe` remains absent; model artifact CONNECT403, model/native
aggregate/runtime/memory admission, configured tunnel DLL/license/notices and
deferred Krita/broader Phase4 boundaries are unchanged.
