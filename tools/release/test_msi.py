"""Synthetic files and mocked image admissions; no child, build or installer runs."""
from contextlib import ExitStack, redirect_stdout
import copy
import io
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import msi as m


def pe(machine=0x8664):
    body = bytearray(128)
    body[:2] = b"MZ"
    struct.pack_into("<I", body, 60, 64)
    body[64:68] = b"PE\0\0"
    struct.pack_into("<H", body, 68, machine)
    return bytes(body)


def cfb():
    # Header shape only: these bytes intentionally are not an MSI database.
    body = bytearray(1536)
    body[:8] = bytes.fromhex("d0cf11e0a1b11ae1")
    struct.pack_into("<5H", body, 24, 0x3e, 3, 0xfffe, 9, 6)
    struct.pack_into("<9I", body, 40, 0, 1, 1, 0, 4096, 0xfffffffe, 0, 0xfffffffe, 0)
    struct.pack_into("<109I", body, 76, 0, *([0xffffffff] * 108))
    return bytes(body)


class MsiBackend(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.base = Path(self.stack.enter_context(tempfile.TemporaryDirectory(prefix="vw-msi-unit-")))
        self.root = self.base / "source"
        self.root.mkdir()
        self.work = self.base / "work"
        self.manifest_name = ".local/msi-toolchain.json"
        self.plan_path = self.work / "plan.json"
        self.process_path = self.work / "process.json"
        self.actual_python = m.sys.executable
        self.actual_windows_directory = m.windows_directory
        # Only the two host-identity oracles use fixture values. The production
        # tool inventory, native path metadata, invocation and OS-env guards run.
        self.stack.enter_context(patch.object(m.sys, "executable", str(self.base / "python/python.exe")))
        self.stack.enter_context(patch.object(m, "windows_directory", return_value=self.base / "Windows"))
        self.put(self.base / "Windows/System32/cmd.exe", pe())
        self.stack.enter_context(patch.object(m, "ROOT", self.root))
        for name in (*m.BACKEND, m.a.REPORTS.BEGIN, m.a.REPORTS.BUILT, "project.txt",
                     "apps/android/build/outputs/apk/debug/android-debug.apk", "target/debug/vw_core.dll",
                     m.a.REPORTS.DIST + "/VisualWorkbenchDev.exe", m.a.REPORTS.DIST + "/vw-mcp.exe"):
            self.put(self.root / name, ("synthetic input " + name).encode())
        for name in ("inventory", "compatibility", "license_review"):
            self.put(self.root / (".local/" + name + ".txt"), ("synthetic evidence " + name).encode())
        self.spec = {"schema": "vw-msi-toolchain-v1", "evidence": {
            name: self.row(self.root, ".local/" + name + ".txt")
            for name in ("inventory", "compatibility", "license_review")}}
        for kind, version, roles in (
            ("jdk", "21.0.12.1+1", {"jpackage": "bin/jpackage.exe", "java": "bin/java.exe"}),
            ("wix", "3.99.0-fixture", {"candle": "bin/candle.exe", "light": "bin/light.exe"}),
            ("python", "3.14.3", {"python": "python.exe"}),
        ):
            tool_root = self.base / kind
            for path in roles.values():
                self.put(tool_root / path, pe(0x14c if kind == "wix" else 0x8664))
            self.put(tool_root / ("Lib/runtime.dat" if kind == "python" else "lib/runtime.dat"),
                     ("synthetic complete " + kind).encode())
            if kind == "python":
                self.put(tool_root / "python314.dll", b"synthetic Python runtime DLL")
                self.put(tool_root / "Lib/os.py", b"# synthetic standard library, never imported\n")
            self.spec[kind] = {"root": str(tool_root), "version": version, "roles": roles,
                               "files": self.tool_rows(tool_root)}
        self.built = {"nonce": "a" * 32, "source": {"project.txt": self.digest(self.root / "project.txt")},
                      "distribution": {"native_inputs": {"vw_core.dll": self.digest(self.root / "target/debug/vw_core.dll")},
                          "files": {name: self.digest(self.root / m.a.REPORTS.DIST / name)
                                    for name in ("VisualWorkbenchDev.exe", "vw-mcp.exe")}}}
        self.actual_image_check = m.a.REPORTS.check
        self.image_check = self.stack.enter_context(patch.object(m.a.REPORTS, "check", return_value=self.built))
        self.personal_check = self.stack.enter_context(patch.object(m.a.PERSONAL, "verify", return_value=(
            {"desktop_build_nonce": self.built["nonce"], "status": "synthetic mocked admission"}, "unused")))
        self.refresh()

    def put(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
        return path

    def digest(self, path):
        return m.blob(path.read_bytes())

    def row(self, root, relative):
        return {"path": relative, **self.digest(root / relative)}

    def tool_rows(self, root):
        return [self.row(root, path.relative_to(root).as_posix()) for path in sorted(root.rglob("*")) if path.is_file()]

    def refresh(self):
        raw = m.a.json_bytes(self.spec)
        self.put(self.root / self.manifest_name, raw)
        self.manifest_sha = m.blob(raw)["sha256"]

    def plan(self, **extra):
        self.planned = m.plan(self.root, self.manifest_name, self.manifest_sha, self.work, 120, **extra)
        self.plan_sha = self.digest(self.plan_path)["sha256"]
        return self.planned

    def check(self):
        return m.check(self.root, self.plan_path, self.plan_sha)

    def envelope(self):
        windows = self.base / "Windows"
        return {"schema": "vw-msi-process-v1", "run_nonce": self.planned["run_nonce"],
                "plan_sha256": self.plan_sha, **{key: self.planned[key]
                    for key in ("executable", "argv", "working_dir", "minimal_env")},
                "os_env": {"SystemRoot": str(windows), "WINDIR": str(windows),
                           "ComSpec": str(windows / "System32/cmd.exe"),
                           "PATH": self.planned["minimal_env"]["PATH"] + ";" + str(windows / "System32")},
                "runner": {"phase": "desktop-msi", "exit_code": 0, "parent_exit_code": 0,
                           "failure_reason": None, "contained_in_windows_job": True,
                           "process_tree_cleanup_confirmed": True, "output_streams_completed": True,
                           "surviving_processes_before_cleanup": [], "compiler_telemetry_cleaned": False,
                           "timeout_seconds": 120, "elapsed_seconds": 2.5}}

    def complete(self, envelope=None, package=None):
        self.put(self.work / "dest" / m.PACKAGE, cfb() if package is None else package)
        self.put(self.process_path, m.a.json_bytes(self.envelope() if envelope is None else envelope))

    def record(self):
        return m.record(self.root, self.plan_path, self.plan_sha, self.process_path)

    def refuses(self, code, call):
        with self.assertRaisesRegex(m.Rejected, "^" + code + "$"):
            call()

    def test_plan_uses_exact_finalized_image_arguments_and_isolated_environment(self):
        planned = self.plan()
        self.assertEqual(planned["argv"], ["--type", "msi", "--app-image", str(self.root / m.a.REPORTS.DIST),
            "--dest", str(self.work / "dest"), "--temp", str(self.work / "temp"),
            "--name", "VisualWorkbenchDev", "--app-version", "0.0.1", "--vendor", "Visual Workbench",
            "--win-upgrade-uuid", "127902df-9261-5ead-8845-df9bc2cd90b4", "--win-per-user-install", "--win-menu"])
        self.assertEqual(planned["minimal_env"], {"PATH": str(self.base / "wix/bin") + ";" + str(self.base / "jdk/bin"),
            "JAVA_HOME": str(self.base / "jdk"), "TEMP": str(self.work / "runtime-temp"), "TMP": str(self.work / "runtime-temp")})
        self.assertEqual({path.name for path in self.work.iterdir()}, {"plan.json", "dest", "temp", "runtime-temp"})
        self.assertEqual(self.check()["status"], "observed-inputs-match-plan")
        self.image_check.assert_called_with(self.root)
        self.personal_check.assert_called_with(self.root, None, None)

    def test_leases_cover_source_native_apk_image_full_tool_runtime_evidence_and_plan(self):
        planned = self.plan()
        leaves = {row["path"] for row in planned["lease_inputs"] if row["directory"] is False}
        for name in (*m.BACKEND, m.a.REPORTS.BEGIN, m.a.REPORTS.BUILT, "target/debug/vw_core.dll",
                     "apps/android/build/outputs/apk/debug/android-debug.apk", m.a.REPORTS.DIST + "/vw-mcp.exe",
                     self.manifest_name, ".local/license_review.txt"):
            self.assertIn(str(self.root / name), leaves)
        self.assertIn(str(self.plan_path), leaves)
        for kind in ("jdk", "wix", "python"):
            self.assertIn(str(self.base / kind / ("Lib/runtime.dat" if kind == "python" else "lib/runtime.dat")), leaves)

    def test_preexisting_work_and_foreign_file_are_preserved(self):
        keep = self.put(self.work / "keep.txt", b"foreign")
        self.refuses("fresh_work_required", self.plan)
        self.assertEqual(keep.read_bytes(), b"foreign")
        self.image_check.assert_not_called()

    def test_manifest_hash_and_image_or_personal_refusal_create_no_work(self):
        original = self.manifest_sha
        self.manifest_sha = "0" * 64
        self.refuses("json_binding", self.plan)
        self.manifest_sha = original
        for gate in (self.image_check, self.personal_check):
            with self.subTest(gate=gate):
                gate.side_effect = m.a.REPORTS.Rejected("fixture_admission_refused")
                with self.assertRaisesRegex(ValueError, "fixture_admission_refused"):
                    self.plan()
                gate.side_effect = None
                self.assertFalse(self.work.exists())

    def test_actual_image_checker_requires_existing_build_receipts(self):
        with patch.object(m.a.REPORTS, "check", wraps=self.actual_image_check):
            # Fixtures are deliberately not real desktop build receipts.
            with self.assertRaises((ValueError, OSError)):
                self.plan()
        self.assertFalse(self.work.exists())

    def test_tool_profiles_unknown_keys_and_roles_are_refused(self):
        changes = (("jdk", "version", "22.0.1", "jdk_profile"),
                   ("wix", "version", "4.0.0", "wix_profile"),
                   ("python", "version", "3.9.9", "python_profile"),
                   ("jdk", "roles", {"jpackage": "bin/jpackage.exe", "java": "bin/java.exe", "other": "tool.exe"}, "jdk_profile"),
                   ("wix", "roles", {"candle": "other/candle.exe", "light": "bin/light.exe"}, "wix_roles"))
        original = copy.deepcopy(self.spec)
        for kind, key, value, code in changes:
            self.spec = copy.deepcopy(original)
            self.spec[kind][key] = value
            self.refresh()
            with self.subTest(kind=kind, key=key):
                self.refuses(code, self.plan)
                self.assertFalse(self.work.exists())
        self.spec = original | {"approved": True}
        self.refresh()
        self.refuses("toolchain_schema", self.plan)

    def test_inventory_unknown_tool_changed_tool_and_duplicate_descriptor_are_refused(self):
        extra = self.put(self.base / "wix/extra.exe", pe())
        self.refuses("tool_inventory_binding", self.plan)
        extra.unlink()
        self.put(self.base / "wix/bin/candle.exe", b"different tool")
        self.refuses("tool_inventory_binding", self.plan)
        self.spec["wix"]["files"].append(copy.deepcopy(self.spec["wix"]["files"][0]))
        self.refresh()
        self.refuses("duplicate_path", self.plan)

    def test_declared_non_pe_or_wrong_architecture_role_is_refused(self):
        self.put(self.base / "jdk/bin/java.exe", pe(0x14c))
        self.spec["jdk"]["files"] = self.tool_rows(self.base / "jdk")
        self.refresh()
        self.refuses("tool_pe", self.plan)

    def test_manifest_path_escape_and_path_environment_separator_are_refused(self):
        self.spec["evidence"]["inventory"]["path"] = "../escape"
        self.refresh()
        self.refuses("unsafe_relative_path", self.plan)
        self.spec["evidence"]["inventory"]["path"] = ".local/inventory.txt"
        self.spec["wix"]["root"] = str(self.base / "wix;outside")
        self.refresh()
        self.refuses("tool_path_separator", self.plan)

    def test_unknown_evidence_key_and_changed_license_evidence_are_refused(self):
        self.spec["evidence"]["approved"] = True
        self.refresh()
        self.refuses("toolchain_evidence", self.plan)
        del self.spec["evidence"]["approved"]
        self.refresh()
        self.put(self.root / ".local/license_review.txt", b"changed review")
        self.refuses("input_binding", self.plan)

    def test_source_change_after_plan_is_refused(self):
        self.plan()
        self.put(self.root / "project.txt", b"changed source")
        self.refuses("input_binding", self.check)

    def test_same_bytes_source_identity_replacement_is_refused(self):
        self.plan()
        path = self.root / "project.txt"
        replacement = path.with_name("replacement.txt")
        replacement.write_bytes(path.read_bytes())
        replacement.replace(path)
        self.refuses("plan_input_drift", self.check)

    def test_tool_or_image_namespace_change_after_plan_is_refused(self):
        self.plan()
        extra = self.put(self.base / "jdk/unknown.dll", b"foreign")
        self.refuses("tool_inventory_binding", self.check)
        extra.unlink()
        (self.root / m.a.REPORTS.DIST / "unknown-empty").mkdir()
        self.refuses("plan_input_drift", self.check)

    def test_work_must_be_outside_checkout_and_all_tool_roots(self):
        for path, code in ((self.root / ".local/work", "work_checkout_overlap"),
                           (self.base / "jdk/work", "tool_work_overlap")):
            with self.subTest(path=path):
                self.work = path
                self.refuses(code, self.plan)
                self.assertFalse(path.exists())

    @unittest.skipUnless(os.name == "posix", "POSIX fixture links")
    def test_redirected_and_hardlinked_tool_inputs_are_refused(self):
        path = self.base / "wix/lib/runtime.dat"
        original = path.read_bytes()
        path.unlink()
        path.symlink_to(self.root / "project.txt")
        self.refuses("redirected_path", self.plan)
        path.unlink()
        path.write_bytes(original)
        os.link(path, self.base / "foreign-hardlink")
        self.refuses("hardlinked_input", self.plan)

    def test_python_virtual_environment_and_all_pth_files_are_refused(self):
        path = self.put(self.base / "python/pyvenv.cfg", b"home = C:\\outside")
        self.spec["python"]["files"] = self.tool_rows(self.base / "python")
        self.refresh()
        self.refuses("python_layout", self.plan)
        path.unlink()
        self.put(self.base / "python/python314._pth", b"../outside\n")
        self.spec["python"]["files"] = self.tool_rows(self.base / "python")
        self.refresh()
        self.refuses("python_layout", self.plan)

    def test_python_base_layout_runtime_and_parent_virtual_environment_are_required(self):
        for name in ("python314.dll", "Lib/os.py"):
            path = self.base / "python" / name
            original = path.read_bytes()
            path.unlink()
            self.spec["python"]["files"] = self.tool_rows(self.base / "python")
            self.refresh()
            with self.subTest(missing=name):
                self.refuses("python_layout", self.plan)
            path.write_bytes(original)
        self.spec["python"]["files"] = self.tool_rows(self.base / "python")
        self.refresh()
        self.put(self.base / "pyvenv.cfg", b"home = outside")
        self.refuses("python_layout", self.plan)

    def test_python_nested_executable_is_refused_even_with_declared_runtime(self):
        self.spec["python"]["roles"] = {"python": "bin/python.exe"}
        self.refresh()
        self.refuses("python_profile", self.plan)

    @unittest.skipUnless(os.name == "nt", "unmocked Windows interpreter identity refusal")
    def test_windows_actual_interpreter_cannot_admit_a_different_python_inventory(self):
        with patch.object(m.sys, "executable", self.actual_python):
            self.refuses("python_runtime_binding", self.plan)
        self.assertFalse(self.work.exists())

    @unittest.skipUnless(os.name == "nt", "unmocked Windows directory identity refusal")
    def test_windows_actual_os_directory_cannot_admit_a_different_os_environment(self):
        self.plan()
        envelope = self.envelope()
        with patch.object(m, "windows_directory", self.actual_windows_directory):
            self.refuses("os_environment", lambda: m.process(envelope, self.planned, self.plan_sha))

    def test_catalog_pair_bytes_and_actual_personal_checker_arguments_are_bound(self):
        path = ".local/catalog.json"
        self.put(self.root / path, b'{"synthetic":true}')
        sha = self.digest(self.root / path)["sha256"]
        self.refuses("catalog_arguments", lambda: self.plan(catalog_name=path))
        self.plan(catalog_name=path, catalog_sha256=sha)
        self.personal_check.assert_called_with(self.root, self.root / path, sha)
        self.put(self.root / path, b'{"changed":true}')
        self.refuses("catalog_binding", self.check)

    def test_plan_hash_unknown_field_invocation_and_lease_tampering_are_refused(self):
        self.plan()
        self.refuses("json_binding", lambda: m.check(self.root, self.plan_path, "0" * 64))
        original = copy.deepcopy(self.planned)
        for key, value, code in (("approved", True, "plan_schema"), ("argv", ["--type", "exe"], "plan_invocation"),
                                 ("lease_inputs", [], "plan_leases")):
            changed = original | {key: value}
            self.put(self.plan_path, m.a.json_bytes(changed))
            self.plan_sha = self.digest(self.plan_path)["sha256"]
            with self.subTest(key=key):
                self.refuses(code, self.check)

    def test_private_work_unknown_files_and_nonempty_preflight_dest_are_refused(self):
        self.plan()
        extra = self.put(self.work / "unknown", b"foreign")
        self.refuses("work_namespace", self.check)
        extra.unlink()
        self.put(self.work / "dest/unknown", b"foreign")
        self.refuses("work_not_empty", self.check)

    def test_successful_record_binds_shape_hash_and_retains_honest_limits(self):
        self.plan()
        self.complete()
        self.put(self.work / "temp/intermediates/build.log", b"synthetic tool output")
        result = self.record()
        self.assertEqual(json.loads((self.work / "receipt.json").read_bytes()), result)
        self.assertEqual(result["package"]["sha256"], m.blob(cfb())["sha256"])
        self.assertEqual(result["plan"]["sha256"], self.plan_sha)
        self.assertEqual(result["package"]["shape"]["format"], "OLE-CFB")
        for field in ("atomic_namespace_verified", "payload_verified", "msi_database_validity_verified",
                      "signature_verified", "installed_or_launched", "product_acceptance"):
            self.assertIs(result[field], False)
        self.assertIn("intermediates/build.log", result["temporary_inventory"]["files"])
        before = (self.work / "receipt.json").read_bytes()
        self.refuses("work_namespace", self.record)
        self.assertEqual((self.work / "receipt.json").read_bytes(), before)

    def test_process_envelope_rejects_other_run_invocation_environment_or_unknown_keys(self):
        self.plan()
        original = self.envelope()
        for key, value in (("run_nonce", "b" * 32), ("plan_sha256", "0" * 64), ("argv", ["--type", "exe"]),
                           ("minimal_env", original["minimal_env"] | {"JAVA_TOOL_OPTIONS": "-javaagent:bad"}),
                           ("approved", True)):
            with self.subTest(key=key):
                self.refuses("process_binding", lambda: m.process(original | {key: value}, self.planned, self.plan_sha))
        for key, value in (("ComSpec", str(self.base / "outside.exe")), ("PATH", "outside"),
                           ("WINDIR", str(self.base / "OtherWindows")), ("unknown", "true")):
            candidate = copy.deepcopy(original)
            candidate["os_env"][key] = value
            with self.subTest(os_key=key):
                self.refuses("os_environment", lambda: m.process(candidate, self.planned, self.plan_sha))

    def test_failed_or_unjoined_process_cannot_publish_receipt(self):
        self.plan()
        for key, value in (("phase", "other"), ("exit_code", 1), ("parent_exit_code", True),
                           ("failure_reason", "timeout"), ("contained_in_windows_job", False),
                           ("process_tree_cleanup_confirmed", False), ("output_streams_completed", False),
                           ("surviving_processes_before_cleanup", [{"pid": 1}]), ("timeout_seconds", 121),
                           ("compiler_telemetry_cleaned", True)):
            envelope = self.envelope()
            envelope["runner"][key] = value
            self.complete(envelope=envelope)
            with self.subTest(key=key):
                self.refuses("process_failed", self.record)
                self.assertFalse((self.work / "receipt.json").exists())
                self.assertTrue((self.work / "dest" / m.PACKAGE).exists())

    def test_nonfinite_or_out_of_bound_process_timing_is_refused(self):
        self.plan()
        for value in (True, 0, -1, float("inf"), float("nan"), 186):
            envelope = self.envelope()
            envelope["runner"]["elapsed_seconds"] = value
            with self.subTest(elapsed=value):
                self.refuses("process_timing", lambda: m.process(envelope, self.planned, self.plan_sha))

    def test_exact_output_filename_and_closed_destination_are_required(self):
        self.plan()
        self.complete()
        for name in ("other.msi", "unlisted.dll"):
            extra = self.put(self.work / "dest" / name, b"foreign")
            self.refuses("msi_output_namespace", self.record)
            self.assertEqual(extra.read_bytes(), b"foreign")
            extra.unlink()
        (self.work / "dest/unknown-empty").mkdir()
        self.refuses("msi_output_namespace", self.record)

    def test_cfb_header_sector_bounds_and_difat_are_checked_without_database_claim(self):
        good = cfb()
        self.assertIs(m.cfb_shape(good[:512], len(good))["msi_database_validity_verified"], False)
        for offset, replacement, code in ((0, b"NOT-MSI!", "msi_cfb_header"),
                                           (26, struct.pack("<H", 5), "msi_cfb_profile"),
                                           (44, struct.pack("<I", 0), "msi_cfb_layout"),
                                           (76, struct.pack("<I", 100), "msi_cfb_difat")):
            body = bytearray(good)
            body[offset:offset + len(replacement)] = replacement
            with self.subTest(offset=offset):
                self.refuses(code, lambda: m.cfb_shape(bytes(body[:512]), len(body)))
        self.refuses("msi_cfb_size", lambda: m.cfb_shape(good[:512], len(good) + 1))
        self.refuses("msi_cfb_header", lambda: m.cfb_shape(b"short", 5))

    def test_wrong_package_shape_retains_failed_tree_without_receipt(self):
        self.plan()
        self.complete(package=b"not an OLE file")
        self.refuses("msi_cfb_header", self.record)
        self.assertFalse((self.work / "receipt.json").exists())
        self.assertEqual((self.work / "dest" / m.PACKAGE).read_bytes(), b"not an OLE file")

    def test_source_or_output_replaced_during_record_refuses_before_receipt(self):
        self.plan()
        self.complete()
        original_shape = m.cfb_shape

        def change_source(raw, size):
            result = original_shape(raw, size)
            self.put(self.root / "project.txt", b"source changed during output validation")
            return result

        with patch.object(m, "cfb_shape", side_effect=change_source):
            self.refuses("input_binding", self.record)
        self.assertFalse((self.work / "receipt.json").exists())

    def test_process_path_is_fixed_and_arbitrary_receipt_output_is_unavailable(self):
        self.plan()
        self.complete()
        self.refuses("process_path", lambda: m.record(self.root, self.plan_path, self.plan_sha,
                                                       self.base / "foreign-process.json"))
        self.assertFalse((self.work / "receipt.json").exists())

    @unittest.skipUnless(os.name == "posix", "POSIX fixture links")
    def test_output_or_temp_symlink_refuses_and_preserves_foreign_target(self):
        self.plan()
        self.complete()
        foreign = self.put(self.base / "foreign", b"preserve")
        link = self.work / "temp/escape"
        link.symlink_to(foreign)
        self.refuses("redirected_path", self.record)
        self.assertEqual(foreign.read_bytes(), b"preserve")
        self.assertFalse((self.work / "receipt.json").exists())

    def test_duplicate_json_constants_and_bounded_inventory_refuse(self):
        for raw, code in ((b'{"schema":1,"schema":2}', "json_duplicate"), (b'{"x":NaN}', "json_constant")):
            self.put(self.root / self.manifest_name, raw)
            self.manifest_sha = m.blob(raw)["sha256"]
            self.refuses(code, self.plan)
        self.refresh()
        with patch.object(m, "MAX_FILES", 1):
            self.refuses("tool_file_count", self.plan)
        self.assertFalse(self.work.exists())

    def test_cli_refusal_is_static_and_does_not_echo_host_path(self):
        arguments = ["msi.py", "check", "--root", str(self.root), "--plan", str(self.base / "not-plan.json"),
                     "--plan-sha256", "0" * 64]
        output = io.StringIO()
        with patch.object(m.sys, "argv", arguments), redirect_stdout(output):
            self.assertEqual(m.main(), 1)
        self.assertEqual(output.getvalue(), "MSI_REJECTED:plan_path\n")


if __name__ == "__main__":
    unittest.main()
