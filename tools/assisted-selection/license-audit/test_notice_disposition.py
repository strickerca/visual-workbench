#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Verify preservation and refusal boundaries for the proposal relocation."""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile
import unittest

import notice_disposition as disposition


class NoticeDispositionTests(unittest.TestCase):
    def fixture(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        shutil.copytree(disposition.REPOSITORY / "docs/license-proposals", root / "docs/license-proposals")
        manifest = disposition.load(root)
        for row in manifest["retained_records"]:
            target = root / row["path"]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(disposition.REPOSITORY / row["path"], target)
        return root, manifest

    def write_manifest(self, root, manifest):
        (root / disposition.MANIFEST).write_text(json.dumps(manifest), encoding="utf-8")

    def test_all_125_files_and_original_records_are_preserved(self):
        manifest = disposition.verify()
        self.assertEqual({"assisted-selection": 118, "scrcpy": 7, "total": 125}, manifest["counts"])
        self.assertEqual(1_428_759, manifest["total_bytes"])
        self.assertFalse(manifest["production_admitted"])

    def test_missing_or_changed_scrcpy_notice_refuses(self):
        root, manifest = self.fixture()
        row = next(row for row in manifest["files"] if row["source"].endswith("scrcpy/SDL-3.4.12-LICENSE.txt"))
        path = disposition.resolve(root, row["source"], manifest)
        path.write_bytes(b"changed notice")
        with self.assertRaisesRegex(disposition.DispositionError, "notice identity differs"):
            disposition.verify(root)
        path.unlink()
        with self.assertRaises(FileNotFoundError):
            disposition.verify(root)

    def test_unlisted_or_restored_packaged_file_refuses(self):
        root, _ = self.fixture()
        extra = root / "docs/license-proposals/notices/scrcpy/unlisted.txt"
        extra.write_bytes(b"unlisted")
        with self.assertRaisesRegex(disposition.DispositionError, "tree differs"):
            disposition.verify(root)
        extra.unlink()
        restored = root / "third_party/notices/scrcpy/unadmitted.txt"
        restored.parent.mkdir(parents=True)
        restored.write_bytes(b"unadmitted")
        with self.assertRaisesRegex(disposition.DispositionError, "inside packaged tree"):
            disposition.verify(root)

    def test_mapping_cannot_redirect_to_package_or_outside_proposals(self):
        root, manifest = self.fixture()
        original = manifest["files"][0]["destination"]
        for replacement in (manifest["files"][0]["source"], "../outside", "docs/license-proposals/other"):
            with self.subTest(destination=replacement):
                manifest["files"][0]["destination"] = replacement
                self.write_manifest(root, manifest)
                with self.assertRaisesRegex(disposition.DispositionError, "mapping differs"):
                    disposition.load(root)
        manifest["files"][0]["destination"] = original
        self.write_manifest(root, manifest)
        with self.assertRaisesRegex(disposition.DispositionError, "unmapped"):
            disposition.resolve(root, "third_party/scrcpy/scrcpy.exe")

    def test_admission_flag_and_changed_historical_record_refuse(self):
        root, manifest = self.fixture()
        manifest["production_admitted"] = True
        self.write_manifest(root, manifest)
        with self.assertRaisesRegex(disposition.DispositionError, "remain unadmitted"):
            disposition.verify(root)
        manifest["production_admitted"] = False
        self.write_manifest(root, manifest)
        (root / manifest["retained_records"][0]["path"]).write_bytes(b"changed original record")
        with self.assertRaisesRegex(disposition.DispositionError, "historical record identity differs"):
            disposition.verify(root)


if __name__ == "__main__":
    unittest.main()
