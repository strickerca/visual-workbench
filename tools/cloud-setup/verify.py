"""Small host/Android compilation checks; no emulator, adb or product build."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import shutil
import struct
import subprocess
import time

HERE = Path(__file__).resolve().parent
PINS = json.loads((HERE / "pins.json").read_text())
ROOT = Path(os.environ["VW_TOOLS_ROOT"])
STATE = ROOT / "setup-state"
SMOKE = STATE / "smoke"
RESULTS: list[dict] = []


def run(name: str, command: list[str], *, cwd: Path | None = None, limit: int = 120) -> str:
    print("CHECK", name, flush=True)
    start = time.monotonic()
    with (STATE / f"{name}.log").open("w") as log:
        process = subprocess.Popen(command, cwd=cwd, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=limit)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            code = 124
    text = (STATE / f"{name}.log").read_text(errors="replace")
    RESULTS.append({"name": name, "exit": code, "seconds": round(time.monotonic() - start, 2)})
    print(name, code, "\n".join(text.splitlines()[-5:]), flush=True)
    if code:
        raise SystemExit(f"{name} failed; task-local log retained")
    return text


def emulator_feasibility() -> dict:
    cpu = Path("/proc/cpuinfo").read_text()
    flags = next((line.partition(":")[2].split() for line in cpu.splitlines()
                  if line.startswith("flags")), [])
    result = {"kvm_present": Path("/dev/kvm").exists(),
              "kvm_accessible": os.access("/dev/kvm", os.R_OK | os.W_OK),
              "vmx": "vmx" in flags, "svm": "svm" in flags,
              "disk_free_bytes": shutil.disk_usage(ROOT).free}
    for name in ("memory.max", "memory.current", "cpu.max"):
        path = Path("/sys/fs/cgroup") / name
        result[name] = path.read_text().strip() if path.exists() else None
    result["mem_available_proc_bytes"] = int(next(
        line.split()[1] for line in Path("/proc/meminfo").read_text().splitlines()
        if line.startswith("MemAvailable:"))) * 1024
    result["nested_virtualization"] = {}
    for kind in ("intel", "amd"):
        path = Path(f"/sys/module/kvm_{kind}/parameters/nested")
        result["nested_virtualization"][kind] = path.read_text().strip() if path.exists() else "not exposed"
    return result


def android() -> dict:
    sdk = ROOT / "android-sdk"
    versions = {}
    for item in PINS["android"]:
        source = sdk / item["directory"] / "source.properties"
        values = dict(tuple(part.strip() for part in line.split("=", 1))
                      for line in source.read_text().splitlines()
                      if "=" in line and not line.lstrip().startswith("#"))
        revision = values["Pkg.Revision"].strip()
        if revision != item["revision"]:
            raise SystemExit(f"Unexpected revision: {item['package']}: {revision}")
        versions[item["package"]] = revision
    run("sdkmanager-version", [str(sdk / "cmdline-tools/22.0/bin/sdkmanager"), "--version"])
    inventory = run("sdkmanager-inventory", [str(sdk / "cmdline-tools/22.0/bin/sdkmanager"),
                                              f"--sdk_root={sdk}", "--list_installed"])
    if any(item["package"] not in inventory for item in PINS["android"]):
        raise SystemExit("SDK manager did not recognize every pinned local package")
    build = sdk / "build-tools/36.1.0"
    run("aapt2-version", [str(build / "aapt2"), "version"])
    ndk_bin = sdk / "ndk/30.0.16248370/toolchains/llvm/prebuilt/linux-x86_64/bin"
    run("ndk-clang-version", [str(ndk_bin / "clang"), "--version"])
    c = SMOKE / "jni.c"
    c.write_text("#include <jni.h>\nJNIEXPORT jint JNICALL Java_setup_Smoke_add("
                 "JNIEnv *env, jclass cls, jint a, jint b) { (void)env; (void)cls; return a+b; }\n")
    native = SMOKE / "libsetup_jni.so"
    run("ndk-jni-compile", [str(ndk_bin / "aarch64-linux-android29-clang"),
                            "-Wall", "-Werror", "-shared", "-fPIC", str(c), "-o", str(native)])
    data = native.read_bytes()
    if data[:4] != b"\x7fELF" or struct.unpack_from("<H", data, 18)[0] != 183:
        raise SystemExit("NDK output is not AArch64 ELF")
    java = SMOKE / "SdkSmoke.java"
    java.write_text("public final class SdkSmoke extends android.app.Activity {}\n")
    classes = SMOKE / "classes"
    dex = SMOKE / "dex"
    classes.mkdir(exist_ok=True)
    dex.mkdir(exist_ok=True)
    platform = sdk / "platforms/android-37.0/android.jar"
    run("android-java-compile", ["javac", "--release", "17", "-classpath", str(platform),
                                 "-d", str(classes), str(java)])
    run("android-dex-compile", [str(build / "d8"), "--min-api", "29", "--lib", str(platform),
                                "--output", str(dex), str(classes / "SdkSmoke.class")])
    if not (dex / "classes.dex").read_bytes().startswith(b"dex\n"):
        raise SystemExit("Missing DEX output")
    rust = SMOKE / "rust"
    (rust / "src").mkdir(parents=True, exist_ok=True)
    (rust / "Cargo.toml").write_text('[package]\nname="vw-cloud-setup-smoke"\nversion="0.0.0"\n'
                                      'edition="2024"\n[lib]\ncrate-type=["cdylib"]\n[workspace]\n')
    (rust / "src/lib.rs").write_text('#[unsafe(no_mangle)]\npub extern "C" fn setup_add(a: i32, b: i32) -> i32 { a + b }\n')
    run("rust-android-lock", ["cargo", "+1.99.0", "generate-lockfile", "--offline"], cwd=rust)
    run("rust-android-compile", ["cargo", "+1.99.0", "ndk", "-t", "arm64-v8a", "--platform", "29",
                                 "build", "--offline", "--locked"], cwd=rust)
    return {"versions": versions, "jni_sha256": hashlib.sha256(data).hexdigest(),
            "platform_tools_installed": (sdk / "platform-tools").exists(),
            "license": json.loads((STATE / "android-license-acceptance.json").read_text())}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--android", action="store_true")
    parser.add_argument("--repo", type=Path, default=HERE.parents[1])
    args = parser.parse_args()
    repo = args.repo.resolve()
    SMOKE.mkdir(parents=True, exist_ok=True)
    for name, command, expected in (
        ("rust-version", ["rustc", "+1.99.0", "--version"], "1.99.0"),
        ("node-version", ["node", "--version"], "v24.21.0"),
        ("jdk-version", ["javac", "--version"], "21.0.12.1"),
        ("deny-version", ["cargo", "deny", "--version"], "0.20.2"),
        ("cargo-ndk-version", ["cargo", "ndk", "--version"], "4.1.2"),
        ("gitleaks-version", ["gitleaks", "version"], "8.30.1"),
    ):
        if expected not in run(name, command):
            raise SystemExit(f"Unexpected tool version: {name}")
    run("cargo-cache-offline", ["cargo", "fetch", "--locked", "--offline"], cwd=repo)
    run("npm-policy", ["python3", str(repo / "tools/check_npm_licenses.py"),
                       "--installed", str(repo / "mcp")], cwd=repo)
    run("gradle-config-offline", ["bash", str(repo / "apps/gradlew"), "-p", str(repo / "apps"),
                                  "help", "--offline", "--console=plain", "--no-daemon",
                                  "--no-parallel", "--max-workers=2"], cwd=repo)
    report = {"checked_at_utc": datetime.now(timezone.utc).isoformat(),
              "android": android() if args.android else None,
              "emulator_feasibility": emulator_feasibility(), "checks": RESULTS,
              "published_environment_changed": False, "devices_used": False}
    (STATE / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print("VERIFIED: prerequisites and small compilation checks; no product/hardware acceptance", flush=True)


if __name__ == "__main__":
    main()
