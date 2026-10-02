"""Exercise real loopback framing; synthetic empty-tile payloads are not HIL evidence."""
import hashlib
import json
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "tools/bench/video-pc/serve_tiles.py"


def exact(connection, count):
    result = bytearray()
    while len(result) < count:
        data = connection.recv(count - len(result))
        if not data:
            raise AssertionError("Unexpected end of fixture stream")
        result.extend(data)
    return bytes(result)


class TileProtocolTests(unittest.TestCase):
    def run_server(self, *, bad_ack=False, malformed=False):
        with tempfile.TemporaryDirectory(prefix="VW-tile-protocol-") as directory:
            root = Path(directory)
            recording = root / "synthetic.vwt"
            payload = struct.pack(">I", 0)
            recording.write_bytes(b"VWJT0002" + struct.pack(">III", 2560, 1440, 1)
                                  + struct.pack(">I", 8_388_609 if malformed else len(payload)) + payload
                                  + struct.pack(">I", len(payload)) + payload)
            output = root / "result"
            process = subprocess.Popen([sys.executable, str(SERVER), str(recording), str(output),
                                        "--profile", "smoke"], stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
            try:
                if not malformed:
                    deadline = time.monotonic() + 30
                    next_progress = time.monotonic() + 10
                    while not (output / "ready.json").exists():
                        if process.poll() is not None or time.monotonic() > deadline:
                            self.fail("Loopback fixture readiness failed")
                        if time.monotonic() > next_progress:
                            print("Tile protocol fixture: waiting for bounded server readiness", flush=True)
                            next_progress += 10
                        time.sleep(.05)
                    port = json.loads((output / "ready.json").read_text())["port"]
                    with socket.create_connection(("127.0.0.1", port), timeout=5) as connection:
                        connection.settimeout(5)
                        self.assertEqual(exact(connection, 8), b"VWJS0001")
                        self.assertEqual(struct.unpack(">IIII", exact(connection, 16)), (2560, 1440, 13, 3))
                        for sequence in range(13):
                            index, length = struct.unpack(">II", exact(connection, 8))
                            self.assertEqual(index, sequence)
                            self.assertEqual(exact(connection, length), payload)
                            digest = b"x" * 32 if bad_ack else hashlib.sha256(payload).digest()
                            connection.sendall(struct.pack(">I", sequence) + digest)
                            if bad_ack:
                                break
                stdout, stderr = process.communicate(timeout=10)
                result_path = output / "result.json"
                if bad_ack or malformed:
                    self.assertNotEqual(process.returncode, 0)
                    self.assertFalse(result_path.exists(), "Rejected input must never get a completed report")
                    self.assertIn("Tile transfer failed", stderr)
                else:
                    self.assertEqual(process.returncode, 0, stderr)
                    result = json.loads(result_path.read_text())
                    self.assertTrue(result["completed"])
                    self.assertEqual(result["measured_frames"], 10)
                    self.assertEqual(result["warmup_frames"], 2)
                    self.assertEqual(result["initial_frame_excluded"], 1)
                    self.assertEqual(result["measured_payload_bytes"], 40)
                    self.assertEqual(len(result["raw_frame_ms"]), 10)
                    self.assertEqual(result["recording_sha256"], hashlib.sha256(recording.read_bytes()).hexdigest())
                    self.assertIn("phone decode/posting", stdout)
            finally:
                if process.poll() is None:
                    process.kill()  # This owned Python server never spawns children.
                process.communicate(timeout=5)

    def test_verified_acknowledgments_exclude_warmups(self):
        self.run_server()

    def test_wrong_payload_hash_cannot_produce_success(self):
        self.run_server(bad_ack=True)

    def test_oversized_recording_frame_rejected_before_listening(self):
        self.run_server(malformed=True)
