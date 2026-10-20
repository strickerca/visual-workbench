"""Small synthetic, stdlib-only checks. No builds, real packages or devices."""
from contextlib import ExitStack, redirect_stdout, nullcontext
import copy
import errno
import hashlib
import io
import json
import os
from pathlib import Path
import re
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import assemble as a


class Assembly(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vw-release-unit-")
        self.base = Path(self.temp.name)
        self.root = self.base / "source"
        self.root.mkdir()
        self.destination = self.base / "delivery"
        self.put("project.txt", b"synthetic source\n")
        self.put("built/artifact.bin", b"synthetic compiled bytes\n")
        self.put("third_party/notices/MIT.txt", b"synthetic full notice\n")
        self.put("third_party/LICENSES", a.json_bytes({"schema_version": 1, "files": [
            {"path": "notices/MIT.txt", "license": "MIT", "source_url": "https://example.test/notice",
             "sha256": self.row("third_party/notices/MIT.txt")["sha256"]}]}))
        artifact = self.row("built/artifact.bin") | {"destination": "artifacts/artifact.bin", "kind": "opaque"}
        self.proof = {"schema": "vw-release-source-binding-v1", "source_revision": "1" * 40,
                      "sources": [self.row("project.txt")], "artifacts": [a.descriptor({key: artifact[key] for key in ("path", "bytes", "sha256")})]}
        self.spec = {"schema": "vw-release-assembly-v1", "release_id": "synthetic-1", "binding": {},
                     "artifacts": [artifact], "notice_manifest": self.row("third_party/LICENSES"),
                     "notices": [self.row("third_party/notices/MIT.txt")], "catalog": None}
        self.refresh()

    def tearDown(self):
        self.temp.cleanup()

    def put(self, relative, body):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
        return path

    def row(self, relative):
        body = (self.root / relative).read_bytes()
        return {"path": relative, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()}

    def refresh(self):
        self.put(".local/binding.json", a.json_bytes(self.proof))
        self.spec["binding"] = self.row(".local/binding.json")
        self.put(".local/manifest.json", a.json_bytes(self.spec))
        self.digest = self.row(".local/manifest.json")["sha256"]

    def run_assembly(self):
        return a.assemble(self.root, ".local/manifest.json", self.digest, self.destination)

    def refuses(self, code, call=None):
        with self.assertRaisesRegex((ValueError, OSError), "^" + re.escape(code) + "$"):
            (call or self.run_assembly)()

    def incomplete(self):
        self.assertTrue(self.destination.is_dir())
        marker = self.destination / "COMPLETE"
        if marker.exists():
            self.assertEqual(marker.read_bytes(), b"")

    def before(self):
        return {p.relative_to(self.root).as_posix(): (p.read_bytes(), a.snapshot(p.stat()))
                for p in self.root.rglob("*") if p.is_file()}

    def replace_artifact(self, relative, body, kind):
        self.put(relative, body)
        self.spec["artifacts"] = [self.row(relative) | {"destination": "artifacts/" + Path(relative).name, "kind": kind}]
        self.proof["artifacts"] = [self.row(relative)]
        self.refresh()

    def test_assembly_hashes_inventory_and_preserves_originals(self):
        before = self.before()
        result = self.run_assembly()
        self.assertEqual(self.before(), before)
        self.assertEqual(result["status"], "assembled-hash-verified")
        self.assertFalse(result["built"])
        self.assertFalse(result["signed_or_signature_verified"])
        self.assertFalse(result["installed_or_launched"])
        self.assertFalse(result["device_or_physical_acceptance"])
        self.assertEqual((self.destination / "artifacts/artifact.bin").read_bytes(), b"synthetic compiled bytes\n")
        inventory = json.loads((self.destination / "inventory.json").read_bytes())
        self.assertEqual(inventory, result)
        delivered = {p.relative_to(self.destination).as_posix() for p in self.destination.rglob("*") if p.is_file()}
        self.assertEqual(delivered, set(result["files"]) | {"inventory.json", "SHA256SUMS", "COMPLETE"})
        for line in (self.destination / "SHA256SUMS").read_text().splitlines():
            digest, relative = line.split("  ", 1)
            self.assertEqual(hashlib.sha256((self.destination / relative).read_bytes()).hexdigest(), digest)
        marker = (self.destination / "COMPLETE").read_text()
        self.assertEqual(marker, hashlib.sha256((self.destination / "inventory.json").read_bytes()).hexdigest() + "  inventory.json\n")
        input_names = {row["path"] for row in result["inputs"]}
        self.assertEqual(input_names, {"project.txt", "built/artifact.bin", ".local/binding.json", ".local/manifest.json",
                                      "third_party/LICENSES", "third_party/notices/MIT.txt"})
        text = (self.destination / "THIRD-PARTY-NOTICES.txt").read_text()
        self.assertIn(self.row("third_party/notices/MIT.txt")["sha256"], text)
        self.assertIn("https://example.test/notice", text)
        self.assertEqual(result["notices"][0]["bytes"], len(b"synthetic full notice\n"))

    def test_existing_destination_is_preserved(self):
        self.destination.mkdir()
        (self.destination / "keep").write_bytes(b"old delivery")
        with self.assertRaises(FileExistsError):
            self.run_assembly()
        self.assertEqual((self.destination / "keep").read_bytes(), b"old delivery")
        self.assertEqual(len(list(self.destination.iterdir())), 1)

    def test_manifest_hash_and_source_hash_refuse_before_destination(self):
        self.digest = "0" * 64
        self.refuses("manifest_binding")
        self.assertFalse(self.destination.exists())
        self.refresh()
        self.put("project.txt", b"wrong current source")
        self.refuses("input_binding")
        self.assertFalse(self.destination.exists())

    def test_bad_artifact_hash_retains_staging_and_preserves_source(self):
        self.put("built/artifact.bin", b"wrong built bytes")
        before = self.before()
        self.refuses("input_binding")
        self.incomplete()
        self.assertEqual(self.before(), before)

    def test_explicit_build_binding_must_exactly_cover_artifacts(self):
        self.proof["artifacts"] = []
        self.refresh()
        self.refuses("binding_artifacts")
        self.assertFalse(self.destination.exists())

    def test_descriptors_are_typed_and_bounded_before_io(self):
        for field, value, code in (("bytes", True, "descriptor_binding"), ("bytes", a.MAX_FILE + 1, "descriptor_binding"),
                                   ("sha256", "A" * 64, "descriptor_binding")):
            spec = copy.deepcopy(self.spec)
            spec["artifacts"][0][field] = value
            self.refuses(code, lambda: a.manifest(spec))
        with patch.object(a, "MAX_ARTIFACTS", 0):
            self.refuses("artifact_count", lambda: a.manifest(self.spec))
        with patch.object(a, "MAX_TOTAL", 1):
            self.refuses("artifact_total", lambda: a.manifest(self.spec))
        with patch.object(a, "MAX_SOURCE_TOTAL", 1):
            self.refuses("source_total", lambda: a.binding(self.proof, self.spec["artifacts"]))
        with patch.object(a, "MAX_JSON", 2):
            self.refuses("input_size")

    def test_unsafe_paths_case_aliases_and_prefix_collisions_refuse(self):
        for value in ("../escape", "/absolute", "a//b", "a/./b", "a\\b", "a:stream", "CON.txt", "trailing.", "x/COM1.log", "a\nname", "a?b"):
            with self.subTest(value=value):
                self.refuses("unsafe_relative_path", lambda: a.name(value))
        self.refuses("duplicate_path", lambda: a.unique_paths([{"path": "A.txt"}, {"path": "a.txt"}]))
        self.refuses("path_collision", lambda: a.unique_paths([{"path": "a"}, {"path": "a/b"}]))
        self.refuses("path_collision", lambda: a.unique_paths([{"path": "a"}, {"path": "a-"}, {"path": "a/b"}]))
        self.spec["artifacts"][0]["destination"] = "inventory.json"
        self.refresh()
        self.refuses("artifact_kind_or_destination")

    def test_global_source_case_alias_is_refused(self):
        self.proof["sources"].append(self.row("built/artifact.bin") | {"path": "BUILT/artifact.bin"})
        self.refresh()
        self.refuses("input_case_alias")

    def test_json_duplicates_unknown_keys_and_constants_refuse(self):
        self.refuses("json_duplicate", lambda: a.parse(b'{"schema":1,"schema":2}'))
        self.refuses("json_constant", lambda: a.parse(b'{"x":NaN}'))
        self.refuses("json_encoding_or_structure", lambda: a.parse(b'\xff'))
        self.refuses("json_object", lambda: a.parse(b'[]'))
        self.refuses("manifest_schema", lambda: a.manifest(self.spec | {"build": True}))

    def test_notice_gate_uses_actual_bounded_input_inventory(self):
        self.spec["notices"].append(self.row("third_party/notices/MIT.txt") | {"path": "third_party/notices/extra.txt"})
        self.put("third_party/notices/extra.txt", b"synthetic full notice\n")
        self.refresh()
        self.refuses("notice_unlisted_source")
        self.incomplete()

    def test_notice_manifest_requires_every_declared_notice(self):
        source = json.loads((self.root / "third_party/LICENSES").read_bytes())
        source["files"].append(source["files"][0] | {"path": "notices/missing.txt"})
        self.put("third_party/LICENSES", a.json_bytes(source))
        self.spec["notice_manifest"] = self.row("third_party/LICENSES")
        self.refresh()
        with self.assertRaises(OSError):
            self.run_assembly()
        self.incomplete()

    def test_notice_hash_change_and_printable_metadata_gate(self):
        source = json.loads((self.root / "third_party/LICENSES").read_bytes())
        source["files"][0]["license"] = "MIT\nforged line"
        self.put("third_party/LICENSES", a.json_bytes(source))
        self.spec["notice_manifest"] = self.row("third_party/LICENSES")
        self.refresh()
        self.refuses("notice_metadata")
        self.incomplete()

    def test_real_apk_validator_is_reused_and_no_acceptance_added(self):
        path = Path(a.__file__).absolute().parents[1] / "ffi-test/test_apk.py"
        with patch.dict(sys.modules, {"check_apk": a.APK}):
            fixtures = a._module("vw_release_apk_test_fixtures", path)
        self.replace_artifact("built/test.apk", fixtures.apk(), "apk")
        result = self.run_assembly()
        checked = result["package_checks"]["artifacts/test.apk"]
        self.assertEqual(checked["status"], "PASS")
        self.assertFalse(checked["signature_verified"])
        self.assertFalse(checked["runtime_16k_verified"])
        self.assertEqual(checked["extracted_files"], 0)
        self.assertEqual(len(checked["libraries"]), 2)

    def test_corrupt_apk_refuses_with_incomplete_retained_tree(self):
        self.replace_artifact("built/test.apk", b"not an apk" * 10, "apk")
        self.refuses("missing_eocd")
        self.incomplete()

    def test_msi_is_only_hash_bound_external_input(self):
        self.replace_artifact("built/test.msi", b"synthetic opaque external package", "external-package")
        result = self.run_assembly()
        self.assertEqual(result["package_checks"], {})
        self.assertIn("MSI/opaque inputs receive hash binding only", result["external_package_validation"])

    def test_missing_release_documents_are_explicit_prerequisites(self):
        result = self.run_assembly()
        self.assertFalse(result["release_ready"])
        self.assertEqual(result["release_prerequisites"], {
            key: {"status": "missing-release-prerequisite"} for key in a.DOCUMENTS})

    def test_release_documents_copy_exact_inputs_without_claiming_tool_versions(self):
        self.put("docs/toolchain.md", b"synthetic recorded pins; producer versions unverified\n")
        self.put("docs/recovery.md", b"synthetic reviewed guide; installation unverified\n")
        self.spec["release_documents"] = {"toolchain_record": self.row("docs/toolchain.md"),
                                          "uninstall_recovery_guide": self.row("docs/recovery.md")}
        self.spec["external_dependencies"] = [{"id": "licensed-decoder", "reason": "No admitted licensed binary supplied"}]
        self.refresh()
        result = self.run_assembly()
        for role, target in a.DOCUMENTS.items():
            row = self.spec["release_documents"][role]
            self.assertEqual((self.destination / target).read_bytes(), (self.root / row["path"]).read_bytes())
            self.assertEqual(result["release_prerequisites"][role]["status"], "supplied-hash-verified-content-not-accepted")
            self.assertEqual(result["files"][target], a.blob(row))
        self.assertFalse(result["release_ready"])
        self.assertEqual(result["external_dependencies"], self.spec["external_dependencies"])

    def test_release_document_and_dependency_limits_refuse(self):
        self.spec["release_documents"] = {"toolchain_record": self.row("project.txt")}
        self.refuses("release_documents", lambda: a.manifest(self.spec))
        del self.spec["release_documents"]
        self.spec["external_dependencies"] = [{"id": "decoder", "reason": "a\nb"}]
        self.refuses("external_dependency", lambda: a.manifest(self.spec))

    def test_packaged_types_cannot_silently_skip_existing_gate(self):
        for relative, kind, code in (("built/test.apk", "opaque", "apk_kind"), ("built/test.msi", "opaque", "msi_kind"),
                                     (a.DESKTOP + "file.bin", "opaque", "desktop_kind")):
            spec = copy.deepcopy(self.spec)
            spec["artifacts"][0].update(path=relative, kind=kind)
            self.refuses(code, lambda: a.manifest(spec))

    def test_desktop_adapter_requires_explicit_apk_and_full_layout(self):
        self.replace_artifact(a.DESKTOP + "app/item.bin", b"synthetic desktop", "desktop-file")
        self.refuses("personal_package_requires_explicit_apk")
        self.incomplete()

    def test_fresh_destination_constructor_failure_retains_owned_directory(self):
        original = a.get_dir
        def fail(path, stack, cache):
            if path == self.destination:
                raise OSError("synthetic directory lease failure")
            return original(path, stack, cache)
        with patch.object(a, "get_dir", fail):
            with self.assertRaises(OSError):
                self.run_assembly()
        self.incomplete()

    def test_copy_failure_retains_new_tree_and_preserves_source(self):
        original = a.Output.write
        def fail(owner, relative, *args, **kwargs):
            value = original(owner, relative, *args, **kwargs)
            if relative == "artifacts/artifact.bin":
                raise OSError("synthetic disk error")
            return value
        before = self.before()
        with patch.object(a.Output, "write", fail):
            with self.assertRaises(OSError):
                self.run_assembly()
        self.incomplete()
        self.assertEqual(self.before(), before)

    def test_same_bytes_replacement_after_copy_is_detected(self):
        original = a.Output.write
        def swap(owner, relative, *args, **kwargs):
            value = original(owner, relative, *args, **kwargs)
            if relative == "artifacts/artifact.bin":
                path = self.root / "built/artifact.bin"
                replacement = path.with_suffix(".replacement")
                replacement.write_bytes(path.read_bytes())
                os.replace(replacement, path)
            return value
        with patch.object(a.Output, "write", swap):
            self.refuses("input_identity_changed")
        self.incomplete()

    def test_same_bytes_output_replacement_refuses_and_preserves_foreign_file(self):
        original = a.Output.write
        def swap(owner, relative, *args, **kwargs):
            value = original(owner, relative, *args, **kwargs)
            if relative == "artifacts/artifact.bin":
                path = owner.root / relative
                replacement = path.with_suffix(".replacement")
                replacement.write_bytes(path.read_bytes())
                os.replace(replacement, path)
            return value
        with patch.object(a.Output, "write", swap):
            self.refuses("output_identity_changed")
        self.assertEqual((self.destination / "artifacts/artifact.bin").read_bytes(), b"synthetic compiled bytes\n")
        self.assertFalse((self.destination / "COMPLETE").exists())

    def test_source_mutation_while_streaming_is_detected(self):
        original = a.Directory.open
        source_path = self.root / "built/artifact.bin"
        source_before = source_path.read_bytes()
        mutation_attempts = []
        # Establish that the fixture admits a writer before the retained lease.
        with source_path.open("r+b") as stream:
            self.assertEqual(stream.read(), source_before)
        def native_writer_is_refused():
            import ctypes
            from ctypes import wintypes
            api = ctypes.WinDLL("kernel32", use_last_error=True)
            api.CreateFileW.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                       wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE)
            api.CreateFileW.restype = wintypes.HANDLE
            api.CloseHandle.argtypes = (wintypes.HANDLE,)
            api.CloseHandle.restype = wintypes.BOOL
            ctypes.set_last_error(0)
            # Request a second writer without creating, truncating or writing.
            handle = api.CreateFileW(str(source_path), 0x40000000, 7, None, 3, 0x00200000, None)
            error = ctypes.get_last_error()
            invalid = wintypes.HANDLE(-1).value
            if handle not in (None, invalid):
                self.assertTrue(api.CloseHandle(handle))
            self.assertEqual(handle, invalid)
            self.assertEqual(error, 32)  # ERROR_SHARING_VIOLATION, not access denied.
        class MutatingStream:
            def __init__(self, stream): self.stream, self.changed = stream, False
            def __enter__(self): return self
            def __exit__(self, *args): return self.stream.__exit__(*args)
            def fileno(self): return self.stream.fileno()
            def read(self, count):
                data = self.stream.read(count)
                if data and not self.changed:
                    self.changed = True
                    mutation_attempts.append(True)
                    if os.name == "nt":
                        native_writer_is_refused()
                    source_path.write_bytes(b"concurrently changed bytes")
                return data
        def changed(parent, leaf, write=False):
            stream = original(parent, leaf, write)
            return MutatingStream(stream) if parent.path / leaf == source_path and not write else stream
        with patch.object(a.Directory, "open", changed):
            if os.name == "nt":
                # The native read lease denies this second writer. CPython's
                # CRT open can expose sharing violation 32 as EACCES without a
                # winerror attribute; neither form permits source mutation.
                with self.assertRaises(PermissionError) as refused:
                    self.run_assembly()
                self.assertEqual(refused.exception.errno, errno.EACCES)
                self.assertIn(getattr(refused.exception, "winerror", None), (None, 32))
                self.assertEqual(refused.exception.filename, str(source_path))
                self.assertEqual(source_path.read_bytes(), source_before)
            else:
                self.refuses("input_changed")
                self.assertEqual(source_path.read_bytes(), b"concurrently changed bytes")
        self.assertEqual(mutation_attempts, [True])
        if os.name == "nt":
            # The same writer is admitted again once the failed read retires.
            with source_path.open("r+b") as stream:
                self.assertEqual(stream.read(), source_before)
        self.incomplete()
        self.assertFalse((self.destination / "COMPLETE").exists())

    def test_simulated_windows_reparse_attributes_refuse(self):
        info = (self.root / "project.txt").stat()
        proxy = SimpleNamespace(st_mode=info.st_mode, st_nlink=1, st_file_attributes=0x400)
        self.refuses("redirected_path", lambda: a.ordinary(proxy, False))

    @unittest.skipUnless(os.name == "posix", "POSIX output hardlink race fixture")
    def test_output_hardlink_created_during_open_refuses_before_writing(self):
        original = a.Directory.open
        alias = self.base / "foreign-output-alias"
        def linked(parent, leaf, write=False):
            stream = original(parent, leaf, write)
            if write and leaf == "artifact.bin":
                os.link(parent.path / leaf, alias)
            return stream
        with patch.object(a.Directory, "open", linked):
            self.refuses("hardlinked_input")
        self.assertEqual(alias.read_bytes(), b"")
        self.assertFalse((self.destination / "COMPLETE").exists())

    @unittest.skipUnless(os.name == "posix", "POSIX no-follow race fixture")
    def test_symlink_leaf_parent_and_root_ancestor_refuse(self):
        path = self.root / "built/artifact.bin"
        target = self.base / "target.bin"
        target.write_bytes(path.read_bytes())
        path.unlink()
        path.symlink_to(target)
        self.refuses("redirected_path")
        self.incomplete()
        self.destination = self.base / "delivery-parent-link"
        path.unlink()
        path.write_bytes(target.read_bytes())
        (self.root / "built").rename(self.root / "original-built")
        (self.root / "built").symlink_to("original-built", target_is_directory=True)
        self.refuses("redirected_path")
        self.incomplete()
        self.destination = self.base / "delivery-root-link"
        alias = self.base / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        self.refuses("redirected_path", lambda: a.assemble(alias, ".local/manifest.json", self.digest, self.destination))

    @unittest.skipUnless(os.name == "posix", "POSIX descriptor-relative race fixture")
    def test_parent_substitution_after_copy_refuses_without_following_link(self):
        original = a.Output.write
        def swap(owner, relative, *args, **kwargs):
            value = original(owner, relative, *args, **kwargs)
            if relative == "artifacts/artifact.bin":
                (self.root / "built").rename(self.root / "prior-built")
                (self.root / "built").symlink_to("prior-built", target_is_directory=True)
            return value
        with patch.object(a.Output, "write", swap):
            self.refuses("redirected_path")
        self.incomplete()

    @unittest.skipUnless(os.name == "posix", "POSIX hardlink/FIFO fixtures")
    def test_hardlink_and_fifo_are_refused_without_reading(self):
        path = self.root / "built/artifact.bin"
        os.link(path, self.base / "hardlink")
        self.refuses("hardlinked_input")
        self.incomplete()
        self.destination = self.base / "delivery-fifo"
        (self.base / "hardlink").unlink()
        path.unlink()
        os.mkfifo(path)
        self.refuses("path_type")
        self.incomplete()

    @unittest.skipUnless(os.name == "posix", "POSIX stale-stat cleanup regression")
    def test_cleanup_preserves_foreign_replacement_after_stale_stat(self):
        with ExitStack() as stack:
            owner = a.Output(self.destination, stack, {})
            owner.write("artifacts/victim.dll", b"owned bytes")
            parent, leaf = owner.parent("artifacts/victim.dll")
            path = self.destination / "artifacts/victim.dll"
            original_stat = a.Directory.stat
            stale = original_stat(parent, leaf)
            replacement = self.base / "foreign-replacement"
            replacement.write_bytes(b"foreign bytes must survive")
            injected = False
            def raced_stat(directory, child):
                nonlocal injected
                if directory.path / child == path:
                    if not injected:
                        os.replace(replacement, path)  # After stat, before the old unlink.
                        injected = True
                    return stale
                return original_stat(directory, child)
            with patch.object(a.Directory, "stat", raced_stat), \
                    patch.object(a.os, "unlink", wraps=os.unlink) as unlink, \
                    patch.object(a.os, "rmdir", wraps=os.rmdir) as rmdir:
                self.assertEqual(a.identity(parent.stat(leaf)), a.identity(stale))
                owner.cleanup()
                unlink.assert_not_called()
                rmdir.assert_not_called()
            self.assertTrue(injected)
            self.assertEqual(path.read_bytes(), b"foreign bytes must survive")
            self.incomplete()

    def test_foreign_file_injected_during_validation_refuses_without_complete(self):
        original = a.validate_packages
        def injected(root, output, *args):
            (output.root / "artifacts/foreign.dll").write_bytes(b"foreign file")
            return original(root, output, *args)
        before = self.before()
        with patch.object(a, "validate_packages", injected):
            self.refuses("output_namespace_unlisted")
        self.incomplete()
        self.assertFalse((self.destination / "COMPLETE").exists())
        self.assertEqual((self.destination / "artifacts/foreign.dll").read_bytes(), b"foreign file")
        self.assertEqual(self.before(), before)

    def test_foreign_empty_directory_is_not_omitted_from_namespace(self):
        original = a.validate_packages
        def injected(root, output, *args):
            (output.root / "foreign-directory").mkdir()
            return original(root, output, *args)
        with patch.object(a, "validate_packages", injected):
            self.refuses("output_namespace_unlisted")
        self.incomplete()
        self.assertTrue((self.destination / "foreign-directory").is_dir())

    @unittest.skipUnless(os.name == "posix", "POSIX foreign symlink namespace fixture")
    def test_foreign_directory_symlink_is_refused_without_traversal(self):
        original = a.validate_packages
        target = self.base / "outside"
        target.mkdir()
        (target / "keep").write_bytes(b"outside content")
        def injected(root, output, *args):
            (output.root / "artifacts/foreign-link").symlink_to(target, target_is_directory=True)
            return original(root, output, *args)
        with patch.object(a, "validate_packages", injected):
            self.refuses("output_namespace_unlisted")
        self.incomplete()
        self.assertTrue((self.destination / "artifacts/foreign-link").is_symlink())
        self.assertEqual((target / "keep").read_bytes(), b"outside content")

    def test_whole_namespace_entry_limit_is_enforced(self):
        with patch.object(a, "MAX_OUTPUT_ENTRIES", 1):
            self.refuses("output_namespace_limit")
        self.incomplete()
        self.assertFalse((self.destination / "COMPLETE").exists())

    def test_duplicate_namespace_entries_refuse_under_fixed_enumeration_bound(self):
        original = a.Directory.entries
        def repeated(directory):
            if directory.path == self.destination:
                return nullcontext([SimpleNamespace(name="release-manifest.json")] * 2)
            return original(directory)
        with patch.object(a.Directory, "entries", repeated):
            self.refuses("output_namespace_duplicate")
        self.incomplete()
        self.assertFalse((self.destination / "COMPLETE").exists())

    def test_final_gate_includes_complete_and_invalidates_owned_marker(self):
        original = a.Output.write
        def injected(owner, relative, *args, **kwargs):
            value = original(owner, relative, *args, **kwargs)
            if relative == "COMPLETE":
                (owner.root / "artifacts/foreign.dll").write_bytes(b"late foreign file")
            return value
        with patch.object(a.Output, "write", injected):
            self.refuses("output_namespace_unlisted")
        self.incomplete()
        self.assertEqual((self.destination / "COMPLETE").read_bytes(), b"")
        self.assertEqual((self.destination / "artifacts/foreign.dll").read_bytes(), b"late foreign file")

    def test_final_gate_rejects_known_file_replacement(self):
        original = a.Output.write
        def replaced(owner, relative, *args, **kwargs):
            value = original(owner, relative, *args, **kwargs)
            if relative == "COMPLETE":
                path = owner.root / "artifacts/artifact.bin"
                replacement = self.base / "late-same-bytes"
                replacement.write_bytes(path.read_bytes())
                os.replace(replacement, path)
            return value
        with patch.object(a.Output, "write", replaced):
            self.refuses("output_identity_changed")
        self.incomplete()

    @unittest.skipUnless(os.name == "posix", "POSIX retained marker descriptor fixture")
    def test_marker_invalidation_targets_owned_descriptor_not_foreign_path(self):
        original = a.Output.write
        moved = self.base / "owned-marker-moved"
        def replaced(owner, relative, *args, **kwargs):
            value = original(owner, relative, *args, **kwargs)
            if relative == "COMPLETE":
                path = owner.root / relative
                path.rename(moved)
                path.write_bytes(b"foreign marker must survive")
            return value
        with patch.object(a.Output, "write", replaced):
            self.refuses("output_identity_changed")
        self.assertEqual((self.destination / "COMPLETE").read_bytes(), b"foreign marker must survive")
        self.assertEqual(moved.read_bytes(), b"")

    def test_output_containment_and_missing_parent_refuse(self):
        before = self.before()
        paths_before = set(self.base.rglob("*"))
        self.destination = self.root
        self.refuses("output_root")
        self.destination = self.root / "built/artifact.bin/child"
        self.refuses("output_overlaps_input")
        self.destination = self.base / "missing/child"
        if os.name == "nt":
            with self.assertRaisesRegex(a.Rejected, "^path_lease_unavailable$"):
                self.run_assembly()
        else:
            with self.assertRaises(FileNotFoundError):
                self.run_assembly()
        self.assertFalse(self.destination.parent.exists())
        self.assertFalse(self.destination.exists())
        self.assertEqual(self.before(), before)
        self.assertEqual(set(self.base.rglob("*")), paths_before)

    def test_cli_returns_only_static_refusal_without_private_paths(self):
        argv = ["assemble.py", "--root", str(self.root), "--manifest", ".local/manifest.json",
                "--manifest-sha256", "0" * 64, "--destination", str(self.destination)]
        output = io.StringIO()
        with patch.object(sys, "argv", argv), redirect_stdout(output):
            self.assertEqual(a.main(), 1)
        self.assertEqual(output.getvalue(), "RELEASE_ASSEMBLY_REJECTED:manifest_binding\n")


