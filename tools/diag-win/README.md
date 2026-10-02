# Windows diagnostics

Private Rust tool, exact windows crate pin 0.62.2, MIT OR Apache-2.0.
Build from the repository root through the license-gated entry point:

    powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-core
    target/debug/diag-win.exe --out docs/evidence/win-diagnostics.json

The process sets Per-Monitor-V2 awareness before any DPI/display/COM API.
Monitor JSON reports physical bounds, effective DPI, scale, rotation, refresh
and advanced-color flags. The legacy advanced-color packet does not distinguish
HDR from wide-color state; gamut/calibration is explicitly untested. Hardware
MFT activation counts are registrations, not successful encoding/decoding,
measured throughput or proof of all GPU/DXVA decoder paths.

Capture through the bounded wrapper (output must have an existing parent outside
the repository):

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/diagnostics/capture-window.ps1 -WindowTitleSubstring "unique synthetic window" -OutFile "<owned-temp-directory>/capture.png"

The native CLI is diag-win --screenshot <title-substring> <out.png>.
Selection must identify exactly one visible non-minimized window. PrintWindow
captures only that HWND; DWM physical bounds remove invisible frame borders.
No screen-copy fallback is used. Existing outputs and repository paths are
rejected. Entirely black captures fail; nonblack output still requires inspection
for the renderer under test. GPU/Compose/browser compatibility is not certified
by the two synthetic WinForms captures. PrintWindow can block, so use the wrapper
with its deadline and contained process cleanup.

Capture memory is zero-initialized and GdiFlush completes drawing before direct
pixel access, following the [CreateDIBSection synchronization contract](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-createdibsection).
The [MFTEnumEx contract](https://learn.microsoft.com/en-us/windows/win32/api/mfapi/nf-mfapi-mftenumex)
defines the hardware-registration filter and owned activation array.

Screenshot files are temporary: inspect, record physical dimensions/DPI/scale,
hash/source binding and result, then dispose the task-owned images before closure.
Do not place them in Git. T0.03 tested 900x600 physical-pixel synthetic windows on
168 DPI (1.75 scale) and 96 DPI (1.00 scale) displays; both PNGs matched the
declared quadrant colors after independent CRC/zlib decoding.
