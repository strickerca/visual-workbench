"""Synthetic only: no JVM, launcher, native loader, network or user preferences."""
import copy
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import warnings
import zipfile

import reports as r


GOOD = ("VW_DESKTOP_READY startup_ms=812 composeDensity=2.0 pmv2=true\n"
        "VW_DESKTOP_SMOKE_FRAME window_dpi=192 window_pmv2=true drawn=true\n"
        "VW_DESKTOP_EXIT startup_smoke=true cleanup=complete\n")


def proof():
    return {"phase": "desktop-startup", "exit_code": 0, "parent_exit_code": 0,
            "failure_reason": None, "contained_in_windows_job": True,
            "process_tree_cleanup_confirmed": True, "output_streams_completed": True,
            "surviving_processes_before_cleanup": [], "compiler_telemetry_cleaned": False,
            "elapsed_seconds": 2.75, "capture_byte_limit": 1024**2, "captured_bytes": len(GOOD),
            "capture_overflow": False, "raw_output_retained": True}


class Reports(unittest.TestCase):
    def test_mcp_requires_its_exact_lifecycle_marker_and_requested_mode(self):
        value = GOOD.replace(r.EXIT, r.MCP + "\n" + r.EXIT)
        self.assertEqual(r.startup(value, "mcp")["mcp_service_cycles"], 2)
        self.assertFalse(r.startup(value, "mcp")["external_client_or_send_acceptance"])
        for text, mode in ((GOOD, "mcp"), (value, "startup"),
                           (value.replace("cycles=2", "cycles=1"), "mcp"),
                           (value.replace("grants=0", "grants=1"), "mcp"),
                           (value.replace("sends=0", "sends=1"), "mcp"),
                           (value.replace(r.MCP, "VW_DESKTOP_MCP failed=true"), "mcp"),
                           (value + r.MCP, "mcp"), (value, "unknown")):
            with self.subTest(mode=mode, text=text), self.assertRaises(r.Rejected):
                r.startup(text, mode)

    def test_normal_readiness_and_exit(self):
        value = r.startup("ordinary logger line\n" + GOOD)
        self.assertEqual(value["main_to_drawn_frame_ms"], 812)
        self.assertEqual(value["window_dpi"], 192)
        self.assertFalse(value["visual_acceptance"])
        self.assertFalse(value["cold_start_or_performance_acceptance"])

    def test_mcp_failure_retains_only_fixed_nonsecret_stage(self):
        value = GOOD.replace(r.EXIT, "VW_DESKTOP_MCP failed=true stage=bundle_inventory\n" + r.EXIT)
        with self.assertRaisesRegex(r.Rejected, "^mcp_bundle_inventory$"):
            r.startup(value, "mcp")
        with self.assertRaisesRegex(r.Rejected, "^mcp_lifecycle$"):
            r.startup(value.replace("bundle_inventory", "private_value"), "mcp")

    def test_missing_duplicate_out_of_order_markers(self):
        lines = GOOD.splitlines()
        for candidate in ("\n".join(lines[:2]), GOOD + lines[0], "\n".join(reversed(lines))):
            with self.subTest(candidate=candidate), self.assertRaises(r.Rejected):
                r.startup(candidate)

    def test_false_dpi_or_cleanup_refused(self):
        for original, replacement in (("pmv2=true", "pmv2=false"), ("drawn=true", "drawn=false"),
                                       ("cleanup=complete", "cleanup=failed")):
            with self.subTest(original=original), self.assertRaises(r.Rejected):
                r.startup(GOOD.replace(original, replacement))

    def test_malformed_and_nonfinite_measurements_refused(self):
        for density in ("NaN", "Infinity", "-1", "0", "9000", "1e2"):
            with self.subTest(density=density), self.assertRaises(r.Rejected):
                r.startup(GOOD.replace("2.0", density))
        for dpi in ("0", "4096", "-96"):
            with self.subTest(dpi=dpi), self.assertRaises(r.Rejected):
                r.startup(GOOD.replace("192", dpi))

    def test_marker_prefix_or_unknown_marker_refused(self):
        for value in ("spoof " + GOOD, GOOD + "VW_DESKTOP_UNKNOWN ignored\n"):
            with self.assertRaises(r.Rejected):
                r.startup(value)

    def test_output_bounds(self):
        for value in (GOOD + "\0", GOOD + "a" * (1024**2)):
            with self.assertRaises(r.Rejected):
                r.startup(value)

    def test_real_process_cleanup_required(self):
        self.assertEqual(r.process_receipt(proof())["exit_code"], 0)
        for key, value in (("exit_code", 124), ("parent_exit_code", 1), ("failure_reason", "timeout"),
                           ("contained_in_windows_job", False), ("process_tree_cleanup_confirmed", False),
                           ("output_streams_completed", False), ("surviving_processes_before_cleanup", [{"pid": 1}]),
                           ("compiler_telemetry_cleaned", True), ("elapsed_seconds", float("nan"))):
            changed = proof(); changed[key] = value
            with self.subTest(key=key), self.assertRaises(r.Rejected):
                r.process_receipt(changed)

    def test_relative_path_ownership(self):
        r.relative_name("runtime/bin/server/jvm.dll")
        for name in ("../escape", "a/../b", "/absolute", "C:/absolute", "a\\b", "a//b", "a.",
                     "a ", "NUL", "dir/COM1.dll", "a\x00b", "a:*", ""):
            with self.subTest(name=name), self.assertRaises(r.Rejected):
                r.relative_name(name)

    def test_preallocation_capture_receipt_required(self):
        for key, value in (("capture_byte_limit", 2 * 1024**2), ("captured_bytes", 1024**2),
                           ("captured_bytes", -1), ("captured_bytes", True),
                           ("capture_overflow", True), ("raw_output_retained", False)):
            changed = proof(); changed[key] = value
            with self.subTest(key=key, value=value), self.assertRaisesRegex(r.Rejected, "startup_capture"):
                r.process_receipt(changed)
        changed = proof(); del changed["capture_byte_limit"]
        with self.assertRaisesRegex(r.Rejected, "startup_capture"):
            r.process_receipt(changed)

    def test_environment_override_refused(self):
        r.environment_ok({})
        for name in ("JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH",
                     "ORG_GRADLE_PROJECT_vwNativeDir", "ORG_GRADLE_PROJECT_vwDesktopBuildId"):
            with self.subTest(name=name), self.assertRaises(r.Rejected):
                r.environment_ok({name: "unbound"})

    def test_duplicate_json_and_noclobber(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "owned.json"
            target.write_text('{"schema":1,"schema":2}', encoding="utf-8")
            with self.assertRaises(r.Rejected):
                r.bounded_json(target)
            before = target.read_bytes()
            with self.assertRaises(FileExistsError):
                r.save(target, {"new": True})
            self.assertEqual(target.read_bytes(), before)


class Distribution(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.directory = self.root / r.DIST
        for name in ("app", "runtime/bin/server", "runtime/lib"):
            (self.directory / name).mkdir(parents=True, exist_ok=True)
        (self.root / "target/debug").mkdir(parents=True)
        self.nonce = "a" * 32
        for name in r.INPUTS:
            (self.root / "target/debug" / name).write_bytes(("synthetic-" + name).encode())
        self.native = {name: r.digest(self.root / "target/debug" / name) for name in r.INPUTS}
        pe = bytearray(160)
        pe[:2] = b"MZ"
        struct.pack_into("<I", pe, 60, 64)
        pe[64:68] = b"PE\0\0"
        struct.pack_into("<H", pe, 68, 0x8664)
        struct.pack_into("<H", pe, 88, 0x20b)
        struct.pack_into("<H", pe, 156, 3)
        (self.directory / r.EXE).write_bytes(pe)
        (self.directory / "runtime/bin/server/jvm.dll").write_bytes(b"synthetic JRE")
        (self.directory / "runtime/lib/modules").write_bytes(b"synthetic modules")
        self.config = self.directory / "app" / (r.APP + ".cfg")
        self.config.write_text("[Application]\napp.mainclass=com.visualworkbench.desktop.MainKt\n"
                               "app.classpath=$APPDIR\\app.jar\n[JavaOptions]\n"
                               "java-options=-Djpackage.app-version=0.0.1\n"
                               "java-options=-Dsun.java2d.dpiaware=true\n"
                               "java-options=-Dskiko.renderApi=DIRECT3D\n", encoding="utf-8")
        self.contents = {"win32-x86-64/" + n: (self.root / "target/debug" / n).read_bytes() for n in r.NATIVES}
        self.contents[r.NONCE_RESOURCE] = (self.nonce + "\n").encode()
        self.contents["com/visualworkbench/desktop/MainKt.class"] = b"synthetic class, never loaded"
        self.contents["vw-native-runtime.sha256"] = "".join(
            f"{self.native[n]['sha256']} {self.native[n]['bytes']} {n}\n" for n in sorted(r.NATIVES)).encode()
        # Include a vendor metadata file to reproduce a Gradle-default exclusion.
        self.mcp = {name: ("synthetic-" + name).encode() for name in r.MCP_REQUIRED}
        self.mcp["mcp/node_modules/vendor/.gitattributes"] = b"fixture metadata"
        self.contents[r.MCP_MANIFEST] = "".join(
            f"{hashlib.sha256(body).hexdigest()} {len(body)} {name}\n"
            for name, body in sorted(self.mcp.items())).encode()
        self.contents.update({"mcp-server/" + name: body for name, body in self.mcp.items()})
        self.jar()

    def jar(self, duplicate=False):
        with zipfile.ZipFile(self.directory / "app/app.jar", "w") as archive:
            for name, body in self.contents.items():
                archive.writestr(name, body)
            if duplicate:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", UserWarning)
                    archive.writestr(r.NONCE_RESOURCE, self.contents[r.NONCE_RESOURCE])

    def test_actual_packaged_inventory(self):
        result = r.distribution(self.root, self.nonce)
        self.assertEqual(result["launcher"], r.EXE)
        self.assertEqual(result["native_inputs"], self.native)
        self.assertIn("runtime/lib/modules", result["files"])

    def test_stale_nonce_rejected(self):
        with self.assertRaisesRegex(r.Rejected, "stale_distribution"):
            r.distribution(self.root, "b" * 32)

    def test_missing_mcp_vendor_resource_refused_before_launch(self):
        self.contents.pop("mcp-server/mcp/node_modules/vendor/.gitattributes")
        self.jar()
        with self.assertRaisesRegex(r.Rejected, "mcp_resource_binding"):
            r.distribution(self.root, self.nonce)

    def test_mcp_payload_hash_is_checked_not_only_manifest(self):
        self.contents["mcp-server/node.exe"] = b"substituted runtime"
        self.jar()
        with self.assertRaisesRegex(r.Rejected, "mcp_resource_binding"):
            r.distribution(self.root, self.nonce)

    def test_unlisted_mcp_resource_refused(self):
        self.contents["mcp-server/extra.mjs"] = b"unlisted"
        self.jar()
        with self.assertRaisesRegex(r.Rejected, "mcp_resource_binding"):
            r.distribution(self.root, self.nonce)

    def test_duplicate_and_traversing_mcp_manifest_refused(self):
        original = self.contents[r.MCP_MANIFEST]
        for suffix in (original.splitlines()[0] + b"\n", b"0" * 64 + b" 1 ../escape\n"):
            self.contents[r.MCP_MANIFEST] = original + suffix
            self.jar()
            with self.subTest(suffix=suffix), self.assertRaises(r.Rejected):
                r.distribution(self.root, self.nonce)

    def test_native_resource_not_just_manifest_is_hashed(self):
        self.contents["win32-x86-64/vw_core.dll"] = b"wrong"
        self.jar()
        with self.assertRaisesRegex(r.Rejected, "native_resource_binding"):
            r.distribution(self.root, self.nonce)

    def test_incomplete_and_duplicate_resources(self):
        saved = self.contents.pop("win32-x86-64/vw_host.dll")
        self.jar()
        with self.assertRaisesRegex(r.Rejected, "packaged_resources_missing"):
            r.distribution(self.root, self.nonce)
        self.contents["win32-x86-64/vw_host.dll"] = saved
        self.jar(duplicate=True)
        with self.assertRaisesRegex(r.Rejected, "jar_duplicate"):
            r.distribution(self.root, self.nonce)

    def test_unbound_external_classpath_rejected(self):
        original = self.config.read_text(encoding="utf-8")
        self.config.write_text(original.replace("$APPDIR\\app.jar", "C:\\external.jar"), encoding="utf-8")
        with self.assertRaisesRegex(r.Rejected, "external_classpath"):
            r.distribution(self.root, self.nonce)

    def test_unknown_jvm_option_rejected(self):
        with self.config.open("a", encoding="utf-8") as stream:
            stream.write("java-options=-javaagent:unreviewed.jar\n")
        with self.assertRaisesRegex(r.Rejected, "launcher_options"):
            r.distribution(self.root, self.nonce)

    def test_confined_compose_packager_options(self):
        with self.config.open("a", encoding="utf-8") as stream:
            stream.write("java-options=-Dcompose.application.resources.dir=$APPDIR\\resources\n"
                         "java-options=-Dskiko.library.path=$APPDIR\n"
                         "java-options=-Dcompose.application.configure.swing.globals=true\n")
        r.distribution(self.root, self.nonce)
        with self.config.open("a", encoding="utf-8") as stream:
            stream.write("java-options=-Dskiko.library.path=$APPDIR\n")
        with self.assertRaisesRegex(r.Rejected, "launcher_options"):
            r.distribution(self.root, self.nonce)

    def test_wrong_launcher_or_console_refused(self):
        path = self.directory / r.EXE
        body = bytearray(path.read_bytes())
        struct.pack_into("<H", body, 156, 2)
        path.write_bytes(body)
        with self.assertRaisesRegex(r.Rejected, "launcher_console"):
            r.distribution(self.root, self.nonce)

    def test_truncated_jar_metadata_rejected_before_zip_reader(self):
        path = self.directory / "app/app.jar"
        path.write_bytes(path.read_bytes()[:-1])
        with self.assertRaises(r.Rejected):
            r.distribution(self.root, self.nonce)

    def test_oversized_jar_central_directory_rejected(self):
        path = self.directory / "app/app.jar"
        body = bytearray(path.read_bytes())
        at = body.rfind(b"PK\x05\x06")
        struct.pack_into("<I", body, at + 12, 17 * 1024**2)
        path.write_bytes(body)
        with self.assertRaisesRegex(r.Rejected, "jar_directory"):
            r.distribution(self.root, self.nonce)

    def test_production_receipt_recheck_detects_jre_drift(self):
        source = {"source": {"bytes": 1, "sha256": "0" * 64}}
        with patch.object(r, "sources", return_value=source), patch.object(r, "environment_ok"):
            begin = r.begin(self.root)
            begin["nonce"] = self.nonce
            built = r.record(self.root, begin)
            (self.root / ".local").mkdir()
            r.save(self.root / r.BEGIN, begin)
            r.save(self.root / r.BUILT, built)
            self.assertEqual(r.check(self.root), built)
            (self.directory / "runtime/lib/modules").write_bytes(b"modified runtime")
            with self.assertRaisesRegex(r.Rejected, "distribution_drift"):
                r.check(self.root)

    def test_failed_new_build_invalidates_old_success(self):
        with patch.object(r, "sources", return_value={}), patch.object(r, "environment_ok"):
            begin = r.begin(self.root); begin["nonce"] = self.nonce
            built = r.record(self.root, begin)
            (self.root / ".local").mkdir()
            begin["nonce"] = "b" * 32
            r.save(self.root / r.BEGIN, begin)
            r.save(self.root / r.BUILT, built)
            with self.assertRaisesRegex(r.Rejected, "build_receipt"):
                r.check(self.root)

    def test_source_drift_before_record_refused(self):
        with patch.object(r, "sources", return_value={}), patch.object(r, "environment_ok"):
            before = r.begin(self.root)
        with patch.object(r, "sources", return_value={"changed": True}), patch.object(r, "environment_ok"):
            with self.assertRaisesRegex(r.Rejected, "source_drift"):
                r.record(self.root, before)


if __name__ == "__main__":
    unittest.main()
