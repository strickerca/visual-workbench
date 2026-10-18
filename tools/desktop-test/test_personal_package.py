"""Synthetic resource fixtures only; no build, native loading, device or real package."""
from pathlib import Path
import json
import tempfile
import unittest
from unittest.mock import patch
import warnings
import zipfile
import personal_package as p


class PersonalPackage(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vw-personal-package-unit-")
        self.root = Path(self.temp.name)
    def tearDown(self):
        self.temp.cleanup()
    def archive(self, name, entries):
        path = self.root / name
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            with zipfile.ZipFile(path, "w") as z:
                for key, value in entries:
                    z.writestr(key, value)
        return path
    def rejected(self, code, call):
        with self.assertRaisesRegex(p.r.Rejected, "^" + code + "$"):
            call()
    def test_exact_notices_and_admitted_catalog_pair(self):
        body = b'{"schema":1}'
        source = self.root / "catalog.json"
        source.write_bytes(body)
        cat = p.catalog(source, p.blob(body)["sha256"])
        marker = f"{p.blob(body)['sha256']} {len(body)} {p.CATALOG}\n".encode()
        jar = self.archive("app.jar", [("NOTICE.txt", b"notice"), (p.CATALOG, body), (p.CATALOG_HASH, marker)])
        actual = p.resources([jar], {"NOTICE.txt": p.blob(b"notice")} | cat)
        self.assertEqual(len(actual["resources"]), 3)
    def test_catalog_half_pair_is_refused(self):
        self.rejected("catalog_arguments", lambda: p.catalog(None, "a" * 64))
        self.rejected("catalog_arguments", lambda: p.catalog(self.root / "missing", None))
    def test_catalog_digest_mismatch_refuses(self):
        source = self.root / "catalog.json"; source.write_bytes(b"different")
        self.rejected("catalog_source_binding", lambda: p.catalog(source, "a" * 64))
    def test_stale_catalog_is_rejected_when_none_admitted(self):
        jar = self.archive("app.jar", [(p.CATALOG.upper(), b"old")])
        self.rejected("unexpected_catalog", lambda: p.resources([jar], {}, forbid_catalog=True))
    def test_partial_catalog_pair_is_missing(self):
        jar = self.archive("app.jar", [(p.CATALOG, b"x")])
        self.rejected("resource_missing", lambda: p.resources([jar], {p.CATALOG: p.blob(b"x"), p.CATALOG_HASH: p.blob(b"y")}))
    def test_notice_bytes_must_match_even_when_same_length(self):
        jar = self.archive("app.jar", [("N.txt", b"bad")])
        self.rejected("resource_binding", lambda: p.resources([jar], {"N.txt": p.blob(b"yes")}))
    def test_notice_must_be_unique_across_jars(self):
        jars = [self.archive(name, [("N.txt", b"x")]) for name in ["one.jar", "two.jar"]]
        self.rejected("resource_identity", lambda: p.resources(jars, {"N.txt": p.blob(b"x")}))
    def test_case_alias_and_duplicate_zip_entry_refuse(self):
        jar = self.archive("case.jar", [("n.txt", b"x")])
        self.rejected("resource_identity", lambda: p.resources([jar], {"N.txt": p.blob(b"x")}))
        jar = self.archive("duplicate.jar", [("N.txt", b"x"), ("N.txt", b"x")])
        self.rejected("resource_archive_duplicate", lambda: p.resources([jar], {"N.txt": p.blob(b"x")}))
    def test_android_notices_must_be_in_assets(self):
        apk = self.archive("app.apk", [("N.txt", b"x")])
        self.rejected("resource_missing", lambda: p.resources([apk], {"N.txt": p.blob(b"x")}, prefix="assets/"))
        apk = self.archive("good.apk", [("assets/N.txt", b"x")])
        self.assertEqual(len(p.resources([apk], {"N.txt": p.blob(b"x")}, prefix="assets/")["resources"]), 1)
    def source_notices(self):
        folder = self.root / "third_party/notices"; folder.mkdir(parents=True)
        (folder / "N.txt").write_bytes(b"notice")
        row = {"path":"notices/N.txt", "license":"MIT", "source_url":"https://example.test/license", "sha256":p.blob(b"notice")["sha256"]}
        (folder.parent / "LICENSES").write_text(json.dumps({"schema_version":1,"files":[row]}))
        return folder
    def test_source_inventory_covers_all_notices(self):
        folder = self.source_notices()
        expected, rows = p.notices(self.root)
        self.assertEqual(set(expected), {"N.txt"}); self.assertEqual(len(rows), 1)
        (folder / "extra.txt").write_bytes(b"unlisted")
        self.rejected("notice_unlisted_source", lambda: p.notices(self.root))
    def test_source_notice_changed_after_manifest_refuses(self):
        folder = self.source_notices(); (folder / "N.txt").write_bytes(b"altered")
        self.rejected("notice_source_binding", lambda: p.notices(self.root))
    def test_runtime_inventory_preserves_every_file_and_requires_legal(self):
        names = ["runtime/bin/server/jvm.dll", "runtime/lib/modules", "runtime/legal/java.base/LICENSE",
                 "runtime/legal/java.base/ADDITIONAL_LICENSE_INFO", "runtime/legal/java.base/ASSEMBLY_EXCEPTION",
                 "runtime/legal/java.desktop/vendor.txt"]
        inventory = {name:p.blob(name.encode()) for name in names}
        inventory["app/unrelated.jar"] = p.blob(b"jar")
        actual = p.runtime_inventory(inventory)
        self.assertEqual(set(actual["files"]), set(names))
        self.assertIn("runtime/legal/java.desktop/vendor.txt", actual["legal_files"])
        del inventory["runtime/legal/java.base/LICENSE"]
        self.rejected("runtime_legal_missing", lambda: p.runtime_inventory(inventory))
    def test_cli_refuses_outputs_inside_package_before_inspection(self):
        (self.root / ".local").mkdir()
        args = ["personal_package.py", "--root", str(self.root), "--output", str(self.root / "new.json"),
                "--notice-index", str(self.root / ".local/notices.txt")]
        with patch.object(p.sys, "argv", args), patch.object(p, "verify", side_effect=AssertionError("must not inspect")):
            self.assertEqual(p.main(), 1)
        self.assertFalse((self.root / "new.json").exists())
    def test_cli_preserves_existing_receipt(self):
        local = self.root / ".local"; local.mkdir()
        receipt = local / "receipt.json"; receipt.write_bytes(b"preserve")
        args = ["personal_package.py", "--root", str(self.root), "--output", str(receipt),
                "--notice-index", str(local / "notices.txt")]
        with patch.object(p.sys, "argv", args), patch.object(p, "verify", side_effect=AssertionError("must not inspect")):
            self.assertEqual(p.main(), 1)
        self.assertEqual(receipt.read_bytes(), b"preserve")
        self.assertFalse((local / "notices.txt").exists())
    def test_cli_rejects_alternate_stream_and_reserved_names_before_inspection(self):
        local = self.root / ".local"; local.mkdir()
        receipt = local / "existing-receipt.json"; receipt.write_bytes(b"preserve")
        for slot in ("--output", "--notice-index"):
            for name in ("existing-receipt.json:extra", "CON.txt", "trailing."):
                with self.subTest(slot=slot, name=name):
                    outputs = {"--output": str(local / "new.json"), "--notice-index": str(local / "notices.txt")}
                    outputs[slot] = str(local / name)
                    args = ["personal_package.py", "--root", str(self.root),
                            "--output", outputs["--output"], "--notice-index", outputs["--notice-index"]]
                    with patch.object(p.sys, "argv", args), patch.object(p, "verify", side_effect=AssertionError("must not inspect")):
                        self.assertEqual(p.main(), 1)
                    self.assertEqual(receipt.read_bytes(), b"preserve")
                    self.assertEqual({entry.name for entry in local.iterdir()}, {receipt.name})
    def test_readable_index_reports_actual_hashes_without_acceptance_claim(self):
        self.source_notices(); _, rows = p.notices(self.root)
        runtime = {"project_exception":"actual runtime", "legal_files":{"runtime/legal/java.base/LICENSE":p.blob(b"license")}}
        text = p.notice_text(rows, runtime)
        self.assertIn(p.blob(b"license")["sha256"], text)
        self.assertIn("do not establish installed signature", text)


if __name__ == "__main__":
    unittest.main()
