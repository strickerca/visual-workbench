"""Synthetic parser/provenance fixtures; no Gradle, adb, device or native calls."""
import json
import os
from pathlib import Path
import struct
import tempfile
import time
import unittest
from unittest.mock import patch
import zipfile

import shared_reports as reports


def junit(platform="android", *, count=1, case=None, attributes="", children=""):
    classname, name = reports.EXPECTED[platform] if case is None else case
    return (f'<testsuite tests="{count}" failures="0" errors="0" {attributes}>'
            f'<testcase classname="{classname}" name="{name}">{children}</testcase>'
            '<system-out>private serial and private filesystem output must be omitted</system-out>'
            '</testsuite>').encode()


class SharedReportsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vw-shared-report-fixture-")
        self.root = Path(self.temp.name)
        for name in ("androidDeviceTest", "jvmTest", "commonTest"):
            (self.root / "apps/shared/src" / name).mkdir(parents=True)

    def tearDown(self):
        self.temp.cleanup()

    def report(self, data, *, root=None, name="TEST-private-device.xml"):
        root = self.root if root is None else root
        root.mkdir(parents=True, exist_ok=True)
        path = root / name
        path.write_bytes(data)
        return path

    def test_actual_expected_test_passes_without_echoing_device_or_output(self):
        start = time.time_ns()
        path = self.report(junit())
        # Filesystem timestamp precision need not match time.time_ns(). Give
        # this synthetic fresh report an explicit time after the phase marker.
        future = start + 60_000_000_000
        os.utime(path, ns=(future, future))
        result = reports.fresh_results(self.root, {}, start, reports.EXPECTED["android"])
        self.assertEqual(result["actual_passed_tests"], 1)
        self.assertEqual(result["fresh_reports"], 1)
        self.assertNotIn("private", json.dumps(result))

    def test_matching_old_report_does_not_count_even_with_future_mtime(self):
        path = self.report(junit())
        future = time.time_ns() + 60_000_000_000
        os.utime(path, ns=(future, future))
        before = reports.report_inventory(self.root)
        with self.assertRaisesRegex(reports.Rejected, "required_golden_not_passed"):
            reports.fresh_results(self.root, before, time.time_ns(), reports.EXPECTED["android"])

    def test_changed_report_with_stale_timestamp_is_refused(self):
        path = self.report(junit())
        before = reports.report_inventory(self.root)
        path.write_bytes(junit(attributes='time="2"'))
        os.utime(path, ns=(1_000_000_000, 1_000_000_000))
        with self.assertRaisesRegex(reports.Rejected, "stale_report"):
            reports.fresh_results(self.root, before, time.time_ns(), reports.EXPECTED["android"])

    def test_zero_unrelated_or_missing_required_test_never_passes(self):
        for data in (b'<testsuite tests="0"/>', junit(case=("other.Class", "otherMethod"))):
            with self.subTest(data=data):
                self.report(data)
                with self.assertRaisesRegex(reports.Rejected, "required_golden_not_passed"):
                    reports.fresh_results(self.root, {}, 0, reports.EXPECTED["android"])

    def test_skips_disabled_and_child_failures_refuse_acceptance(self):
        for kwargs in ({"attributes": 'skipped="1"'}, {"attributes": 'disabled="1"'},
                       {"children": "<failure/>"}, {"children": "<error/>"}, {"children": "<skipped/>"}):
            with self.subTest(kwargs=kwargs), self.assertRaises(reports.Rejected):
                reports.parse_junit(junit(**kwargs))

    def test_declared_counts_cannot_replace_actual_testcases(self):
        for count in (0, 2, -1, "NaN"):
            with self.subTest(count=count), self.assertRaises(reports.Rejected):
                reports.parse_junit(junit(count=count))

    def test_duplicate_cases_across_device_reports_are_refused(self):
        self.report(junit(), name="TEST-one.xml")
        self.report(junit(), name="TEST-two.xml")
        with self.assertRaisesRegex(reports.Rejected, "duplicate_case"):
            reports.fresh_results(self.root, {}, 0, reports.EXPECTED["android"])

    def test_dtd_utf16_nested_and_oversized_xml_are_refused(self):
        for data in (b'<!DOCTYPE testsuite [<!ENTITY x "x">]>' + junit(),
                     junit().decode().encode("utf-16"),
                     b'<testsuite tests="1"><testsuite tests="0"/></testsuite>',
                     b"x" * (1024**2 + 1)):
            with self.subTest(length=len(data)), self.assertRaises(reports.Rejected):
                reports.parse_junit(data)

    def test_redirection_is_refused_when_supported(self):
        actual = self.root / "real"
        actual.mkdir()
        target = actual / "file"
        target.write_text("x")
        link = self.root / "link"
        try:
            link.symlink_to(actual, target_is_directory=True)
        except OSError:
            self.skipTest("Synthetic symlinks are unavailable to this process")
        with self.assertRaisesRegex(reports.Rejected, "redirected_path"):
            reports.digest(link / "file")
        link.unlink()

    def test_duplicate_json_keys_are_refused(self):
        path = self.root / "state.json"
        path.write_text('{"schema":1,"schema":2}')
        with self.assertRaisesRegex(reports.Rejected, "duplicate_json_key"):
            reports.bounded_json(path)

    def test_source_and_binary_drift_are_refused_between_platforms(self):
        with patch.object(reports, "source_inventory", return_value={"native": "a"}):
            state = reports.begin(self.root)
            reports.mark(self.root, state, "desktop")
        with patch.object(reports, "source_inventory", return_value={"native": "b"}):
            with self.assertRaisesRegex(reports.Rejected, "source_drift"):
                reports.mark(self.root, state, "android")

    def test_full_collection_requires_both_wrappers_and_same_fixture_hashes(self):
        fixture = self.root / "tools/ffi-test/fixtures/ffi-golden.properties"
        fixture.parent.mkdir(parents=True)
        fixture.write_text("state_hash=" + "a" * 64 + "\nexport_blake3=" + "b" * 64 + "\n")
        inventory = self.root / "tools/ffi-test/android-inventory.json"
        inventory.write_text(json.dumps({"schema": 1, "expected_tests": [list(reports.EXPECTED["android"])]}))
        source = self.root / "apps/shared/src/androidDeviceTest/AndroidCoreSmokeTest.kt"
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text("package com.visualworkbench.shared\nclass AndroidCoreSmokeTest { @Test public fun rustGoldenAndBoundary() {} }")
        with patch.object(reports, "source_inventory", return_value={"fixture": "digest"}):
            state = reports.begin(self.root)
            for phase in reports.EXPECTED:
                reports.mark(self.root, state, phase)
                path = self.report(junit(phase), root=self.root / reports.REPORT_ROOTS[phase])
                future = state["phases"][phase]["started_ns"] + 60_000_000_000
                os.utime(path, ns=(future, future))
            result = reports.collect(self.root, state)
        self.assertTrue(result["assertions_passed"])
        self.assertEqual(result["golden"]["state_hash"], "a" * 64)
        self.assertEqual({p: x["actual_passed_tests"] for p, x in result["platforms"].items()}, {"android": 1, "desktop": 1})
        self.assertNotIn("private", json.dumps(result))

    def test_golden_alone_cannot_stand_in_for_required_android_cases(self):
        self.report(junit())
        expected = reports.EXPECTED["android"]
        with self.assertRaisesRegex(reports.Rejected, "required_case_inventory"):
            reports.fresh_results(self.root, {}, 0, expected,
                                  {expected, ("com.visualworkbench.shared.KeysTest", "encrypted")})

    def test_android_source_census_refuses_added_test_or_skip(self):
        inventory = self.root / "tools/ffi-test/android-inventory.json"
        inventory.parent.mkdir(parents=True)
        inventory.write_text(json.dumps({"schema": 1, "expected_tests": [list(reports.EXPECTED["android"])]}))
        source = self.root / "apps/shared/src/androidDeviceTest/AndroidCoreSmokeTest.kt"
        source.parent.mkdir(parents=True, exist_ok=True)
        text = "package com.visualworkbench.shared\nclass AndroidCoreSmokeTest { @Test public fun rustGoldenAndBoundary() {} }"
        source.write_text(text)
        self.assertEqual(reports.android_inventory(self.root), {reports.EXPECTED["android"]})
        for changed in (text.replace("@Test", "@Ignore @Test"), text.replace(" {} }", " {} @Test fun added() {} }")):
            source.write_text(changed)
            with self.assertRaises(reports.Rejected):
                reports.android_inventory(self.root)

    def test_device_inventory_includes_jvm_and_common_cases_and_rejects_duplicates(self):
        inventory = self.root / "tools/ffi-test/android-inventory.json"
        inventory.parent.mkdir(parents=True)
        cases = []
        for source_set, name in (("androidDeviceTest", "DeviceTest"), ("jvmTest", "JvmTest"),
                                 ("commonTest", "CommonTest")):
            path = self.root / "apps/shared/src" / source_set / (name + ".kt")
            path.write_text(f"package com.visualworkbench.shared\nclass {name} {{ @Test fun contract() {{}} }}")
            cases.append(["com.visualworkbench.shared." + name, "contract"])
        inventory.write_text(json.dumps({"schema": 1, "expected_tests": sorted(cases)}))
        self.assertEqual(reports.android_inventory(self.root), set(map(tuple, cases)))
        duplicate = self.root / "apps/shared/src/jvmTest/Duplicate.kt"
        duplicate.write_text("package com.visualworkbench.shared\nclass CommonTest { @Test fun contract() {} }")
        with self.assertRaisesRegex(reports.Rejected, "android_test_duplicate"):
            reports.android_inventory(self.root)

    def test_device_inventory_refuses_omitted_common_case(self):
        inventory = self.root / "tools/ffi-test/android-inventory.json"
        inventory.parent.mkdir(parents=True)
        inventory.write_text(json.dumps({"schema": 1, "expected_tests": [list(reports.EXPECTED["android"])]}))
        (self.root / "apps/shared/src/androidDeviceTest/AndroidCoreSmokeTest.kt").write_text(
            "package com.visualworkbench.shared\nclass AndroidCoreSmokeTest { @Test fun rustGoldenAndBoundary() {} }")
        (self.root / "apps/shared/src/commonTest/CommonTest.kt").write_text(
            "package com.visualworkbench.shared\nclass CommonTest { @Test fun contract() {} }")
        with self.assertRaisesRegex(reports.Rejected, "android_test_inventory_drift"):
            reports.android_inventory(self.root)

    def test_apk_must_be_single_self_targeting_library_instrumentation(self):
        output = self.root / "apps/shared/build/outputs/apk/androidTest"
        output.mkdir(parents=True)
        (output / "test.apk").write_bytes(b"synthetic packaging metadata fixture")
        metadata = output / "output-metadata.json"
        metadata.write_text(json.dumps({"applicationId": reports.PACKAGE, "elements": [{"outputFile": "test.apk", "filters": []}]}))
        manifest = self.root / "apps/shared/build/intermediates/merged_manifests/androidTest/AndroidManifest.xml"
        manifest.parent.mkdir(parents=True)
        def write(target):
            manifest.write_text(f'<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="{reports.PACKAGE}"><instrumentation android:name="{reports.RUNNER}" android:targetPackage="{target}"/></manifest>')
        write(reports.PACKAGE)
        self.assertEqual(reports.apk_info(self.root, 0)["package"], reports.PACKAGE)
        write("com.visualworkbench.android")
        with self.assertRaisesRegex(reports.Rejected, "instrumentation_target"):
            reports.apk_info(self.root, 0)

    def test_apk_native_and_golden_bytes_must_match_exact_inputs(self):
        native = self.root / "target/android-jni/arm64-v8a/libvw_core.so"
        fixture = self.root / "tools/ffi-test/fixtures/ffi-golden.properties"
        native.parent.mkdir(parents=True); fixture.parent.mkdir(parents=True)
        native.write_bytes(b"synthetic native"); fixture.write_bytes(b"synthetic golden")
        apk = self.root / "test.apk"
        with zipfile.ZipFile(apk, "w") as archive:
            archive.writestr("lib/arm64-v8a/libvw_core.so", native.read_bytes())
            archive.writestr("ffi-golden.properties", fixture.read_bytes())
        self.assertTrue(reports.bind_apk(self.root, apk)["native_and_golden_match"])
        native.write_bytes(b"different native")
        with self.assertRaises(reports.Rejected):
            reports.bind_apk(self.root, apk)

    def test_apk_directory_bound_precedes_zipfile_allocation(self):
        apk = self.root / "unbounded.apk"
        # A tiny malicious end record claims a huge central directory. Do not
        # give ZipFile the opportunity to allocate from that claim.
        apk.write_bytes(struct.pack("<4s4H2IH", b"PK\x05\x06", 0, 0, 1, 1, 64 * 1024**2, 0, 0))
        with patch.object(reports.zipfile, "ZipFile") as archive:
            with self.assertRaisesRegex(reports.Rejected, "apk_directory_bound"):
                reports.bind_apk(self.root, apk)
            archive.assert_not_called()


if __name__ == "__main__":
    unittest.main()
