# Hardware truth — T0.03

Collected 2026-10-01. These are capability/snapshot facts, not product acceptance.
The first 21 rows preserve BUILD_SPECIFICATION §2.1's row names and order.
Secure Boot is owner-confirmed True from administrator PowerShell.
The connected IN2019 is a shared startup device; no foreground/app/reset action was taken.
At this initial snapshot, S23/S Pen observations were unavailable. The table is
dated historical evidence; later updates below record S23 diagnostics/codec tests
and the owner's latest device instruction. No physical S Pen acceptance is implied.

PC command: powershell -NoProfile -ExecutionPolicy Bypass -File tools/diagnostics/pc-diagnostics.ps1 -OutFile docs/evidence/pc-diagnostics.json -OwnerSecureBootAnswer True

Phone command: powershell -NoProfile -ExecutionPolicy Bypass -File tools/diagnostics/phone-diagnostics.ps1 -OutFile docs/evidence/phone-diagnostics.txt

Windows command: target/debug/diag-win.exe --out docs/evidence/win-diagnostics.json (bounded, 30 s).

| Item | Value | Status / reason | Source command or API |
|---|---|---|---|
| PC model | HP Spectre x360 Convertible 15-eb1xxx | verified 2026-10-01 | Win32_ComputerSystem.Model |
| CPU / RAM | 11th Gen Intel(R) Core(TM) i7-1165G7 @ 2.80GHz; 4 cores/8 threads; 16801923072 usable bytes (15.65 GiB), nominal 16 GB | verified 2026-10-01 | Win32_Processor; Win32_ComputerSystem.TotalPhysicalMemory |
| Windows | Microsoft Windows 11 Home 25H2; build 26200, revision 9457 | verified 2026-10-01 | Win32_OperatingSystem; CurrentVersion registry |
| Memory Integrity | On, running; configured registry=1 | verified 2026-10-01 | Win32_DeviceGuard.SecurityServicesRunning contains 2 |
| Secure Boot | True; owner-confirmed, not a successful elevated query by this agent | verified 2026-10-01 | Owner administrator PowerShell: Confirm-SecureBootUEFI |
| Smart App Control | off | verified 2026-10-01 | CI Policy.VerifiedAndReputablePolicyState=0 |
| Graphics driver | Intel Iris Xe; version components major 32, minor 0, branch 101, revision 7088 | verified 2026-10-01 | Win32_VideoController.DriverVersion |
| Display | Monitor 1: 3840x2160, DPI 168, scale 1.75, 60.004 Hz, rotation 0; monitor 2: 1920x1080, DPI 96, scale 1.00, 119.998 Hz, rotation 0. Advanced color off/on respectively; gamut and distinct HDR classification untestable without calibration/newer packet | verified 2026-10-01 (physical geometry/DPI/refresh/advanced-color flags); gamut/HDR distinction untestable | diag-win: EnumDisplayMonitors, GetDpiForMonitor, QueryDisplayConfig, DisplayConfigGetDeviceInfo |
| Free disk C: | 51597021184 bytes (48.05 GiB) at snapshot; storage warning waived by owner | verified 2026-10-01 | Win32_LogicalDisk C: |
| Creative apps | No matching supported creative-app install in enumerated classic uninstall keys or current-user Store packages; portable/web/manual installs not surveyed | verified 2026-10-01 (enumerated install sources only) | Uninstall registry keys; Get-AppxPackage filtered supported names |
| Paint | Paint version components 11 / 2605 / 81 / 0 | verified 2026-10-01 | Get-AppxPackage Microsoft.Paint |
| HEIF/HEVC extensions | HEIF and HEVC installed for current user; AV1/WebP/Raw also present; processing not tested | verified 2026-10-01 | Get-AppxPackage extension-name allowlist |
| adb | 37.0.1 | verified 2026-10-01 | adb.exe version; only numeric platform-tools version retained |
| Phone | Product target: Galaxy S23 Ultra; actual connected startup device: OnePlus IN2019 | untestable: target Galaxy S23 Ultra is not connected; IN2019 startup evidence does not verify the target | phone-diagnostics.ps1 (selected physical device, read-only adb probes) |
| Phone OS | S23 OS/build untestable; actual IN2019 Android 11, API 30, build RP1A.201005.001, security patch 2023-02-01 | untestable: target Galaxy S23 Ultra is not connected; IN2019 startup evidence does not verify the target | adb shell getprop selected software fields |
| S Pen pressure | S23 pressure levels/normalization untestable; connected IN2019 listing has no BTN_TOOL_PEN device | untestable: target Galaxy S23 Ultra is not connected; IN2019 startup evidence does not verify the target | adb shell getevent -lp; axes bound to BTN_TOOL_PEN block |
| S Pen tilt / orientation | ABS_TILT_X / ABS_TILT_Y and orientation on S23 untestable; no pen device on connected IN2019 | untestable: target Galaxy S23 Ultra is not connected; IN2019 startup evidence does not verify the target | adb shell getevent -lp; T0.04 physical traces pending |
| S Pen hover | ABS_DISTANCE and hover behavior on S23 untestable; no pen device on IN2019 | untestable: target Galaxy S23 Ultra is not connected; IN2019 startup evidence does not verify the target | adb shell getevent -lp; T0.04 hover traces pending |
| S Pen report rate | S23 report rate untestable without historical MotionEvent/physical traces | untestable: target Galaxy S23 Ultra is not connected; IN2019 startup evidence does not verify the target | T0.04 recorder and timestamp analysis pending |
| Air Actions | S23 Air Actions and barrel/Air Command interaction untestable on IN2019 | untestable: target Galaxy S23 Ultra is not connected; IN2019 startup evidence does not verify the target | T0.04 declarative KeyEvent/owner actions pending |
| Phone video decode | S23 hardware decode, low latency and 4:4:4 paths untestable. IN2019 vendor XML declares AVC/HEVC/VP9 and software AV1 names; declarations do not prove hardware execution | untestable: target Galaxy S23 Ultra is not connected; IN2019 startup evidence does not verify the target | adb shell cat /vendor/etc/media_codecs*.xml; MediaCodec processing probes pending |
| Actual startup phone | OnePlus IN2019; Android 11/API 30; RP1A.201005.001; patch 2023-02-01; starter 0.0.1-dev | verified 2026-10-01 | phone-diagnostics.ps1 (selected physical device, read-only adb probes) |
| Actual phone display | 1080x2400 physical, density 450; declared 60 and 90 Hz display modes | verified 2026-10-01 (static mode listing) | adb shell wm size; wm density; dumpsys display numeric fields |
| Actual phone pen axes | Zero BTN_TOOL_PEN devices in readable listing; ABS_PRESSURE/DISTANCE/TILT_X/TILT_Y are not established for any pen | verified 2026-10-01 (static listing only) | adb shell getevent -lp; pen-scoped parser |
| Phone temperature snapshot | 28.4 C; battery 100%; no ten-minute baseline | verified 2026-10-01 (snapshot only) | adb shell dumpsys battery |
| Wi-Fi | 5 GHz; 802.11ax; channel 44; no network identifiers retained | verified 2026-10-01 | netsh wlan show interfaces; only band/radio/channel retained |
| Developer Mode | On | verified 2026-10-01 | AppModelUnlock.AllowDevelopmentWithoutDevLicense=1 |
| Documents cloud sync | True (path withheld); provider configuration/in-flight sync not assessed | verified 2026-10-01 (folder path classification) | Environment.GetFolderPath(MyDocuments); boolean only |
| USB controller | Intel(R) USB 3.10 eXtensible Host Controller - 1.20 (Microsoft) | verified 2026-10-01 (controller inventory, not negotiated phone speed) | Win32_USBController.Name |
| Test signing | untestable: bcdedit unavailable without elevation | untestable: bcdedit query not elevated | bcdedit /enum {current} exit=1; raw output withheld |
| Windows hardware encoder registrations | H.264 count 2; HEVC count 2; duplicate friendly names retained as separate registrations; throughput/profile/latency untested | verified 2026-10-01 (enumeration only) | MFTEnumEx with MFT_ENUM_FLAG_HARDWARE; output subtype filter |
| Windows hardware decoder registrations | H.264 count 0; HEVC count 0 under this hardware-MFT filter; this does not establish absence of DXVA/GPU decode | verified 2026-10-01 (enumeration only) | MFTEnumEx with MFT_ENUM_FLAG_HARDWARE; input subtype filter |
| S23 thermal/recovery baseline | No S23 thermal session or carrier recovery measurements | untestable: target phone and implemented probes/transport unavailable | T0.04 thermal; T0.06 recovery; PERF-template.md |

