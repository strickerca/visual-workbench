# F tunnel owner, paired capture and mirror guard — source handoff

Base: `6ae93d495fdfffd511f1e882facb80aaa0ab2dd4`. This candidate adds only
the F-granted new host module/tests and this handoff. It does not change the
accepted `55505c6` release preparation, root manifests, existing host process,
shared contracts, app/controller files, or the license/runtime admission state.

## Owned behavior

`AndroidTunnelOwner` admits one session at a time and requires an injected
`RuntimeAdmission` from root. Missing admission returns `RuntimeUnavailable`
before any executable launch. Its runtime value has no public constructor or
approval flag. Root must construct it only after checking the actual retained
scrcpy/FFmpeg/libusb/SDL bytes and aggregate legal/build/replaceability/notices
policy; hashes and a source recipe alone cannot open the gate.

An owner selects one absolute `platform-tools/adb.exe` and an explicit USB model
and nonzero transport ID. The parser refuses network serials, malformed,
ambiguous, unauthorized, offline, mismatched and missing selections. The selected
private serial is only used inside exact argv; external capture results carry an
opaque session-bound token. A new `devices -l` census is required around each
capture phase and before device scratch cleanup. Windows pins the actual adb
file/ancestors and refuses the excluded official ZIP adb digest. It never uses
PATH resolution or invokes adb server reset/kill/connect. Each adb client runs
in its own one-process kill-on-close Job, so a new adb server child cannot be
admitted; a pre-existing shared server is not attached to this Job.

Windows pins the admitted client/server/six DLL files, hashes their actual
opened bytes, and launches scrcpy with an exact `--serial`, `--no-audio`,
`--no-control` argv and explicit `ADB`, `SCRCPY_SERVER_PATH` and PATH
environment. CreateProcess is suspended, assigned to a kill-on-close Job, then
its opened image/ancestors are checked before resume. Unknown startup
retirement retains the process handle for a later retry. Bounded adb stdout
collection runs in one joined IO worker per command; a timeout/cancel kills
only its owned Job. Whole-tree exit and joined IO are required before release.

The paired operation collects the original bounded PNG through
`exec-out screencap -p`, validates its complete chunk envelope/CRC, physical
dimensions and <=64 MiB RGBA decode budget, and records actual command start/
finish time. It atomically creates a session-specific `/sdcard/vw-.../`
directory, runs `uiautomator dump` inside it, then reads bounded XML through
`exec-out cat`. XML uses pinned roxmltree with DTD disallowed and limits of
2 MiB, 10,000 nodes, depth 32, per-field 4 KiB and aggregate semantic text
128 KiB. Element bounds must fit the screenshot. Missing hierarchy remains
`Missing`; >2-second source/hierarchy delta becomes `Stale`, never an invented
simultaneous result. Every branch that may have created scratch must verify its
known-file removal and directory removal before returning a successful pair.
Unknown cleanup retains the owner,
blocks a new session and retries on close/re-entry.
If `mkdir` has an uncertain outcome, the owner retains that path without
deleting it; an absence query can settle it, while a present directory needs
owner reconciliation before release. This prevents cleanup from deleting a
pre-existing or contested device path.

The later review repairs retain an in-flight IO reader/Job until it is retired
and joined before any device cleanup command. Scratch cleanup tracks file
removal, directory removal and final absence separately; a failed absence
query after successful `rmdir` retries only the query. Uncertain mutation
responses never trigger a destructive retry against a potentially contested
path. The suspended-launch unresolved-child queue is included in the public
retirement count and close/unload fence. Hierarchy freshness uses ordered
image/dump/XML command intervals and completed acquisition time, so a ten-second
dump cannot appear fresh because it began ten milliseconds after the image.

`check_source` queries the current selected native window/monitor/region and
enumerates windows owned by the actual retained scrcpy process handle. It
checks liveness/PID before and after enumeration, rechecks a selected window's
geometry, and refuses any visible owned-mirror intersection or child relation,
including signed negative desktop coordinates. Missing mirror windows or
unknown enumeration refuse. Root must call this guard before actual capture
start/readback and before publication; this source alone does not wire callers.

## Exact root changes requested

1. In `host-win/crates/vw-remote-host/src/lib.rs`, register
   `pub mod android_tunnel;` once the gate and caller are ready.
2. In the root-owned host Cargo manifest, add direct locked
   `roxmltree = "=0.21.1"`. Existing `ring` and `blake3` are reused. Add
   `Win32_Graphics_Gdi` and `Win32_UI_WindowsAndMessaging` to the existing
   `windows-sys` feature list for native mirror enumeration.
3. Implement `RuntimeAdmission` only in the root-owned exact-byte policy
   module. Construct `AdmittedRuntime::from_root_admission` after actual DLL
   configured-license, native aggregate, dynamic replaceability and notices
   admission; do not use a public receipt or caller-supplied hash as authority.
4. Bind owner UI consent and fresh session UUID/source generation to
   `AndroidTunnelOwner::open`. Pass only explicit platform-tools adb and USB
   selection. Consume `CapturePair` into immutable original + semantic asset
   import, retaining its actual timing/missing/stale state. Route `check_source`
   before the capture start/readback/publication points. Close and retry pending
   retirement before session replacement or native unload.

## Verification boundary

The authored synthetic Rust fixtures cover exact device selection and argv,
missing admission, bounded/truncated PNG, XML entities/depth/bounds, signed
mirror intersection/unknown census, paired command order and repeated close.
No Rust toolchain is present in this execution environment, so those tests,
rustfmt, Windows compilation and actual Job/window/device/DLL behavior were
not run here. Only repository/static checks can be recorded for this source
checkpoint. Do not treat the source as a passed runtime or product gate.

The Windows path, scrcpy v4.1 argv/environment, adb server noninterference,
device scratch cleanup, OS window ownership, real PNG/XML fidelity, packaging,
and root caller hooks require later authorized Windows/device validation.
The current scrcpy launch uses `--no-control`; it does not implement the
TUNNEL-005 phone-control path. That remains a separate code gap, not a tested
or accepted control feature.
Krita, paid services, credentials, deployment and GUI/device actions were not
used for this candidate.
