# Windows connection assistance source handoff

This additive T1.10 candidate implements explicit ADB reverse recovery and typed
UAC metric actions. It has not been built or executed in this worktree. It does
not establish S23/OnePlus timing, pairing, route, security, or product acceptance.

## Root integration

The desktop worktree's old host crate manifest/library are deliberately unchanged.
Import only `src/connection_assist/**`, `src/bin/vw-connection-helper.rs`, and this
note into the root's reviewed clipboard/DPI host crate. Add beside its existing
modules, preserving all current exports:

```rust
mod connection_assist;
pub use connection_assist::*;
```

The reviewed host already has exact approved `uniffi=0.32.2`, `thiserror=2.0.21`,
`tokio=1.53.1` with `sync`, `blake3=1.8.7` with `std,pure`, `serde=1.0.229` with
`derive`, `serde_json=1.0.151`, and `windows-sys=0.61.2`. No new dependency version
is proposed. Extend the existing Windows features with:

```text
Win32_Security
Win32_Security_Cryptography
Win32_System_JobObjects
Win32_System_Pipes
Win32_System_IO
Win32_System_Com
Win32_System_SystemInformation
Win32_System_LibraryLoader
Win32_System_Registry
Win32_UI_Shell
Win32_NetworkManagement_IpHelper
Win32_NetworkManagement_Ndis
Win32_Networking_WinSock
```

Keep its existing Foundation, FileSystem, Threading and WindowsAndMessaging
features. The root workspace is the repository `Cargo.toml`. Extend the existing
serialized Windows native build invocation to include the helper binary:

```text
cargo build --locked -p vw-host-ffi --lib --bin vw-connection-helper
```

Retain the root's existing profile/target arguments, license gate, concurrency
limits and other packages. The normal Cargo binary auto-discovery finds the new
`src/bin` target; do not add a second concurrent Cargo invocation. Package
`vw-connection-helper.exe` from the same output directory as `vw_host.dll` and
`vw_core.dll`, regenerate the host Kotlin bindings, then run the existing Gradle
lane with that directory as `vwNativeDir`. `packageConnectionRuntime` hashes all
three files and packages the helper plus `vw-native-runtime.sha256`; shared
already packages both DLLs. Preserve the root's additional
`resources.srcDir(rootProject.file("../third_party/notices"))` during the narrow
desktop Gradle merge. No shared API or native session files need changing.

The desktop source delta adds `ConnectionAssistance.kt`,
`ConnectionAssistancePanel.kt`, `DesktopNativeRuntime.kt`, and two focused test
files, with narrow `Main.kt`, `SessionDialog.kt`, Gradle and README changes. Its
prior reviewed r2 source was copied and hash-verified before these edits. Main
extracts/verifies the native runtime before host bindings and calls PMv2 before
the first AWT/Compose window.

## Native contracts and bounded ownership

`ConnectionRequest` is a one-use cancel token. Exported asynchronous functions
admit at most three dedicated workers; no unbounded work queue is created.

- `inspect_adb_tool(path, request)` returns the supported protocol version.
- `list_adb_devices(path, request)` returns at most 64 selected-server entries.
- `start_adb_reverse(config, request)` returns an owned `AdbReverseWatch`, with
  synchronous bounded `snapshot()` and asynchronous idempotent `shutdown()`.
  The exported name avoids UniFFI's generated `AutoCloseable.close()` collision;
  the handwritten desktop lease retains its `close()` interface.
- `prepare_route_metric_fix(interface_index, family, request)` performs read-only
  admission and returns an opaque `RouteMetricAction`.
- `RouteMetricAction.details()` exposes only typed old/new metric values;
  `apply(request)` consumes it and returns a verified receipt with an opaque
  inverse action. A returned Busy, pre-setter Cancelled or ElevationDeclined
  re-arms that exact action for explicit retry; uncertain outcomes remain consumed.
  Drop/close does not apply an inverse.

The desktop uses an independent producer with two admission slots. Cancellation
signals the native request and waits for settlement before disposing late native
watch/action objects. The app closes its controller before native service shutdown.
At most one reverse watcher, one pending metric action, and eight retained inverse
actions/recovery entries exist per app. Inverses are consumed newest first.
A refused Revert retains its exact native action when no setter could run;
stale/uncertain outcomes keep the original typed values visibly after disposing
the consumed handle. Later unrelated fixes do not erase those values. They clear
only after a verified matching revert or explicit owner resolution. Tool paths, selected
serials and route state are not persisted or logged; display of the explicitly
selected device/path stays in the Devices sheet. Error strings are fixed and
redact process output, addresses and identifiers.

## ADB protocol and persistent mapping policy

The owner explicitly selects an installed local `adb.exe`. The file is pinned
against write/delete and only its local `version` command is run. The expected
first line is protocol `1.0.41`; a missing/different tool is refused. There is no
PATH lookup, download, `start-server`, `kill-server`, device reset, shell command,
or server-version remediation. Ordinary ADB clients can restart a mismatched
server, so the watcher uses its own bounded smart socket to `127.0.0.1:5037`.

