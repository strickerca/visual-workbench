"""Offline tests use synthetic ZIPs only; no release binary, adb or DLL is run."""

import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
import warnings
import zipfile
from urllib.request import Request


SPEC = importlib.util.spec_from_file_location("scrcpy_runtime", Path(__file__).with_name("scrcpy_runtime.py"))
assert SPEC and SPEC.loader
runtime = importlib.util.module_from_spec(SPEC)
import sys
sys.modules[SPEC.name] = runtime
SPEC.loader.exec_module(runtime)

DISPOSITION_SPEC = importlib.util.spec_from_file_location(
    "notice_disposition", runtime.ROOT / "tools/assisted-selection/license-audit/notice_disposition.py")
assert DISPOSITION_SPEC and DISPOSITION_SPEC.loader
notice_disposition = importlib.util.module_from_spec(DISPOSITION_SPEC)
DISPOSITION_SPEC.loader.exec_module(notice_disposition)


def pe64() -> bytes:
    content = bytearray(256)
    content[:2] = b"MZ"
    struct.pack_into("<I", content, 0x3C, 128)
    content[128:132] = b"PE\0\0"
    struct.pack_into("<H", content, 132, 0x8664)
    struct.pack_into("<H", content, 152, 0x20B)
    return bytes(content)


def bundle(directory: Path, *, drop: str | None = None, extra: str | None = None,
           duplicate: str | None = None, bad_pe: bool = False,
           unsafe_link: bool = False) -> tuple[Path, runtime.Pin]:
    content = {name: (pe64() if name in runtime.PE_FILES else name.encode())
               for name in runtime.RETAIN | runtime.OMIT}
    content["scrcpy-server"] = b"synthetic-server"
    if bad_pe:
        content["avcodec-62.dll"] = b"invalid executable"
    archive = directory / "synthetic.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as out:
        out.writestr(runtime.PREFIX, b"")
        for name, data in sorted(content.items()):
            if name != drop:
                if unsafe_link and name == "SDL3.dll":
                    info = zipfile.ZipInfo(runtime.PREFIX + name)
                    info.create_system = 3
                    info.external_attr = (0o120777 << 16)
                    out.writestr(info, data)
                else:
                    out.writestr(runtime.PREFIX + name, data)
        if extra:
            with warnings.catch_warnings():
                warnings.filterwarnings("ignore", message="Duplicate name:.*", category=UserWarning)
                out.writestr(extra, b"unexpected")
        if duplicate:
            with warnings.catch_warnings():
                warnings.filterwarnings("ignore", message="Duplicate name:.*", category=UserWarning)
                out.writestr(runtime.PREFIX + duplicate, content[duplicate])
    data = archive.read_bytes()
    pin = runtime.Pin("4.1-test", runtime.PIN.url, len(data), hashlib.sha256(data).hexdigest(),
                      hashlib.sha256(content["scrcpy-server"]).hexdigest(),
                      {name: len(value) for name, value in content.items()},
                      runtime.RETAIN, runtime.OMIT, runtime.PE_FILES)
    return archive, pin


