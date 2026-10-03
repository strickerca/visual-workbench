# Pairing CLI

This harness exercises real TLS-exporter QR or SPAKE2 pairing, a separately
authenticated ordinary carrier and Hello handshake, one synthetic OPS
transaction with an independently computed acknowledgment, durable PC revocation,
and refusal through the already constructed listener/TLS configuration. It does
not install firewall rules or change routes. The separate `run_adb.ps1` harness
creates and removes only its two owned reverse mappings and private test files.
The default ordinary carrier is QUIC. Add `-tcp` to both commands for TCP/ADB
fallback. The restricted onboarding channel uses TLS 1.3 over TCP for both modes.

The Windows host uses current-user DPAPI in the fixed application trust folder.
The Android CLI identity is **ephemeral for this process only**. The real Android
app must implement the Keystore callback in T1.10 before persistent pairing is
accepted. All retained output consists of stage markers and counts, with static
errors; no endpoint, certificate, code, QR, key or user path is logged.

Build through `build.ps1 build-pairing`, then run `build.ps1 hil-test pairing`
for the explicitly selected OnePlus IN2019. The owned ADB-reverse harness has
passed QR pairing, one exact OPS acknowledgment, revocation refusal on both
sides, and cleanup. See `docs/evidence/T1.06b.md` for current source/artifact
bindings and limitations. Direct tether and interactive code-pairing hardware
runs remain pending. For manual commands below, substitute explicitly selected
interface addresses, never wildcard pairing binds, and use distinct nonzero ports.

1. Start `pair-cli host-qr <pair-bind-address:port> <ops-bind-address:port> <transfer-name>`.
2. After `PAIRING_READY`, the private, single-use transfer file is
   `%LOCALAPPDATA%\Visual Workbench\trust\<transfer-name>.qr`. The name is restricted
   to 1â€“80 ASCII letters, digits, hyphen or underscore. Existing files are never
   overwritten. Transfer this file outside the repository to the task-owned
   phone directory, set phone mode `600`, and never capture its contents.
3. Run `pair-cli phone-qr <private-transfer-file> <PC-ops-address:port>` on the phone.
   It reads and removes its copy. The host removes its copy after pairing or on
   ordinary process cleanup. If a process is forcibly killed, the owner must
   remove its exact task-owned transfer file; the credential still expires at
   five minutes and is unusable after success.
4. Require both `COMPLETE role=host ops=1 refused=1` and
   `COMPLETE role=phone ops=1 refused=1 local_revoked=1`, with exit code zero.
   Other errors, especially a timeout, are failures and do not establish refusal.
   Existing immutable revoked records remain in the bounded trust store.

Interactive fallback: `host-code <pair-bind> <ops-bind>` and
`phone-code <pair-address> <ops-address>`. Both require a real terminal; no
auto-confirm option exists. The host displays the eight digits; the phone reads
them from standard input, never a command-line argument. Both display the full
PC fingerprint and require `yes` after the user compares them. Do not retain
terminal recordings/screenshots containing pairing material. Each PAKE attempt
is charged durably before the host sends any PAKE response, including abandoned
attempts; five attempts lock pairing for ten minutes. A successful fifth attempt
may complete and clears the failure budget. Refreshing the displayed offer does
not clear failed attempts.

Native Windows route inspection is available through
`vw_host_win::routes::WindowsRoutes` and the pure
`vw_net::route::detect_tether_default` API. Metric fix/revert text requires user
review and an administrator action; this executable never executes it.

Dependency provenance: stable `spake2 =0.4.0` (Apache-2.0 OR MIT),
[upstream documentation](https://docs.rs/spake2/0.4.0/spake2/), has no independent
security audit and upstream notes timing limitations. `mdns-sd =0.21.3`
(Apache-2.0 OR MIT) is the approved exact stable pin,
[upstream release](https://github.com/keepsimple1/mdns-sd/releases/tag/v0.21.3).
Those limitations remain open for security review. Software tests or a CLI
hardware run do not establish production-security acceptance or T1.10 app gates.
