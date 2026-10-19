#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Small refusal fixtures for the offline license census."""
from __future__ import annotations

import hashlib
import io
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

import audit
import prepare_sdists


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def wheel(self, extra_native=False):
        path = self.root / "sample-1.0-py3-none-any.whl"
        metadata = b"Name: sample\nVersion: 1.0\nLicense: MIT\n"
        native = b"native-fixture"
        notice = b"MIT license fixture\n"
        with ZipFile(path, "w") as archive:
            archive.writestr("sample-1.0.dist-info/METADATA", metadata)
            archive.writestr("sample/libsample.so", native)
            archive.writestr("sample-1.0.dist-info/LICENSE", notice)
            if extra_native:
                archive.writestr("sample.libs/libunlisted.so", b"unknown")
        row = {"distribution": "sample", "version": "1.0", "bytes": path.stat().st_size,
               "sha256": sha(path.read_bytes()), "metadata_sha256": sha(metadata),
               "native_files": [{"path": "sample/libsample.so", "bytes": len(native), "sha256": sha(native)}],
               "notices": [{"path": "sample-1.0.dist-info/LICENSE", "bytes": len(notice),
                            "sha256": sha(notice)}]}
        return path, row

    def test_exact_wheel_and_unlisted_native_refusal(self):
        path, row = self.wheel()
        audit.verify_wheel(path, row)
        changed, changed_row = self.wheel(extra_native=True)
        with self.assertRaisesRegex(audit.AuditError, "native census"):
            audit.verify_wheel(changed, changed_row)

    def test_changed_wheel_and_child_refuse(self):
        path, row = self.wheel()
        row["sha256"] = "0" * 64
        with self.assertRaisesRegex(audit.AuditError, "wheel identity"):
            audit.verify_wheel(path, row)
        row["sha256"] = sha(path.read_bytes())
        row["notices"][0]["sha256"] = "f" * 64
        with self.assertRaisesRegex(audit.AuditError, "archive child hash"):
            audit.verify_wheel(path, row)

    def test_missing_and_changed_notice_copy_refuse(self):
        lock = json.loads(audit.LOCK.read_text())
        sources = json.loads(audit.SOURCES.read_text())
        for item in sources["notices"]:
            source = audit.REPOSITORY / item["path"]
            target = self.root / item["path"]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
        audit.verify_notice_sources(self.root, lock, sources)
        sample = self.root / sources["notices"][0]["path"]
        sample.write_bytes(b"changed text")
        with self.assertRaisesRegex(audit.AuditError, "notice text identity"):
            audit.verify_notice_sources(self.root, lock, sources)
        sample.unlink()
        with self.assertRaises((FileNotFoundError, audit.AuditError)):
            audit.verify_notice_sources(self.root, lock, sources)

    def test_unknown_license_is_not_admission(self):
        census = {"status": "incomplete-no-production-admission", "host_wheels": [],
                  "model": {"checkpoint": {"license_status": "unresolved"}}}
        with self.assertRaisesRegex(audit.AuditError, "license admission is open"):
            audit.require_closed(census)
        census["status"] = "closed-production-admission"
        census["host_wheels"] = [{"review_state": "source-text-only"}]
        with self.assertRaisesRegex(audit.AuditError, "host component review incomplete"):
            audit.require_closed(census)

    def test_source_only_census_refuses_changed_wheel_identity(self):
        self.assertEqual(audit.source_only()["status"], "incomplete-no-production-admission")
        altered = json.loads(audit.CENSUS.read_text())
        altered["host_wheels"][0]["wheel_sha256"] = "0" * 64
        path = self.root / "changed-census.json"
        path.write_bytes(audit.canonical(altered))
        with patch.object(audit, "CENSUS", path), self.assertRaisesRegex(
            audit.AuditError, "source wheel identity"
        ):
            audit.source_only()

    def test_sdist_preparation_reuses_exact_bytes_and_rejects_tampering(self):
        content = b"pinned source license archive fixture"
        row = ("example.tar.gz", len(content), sha(content), "https://files.pythonhosted.org/example")
        with patch.object(prepare_sdists, "SOURCES", (row,)), patch.object(
            prepare_sdists.urllib.request, "urlopen", return_value=io.BytesIO(content)
        ) as request:
            prepare_sdists.prepare(self.root)
            prepare_sdists.prepare(self.root)
            self.assertEqual(request.call_count, 1)
            (self.root / row[0]).write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "sdist size/type"):
                prepare_sdists.prepare(self.root)


if __name__ == "__main__":
    unittest.main()
