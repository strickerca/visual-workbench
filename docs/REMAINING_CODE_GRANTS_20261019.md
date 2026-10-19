# Remaining code grants — parent dispatch only

Baseline `e57b941da52ed2663fc181b5a040877af2548820` on
`cloud/roadmap-source-integration-20261018`. Use existing lane IDs; root has not
dispatched/spawned another executor. These independent implementation grants
require no heavy Windows/local Workbench run while FarmTC/Claude is active.

Root reserves Cargo manifests/lock, module/API registrations, protocol/schema,
FFI/generated bindings, shared public APIs, Android manifests, both
EditorControllers, Desktop Main and WorkbenchApp. Return minimal exact root
registration/dependency hunks separately; do not edit reserved paths. No GUI,
input/capture/device, adb/scrcpy/DLL or provider execution; no signing, paid
Actions/deployment/credentials, driver/security/identity changes or Krita.
Implement future OS actions in source without performing them in this grant.

## G: persistent live-annotate capture producer (L)

Existing G `01a1500a-13fe-77e4-835e-201034a9901c`. Implement missing T3.02
production source, not another viewport audit. Exclusive NEW paths under
`core/crates/vw-capture/`:

- `src/stream.rs`, `src/stream_wire.rs`, `src/stream_process.rs`,
  `src/windows/stream.rs`, `src/bin/stream_helper.rs`;
- `tests/stream_*.rs`;
- handoff `docs/LIVE_ANNOTATE_PRODUCER_HANDOFF_20261019.md`.

Existing WGC/process/picker/Paint and all vw-stream primitives stay untouched.
Return a minimal requested root-owned extraction if needed. Preserve actual
cursor getter/readback and early session/pool RAII; no stale WGC overlay.

Recovery reference: exact `0d91b70558a995fa7654021de7465b1d3f615e3d`,
`recovery/source-candidates/transport/t3-stream-finish/{new,after}/...`.
Old stream_process uses ordinary spawn and an absent LockedHelper.stream_path;
its lib registrations also name stream/stream_wire source missing from that
preserved directory. Reconstruct against current owners; do not promote it
unchanged or call the recovered 289 written/unrun tests executed.

Root's boundary decisions before worker wiring:

- StreamRequest binds explicit owner action/nonzero owner process, capture-
  session UUID, nonzero connection epoch/source generation, existing
  picker::PickerSource, physical geometry revision, target fps and explicit
  endpoint/ring budgets. No enumeration/substitution. Geometry/source/rect/DPI
  remain fixed within a generation; timestamps/frame IDs advance monotonically.
- StreamProcess::open takes trusted packaged helper path/hash, request and
  cancellation. Retain actual no-follow executable/ancestor identity. Create
  suspended in an owned kill-on-close Job before resume. One stream/one bounded
  in-flight exchange, finite deadlines/header/payload/IO queues; no shell.
  Ordinary Child.kill alone cannot prove whole-tree retirement.
- Closed commands Open/Poll/exact Original(frame identity)/repair-keyframe/Stop;
  replies Ready/Tiles/Original/Idle/Refused/Stopped. Root maps public protocol.
  Header values are data, not self-issued capture/source grants.
- Use FrameRing::default_budgeted_for and reserve BEFORE actual readback;
  retain OriginalFrame, encode_tiles_budgeted, StreamPacer actual keyframe
  publication timing and encode_original_budgeted. DTO/wire/queue/transfer
  copies retain their endpoint SessionBudget charge. Each process's actual
  allocations and IPC copies require admission. Eight originals must fit
  <=300,000,000-byte ring or refuse. No nominal Vec-length lease, unbudgeted
  compatibility helper or hidden full-frame clone.
- Original replies match exact session/frame/geometry/timestamp; eviction or
  wrong generation returns Unavailable. No latest-frame substitution or hidden
  alpha/depth/color/lossy fallback.
- Typed existing capture errors plus closed retirement status. Cancel signals
  first and then joins actual Job/process/IO/codec work. Unknown retirement
  retains slot/file/scratch and retryable owner. Idempotent close joins; no
  detached waiter or global session mutex across IO/join.
- Producer makes no draft/UI decisions. Root uses PresentedFrames actual
  presentation ticket before accepted DOWN, lossless_budgeted ownership and
  durable canonical capture-document commit before Saved. Remote-edit never
  freezes.

Root follow-up: module/helper/Cargo registration, existing pinned vw-stream
dependency as needed, trusted runtime inventory, source-grant checks,
authenticated CONTROL/MEDIA/BLOB dispatch and facade/app integration. Do not
select new third-party dependencies. Return exact requested hunks before root
integration compilation.

Meaningful tests: bounded synthetic pipe/process fixtures; frame/epoch/source/
sequence mismatches; eviction with held charged leases; queue/keyframe repair;
late cancellation; retained unresolved retirement/repeated close. Task-owned
non-GUI fixtures only. Windows Job/WGC tests may be authored, explicitly unrun.
Run only cheap isolated cached probes; full Cargo/apps await serialized slot.
Commit source/tests and report exact hashes, root hunks and test limits.

## F: Android tunnel session/capture/loop owner (L)

