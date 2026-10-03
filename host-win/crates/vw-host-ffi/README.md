# Windows host binding (T1.07 / T1.11)

This independent UniFFI component produces `vw_host.dll` and Kotlin package
`com.visualworkbench.bindings.host`. The original C smoke ABI remains available.
All Win32 operations, PNG decoding and clipboard waits run on a dedicated native
thread. The Kotlin wrapper should create the service and transfer large byte
arrays on `Dispatchers.IO`; UniFFI's array marshalling itself copies data and is
not a measured sub-2-ms UI operation.

## Surface

- `set_process_per_monitor_v2()` is an explicit bounded synchronous startup call
  made before any AWT/Compose window. It verifies the resulting process context,
  including already configured PMv2, and fails for an incompatible locked mode.
  This function never runs automatically when the DLL loads.
- `create_host_service()` asynchronously creates an owned `HostService`.
- `dpi_at_point(x, y)` accepts physical desktop pixels, including negative origins,
  and returns monitor/work rectangles, DPI, scale and verified PMv2 context.
- `window_dpi_info(hwnd)` returns physical Win32 window/client rectangles and both
  the target window's DPI and its monitor's DPI. A DPI-unaware target can report
  96 while its monitor has a different DPI. A private hidden PMv2 probe measures
  the selected monitor; the caller's process/thread awareness is not changed.
  `GetWindowRect` includes its usual invisible resize borders. Capture-specific
  DWM bounds and geometry revisions belong to the capture adapter.
- `set_clipboard_png(bytes, operation)` decodes a PNG through `vw-raster`, then
  publishes the exact original bytes in Windows' registered `PNG` format. It
  does not silently resize, flatten alpha, reduce 16-bit samples or relabel color.
  The receipt gives decoded D-space dimensions/depth, byte count and the clipboard
  sequence observed while this publication holds the clipboard.
- Create a fresh `HostOperation` for each clipboard attempt. `status()` exposes
  queued, validating, waiting, publishing and terminal states. `cancel()` returning
  true before `Publishing` guarantees that this request did not replace clipboard
  contents. Dropping its pending Rust future also cancels before that commit point.
  Cancellation after publication begins returns false; it does not undo a copy.
- Await `shutdown()` once, then release the generated Kotlin object. Shutdown
  rejects queued work and awaits destruction of native windows and input buffers.
  Dropping the service requests the same cooperative cleanup without blocking the
  UI on a native thread join. Keep the DLL loaded until shutdown completes.

There are at most two host workers, eight queued requests per worker, 32 MiB per
encoded PNG and 64 MiB retained encoded bytes per worker. PNG decoding is capped
at 16 million pixels and a 512 MiB working-memory estimate. Queue/byte saturation
returns `Busy`. A request expires after 30 seconds before publication and waits
at most 500 ms for the clipboard lock. Cancellation/deadlines are checked before
and after the bounded decoder and throughout lock retries; an in-flight codec or
Win32 call is cooperative, not forcibly terminated.

PNG parsing, allocation and format registration all finish before `EmptyClipboard`.
A live owned hidden HWND satisfies Windows' clipboard-owner contract. HGLOBAL
storage uses `GMEM_MOVEABLE`; every unsuccessful path frees its allocation and
closes the clipboard, while a successful `SetClipboardData` transfers ownership
exactly once. Errors distinguish refusal before replacement from failure after
replacement began, when clipboard contents may have changed. Clipboard data,
window titles, handles and pixel contents are never logged.

The original `set_clipboard_png` remains a PNG-only compatibility API. The T1.11
surface below adds dual-format copy, explicit image paste and leased drag files.
Non-Windows creation returns `UnsupportedPlatform`; no Android or common module
receives a success stub.

## T1.11 handoff surface

- `handoff_limits()` reports the hard admission limits. Clipboard PNG is at most
  32 MiB encoded / 16 million pixels; DIB is at most 64 MiB. The decoder and DIB
  working estimates must also fit 512 MiB, so the pixel cap does not guarantee
  every image below it is admitted. Refuse visibly and offer file drag when an
  image exceeds clipboard limits. Never resize to make it fit.
