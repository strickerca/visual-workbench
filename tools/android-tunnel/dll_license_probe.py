#!/usr/bin/env python3
"""Future Windows-only diagnostic; loading DLLs is never part of preparation.

Run only after separate authorization on reviewed bytes. This reports what the
four pinned FFmpeg DLLs themselves return, not a distribution approval.
"""

import argparse
import ctypes
import json
import os
from pathlib import Path
import shlex

from scrcpy_runtime import FFMPEG, PIN, Refusal, verify


MAX_LICENSE_BYTES = 128
MAX_CONFIGURATION_BYTES = 8192
EXPECTED_LICENSE = "LGPL version 2.1 or later"
# Exact feature/toolchain options from scrcpy v4.1 app/deps/ffmpeg.sh for
# win64/cross/shared. Only the absolute build prefix is environment-dependent.
EXPECTED_CONFIGURATION = frozenset({
    "--pkg-config-flags=--static", "--extra-cflags=-O2 -fPIC",
    "--disable-programs", "--disable-doc", "--disable-swscale", "--disable-avfilter",
    "--disable-network", "--disable-everything", "--disable-vulkan", "--disable-vaapi",
    "--disable-vdpau", "--enable-swresample", "--enable-libdav1d",
    "--enable-decoder=h264", "--enable-decoder=hevc", "--enable-decoder=av1",
    "--enable-decoder=vp8", "--enable-decoder=vp9", "--enable-decoder=libdav1d",
    "--enable-decoder=pcm_s16le", "--enable-decoder=opus", "--enable-decoder=aac",
    "--enable-decoder=flac", "--enable-decoder=png", "--enable-protocol=file",
    "--enable-demuxer=image2", "--enable-parser=png", "--enable-zlib",
    "--enable-muxer=matroska", "--enable-muxer=mp4", "--enable-muxer=opus",
    "--enable-muxer=flac", "--enable-muxer=wav", "--disable-avdevice",
    "--disable-static", "--enable-shared", "--enable-cross-compile",
    "--cross-prefix=x86_64-w64-mingw32-", "--cc=x86_64-w64-mingw32-gcc",
    "--target-os=mingw32", "--arch=x86_64",
})


def _bounded_c_text(address: int | None, maximum: int, field: str) -> str:
    # c_void_p retains the pointer. c_char_p would copy an unbounded C string
    # before Python could check its size. Read one byte at a time, stopping at
    # NUL or the cap; no FFmpeg DLL is loaded in this grant's tests.
    if not isinstance(address, int) or address <= 0:
        raise Refusal(f"Missing {field} pointer")
    raw = bytearray()
    for offset in range(maximum + 1):
        byte = ctypes.c_ubyte.from_address(address + offset).value
        if byte == 0:
            if not raw:
                raise Refusal(f"Empty {field} response")
            try:
                return raw.decode("ascii", errors="strict")
            except UnicodeError as error:
                raise Refusal(f"Non-ASCII {field} response") from error
        if offset == maximum:
            raise Refusal(f"{field} response exceeds {maximum} bytes")
        raw.append(byte)
    raise Refusal(f"Unterminated {field} response")


def _query_ffmpeg(library: object, stem: str) -> tuple[str, str]:
    values = []
    for suffix, maximum in (("license", MAX_LICENSE_BYTES),
                            ("configuration", MAX_CONFIGURATION_BYTES)):
        function = getattr(library, f"{stem}_{suffix}")
        function.argtypes = []
        function.restype = ctypes.c_void_p
        values.append(_bounded_c_text(function(), maximum, suffix))
    return values[0], values[1]


def _validate_ffmpeg_claim(license_text: str, configuration: str) -> None:
    if license_text != EXPECTED_LICENSE:
        raise Refusal("FFmpeg DLL returned an unknown or non-LGPL-2.1 license")
    if not configuration or len(configuration) > MAX_CONFIGURATION_BYTES:
        raise Refusal("FFmpeg DLL returned empty or oversized configuration")
    try:
        options = shlex.split(configuration, posix=True)
    except ValueError as error:
        raise Refusal("FFmpeg DLL returned malformed configuration") from error
    prefixes = [option for option in options if option.startswith("--prefix=")]
    if len(prefixes) != 1 or not prefixes[0].startswith("--prefix=/") or \
       not prefixes[0].endswith("/app/deps/work/install/win64-cross-shared") or \
       len(options) != len(EXPECTED_CONFIGURATION) + 1 or \
       set(options) - {prefixes[0]} != EXPECTED_CONFIGURATION:
        raise Refusal("FFmpeg DLL configuration differs from the v4.1 Win64 shared recipe")


def probe(destination: Path, archive: Path | None = None) -> dict:
    if os.name != "nt":
        raise Refusal("Actual DLL diagnostics require an authorized Windows host")
    inventory = verify(destination, archive, PIN)
    records = {row["name"]: row for row in inventory["files"]}
    results = []
    # LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS.
    # No current directory or PATH DLL search is permitted.
    secure_search = 0x00000100 | 0x00001000
    for name in sorted(FFMPEG):
        path = destination / name
        library = ctypes.WinDLL(str(path), winmode=secure_search)
        stem = name.split("-")[0]
        row = {"name": name, "sha256": records[name]["sha256"]}
        row["license"], row["configuration"] = _query_ffmpeg(library, stem)
        _validate_ffmpeg_claim(row["license"], row["configuration"])
        results.append(row)
    if len({row["configuration"] for row in results}) != 1:
        raise Refusal("The four FFmpeg DLLs report different configurations")
    return {"schema": "vw.scrcpy-dll-license-probe.v1", "version": PIN.version,
            "status": "diagnostic only; aggregate legal/build review still required", "dlls": results}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--execute-reviewed-dlls", action="store_true", required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(probe(args.destination, args.archive), indent=2, sort_keys=True))
    except (Refusal, OSError, AttributeError, UnicodeError) as error:
        parser.exit(2, f"DLL license probe refused: {error}\n")


if __name__ == "__main__":
    main()