class ScrcpyRuntimeTest(unittest.TestCase):
    def test_checked_in_inventory_and_unadmitted_notice_proposals_are_complete(self):
        pinned = runtime._pinned_inventory()
        self.assertEqual(runtime.PIN.retain, {row["name"] for row in pinned["files"]})
        self.assertEqual(runtime.PIN.omit, {row["name"] for row in pinned["excluded"]})
        disposition = notice_disposition.load(runtime.ROOT)
        notices = notice_disposition.resolve(
            runtime.ROOT, "third_party/notices/scrcpy/SOURCES.json", disposition).parent
        sources = json.loads((notices / "SOURCES.json").read_text(encoding="utf-8"))
        self.assertEqual("vw.scrcpy-source-notices.v1", sources["schema"])
        for row in sources["files"]:
            path = notices / row["path"]
            data = path.read_bytes()
            self.assertEqual(row["bytes"], len(data))
            self.assertEqual(row["sha256"], hashlib.sha256(data).hexdigest())
            if row["component"] == "FFmpeg 8.1.2":
                self.assertIn("/38b88335f99e76ed89ff3c93f877fdefce736c13/", row["source_url"])
            if row["component"] == "dav1d 1.5.3":
                self.assertIn("/b546257f770768b2c88258c533da38b91a06f737/", row["source_url"])
        proposal = json.loads((runtime.ROOT / "tools" / "android-tunnel" /
                               "root-manifest-proposal.json").read_text(encoding="utf-8"))
        self.assertEqual({"scrcpy/" + row["name"]: row["sha256"] for row in pinned["files"]},
                         {row["path"]: row["sha256"] for row in proposal["binary_and_resource_entries"]})
        self.assertEqual({"notices/scrcpy/" + row["path"]: row["source_url"] for row in sources["files"]},
                         {row["path"]: row["source_url"] for row in proposal["source_notice_entries"]})
        for row in proposal["source_notice_entries"]:
            path = notice_disposition.resolve(runtime.ROOT, "third_party/" + row["path"], disposition)
            self.assertEqual(notices / Path(row["path"]).name, path)
            self.assertEqual((row["bytes"], row["sha256"]), notice_disposition.file_identity(path))
        self.assertFalse(disposition["production_admitted"])
        self.assertIn("blocked", proposal["admission"])
        self.assertFalse((runtime.ROOT / "third_party/notices/scrcpy").exists())

    def test_each_redirect_hop_requires_https_and_an_approved_host(self):
        handler = runtime._ReleaseRedirects()
        initial = Request(runtime.PIN.url)
        approved = "https://release-assets.githubusercontent.com/asset?token=synthetic"
        self.assertEqual(approved, handler.redirect_request(initial, None, 302, "Found", {}, approved).full_url)
        for rejected in (
            "http://release-assets.githubusercontent.com/asset",
            "https://unapproved.example/step",
            "https://github.com.evil.example/step",
            "https://github.com:444/step",
            "https://user@github.com/step",
        ):
            with self.subTest(redirect=rejected), self.assertRaises(runtime.Refusal):
                handler.redirect_request(initial, None, 302, "Found", {}, rejected)
        # A later GitHub final URL cannot redeem an unsafe intermediate hop.
        with self.assertRaises(runtime.Refusal):
            handler.redirect_request(Request("http://unapproved.example/step"), None, 302,
                                     "Found", {}, runtime.PIN.url)

    def test_verified_archive_censuses_all_bytes_and_never_delivers_adb(self):
        with tempfile.TemporaryDirectory() as location:
            root = Path(location)
            archive, pin = bundle(root)
            destination = root / "prepared"
            inventory = runtime.prepare(archive, destination, pin)
            self.assertEqual(runtime.SCHEMA, inventory["schema"])
            self.assertEqual(pin.archive_sha256, inventory["archive"]["sha256"])
            self.assertEqual(pin.retain | {"inventory.json"}, {p.name for p in destination.iterdir()})
            self.assertEqual(pin.omit, {row["name"] for row in inventory["excluded"]})
            self.assertTrue(all(row["disposition"] == "excluded; never extracted" for row in inventory["excluded"]))
            self.assertEqual(inventory, runtime.verify(destination, archive, pin))
            self.assertEqual("PE32+ AMD64 (0x8664)", next(row for row in inventory["files"]
                             if row["name"] == "avcodec-62.dll")["architecture"])

    def test_changed_archive_and_prepared_byte_fail_closed(self):
        with tempfile.TemporaryDirectory() as location:
            root = Path(location)
            archive, pin = bundle(root)
            destination = root / "prepared"
            runtime.prepare(archive, destination, pin)
            (destination / "scrcpy-server").write_bytes(b"tampered-server")
            with self.assertRaises(runtime.Refusal):
                runtime.verify(destination, archive, pin)
            with archive.open("ab") as sink:
                sink.write(b"!")
            with self.assertRaises(runtime.Refusal):
                runtime.inspect_archive(archive, pin)

    def test_missing_unknown_duplicate_unsafe_and_adb_containing_entries_fail(self):
        for mutation in ({"drop": "SDL3.dll"}, {"extra": runtime.PREFIX + "rogue.dll"},
                         {"extra": "../../outside"}, {"extra": runtime.PREFIX}, {"duplicate": "scrcpy.exe"},
                         {"extra": "platform-tools/adb.exe"}, {"unsafe_link": True}):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as location:
                root = Path(location)
                archive, pin = bundle(root, **mutation)
                with self.assertRaises(runtime.Refusal):
                    runtime.prepare(archive, root / "prepared", pin)
                self.assertFalse((root / "prepared").exists())
                self.assertFalse((root / "outside").exists())

    def test_bad_server_or_pe_and_incomplete_owned_directory_fail(self):
        with tempfile.TemporaryDirectory() as location:
            root = Path(location)
            archive, pin = bundle(root, bad_pe=True)
            with self.assertRaises(runtime.Refusal):
                runtime.prepare(archive, root / "bad", pin)
            archive, pin = bundle(root)
            wrong_server = runtime.Pin(pin.version, pin.url, pin.archive_bytes, pin.archive_sha256,
                                       "0" * 64, pin.members, pin.retain, pin.omit, pin.pe_files)
            with self.assertRaises(runtime.Refusal):
                runtime.prepare(archive, root / "bad-server", wrong_server)
            target = root / "prepared"
            runtime.prepare(archive, target, pin)
            (target / "scrcpy-server").unlink()
            with self.assertRaises(runtime.Refusal):
                runtime.verify(target, archive, pin)

    def test_existing_destination_and_unrelated_files_are_preserved(self):
        with tempfile.TemporaryDirectory() as location:
            root = Path(location)
            archive, pin = bundle(root)
            existing = root / "prepared"
            existing.mkdir()
            (existing / "keep.txt").write_text("owner data")
            with self.assertRaises(runtime.Refusal):
                runtime.prepare(archive, existing, pin)
            self.assertEqual("owner data", (existing / "keep.txt").read_text())
            self.assertEqual({"keep.txt"}, {p.name for p in existing.iterdir()})
            target = root / "good"
            runtime.prepare(archive, target, pin)
            (target / "unexpected.txt").write_text("do not delete")
            with self.assertRaises(runtime.Refusal):
                runtime.verify(target, archive, pin)
            self.assertEqual("do not delete", (target / "unexpected.txt").read_text())

    def test_inventory_is_byte_bounded_and_canonical_even_when_json_object_matches(self):
        with tempfile.TemporaryDirectory() as location:
            root = Path(location)
            archive, pin = bundle(root)
            target = root / "prepared"
            runtime.prepare(archive, target, pin)
            inventory = target / "inventory.json"
            original = inventory.read_bytes()
            inventory.write_bytes(original + b" " * (8 * 1024 * 1024))
            with self.assertRaisesRegex(runtime.Refusal, "16 KiB byte limit"):
                runtime.verify(target, archive, pin)
            inventory.write_bytes(original + b" ")
            with self.assertRaisesRegex(runtime.Refusal, "differs"):
                runtime.verify(target, archive, pin)
            inventory.write_bytes(original)
            self.assertEqual(pin.archive_sha256, runtime.verify(target, archive, pin)["archive"]["sha256"])


if __name__ == "__main__":
    unittest.main()