- `set_clipboard_image(png, binding, dib_options, operation)` validates a completed
  core export. `ExportBinding` contains `png_blake3`, `source_asset` and the exact
  revision string `r{seq}-{hash8}`. The worker checks the complete PNG hash and its
  bounded, uncompressed `VisualWorkbench` iTXt receipt, including dimensions and
  CRC. Animated or EXIF-bearing inputs are refused; core exports have already
  consumed orientation. Arbitrary imported PNGs must first pass through core
  export to obtain this binding.
- Both registered PNG and CF_DIBV5 allocations are prepared before opening and
  emptying the clipboard. PNG bytes and their metadata remain exact. DIBV5 is a
  top-down BGRA companion with explicit channel/alpha masks. An RGB ICC profile
  is embedded unchanged; Gray profiles require an explicit upstream conversion
  to RGB/sRGB. An untagged companion needs `assume_untagged_srgb=true`. A 16-bit
  source needs `allow_depth_reduction=true` to make its 8-bit DIB companion; the
  PNG remains 16-bit. The receipt records those choices. Alpha is preserved.
- Failure after either format begins publication returns
  `ClipboardPublicationFailed`; a successful first HGLOBAL is already owned by
  Windows and is never freed or rolled back by this worker. Cancellation before
  `Publishing` prevents replacement. Cancellation after that point cannot undo
  publication and must not be reported as a successful cancellation.
- `read_clipboard_image(options, operation)` is an explicit Paste action. It
  reads only requested image formats, never text, file lists or clipboard
  history. `PreferPng` chooses PNG, otherwise DIBV5, otherwise DIB; a malformed
  selected format fails instead of falling back to potentially different data.
  Bytes and sequence are captured under one clipboard lock and decoded only
  after closing it. PNG is returned exactly; DIB is normalized to a lossless PNG
  with its original-payload hash, normalized PNG hash and color-assumption flag.
- DIB import accepts INFO/V4/V5 headers with uncompressed 16/24/32-bit truecolor,
  including contiguous bitfields with at most 8 bits per channel. Wider fields
  such as 10:10:10:2 require explicit upstream depth conversion and return
  `DepthConversionRequired`; adjacent source samples are never silently merged.
  RGB24 row padding, top-down/bottom-up rows,
  RGB555/RGB565 and explicit alpha masks are handled. A BI_RGB 32-bit fourth byte
  is reserved and therefore opaque. Palettes, RLE, JPEG/PNG-compressed DIBs,
  calibrated non-sRGB color and linked external ICC paths are explicitly refused.
  Embedded profiles pass the shared raster ICC admission guard before parsing.
  Plain INFO headers require the explicit untagged-sRGB assumption.

No API monitors or reads the clipboard on startup. No payload, profile, title,
path or handle is logged. Delayed-rendering `GetClipboardData`, filesystem calls
and codecs are synchronous native work and cooperatively cancellable around the
call; they never run on the UI thread. A Windows clipboard lock wait is bounded
to 500 ms; a handoff request has a 30-second cooperative deadline.

## Drag files and ownership

`stage_drag_png(source_path, binding, operation)` reads a completed private export
file, copies/hashes it in 64 KiB chunks, checks its embedded binding, then validates
PNG rows with a 16 MiB decoder allowance and the shared ICC guard. It supports up
to 50 million pixels / 512 MiB encoded without retaining a full decoded frame.
Row/metadata limits can still refuse extreme layouts. The caller's source is
never renamed, overwritten or deleted.

The return value is an owned `DragFile` object. `receipt()` returns the staged
absolute path, source/revision binding, dimensions, byte count, lease ID and
retention deadline. Desktop offers that path as an AWT file-list drag and keeps
the `DragFile` alive through the gesture. It then calls `release()` and destroys
the generated object; release is idempotent and nonblocking. Dropping a native
future/result also releases its lease. `HostService.release_drag_file(id)` is an
alternate explicit release API. Service shutdown invalidates remaining handles.
The worker reaps released leases; an active Windows image handle denies write
and delete while allowing readers. The caller must await cancellation settlement
before deleting its own input staging file.

