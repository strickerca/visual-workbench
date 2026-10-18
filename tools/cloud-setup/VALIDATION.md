# Cloud setup validation — 2026-10-18

Status: PASS for setup prerequisites and focused compilation; product/hardware acceptance pending.

Source base: `67b0af376612f999a0ea2543ad99a98fddd69b22`.
Only new `tools/cloud-setup/` files belong to this change. Main/product code was not edited.

Commands, from the setup branch checkout, targeting the existing product checkout:

```bash
bash tools/cloud-setup/install.sh --android --accept-android-sdk-license --repo /workspace/visual-workbench
bash tools/cloud-setup/verify.sh --android --repo /workspace/visual-workbench
```

The final scripts were then copied to `/workspace/.vw-tools/cloud-setup` and the
same install/verify pair ran again from that standalone location with explicit
`--repo /workspace/visual-workbench`. No file from the earlier private task report
directory is an installer/runtime dependency.

- First final install/verify pair: exit 0, 10.70 / 8.21 seconds.
- Standalone second install/verify pair: exit 0, 10.81 / 8.03 seconds.
- All 18 verifier commands passed on each final run.
- Idempotency: 16 installed auxiliary binaries, SDK metadata/license files and
  the acceptance receipt compared byte-for-byte; zero changed files.
- SDK manager recognizes command-line tools 22.0, platform 37.0 revision 2,
  build-tools 36.1.0 and NDK 30.0.16248370. No platform-tools installed.
- NDK Clang 21.0.0 compiled JNI C with warnings denied into AArch64 ELF at API 29.
- javac 21.0.12.1 compiled against API 37.0 android.jar; build-tools D8 emitted DEX.
- cargo-ndk 4.1.2 / Rust 1.99.0 linked a small arm64-v8a/API 29 Rust cdylib offline.
- Offline locked Cargo fetch, reviewed installed npm policy (20 packages), and
  offline Gradle 9.7.0 project configuration passed. No full app suite ran.
- Owner Android acceptance was explicitly verified before installation. The
  task-local receipt records android-sdk-license, acceptance timestamp, official
  terms URL, package-text SHA256 and sdkmanager SHA1. Package text is dated
  January 16, 2019; public terms page observed separately is dated April 28, 2026.
  The script fails if the pinned downloadable package agreement changes.
- Source scripts passed shell/Python syntax checks and a redacted gitleaks scan.
  Exact staged files are separately scanned before publication.

Setup-only corrections found during validation: writable Android user home,
Java proxy configuration, property-key whitespace handling, and registration
of official SDK local-package metadata. An early duplicate NDK download was
stopped, its complete verified prefetch reused, and the install resumed. Failed
or interrupted task-local receipts were retained; they are not product evidence.

## Read-only emulator feasibility

- `/dev/kvm`: absent; accessible acceleration: false.
- CPU `vmx` / `svm`: both absent.
- Intel/AMD nested-virtualization settings: not exposed.
- CPU quota: `400000 100000` (4 effective CPUs).
- RAM cgroup limit: 17179869184 bytes (16 GiB), no swap.
- Post-verification cgroup memory.current: 6675226624 bytes, including
  cache; limit-minus-current headroom: 10504642560 bytes. This is not a claim
  that all host-reported MemAvailable is usable within the container limit.
- Exact workspace free space after SDK install and verification:
  25869938688 bytes (24.09 GiB).

Normal instrumentation can run on a suitable emulator headlessly with software
rendering, without browser or GPU UI tools. This VM has no usable accelerated
CPU path; software CPU emulation would contend with four-CPU builds and is not
recommended. The product packages arm64-v8a JNI only, so an x86_64 system image
also needs an appropriate native-ABI build lane. No emulator/image was installed,
no permissions or nested virtualization changed, no capacity bought and no phone
connected. Prefer an explicitly authorized physical OnePlus for suitable software
instrumentation; it does not pass S23/S Pen latency or physical acceptance.

Environment policy/configuration and runtime billing are not exposed by this task.
No claim of zero incremental runtime credits or successful saved-environment
republishing is made. Task-local workers can apply the setup scripts immediately;
reviewing and publishing the saved environment remains a separate UI action.
