"""Inspect actual debug APK native bytes and Android 16 KB ZIP alignment."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apk", type=Path, default=ROOT / "apps/android/build/outputs/apk/debug/android-debug.apk")
    parser.add_argument("--zipalign", required=True, type=Path)
    args = parser.parse_args()
    apk = args.apk.resolve(strict=True)
    artifact_path = apk.relative_to(ROOT).as_posix()
    result = subprocess.run([str(args.zipalign), "-c", "-P", "16", "4", str(apk)],
                            capture_output=True, text=True, timeout=30, check=False)
    if result.returncode:
        raise ValueError(f"zipalign 16 KB check failed (exit {result.returncode})")
    libraries = []
    with zipfile.ZipFile(apk) as archive:
        for entry in archive.infolist():
            if not entry.filename.startswith("lib/") or not entry.filename.endswith(".so"):
                continue
            if not entry.filename.startswith("lib/arm64-v8a/") or entry.compress_type != zipfile.ZIP_STORED:
                raise ValueError("APK contains an unexpected ABI or compressed native library")
            data = archive.read(entry)
            if (len(data) < 64 or data[:6] != b"\x7fELF\x02\x01"
                    or struct.unpack_from("<H", data, 18)[0] != 183):
                raise ValueError("Native library is not a little-endian ARM64 ELF")
            offset = struct.unpack_from("<Q", data, 32)[0]
            width, count = struct.unpack_from("<HH", data, 54)
            if width < 56 or offset + width * count > len(data):
                raise ValueError("Native ELF program header table is invalid")
            alignments = []
            for index in range(count):
                header = offset + width * index
                if struct.unpack_from("<I", data, header)[0] != 1:
                    continue
                file_offset, address = struct.unpack_from("<QQ", data, header + 8)
                file_size = struct.unpack_from("<Q", data, header + 32)[0]
                alignment = struct.unpack_from("<Q", data, header + 48)[0]
                if (alignment < 16384 or alignment & (alignment - 1)
                        or file_offset % alignment != address % alignment
                        or file_offset + file_size > len(data)):
                    raise ValueError("Native ELF LOAD segment fails 16 KB alignment or bounds")
                alignments.append(alignment)
            if not alignments:
                raise ValueError("Native ELF has no LOAD segments")
            libraries.append({"path": entry.filename, "sha256": hashlib.sha256(data).hexdigest(),
                              "load_alignments_bytes": alignments, "zip_compression": "stored"})
    if not any(row["path"] == "lib/arm64-v8a/libvw_core.so" for row in libraries):
        raise ValueError("APK does not contain the actual Rust core library")
    receipt = {"artifact": artifact_path, "apk_sha256": hashlib.sha256(apk.read_bytes()).hexdigest(),
               "apk_bytes": apk.stat().st_size, "zipalign_16kb_exit_code": 0, "libraries": libraries,
               "artifact_check_only": True, "native_16kb_device_execution_verified": False}
    output = ROOT / "docs/evidence/APK_VERIFICATION.json"
    output.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(f"APK ARTIFACTS: PASS ({len(libraries)} ARM64 libraries; ELF LOAD >=16 KB; ZIP 16 KB check)")


if __name__ == "__main__":
    main()
