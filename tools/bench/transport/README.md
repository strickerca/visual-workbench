# Transport measurement spike (T0.06)

This tool measures synthetic traffic between a Windows binary and the same Rust
binary on Android. It is separate from the product's protocol, pairing, persisted
OPS queues and TCP multiplexer. Plain TCP here is the explicitly requested spike;
production TCP still requires TLS. QUIC uses TLS 1.3 with the ring provider and
trusts only the supplied throwaway public certificate. No private key is written.

## Build and run

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-transport
python tools/bench/transport/run_host.py --profile smoke
python tools/bench/transport/run_host.py --profile full
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test transport -TimeoutSeconds 1250
powershell -NoProfile -ExecutionPolicy Bypass -File tools/bench/transport/run_wifi.ps1 -Profile full
python tools/bench/transport/verify_tls.py
```

Run commands through `tools/process.psm1` in automation. It contains the Windows
process tree, prints progress and keeps text receipts. The build runs the license
gates, then builds release binaries for Windows and Android arm64/API 29. It uses
at most two compiler workers. Host runs use loopback and are software integration
evidence, never phone/carrier measurements. Hardware runs select only SM-S918U
and reject missing, ambiguous, offline or emulator matches without fallback.
The build records source and binary hashes in ignored `.local/transport-build.json`;
all runners check that receipt before traffic and reject stale binaries.

The full profile measures 1,000 verified echoes at each of 64 B, 4 KiB and 1 MiB,
after ten excluded warmup echoes per size, then uploads 256 MiB with a verified
server acknowledgment. Reports include all RTT samples, nearest-rank p50/p95/p99,
and jitter defined as mean absolute difference between successive RTTs. Payload
checking happens outside the echo clock interval. Bulk time includes receiver
verification and the final ACK, so it is application throughput, not link rate.

QUIC additionally sends 1,200 1-KiB datagrams on a 120-Hz schedule, then allows two
seconds for delayed echoes. Reports expose send lateness, duplicate echoes,
missing echoes and RTT using one monotonic client clock. Echo loss includes either
direction; it is not a one-way loss estimate. Timer scheduling is not assumed
perfect. There is no one-way latency claim or subtraction of unrelated clocks.

The smoke profile uses ten measured echoes per size, 1 MiB bulk and 120 datagrams.
Reports explicitly identify the profile. Incomplete operations leave no completed
report. Frames, allocations, IO waits, total run time and accepted connections are
bounded. Servers accept one synthetic session and expire. Malformed sizes and
corrupt payloads fail without success acknowledgment. Errors omit peer addresses.

## Direct use

```text
transport-bench serve <tcp|quic> <bind-address:port> <new-directory> <30..1800-seconds> [--allow-lan]
transport-bench run <tcp|quic> <peer-address:port> <public-DER-certificate|-> <new-report.json> <smoke|full> [--allow-lan]
```

Wildcard/multicast listeners are rejected. Loopback is the default allowed scope;
explicit `--allow-lan` is required for a selected interface address. The server
exports only `server.der`, `ready.json` (port, no IP) and completion status. Never
pass an existing document or secret as benchmark input; there is no file-transfer
option. Do not expose this diagnostic service beyond the selected local devices.

`run_adb.ps1` starts an owned loopback server, creates a fresh remote directory,
uses `adb reverse --no-rebind` and removes only a mapping it successfully created.
It does not reset adb, replace another task's mapping, stop another app or access
the OnePlus. Native Android execution uses no activity and does not prove foreground
service/background behavior. Cleanup checks process identity before stopping an
owned client and confirms removal of the exact UUID directory. Raw numeric samples
and build hashes stay under ignored `.local/transport-*`; public evidence must
contain sanitized measurements and hashes only.

`run_wifi.ps1` selects the S23's private wlan0 address and reverses the server/client
roles: the phone hosts QUIC and Windows initiates the connection. It transfers the
throwaway public certificate over the already authorized adb link before connecting.
This measures actual Wi-Fi traffic without changing Windows inbound firewall rules;
it does not validate the planned Windows-host listener or that listener's firewall.
The receipt explicitly records roles, phone Wi-Fi frequency and cleanup. No address
is printed in the report; the generated address-bearing shell script is discarded.

## Remaining physical/manual measurements

USB tethering and Wi-Fi runs require the actual link and a documented firewall
check. Do not classify a carrier as failed while firewall interference is unknown.
The implementation does not change firewall rules, routes or interface metrics.
Any later temporary firewall rule must name the exact executable, local subnet and
owned rule; remove it after the experiment. Preserve unrelated block rules.

Cable unplug/replug and tethering re-enable timing require owner actions. Record
availability delay separately from reconnect delay. A software restart or recreated
adb mapping is not a physical cable-recovery measurement. Background throttling
needs a foreground-service APK comparison; this native shell benchmark does not
provide that evidence. D7 stays provisional until carrier/recovery evidence exists.

## Dependency sources, checked 2026-10-02

Exact stable pins and licenses were checked against the crates.io API. Quinn and
Rustls defaults are disabled to exclude aws-lc-rs; the lockfile fixes transitive
versions. The resolved graph passed cargo-deny license/source/ban policy.

- [Quinn 0.11.12](https://docs.rs/quinn/0.11.12/quinn/): MIT OR Apache-2.0.
- [Rustls 0.23.45](https://docs.rs/rustls/0.23.45/rustls/): Apache-2.0 OR ISC OR MIT.
- [Tokio 1.53.1](https://docs.rs/tokio/1.53.1/tokio/): MIT.
- [rcgen 0.14.10](https://docs.rs/rcgen/0.14.10/rcgen/): MIT OR Apache-2.0.
- serde 1.0.229, serde_json 1.0.151, thiserror 2.0.21: MIT OR Apache-2.0.
