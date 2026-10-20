#!/usr/bin/env python3
"""Private fixed-byte Windows stat probe; print comparisons, never host values."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import importlib.util
import json
import os
from pathlib import Path
import sys


def differing(left, right) -> list[str]:
    names = ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns", "st_nlink", "st_birthtime_ns")
    return [name for name in names if getattr(left, name, None) != getattr(right, name, None)]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work", type=Path, required=True, help="fresh private directory; parent must exist")
    args = parser.parse_args()
    result = {"schema": "vw-windows-metadata-probe-v1", "windows": os.name == "nt"}
    if os.name != "nt":
        print(json.dumps(result, sort_keys=True))
        return 1
    try:
        spec = importlib.util.spec_from_file_location("vw_metadata_probe_assemble", Path(__file__).with_name("assemble.py"))
        if spec is None or spec.loader is None:
            raise ValueError()
        a = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = a
        spec.loader.exec_module(a)
        with ExitStack() as stack:
            cache = {}
            work = a.absolute(args.work)
            parent = a.get_dir(work.parent, stack, cache)
            parent.mkdir(work.name)
            directory = a.get_dir(work, stack, cache)
            path = work / "owned.bin"
            with directory.open("owned.bin", True) as stream:
                stream.write(b"Visual Workbench metadata probe\n")
                stream.flush()
                os.fsync(stream.fileno())
                python_handle = os.fstat(stream.fileno())
                python_path = path.lstat()
                native_handle = a.fstat_metadata(stream.fileno())
                native_path = directory.stat("owned.bin")
                result["python_path_vs_handle_differing_fields"] = differing(python_path, python_handle)
                result["native_path_vs_handle_differing_fields"] = differing(native_path, native_handle)
                result["python_path_ctime_equals_creation"] = python_path.st_ctime_ns == python_path.st_birthtime_ns
                result["python_handle_ctime_equals_native_change"] = python_handle.st_ctime_ns == native_handle.st_ctime_ns
                result["native_open_snapshot_agrees"] = a.snapshot(native_handle) == a.snapshot(native_path)
            closed = directory.stat("owned.bin")
            result["native_before_vs_after_close_differing_fields"] = differing(native_handle, closed)
            result["native_closed_snapshot_agrees"] = a.snapshot(native_handle) == a.snapshot(closed)
            result["private_fixed_byte_work_retained"] = True
        result["completed"] = True
        print(json.dumps(result, sort_keys=True))
        return 0
    except Exception:
        result["completed"] = False
        result["refused"] = True
        print(json.dumps(result, sort_keys=True))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
