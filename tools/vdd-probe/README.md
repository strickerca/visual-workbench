# Virtual display diagnostic

This independent Rust client speaks the pinned SudoVDA 0.2.1 test-build ABI. It
does not include or link driver code, and installs nothing. Build through:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-vdd-probe
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test vdd
```

Default inventory enumerates only the exact SudoVDA interface class and counts
active DisplayConfig paths. It does not open the driver, send IOCTLs, or reveal
device paths, serial numbers or monitor names. A zero device count is a valid
inventory result, never installation acceptance.

After completing the driver prerequisites and reserving the desktop and entire
SudoVDA instance with the owner:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test vdd -VddScenario normal -OwnerReady
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test vdd -VddScenario watchdog -OwnerReady
```

`normal` exercises landscape then portrait at 60 Hz. It checks active physical
dimensions and rational refresh rate through DisplayConfig, sends three pings at
one-second intervals, and removes only each fresh synthetic monitor GUID. A
six-second transition deadline and the outer 90-second process-tree deadline
contain failure. A failed add attempts removal of its own GUID, including when a
malformed response prevents a usable target result. These paths require live
hardware validation; the outer timeout also contains a synchronous driver hang.

`watchdog` launches this same executable as an owned child, waits at most 15
seconds for a confirmed active target, checks that it is still active, then kills
and reaps that exact child. The parent sends no driver IOCTL while polling for
removal. It records elapsed time from before termination, including reaping.
More than four seconds is explicitly flagged; six seconds fails the bounded
transition. Return of real application windows is a separate owner observation.
Another driver's client can invalidate this test by resetting the shared timer.

The child and all compiler/test processes inherit the runner's Windows job. Do
not use raw executable invocations as an alternative to bounded execution.
Source/build hashes and redacted text receipts are retained in ignored `.local`.
There are no arbitrary device paths, dimensions, registry writes, driver installs,
test-signing toggles or certificate operations in this client.

Platform API references:
- [CM_Get_Device_Interface_ListW](https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_get_device_interface_listw)
- [QueryDisplayConfig](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-querydisplayconfig)
- [DeviceIoControl](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-deviceiocontrol)

Wire layout and malformed-response tests run without a driver. They do not prove
the runtime behavior documented above. See docs/evidence/T0.08.md for what ran.
