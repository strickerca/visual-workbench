# Hardware truth — T0.03

Collected 2026-10-01. These are capability/snapshot facts, not product acceptance.
The first 21 rows preserve BUILD_SPECIFICATION §2.1's row names and order.
Secure Boot is owner-confirmed True from administrator PowerShell.
The connected IN2019 is a shared startup device; no foreground/app/reset action was taken.
All S23/S Pen observations remain untestable until the actual target is available.

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
