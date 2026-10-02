# Windows pen probe (T0.05)

This is an isolated Phase 0 experiment, not the Phase 4 remote-control service.
It has no phone, networking, mouse/keyboard injection, grant UI or persistent
input queue. S23 physical work is deferred; the OnePlus is reserved by another
project. These host tools never call adb.

## Host-only build and tests

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-pen-inject
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 test-all
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 lint-all
target/debug/pen-inject.exe --plan .local/pen-plan.csv
```

`--plan` requires a fresh output file and does not query windows, create a pointer
device, or inject input. Builds run the license policy first. The existing exact
`windows = 0.62.2` pin is reused; no new third-party packages are added.

## Recorder and scripted input

`pen-harness --out <new-directory> <10..300 seconds>` creates a PMv2 window,
without requesting foreground activation. Click it to make it the target. Escape
or the deadline closes it. `target.json` records its numeric HWND/PID/DPI and
physical client rectangle. `received.csv` logs native WM_POINTER pen histories:
coordinates, pressure, both tilts, rotation, pen and pointer flags, mask, frame,
pointer ID, Windows tick/QPC timestamps, history count and receipt time. Coalesced
history is read immediately and reversed into chronological order; overlapping
history entries are deduplicated by pointer/frame/QPC/flags. Read/write failures
and bounds exhaustion fail the recorder. Pressure controls GDI line width;
barrel input is blue and eraser input draws white. This illustrates fields, not
editor semantics. A healthy `receiver.json` is written on graceful completion.
An owned bitmap and cached GDI pens draw new segments incrementally; repainting
does not redraw the complete sample history on every pointer event.

`pen-inject --run <decimal-HWND> <PID> --owner-ready <new.csv> normal` starts with
a five-second focus countdown. The 472 planned samples cover seven separate rows:
65 contact pressures from 0 to 1024; ±60-degree tilt; 0..359-degree rotation;
barrel segment; inverted/eraser stroke; stationary contact at a scheduled 50 Hz;
and in-range hover without contact. UP keeps the last contact position. All
positions derive from the target's physical client bounds, including negative
virtual-screen coordinates. A completion receipt is written only after all API
calls and journal writes succeed. A partial CSV or empty receipt never passes.

Every API call checks target foreground, top-level identity, PID/thread/process
creation time, window and client geometry, DPI, visibility, minimization,
enabled state, integrity and hit-test root. A failed OS read, invalid point,
changed target or failed injection permanently suspends that injector. The
50 ms contact deadline is enforced; slow scheduling fails rather than claiming
20 Hz. Teardown destroys the synthetic device and sends no unguarded UP. Starting
the command again is the explicit resume action. Windows does not expose an
atomic guard-and-inject API; live race measurements remain necessary. This
local scripted session is not the later transport InputSessionId/control grant.

Sender/receiver journals use bounded 256 KiB buffers; the sender flushes between
strokes rather than performing disk writes in the contact loop. The input thread
temporarily uses at least ABOVE_NORMAL thread priority, then restores its prior
level. The process priority class, system timer policy and other processes are
unchanged. This does not guarantee real-time scheduling: the same 50 ms stop
condition is checked again after the native target guard and before injection.

## Owner-attended Windows tests

Run these only after the owner reserves the laptop's input and agrees not to
touch the keyboard or mouse during injection. They do not need a phone.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test win-pen -OwnerReady
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test win-pen -OwnerReady -WinPenScenario no-refresh
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test win-pen -OwnerReady -WinPenScenario guards
```

The runner reuses a gated build only when all bound source/manifests and both
binary hashes still match; otherwise it builds first. It starts only its own
harness process(es), validates
readiness, runs with a 90-second deadline, drains messages, closes its children,
and analyzes fresh text receipts under ignored `.local/win-pen-<id>`. It prints
progress and never retries by taking over another app. In the normal test,
the injector asks Windows to activate its own harness before the countdown.
Arbitrary editor targets still require manual focus. Windows may deny harness
activation; this is inconclusive, and the runner uses no focus bypass.
The harness cooperatively grants foreground permission only to the verified
sibling `pen-inject.exe`, never to all processes. The grant is renewed after
input when needed; Windows still decides whether activation is allowed.

The comparator pairs samples by coordinate/lifecycle and order, not by pressure
or tilt. Pressure correlation requires all 65 ramp contacts; missing samples,
field/mask mismatches, reordered samples, unexpected native events, or keepalive
misses prevent acceptance. It preserves command/receiver SHA-256 values.
The no-refresh scenario inserts 1500 ms without input during stationary contact;
it is a measurement experiment, never normal keepalive acceptance. API rejection
may leave a partial journal; inspect that and the native recorder/log together.

The guard suite runs 25 each of move, resize, minimize and focus changes, checks
the next update is rejected and that restoring the target does not resume the
old session. It restricts mutations to the two explicitly selected harness
processes and rechecks identity before changing windows. The analyzer requires
100 native baseline DOWN samples and a healthy sink with zero pen samples.
Each trial waits up to 250 ms for the native recorder to acknowledge its DOWN
before mutating the window. Only the expected geometry, minimization or focus
rejection counts; a timing miss or unrelated API error cannot pass a trial.
That observes the chosen target and sink, not every unrelated desktop window;
do not claim a global zero-stray-event proof from it.

## Editors and elevation

Before installing Krita or GIMP, obtain the owner's explicit installation
approval. For each installed editor, select a disposable blank document and a
pressure-sensitive brush, record exact app/browser version and tool settings,
then select its HWND/PID for the guarded injector. Do not target existing work.
Krita uses Windows 8+ Pointer Input. Record pointer/pressure/tilt/eraser separately;
a visible line alone does not establish pressure. Photopea in Edge and (if
installed) Chrome are separate rows. See `docs/compat/injection-smoke.md`.

The guard refuses a target with higher integrity. Demonstrating this against
elevated Notepad requires the owner to approve its UAC prompt. A guarded refusal
does not prove the raw Windows API's UIPI behavior. No bypass mode is provided.

Verification screenshots, when needed for editor testing, must be created
outside the repository with `diag-win --screenshot`, inspected, then disposed;
keep text hashes/counts and disposal receipts only. No screenshots are generated
by the probe's automated runners.

## API references

- [Synthetic pointer injection](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-injectsyntheticpointerinput)
- [Pen field ranges](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_pen_info)
- [Coalesced pen history](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerpeninfohistory)
- [Pointer device teardown](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-destroysyntheticpointerdevice)
- [Foreground permission handoff](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow)
- [Input thread scheduling](https://learn.microsoft.com/en-us/windows/win32/procthread/scheduling-priorities)
- [Pen flag constants](https://learn.microsoft.com/en-us/windows/win32/inputmsg/pen-flags-constants)

These documents guide implementation; they are not measurements on this laptop.
