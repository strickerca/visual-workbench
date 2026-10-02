"""Windows CLI-only libvips spike; exact OS peak working set, bounded subprocesses.

Run under Invoke-VwProcess so the whole worker tree is contained in a Windows job.
First run is not a cold-cache claim: no OS cache flush is performed.
"""
from __future__ import annotations
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


class MemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in ("PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
        "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage")]


def measure(arguments: list[str], phase: str) -> dict:
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD]
    begin = time.perf_counter()
    process = subprocess.Popen(arguments, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    handle = kernel.OpenProcess(0x0400 | 0x0010, False, process.pid)
    if not handle:
        process.kill()
        process.communicate()
        raise OSError("Cannot bind process memory counter")
    try:
        while True:
            try:
                stdout, stderr = process.communicate(timeout=10)
                break
            except subprocess.TimeoutExpired:
                elapsed = time.perf_counter() - begin
                print(f"{phase}: active {elapsed:.0f}s / 180s", flush=True)
                if elapsed >= 180:
                    process.kill()
                    process.communicate()
                    raise TimeoutError(phase)
        duration = (time.perf_counter() - begin) * 1000
        counters = MemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
            raise OSError("Cannot read process peak working set")
        if process.returncode:
            raise RuntimeError(f"{phase} failed: {stderr.decode('utf-8', errors='replace')[:1000]}")
        return dict(phase=phase, elapsed_ms=duration, peak_working_set_bytes=counters.PeakWorkingSetSize,
                    peak_pagefile_bytes=counters.PeakPagefileUsage,
                    result=stdout.decode("utf-8", errors="replace").strip())
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate()
        kernel.CloseHandle(handle)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--vips", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if os.name != "nt" or args.receipt.exists():
        raise ValueError("Requires Windows and a fresh receipt")
    source = args.fixtures / "200mp.jpg"
    manifest = json.loads((args.fixtures / "manifest.json").read_text())
    with source.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    record = next(item for item in manifest["files"] if item["path"] == "200mp.jpg")
    if digest != record["sha256"]:
        raise ValueError("Fixture bytes changed")
    prefix = [str(args.vips), "--vips-concurrency=2", "--vips-cache-max-memory=134217728"]
    temporary = Path(tempfile.mkdtemp(prefix="VW-image-benchmark-")).resolve()
    temporary_parent = Path(tempfile.gettempdir()).resolve()
    receipt = dict(schema=1, fixture_sha256=digest, source_dimensions=[16320, 12240],
                   libvips_executable_sha256=hashlib.sha256(args.vips.read_bytes()).hexdigest(),
                   version=subprocess.check_output([str(args.vips), "--version"], timeout=15, text=True).strip(),
                   memory_method="Windows GetProcessMemoryInfo PeakWorkingSetSize retained process handle",
                   timing_method="wall time including process startup and output I/O; OS cache not flushed",
                   tile_format="JPEG Q90 at all levels, 256px, zero overlap; not the product mixed WebP/JPEG pipeline",
                   measurements=[], completed=False, temporary_outputs_disposed=False, screenshots_created=0)
    try:
        for attempt in range(3):
            for operation in ("decode", "thumbnail", "pyramid"):
                output = temporary / f"{operation}-{attempt}"
                if operation == "decode":
                    command = prefix + ["avg", str(source)]  # Forces all pixels through the decoder.
                elif operation == "thumbnail":
                    command = prefix + ["thumbnail", str(source), str(output) + ".jpg", "2040", "--height=1530", "--size=down"]
                else:
                    command = prefix + ["dzsave", str(source), str(output), "--tile-size=256", "--overlap=0", "--suffix=.jpg[Q=90]"]
                result = measure(command, f"{operation}-{attempt}")
                if operation == "decode":
                    if not 0 < float(result["result"]) < 255:
                        raise ValueError("Decode did not evaluate expected pixel mean")
                elif operation == "pyramid":
                    files = list(temporary.glob(f"pyramid-{attempt}_files/**/*.jpg"))
                    result["tile_count"] = len(files)
                    if not (temporary / f"pyramid-{attempt}.dzi").is_file() or len(files) < 3000:
                        raise ValueError("Pyramid output incomplete")
                elif not Path(str(output) + ".jpg").is_file():
                    raise ValueError("Thumbnail output missing")
                receipt["measurements"].append(result)
                print(f"{result['phase']}: {result['elapsed_ms']:.1f}ms; peak {result['peak_working_set_bytes']/1048576:.1f}MiB", flush=True)
        receipt["completed"] = True
    finally:
        # This fresh directory was created above. Never delete a caller-supplied path.
        if temporary.parent != temporary_parent or not temporary.name.startswith("VW-image-benchmark-") or temporary.is_symlink():
            raise ValueError("Temporary output containment failed")
        receipt["disposed_files"] = sum(p.is_file() for p in temporary.rglob("*"))
        shutil.rmtree(temporary)
        receipt["temporary_outputs_disposed"] = not temporary.exists()
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