class WindowsMetadata(unittest.TestCase):
    def native(self, **changes):
        fields = {"attributes": 0x80, "size": 7, "links": 1, "volume": 42,
                  "file_id": bytes(range(1, 17)), "written": 116444736000000123,
                  "changed": 116444736000000456, "created": 116444736000000001}
        return a.windows_metadata(**(fields | changes))

    def test_native_snapshot_keeps_change_time_distinct_from_creation_time(self):
        native = self.native()
        legacy_path = SimpleNamespace(**vars(native))
        legacy_path.st_ctime_ns = legacy_path.st_birthtime_ns
        self.assertNotEqual(a.snapshot(native), a.snapshot(legacy_path))
        self.assertEqual(native.st_ctime_ns, 45600)
        self.assertEqual(native.st_birthtime_ns, 100)
        self.assertNotEqual(a.snapshot(native), a.snapshot(self.native(changed=116444736000000457)))
        self.assertEqual(a.identity(native), a.identity(self.native(changed=116444736000000457)))

    def test_native_identity_size_mtime_links_and_reparse_checks_remain_active(self):
        original = self.native()
        for changed in ({"volume": 43}, {"file_id": b"x" * 16}, {"size": 8}, {"links": 2},
                        {"written": 116444736000000124}):
            with self.subTest(changed=changed):
                self.assertNotEqual(a.snapshot(original), a.snapshot(self.native(**changed)))
        with self.assertRaisesRegex(a.Rejected, "^hardlinked_input$"):
            a.ordinary(self.native(links=2), False)
        with self.assertRaisesRegex(a.Rejected, "^redirected_path$"):
            a.ordinary(self.native(attributes=0x480), False)
        with self.assertRaisesRegex(a.Rejected, "^native_metadata$"):
            self.native(file_id=bytes(16))

    def test_native_handle_queries_all_required_fields_without_clock_fallback(self):
        calls = []

        class Api:
            def GetFileInformationByHandleEx(owner, handle, kind, pointer, size):
                calls.append((kind, size))
                value = pointer._obj
                if kind == 0:
                    value.created, value.written, value.changed, value.attributes = (
                        116444736000000001, 116444736000000123, 116444736000000456, 0x80)
                elif kind == 1:
                    value.size, value.links, value.directory = 7, 1, 0
                elif kind == 18:
                    value.volume = 42
                    value.file_id[:] = bytes(range(1, 17))
                return 1

        native = a.windows_handle_metadata(123, Api())
        self.assertEqual(a.snapshot(native), a.snapshot(self.native()))
        self.assertEqual(calls, [(0, 40), (1, 24), (18, 24)])
        failed = SimpleNamespace(GetFileInformationByHandleEx=lambda *_: 0)
        with self.assertRaisesRegex(a.Rejected, "^native_metadata_unavailable$"):
            a.windows_handle_metadata(123, failed)

    def test_path_metadata_uses_short_shared_query_and_closes_it(self):
        expected = self.native()
        events = []

        class Lease:
            def __init__(owner, path, directory, *, metadata_only):
                events.append((path, directory, metadata_only))
            def metadata(owner):
                return expected
            def close(owner):
                events.append("closed")

        path = Path("fixture")
        with patch.object(a, "WindowsLease", Lease):
            self.assertIs(a.windows_path_metadata(path), expected)
        self.assertEqual(events, [(path, None, True), "closed"])

    def test_change_time_only_mutation_during_input_read_is_refused(self):
        with tempfile.TemporaryDirectory(prefix="vw-metadata-unit-") as tmp, ExitStack() as stack:
            root = Path(tmp)
            (root / "input.bin").write_bytes(b"unchanged bytes")
            source = a.Inputs(root, stack, {})
            original = a.fstat_metadata
            calls = 0

            def changed(fd):
                nonlocal calls
                current = original(fd)
                calls += 1
                if calls == 1:
                    return current
                return SimpleNamespace(**{key: (current.st_ctime_ns + 100 if key == "st_ctime_ns" else getattr(current, key))
                    for key in ("st_dev", "st_ino", "st_mode", "st_size", "st_mtime_ns", "st_ctime_ns", "st_nlink")})

            with patch.object(a, "fstat_metadata", changed), self.assertRaisesRegex(a.Rejected, "^input_changed$"):
                source.read("input.bin", None, 1024)

    @unittest.skipUnless(os.name == "nt", "actual Windows path/handle metadata witness")
    def test_windows_open_writer_and_closed_path_have_coherent_native_metadata(self):
        with tempfile.TemporaryDirectory(prefix="vw-metadata-unit-") as tmp, ExitStack() as stack:
            directory = a.get_dir(Path(tmp), stack, {})
            with directory.open("owned.bin", True) as stream:
                stream.write(b"owned bytes")
                stream.flush()
                os.fsync(stream.fileno())
                opened = a.fstat_metadata(stream.fileno())
                by_path = directory.stat("owned.bin")
                self.assertEqual(a.snapshot(opened), a.snapshot(by_path))
            self.assertEqual(a.snapshot(opened), a.snapshot(directory.stat("owned.bin")))

    @unittest.skipUnless(os.name == "nt", "actual Windows restored-mtime mutation witness")
    def test_windows_restoring_mtime_does_not_hide_change_clock(self):
        with tempfile.TemporaryDirectory(prefix="vw-metadata-unit-") as tmp, ExitStack() as stack:
            root = Path(tmp)
            path = root / "input.bin"
            path.write_bytes(b"original")
            directory = a.get_dir(root, stack, {})
            before = directory.stat("input.bin")
            source = a.Inputs(root, stack, {})
            source.read("input.bin", None, 1024)
            path.write_bytes(b"modified")
            os.utime(path, ns=(before.st_mtime_ns, before.st_mtime_ns))
            after = directory.stat("input.bin")
            self.assertEqual(before.st_mtime_ns, after.st_mtime_ns)
            self.assertEqual(before.st_size, after.st_size)
            self.assertNotEqual(before.st_ctime_ns, after.st_ctime_ns)
            with self.assertRaisesRegex(a.Rejected, "^input_identity_changed$"):
                source.read("input.bin", None, 1024)


if __name__ == "__main__":
    unittest.main()
