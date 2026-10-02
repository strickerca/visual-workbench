"""Exact fixture binding rejects substitution/escape before any image decoder runs."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("fixture_verify", ROOT / "fixtures/verify.py")
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


class FixtureBindingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="vw-fixture-binding-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        data = b"deliberately malformed fixture; not an image"
        (self.directory / "input.png").write_bytes(data)
        self.row = dict(path="input.png", sha256=hashlib.sha256(data).hexdigest(), bytes=len(data),
                        provenance="synthetic test", license="LicenseRef-VisualWorkbench-Proprietary")

    def test_malformed_bytes_verified_without_decoding(self):
        self.assertEqual(VERIFY.verify(self.directory, {"files": [self.row]}), 1)

    def test_same_size_substitution_rejected(self):
        path = self.directory / "input.png"
        path.write_bytes(b"x" * self.row["bytes"])
        with self.assertRaisesRegex(ValueError, "hash"):
            VERIFY.verify(self.directory, {"files": [self.row]})

    def test_missing_license_rejected(self):
        self.row.pop("license")
        with self.assertRaisesRegex(ValueError, "provenance/license"):
            VERIFY.verify(self.directory, {"files": [self.row]})

    def test_duplicate_and_escaping_paths_rejected(self):
        for name in ("../input.png", "sub/input.png", "C:\\input.png", "input.png:stream"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                VERIFY.verify(self.directory, {"files": [{**self.row, "path": name}]})
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            VERIFY.verify(self.directory, {"files": [self.row, self.row]})


if __name__ == "__main__":
    unittest.main()
