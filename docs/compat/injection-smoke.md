# Windows injection compatibility — T0.05

Inventory date: 2026-10-02. No editor injection has been run. These are pending
measurement rows, not negative compatibility findings. Software test fixtures
cannot establish editor behavior. The OnePlus is reserved by another project;
S23 physical work is owner-deferred.

| Target | Local version/inventory | Pointer | Pressure | Tilt | Eraser | Evidence / next check |
|---|---|---|---|---|---|---|
| Native T0.05 harness | workspace 0.1.0; Windows 11 build 26200; DPI 168 (175%) | Partial: 472 native samples; eraser lifecycle differs | Yes: 65 ramp pairs, Pearson r = 1.000 | Yes: both axes -60 to +60 degrees | No for the tested combined INVERTED + ERASER flags; editor semantics untested | [T0.05 evidence](../evidence/T0.05.md); 469/472 coordinate/lifecycle matches; rotation and barrel matched; maximum normal contact interval 26.480 ms |
| Paint | Microsoft.Paint 11.2605.81.0 | Not tested | Not tested | Not tested | Not tested | Installed Appx inventory; disposable canvas and brush selection needed |
| Krita | Not found in HKCU/HKLM uninstall inventory | Not tested | Not tested | Not tested | Not tested | Owner approval before install; choose Windows 8+ Pointer Input |
| GIMP | Not found in HKCU/HKLM uninstall inventory | Not tested | Not tested | Not tested | Not tested | Owner approval before install; record brush/input configuration |
| Photopea / Edge | Edge 154.0.4258.48; Photopea build not queried | Not tested | Not tested | Not tested | Not tested | Use blank disposable document; record web app build/date and brush |
| Photopea / Chrome | Chrome 154.0.8037.97; Photopea build not queried | Not tested | Not tested | Not tested | Not tested | Chrome already installed; separate browser measurement |
| Elevated Notepad | Not launched | Not tested | Not applicable | Not applicable | Not applicable | Requires owner UAC confirmation; guarded refusal and raw UIPI are different observations |

The inventory reads `DisplayName` and `DisplayVersion` from Windows uninstall
keys plus the Microsoft.Paint Appx package. Portable/manual installations may
not appear in those keys. No package was installed or changed.

The native result applies to the exact recorded script and host. It does not
establish editor compatibility or universal support for individual eraser flags.
Combined flag value 6 arrived as 0 and changed lifecycle delivery around the
eraser row. The comparator intentionally reports failure for those discrepancies.
During a separate 1500 ms no-refresh experiment, Windows delivered an automatic
UP 500 ms after DOWN, without POINTER_FLAG_CANCELED; the next update became DOWN.

For each later editor run, retain the exact version, brush settings, physical
client geometry, DPI, command/source binding, receiver or visual observations,
and separate yes/no/partial results for each field. A visible line does not prove
pressure or tilt. Record elevated-window guard refusal separately from the
unmeasured behavior of the raw injection API.

Screenshots: zero created for this slice. Future verification images must be
outside Git, cropped to the test window, inspected and disposed before closure.
Retain their text hashes/counts and disposal receipt, not image files.
