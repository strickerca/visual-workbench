# Pinned Linux cloud setup

These setup-only scripts target the audited Debian 13 x86_64 cloud base. They do
not modify product source, publish an environment, connect devices, install an
emulator, use paid services, or make Linux a Windows validation environment.

The base must already contain rustup at
`/workspace/.vw-tools/cargo/bin/rustup`. The audited image also contains Node
24.21.0. Missing Node can be restored from its pinned official archive; missing
JDK compiler tools are overlaid onto a task-local copy of the matching existing
OpenJDK 21.0.12.1 runtime. A different base runtime fails explicitly.

## Install and start

From the repository root, use this install/cache hook **after explicit approval
of the Android SDK agreement**:

```bash
bash tools/cloud-setup/install.sh --android --accept-android-sdk-license
bash tools/cloud-setup/verify.sh --android
```

Without Android approval, omit both Android flags and use the core-only verifier.
Never treat the presence of this script or a previously installed Windows SDK
as approval. The current owner approved the named Android cloud installation on
2026-10-18; that authorization is recorded by the coordinating conversation,
not a reusable unconditional grant for arbitrary users or future terms.

Runtime/start hook and each worker command shell:

```bash
source tools/cloud-setup/start.sh
```

Run from the intended worktree. Installation does not switch branches or update
main. Exports in an install subprocess do not persist into later worker shells;
use the platform's persistent runtime settings or source `start.sh` every time.
The hooks recompute proxy properties from the existing task proxy environment,
copy no credentials, change no certificate checks or network permissions, and
fail for authenticated proxy URLs rather than inventing credential setup.

All tools, downloads, license receipts, logs and generated compilation probes
are outside source under `/workspace/.vw-tools`. Each worktree keeps its own
Cargo target and Gradle output directories. No machine-wide package installer
or security setting is changed. Preserve extracted upstream notices with the
download cache. Auxiliary cargo-deny, cargo-ndk and gitleaks release downloads
required no interactive EULA acceptance or new network approval in the audited
environment. A differently configured environment may have different access.

## Exact Android selection and agreement

| Package | Revision |
|---|---|
| `cmdline-tools;22.0` | 22.0 |
| `platforms;android-37.0` | 2 |
| `build-tools;36.1.0` | 36.1.0 |
| `ndk;30.0.16248370` | 30.0.16248370 |

No `platform-tools`, emulator, system image, Android Studio or Docker package is
requested. `compileSdk = 37` is bound to the exact final 37.0 platform, not the
newer 37.1/37.2 packages. Do not use mutable `latest` command-line tools (currently
23.0) in place of the selected 22.0 tools.

The public agreement page is
[Android SDK License Agreement](https://developer.android.com/studio/terms).
The exact downloadable package agreement is separately obtained from Google's
SDK repository metadata as `android-sdk-license`; its retained package text is
dated January 16, 2019. The public page observed during the audit is dated
April 28, 2026; do not claim they are byte-identical. `pins.json` binds the exact
package text SHA256 and standard sdkmanager license SHA1. A changed text fails
before acceptance and requires renewed review. With the explicit acceptance
flag, the installer records only that approved agreement in the SDK license
store and saves its exact text, digest, URL and acceptance timestamp under
`setup-state/`. It never accepts every license returned by `sdkmanager --licenses`.

Official archive SHA256 pins are retained for Node/JDK/auxiliary tools. Google
SDK archive SHA1 values are the integrity checks published in its repository
metadata; do not misrepresent them as stronger signed attestations. cargo-ndk's
release supplied no checksum asset in its listing, so its pinned SHA256 is the
locally recorded hash of the official HTTPS release download. No secrets,
private proxy addresses or device identifiers belong in source or receipts.
The installer also registers the exact official package metadata in each local
`package.xml`; raw archive extraction alone did not make sdkmanager recognize
every installed package. The verifier requires all four inventory entries.

## What verification establishes

The verifier checks exact installed package revisions and tool versions, offline
Cargo resolution, the reviewed installed npm dependency policy, offline Gradle
project configuration, and SDK manager inventory. Tiny task-owned probes compile
C JNI to AArch64 ELF with NDK/API 29, Java against android.jar then DEX with D8,
and a Rust cdylib through cargo-ndk. No probe is installed or run on a phone.
Logs and the structured receipt are in `setup-state/verification.json` and its
neighboring text logs. These checks do not establish a full product build,
Android runtime behavior, Windows capture/input, S Pen latency or owner acceptance.

The source build orchestration still contains Windows executables, DLL loading,
Windows PowerShell and a Windows NDK objdump path. Those require a separate
portability/Windows lane; this setup does not edit them.

## Resources and emulator feasibility

The audited allocation has a four-CPU cgroup quota and 16 GiB RAM with no swap.
The resource recommendation is at most two build-capable workers per VM, one
heavy build at a time, plus up to two light review/coordination workers. Shell
wrappers use a shared per-VM lock, bounded execution and two Cargo/Gradle workers.
Independent VMs have independent resources and locks. Separately, AGENTS.md has
an unqualified one-heavy-build/two-build-worker policy; do not silently override
that project policy when planning global concurrency. Coordinate shared devices
and integration ownership regardless of VM separation.

The read-only feasibility probe records `/dev/kvm`, its accessibility, CPU
vmx/svm flags, exposed nested-virtualization settings and exact post-install
disk/cgroup memory values. No KVM device or CPU virtualization flags were
exposed in the audited VM. An Android emulator can normally run instrumentation
headlessly with a software renderer without browser/GPU UI tools, but software
CPU emulation here would compete with builds and was not demonstrated practical.
No emulator or image is installed by these scripts. Prefer an explicitly
authorized physical OnePlus when hardware-free cloud instrumentation is blocked;
it does not substitute for S23/S Pen acceptance.
The current app also filters JNI packaging to arm64-v8a. An x86_64 emulator would
need an appropriate native ABI/product build lane; an ARM guest on this x86_64
VM would not provide a useful accelerated substitute. No extra service or
capacity purchase was made. Pricing/credit policies are not visible inside the
task, so no guarantee about incremental runtime billing is possible.

Official emulator references:
[headless command-line operation](https://developer.android.com/studio/run/emulator-commandline)
and [CPU/GPU acceleration requirements](https://developer.android.com/studio/run/emulator-acceleration).

The published environment definition, UI hooks and cache persistence policy are
not exposed inside this task. Publishing a Git branch is not republishing the
saved environment. Review its install/start fields, explicitly republish the
chosen revision, and verify a fresh worker before claiming those settings fixed.