Files live only under `%TEMP%\VisualWorkbench\drag-v1\drag-{id}\image.png`.
Each service has at most 16 active leases. The shared root admits at most 256
handoff entries and 1 GiB of retained image/ownership bytes. A persistent empty
`admission.lock` serializes quota checks, staging and cleanup across services and
processes; it is not a handoff entry and is never deleted. Stage lock waits honor
the request's cancellation and deadline, while explicit cleanup waits at most
500 ms. The quota reserves both the image and owner marker before writing them.
A released file stays until its 24-hour retention deadline,
so receiving apps can open paths after a drop callback returns.
`cleanup_drag_files()` and subsequent staging remove expired, unlocked handoffs.
A process crash closes its exclusive lease lock; a later cleanup can reclaim
the expired file. Active leases in other processes are skipped. Cleanup checks
the versioned owner marker, exact child names, absolute containment and every
ancestor for symlinks/junctions; Windows directory handles prevent ancestor
rename/delete during use. It never recurses. Unknown, malformed, linked or
unexpected entries remain intact and are counted as retained; an unrecoverable
partial owner marker may therefore need deliberate owner cleanup later.

The backend prepares a real PNG file; it does not synthesize mouse gestures,
target another app, publish CF_HDROP automatically, or send an image anywhere.
Actual file-list drag initiation, UI choices and third-party acceptance belong
to the desktop integration.

## Verification boundary

Private backend tests cover real PNG decoding at 8/16 bits, malformed/truncated
data, encoded/pixel limits, off-caller execution, exact-byte handoff, cancellation,
future drop, queue/byte budgets, native resource lifetime, failure after publication
starts and worker-count bounds. A Windows test reads real DPI using hidden owned
helpers and never writes the clipboard. No normal test mutates the user's clipboard.

Source delivery has not run those tests. Root owns serialized build, generated
binding checks and actual Windows clipboard/paste verification. A mock publication
receipt is not evidence of OS or target-application clipboard acceptance.

`handoff_tests.rs` adds injected clipboard tests and private temp-root filesystem
tests for export binding/CRC checks, dual-format partial publication, cancellation,
PNG/DIB provenance, known-answer row orientation/padding/bitfields/alpha, explicit
16-bit and color choices, malformed headers/profiles, file leases, dropped results,
active-count limits, concurrent shared-root byte/entry quotas and orphan cleanup.
No test invokes a real clipboard read or
write, initiates a drag, or opens another app. All image content is generated.
These additions have not been compiled or executed in this worktree.

Integration must use the root's reviewed `vw-raster` ICC admission repairs;
older parser code did not bound aliased localized-text expansion. Desktop UI,
generated bindings, actual PNG/DIBV5 clipboard compatibility, file drag and the
owner's Claude Code paste remain unverified. Those are T1.11 acceptance gaps.

## Dependency and platform sources

UniFFI is the parent-approved exact `0.32.2` (MPL-2.0), with default features off.
Existing exact `windows-sys 0.61.2`, `tokio 1.53.1` and `thiserror 2.0.21` pins are
reused. T1.11 promotes the existing `png 0.18.1` test dependency to production for
bounded lossless DIB normalization/row validation. It reuses approved exact
`blake3 1.8.7` (std/pure), `serde 1.0.229`, `serde_json 1.0.151`, `fs2 0.4.3`, and
local `vw-model 0.1.0`; test fixtures reuse `moxcms 0.8.1` and `tempfile 3.27.0`.
`vw-raster` remains the shared pixel/ICC validator. Root owns the lock and license
gate; no dependency was downloaded by this source implementation.

- [UniFFI 0.32.2 API](https://docs.rs/uniffi/0.32.2/uniffi/)
- [UniFFI asynchronous execution model](https://mozilla.github.io/uniffi-rs/next/internals/async-overview.html)
- [Windows SetClipboardData ownership rules](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setclipboarddata)
- [Windows clipboard operations](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-operations)
- [GetDpiForWindow awareness semantics](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforwindow)
- [Thread-scoped DPI context](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setthreaddpiawarenesscontext)
- [BITMAPV5HEADER and packed embedded profiles](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapv5header)
- [Windows standard clipboard formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/standard-clipboard-formats)
