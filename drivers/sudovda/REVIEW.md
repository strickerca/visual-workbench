# Pinned SudoVDA review, T0.08

Reviewed all retained source, project and INF files, plus the two locally held
EDID files. These are source observations, not exploit or installed-driver tests.

- ABI: interface `e5bcc234-1e0c-418a-a0d4-ef8b7501414d`, protocol 0.2.1/test build;
  ADD input 56 bytes, result 12 bytes; REMOVE input a 16-byte Windows GUID.
  Refresh values at least 1000 represent millihertz, so the probe sends 60000.
- The INF grants world read/write device access and IOCTLs use FILE_ANY_ACCESS.
  There is no per-client monitor ownership or authentication in this protocol.
  Reserve this entire driver for testing; the probe uses fresh synthetic GUIDs
  and removes only its own GUIDs. These safeguards do not harden the driver.
- Watchdog state is global, not per handle. Every IOCTL except GET_WATCHDOG resets
  it, including GET_PROTOCOL_VERSION. Inventory therefore enumerates only; it
  sends no IOCTL. Another client can keep a crashed client's display alive.
- Watchdog timeout defaults to three seconds but a registry setting can disable
  it. The probe refuses any timeout other than three seconds. Upstream reads and
  writes countdown/list state across threads without consistently taking the
  monitor mutex; race behavior needs a driver-specific review before release.
- ADD checks positive dimensions but lacks conservative upper bounds. The probe
  exposes only the two planned 60 Hz phone modes. Upstream `generate_edid` calls
  strlen on each char[14] field, so both probe fields are fixed and NUL terminated.
- A cursor event is created with a global fixed name and has no visible close in
  AssignSwapChain. Monitor creation installs no monitor context cleanup callback;
  allocation/arrival failure and unload paths warrant leak/lifetime testing.
- The sample swap-chain consumer releases frames without encoding/transmitting;
  this is only a virtual monitor driver. Stream integration belongs to T4.05.
- INF metadata still needs StampInf; build selects UMDF 2.25, IddCx 1.10 with
  minimum 1.4, Spectre mitigation and the UMDF driver toolset. No setting was
  weakened to accommodate missing libraries. A full compile has not been run.
- The custom EDID blob's output licensing is unresolved (LICENSES.md). Its two
  files are omitted from the tracked vendor. No forbidden source was copied or
  translated. No upstream code changes are claimed as tested fixes.

These findings keep D11 provisional. The probe's offline tests validate its wire
encoding and safety guards, not driver correctness, signing acceptance, watchdog
latency, window recovery, Secure Boot compatibility or clean uninstall.