Commands are restricted to `host:version`, `host:devices`, the exact validated
`host:transport:<selected serial>`, `reverse:list-forward`, and
`reverse:forward:norebind:tcp:<phone>;tcp:<host>`. Serial bytes, explicit nonzero
ports, ASCII framing, 64 KiB replies, 64 device entries and 256 mapping entries
are checked. Creation checks both documented OKAY/FAIL acknowledgments. A
different occupant is never rebound. A racing `norebind` refusal is re-read as a
matching, contested, or refused mapping. No command touches another device.

Each cycle has a 600 ms budget with short cancellable socket operations, followed
by a 200 ms cancellable polling delay. This targets recovery within one second
when the local server/device are available; physical timing has not been measured.
Unavailable, offline, unauthorized, contested and timed-out states are distinct.
No process or network operation runs merely from constructing the controller.

Stopping the watch **leaves mappings in place**. ADB has no atomic conditional
remove tied to creator ownership; even an unchanged matching map may have been
adopted by another app. The snapshot records whether this watch created a mapping,
and the UI exposes this retention. Explicit cleanup through the owner's tool is
outside this watcher. Other mappings and existing ADB processes are preserved.

Official protocol references:
[ADB services](https://android.googlesource.com/platform/packages/modules/adb/+/HEAD/docs/dev/services.md),
[ADB client command handling](https://android.googlesource.com/platform/packages/modules/adb/+/HEAD/client/commandline.cpp),
[ADB client version handling](https://android.googlesource.com/platform/system/core/+/d427b54c535d3f56e9db947efa4878346541f890/adb/client/adb_client.cpp).

## Route actions, UAC and lifetime limits

The read-only adapter admits at most 4096 OS routes and retains only live default
routes. It hashes family, interface index/LUID, next-hop identity, route metric,
interface metric and automatic mode. It proposes demotion only if a distinct live
same-family alternate default exists and currently loses/ties to the selected
interface. The proposed manual interface metric is at least 500, at most 9999,
and puts the selected effective route at least 50 above the alternate. It never
deletes a route, changes an address/DNS/firewall, or disables the sole default.

Only an explicit Apply/Revert invokes the packaged helper. It accepts a bounded
typed JSON envelope encoded as lowercase hex, never a shell command. The envelope
binds the exact plan, parent process ID/creation time, random 192-bit local cancel
event, and a 45-second boot-relative expiry. A non-elevated owned launcher calls
Windows `runas`; the elevated helper opens the existing authorization event and
rechecks parent identity, expiry, routes and interface values before setting only
the requested metric/automatic fields. The IPv4 SitePrefixLength input follows
the Windows API's required zero value. A fresh post-read verifies all other
observed routes remained unchanged before returning a receipt.

Local `adb version` and the launcher use suspended creation, an explicit inherited
handle whitelist, a dedicated kill-on-close Job Object, bounded output (64 KiB),
and hidden windows. The 2-second version operation and 45-second UAC request
terminate only their owned child/job. The elevated helper is launched by Windows
and uses its own 5-second watchdog plus cancellation/parent checks; the app does
not acquire kill rights to unrelated elevated processes. Late UAC approval cannot
authorize an expired request. Individual Win32 calls remain cooperative. A setter
already in flight may complete on timeout/cancel; that outcome is explicitly
uncertain, with no automatic retry or rollback. Windows provides no atomic
compare-and-set over route snapshots, so a simultaneous external mutation cannot
be fully excluded; stale preflight and postcondition failures refuse success.

References: [SetIpInterfaceEntry](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-setipinterfaceentry),
[ShellExecuteExW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw),
[SHELLEXECUTEINFOW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shellexecuteinfow),
[AssignProcessToJobObject](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject).

## Runtime packaging and cleanup

The SHA-256 resource manifest admits exactly `vw_core.dll`, `vw_host.dll`, and
`vw-connection-helper.exe`, each at most 256 MiB and together at most 512 MiB.
Extraction uses fresh files inside an owned manifest-hash directory, validates
size/hash while streaming, fsyncs each file, and writes readiness last. Existing
marked caches must match exactly and are never overwritten. Every parent is
checked/pinned before creating children; Windows handle attributes reject reparse
points and resolved handle paths must remain exact. Read leases deny binary
write/delete while loaded, and absolute UniFFI component overrides keep JNA from
falling back to an unrelated classpath extraction path. The helper is adjacent
to its calling DLL. The native helper admission additionally caps its executable
at 128 MiB. The JVM contains at most 32 retained cache entries before refusing a
new one; unknown entries count and are preserved.

Runtime close releases leases but does not delete caches: loaded Windows DLLs may
outlive the logical app controller. Failed preparation may remove only the same
newly created, never-loaded files whose file identity and containment still match.
Other/changed files and interrupted caches are preserved. Old marked runtime
directories require deliberate cleanup after all app/helper processes have exited.
No shutdown/startup recursive cleanup or claim of automatic file disposal is made.

Twenty-seven native test functions cover pure protocol/selection/recovery decisions,
stale routes, exact inverse values, automatic-metric recomputation, typed envelope
limits and argv quoting. Fifteen injected JVM controller tests cover explicit
actions, late-result ownership, status, cancellation, refused-revert retry and
original recovery values surviving a later fix/revert. Eight
private-file bootstrap tests cover admission, exact hashes, no-clobber reuse,
reparse refusal and preserved existing files. No tests call real ADB, sockets,
route getters/setters, UAC, clipboard or task DLL loading. All are written but
unrun here. Root owns compilation, bindings, focused tests and platform acceptance.
