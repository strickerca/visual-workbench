"""Bounded direct-adb capture inside the parent's Windows Job. No import effects.

Only the central runner invokes this helper. Raw status bytes stay in a private
temporary file. No adb server restart, device reset, or shell composition.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time

LIMIT = 8 * 1024 * 1024


def capture(command: list[str], output: Path, seconds: int) -> int:
    if not command or Path(command[0]).name.lower() != "adb.exe" or not 1 <= seconds <= 3600:
        raise ValueError("command")
    if any(not isinstance(a, str) or len(a) > 8192 or "\0" in a for a in command):
        raise ValueError("argument")
    pending: queue.Queue[bytes | None] = queue.Queue(maxsize=16)
    # Keep the private capture observable during a failing test instead of
    # delaying short status packets until a buffer fills or adb exits.
    with output.open("xb", buffering=0) as target:
        process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, shell=False, close_fds=True)
        def reader() -> None:
            try:
                while data := process.stdout.read1(4096):
                    pending.put(data)
            finally:
                pending.put(None)
        threading.Thread(target=reader, daemon=True).start()
        start = time.monotonic()
        progress = start + 5
        total = 0
        try:
            while True:
                now = time.monotonic()
                if now - start >= seconds:
                    raise TimeoutError("instrumentation deadline")
                if now >= progress:
                    print(f"APP_HIL_CAPTURE_PROGRESS elapsed_seconds={int(now-start)} bytes={total}", flush=True)
                    progress = now + 5
                try:
                    chunk = pending.get(timeout=0.1)
                except queue.Empty:
                    continue
                if chunk is None:
                    break
                total += len(chunk)
                if total > LIMIT:
                    raise ValueError("output bound")
                target.write(chunk)
            code = process.wait(timeout=max(0.1, seconds - (time.monotonic() - start)))
            target.flush()
            os.fsync(target.fileno())
            print(f"APP_HIL_CAPTURE_DONE bytes={total} exit_code={code}", flush=True)
            return code
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
            # Outer Invoke-VwProcess confirms the entire Windows Job retires.


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--command", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=int, required=True)
    args = parser.parse_args()
    try:
        if args.command.stat().st_size > 64 * 1024:
            raise ValueError("command bound")
        command = json.loads(args.command.read_text(encoding="utf-8-sig"))
        if not isinstance(command, list):
            raise ValueError("command")
        return capture(command, args.output, args.seconds)
    except Exception:
        print("APP_HIL_CAPTURE_FAILED", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
