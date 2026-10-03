"""Exercise partial pipe delivery, output refusal and deadline cleanup."""
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import capture as c


class PipeProcess:
    """An actual pipe stays open until the test or cancellation releases it."""

    def __init__(self, payload=b""):
        reader, writer = os.pipe()
        self.stdout = os.fdopen(reader, "rb")
        self.release = threading.Event()
        self.done = threading.Event()
        self.terminated = False

        def produce():
            try:
                with os.fdopen(writer, "wb", buffering=0) as stream:
                    if payload:
                        stream.write(payload)
                    self.release.wait(10)
            finally:
                self.done.set()

        self.worker = threading.Thread(target=produce, daemon=True)
        self.worker.start()

    def poll(self):
        return 0 if self.done.is_set() else None

    def wait(self, timeout):
        if not self.done.wait(timeout):
            raise subprocess.TimeoutExpired("synthetic pipe", timeout)
        return 0

    def terminate(self):
        self.terminated = True
        self.release.set()

    kill = terminate

    def close(self):
        self.release.set()
        self.worker.join(3)
        self.stdout.close()


class CaptureTests(unittest.TestCase):
    def test_short_status_packet_is_visible_before_process_exit(self):
        process = PipeProcess(b"INSTRUMENTATION_STATUS_CODE: 1\n")
        self.addCleanup(process.close)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "status.txt"
            results = []

            def run():
                try:
                    results.append(c.capture(["adb.exe"], output, 5))
                except BaseException as error:
                    results.append(error)

            with patch.object(c.subprocess, "Popen", return_value=process):
                worker = threading.Thread(target=run)
                worker.start()
                try:
                    deadline = time.monotonic() + 3
                    while time.monotonic() < deadline:
                        if output.exists() and output.stat().st_size > 0:
                            break
                        time.sleep(.01)
                    self.assertEqual(output.read_bytes(), b"INSTRUMENTATION_STATUS_CODE: 1\n")
                    self.assertFalse(process.done.is_set())
                finally:
                    process.release.set()
                    worker.join(6)
            self.assertFalse(worker.is_alive())
            self.assertEqual(results, [0])

    def test_overflow_refuses_before_writing_and_terminates_producer(self):
        process = PipeProcess(b"x" * 64)
        self.addCleanup(process.close)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "status.txt"
            with patch.object(c.subprocess, "Popen", return_value=process), patch.object(c, "LIMIT", 32):
                with self.assertRaisesRegex(ValueError, "output bound"):
                    c.capture(["adb.exe"], output, 5)
            self.assertLessEqual(output.stat().st_size, 32)
            self.assertTrue(process.terminated)
            self.assertTrue(process.done.is_set())

    def test_deadline_reaps_stalled_producer(self):
        process = PipeProcess()
        self.addCleanup(process.close)
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(c.subprocess, "Popen", return_value=process):
                with self.assertRaisesRegex(TimeoutError, "instrumentation deadline"):
                    c.capture(["adb.exe"], Path(directory) / "status.txt", 1)
            self.assertTrue(process.terminated)
            self.assertTrue(process.done.is_set())


if __name__ == "__main__":
    unittest.main()