Media Foundation names: Intel Quick Sync H.264 Encoder MFT and Intel Hardware H265 Encoder MFT.
Two registrations each are reported; friendly-name duplicates were not erased.
The hardware flag's zero decoder counts do not establish absence of OS/DXVA decoder paths.
Advanced-color support is true on both displays; enabled is false/true; wide-color-enforced is true/false; 8 bits/channel are reported.
Distinct HDR classification and gamut/colorimeter calibration are explicitly untested.

Evidence: pc-diagnostics.json, phone-diagnostics.txt, win-diagnostics.json and T003_SCREENSHOT_VERIFICATION.json.
Synthetic screenshot PNGs were inspected and disposed; only text hash/pixel/cleanup receipts remain.
No display device names, network names/addresses, input device paths/names or serials are retained.

## T0.04 update — 2026-10-01

The separate pen probe, private JSON recording, replay instrumentation and thermal
logger are implemented. The connected test device remains the OnePlus IN2019;
injected stylus events are software verification only. Zero S23 owner traces and
zero ten-minute drawing sessions have been recorded in this task. Samsung
pressure, hover, tilt/orientation signs, report rates, barrel/eraser behavior,
Air Actions and Air Command interference remain unmeasured, as listed in T0.04.md.
They are not classified as “not reported by device” without a physical S23 test.