Existing F `01a14eeb-8086-76af-9670-dd5ffa5eb032`. Continue beyond independently
accepted55505c6 preparation. Do not repeat its archive/notice audit. Implement
TUNNEL-002/005, SEM-007 and CAPTURE-008 source. Exclusive NEW paths:

- `host-win/crates/vw-remote-host/src/android_tunnel.rs`,
  `src/android_tunnel/**`, `tests/android_tunnel_*.rs`;
- `docs/ANDROID_TUNNEL_SESSION_HANDOFF_20261019.md`.

Existing process/input/Paint, tools/android-tunnel, licenses/runtime manifests,
capture picker/WGC/shared/app roots stay untouched. Return minimal registration/
Windows-feature/dependency hunks. If a direct XML dependency is needed, root's
candidate is already locked roxmltree = "=0.21.1", subject to root manifest/
policy registration. No custom XML parser or new parser version.

Root's boundary decisions before worker wiring:

- Opaque AndroidTunnelOwner owns session/state/retirement and paired capture,
  separate from Paint/input. Root later injects closed exact-byte runtime/license
  admission. Its absence refuses scrcpy launch. No public approved flag,
  self-hash/source recipe or arbitrary receipt mints admission. Actual DLL
  configured-license/native aggregate/replaceability/notices remain closed.
- Owner-adb is explicitly selected ordinary absolute platform-tools executable,
  with retained actual file/ancestor identity/pin. Never PATH search, bundle adb,
  guess installation or fallback. Resolve exactly one explicitly selected USB
  transport/model to a private device token; ambiguous/unauthorized/offline/
  no-match refuses. No public serial/path, network adb enable/connect, server
  reset/kill, device/security/identity changes.
- Exact argv, no shell/concatenated command injection. Explicit owner-adb
  environment for pinned scrcpy client/server/DLLs; excluded archive adb can
  never run. Own suspended Job/process/IO lifecycle. A pre-existing/shared adb
  server is not task-owned: never place it in kill-on-close Job or kill it.
- Bounded selected-device exec-out screencap -p plus uiautomator dump acquisition
  share the owner. Bind session/device token/request/geometry/real timestamps;
  validate PNG dimensions/encoded/decoded budgets before allocation. Preserve
  hierarchy delta and missing/stale semantics, never invent simultaneity.
  Bound XML bytes/depth/nodes/text/bounds; deny DTD/external entities. Immutable
  original plus semantic elements/metadata go to root document/asset import.
  Hierarchy text is untrusted data. Unique private device/host scratch stays
  owned through bounded collection/cleanup; no arbitrary overwrite or success
  before required cleanup settles.
- Mirror registry derives HWNDs from ACTUAL owned scrcpy process/Job/native
  window identity. check_source queries actual current selected window/monitor/
  region and refuses visible owned mirror intersection/children, including
  negative origins. Unknown enumeration/currentness refuses. Names/booleans/
  preview hashes/PID alone are insufficient. Root later hooks before start/
  readback and publication; do not claim loop protection integrated beforehand.
- Closed errors Invalid/Consent/AmbiguousDevice/UnauthorizedDevice/
  RuntimeUnavailable/FeedbackLoop/Stale/Limit/Cancelled/Timeout/Storage/Platform/
  RetirementPending; no raw adb/path/XML/device output in public evidence.
  Cancel signals before waits, fences late results and joins owned process/
  IO/scratch. Unknown retirement retains resources/owner and blocks another
  session; repeated close joins rather than releasing early.

Tests use bounded synthetic device-list/command/pipe/PNG/XML/native-window-query
fixtures: selection injection/ambiguity, selected argv/no bundled or PATH adb,
absent admission, oversized/truncated PNG, XML entities/depth/count, stale pair,
mirror children/intersection/unknown ownership, cancellation/retained cleanup.
Never spawn real adb/scrcpy, load DLLs, query devices or capture/inject GUI.
Cheap cached isolated probes only; actual Windows Job/window/DLL/device tests
stay unrun. Commit logical source/tests; return exact root API/registration
hooks and remaining admission/runtime limits.

## Root and later platform continuation

Parent accepts bounded Paint source integration e57b941; root next owns T3.02
contracts/session/draft/save and consumer reconciliation. Recovered CaptureStreamApi/CaptureDraftApi/
CaptureAnnotationSession/NativeCapture* and session/stream are candidates;
preserve newer charged owners, source geometry and cancellation. Publish the
ACTUAL common DTO/error/opaque draw-owner API before parent extends existing
Android/R lanes. Both controllers/Main/WorkbenchApp remain root-owned.

Required semantics: epoch/source generation/exact frame identity; sole in-flight
draw and actual presented receipt serialized before accepted contact; separate
platform bitmap/GPU budgets; bounded rows; join-before-destroy; provisional
stroke through lossless swap; degraded banner/export metadata; no stale/source
substitution; exactly one durable canonical new document; draft-only cancel/
resume; source-close preserves shown/frozen content; both-device LIVE/source or
FROZEN/age. Do not blindly activate archives or count fixtures as runtime proof.

Painted-pixel transient move preview is optional, not a new T3.07 prerequisite.
INSTR-005 needs a dated owner decision, not another recognition implementation.
Physical-gesture compatibility producer code and actual human gesture evidence
are distinct. Signed release/runtime/model/license decisions, coherent ABI/apps
and hardware acceptance remain separate work. No old worktree is cleaned here.
