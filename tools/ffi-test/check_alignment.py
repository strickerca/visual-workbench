"""Check the same Android 16 KiB ELF LOAD contract as check-alignment.ps1."""

import argparse
from pathlib import Path
import re
import subprocess


def check_load_alignment(output: str) -> int:
    loads = [line for line in output.splitlines() if re.match(r"^\s*LOAD\s", line)]
    if not loads:
        raise ValueError("No ELF LOAD segments found")
    for line in loads:
        alignment = re.search(r"\balign\s+2\*\*(\d+)\s*$", line)
        if alignment is None:
            raise ValueError("Unrecognized ELF LOAD alignment")
        exponent = int(alignment.group(1))
        if exponent < 14:
            raise ValueError("Android ELF LOAD alignment is below 16 KiB")
        if exponent > 63:
            raise ValueError("Unrecognized ELF LOAD alignment")
    return len(loads)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", required=True, type=Path)
    parser.add_argument("--objdump", required=True, type=Path)
    options = parser.parse_args()
    if not options.library.is_file():
        parser.exit(1, "Native Android library is missing\n")
    if not options.objdump.is_file():
        parser.exit(1, "Pinned NDK llvm-objdump is missing\n")
    try:
        result = subprocess.run(
            [str(options.objdump), "-p", str(options.library)],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=30,
        )
        segments = check_load_alignment(result.stdout)
    except (OSError, subprocess.SubprocessError):
        parser.exit(1, "llvm-objdump failed or exceeded its deadline\n")
    except ValueError as error:
        parser.exit(1, f"{error}\n")
    print(f"Android 16 KiB LOAD alignment: PASS; segments={segments}")


if __name__ == "__main__":
    main()
