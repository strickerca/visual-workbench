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

from scrcpy_runtime import FFMPEG, PIN, Refusal, verify


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
        for suffix in ("license", "configuration"):
            function = getattr(library, f"{stem}_{suffix}")
            function.argtypes = []
            function.restype = ctypes.c_char_p
            raw = function()
            if raw is None or len(raw) > 8192:
                raise Refusal(f"{name}: missing or oversized {suffix} response")
            row[suffix] = raw.decode("ascii", errors="strict")
        if "LGPL" not in row["license"].upper() or any(
            flag in row["configuration"] for flag in ("--enable-gpl", "--enable-nonfree")
        ):
            raise Refusal(f"{name}: GPL, nonfree or unknown actual FFmpeg configuration")
        results.append(row)
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