## Historical authorized S23 update, 2026-10-02

At this checkpoint the S23 was connected and authorized for this project. Earlier
OnePlus entries remained historical startup evidence for that run.
Read-only S23 diagnostics reported SM-S918U, Samsung, Android 16/API 36,
build BP4A.251205.006, security patch 2026-09-05, One UI raw value 80500 and
chipset identifier kalama. `wm` reports 1080 x 2316 rendering size and density
450; declared display modes also include 1440 x 3088 at approximately 120, 96,
60, 48, 30, 24 and 10 Hz. Rendering size is not a claim about physical panel
capability. No display mode was changed.

The static input parser returned zero pen-device entries. This is inconclusive,
not evidence that the S23 lacks a pen. Physical pressure, hover, tilt, buttons,
Air Actions and the ten-minute thermal baseline still need T0.04 owner input.
The one-time battery temperature was 32.5 degrees C; it is not a sustained-load
baseline. Video decode was then pending; T0.07 subsequently measured it as below.

The owner requested charging stay-awake. `svc power stayon true` was applied;
`stay_on_while_plugged_in` read back 15, with `mWakefulness=Awake` and
`mStayOn=true`. The previous value (7) is retained locally. Keep this setting
as requested. The S23's Wi-Fi was disabled during T0.06; no USB-tether network
interface was present. See T0.06.md for the real adb TCP measurement and gaps.

Text-only diagnostic receipt: s23-sanitized-hardware-facts-678c7f1173d24aaca86f8b8a37946cfa.log.
No serial, network identifier, screen capture or other app content is retained here.

## Phase 0 reconciliation and current device scope — 2026-10-02

T0.07 subsequently ran the S23 hardware HEVC decoder for both 1440x3088 and
3840x2160 profiles (130 outputs each, including ten warmups). T0.09 ran generated
JPEG/HEIF decoder probes. These results supersede the earlier pending-codec
statements for those specific workloads only; low-latency control effectiveness,
4:4:4, sustained integrated presentation and physical S Pen behavior remain open.
See [T0.07](T0.07.md), [T0.09](T0.09.md) and [T0.12](T0.12.md) for exact results,
source bindings, retained failures and capacity boundaries.

The latest owner instruction takes the S23 away and defers checks requiring it.
The shared OnePlus IN2019 is authorized again for applicable software tests,
selected explicitly without fallback. This is the current testing scope, not a
new hardware inventory. Historical S23 stay-awake and display facts do not assert
its present connection or settings. OnePlus tests never certify S23/S Pen or
target-device performance. Physical thermal/recovery baselines remain absent.
