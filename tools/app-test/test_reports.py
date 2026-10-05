"""Synthetic parser fixtures only; never starts Gradle, adb, or native code."""
import json
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import reports

A = "com.visualworkbench.android.SyntheticInstrumentedTest#first"
B = "com.visualworkbench.android.SyntheticInstrumentedTest#second"


def block(case, current, total, code):
    owner, method = case.split("#")
    return (f"INSTRUMENTATION_STATUS: class={owner}\nINSTRUMENTATION_STATUS: current={current}\n"
            f"INSTRUMENTATION_STATUS: id=AndroidJUnitRunner\nINSTRUMENTATION_STATUS: numtests={total}\n"
            f"INSTRUMENTATION_STATUS: stream=.\nINSTRUMENTATION_STATUS: test={method}\n"
            f"INSTRUMENTATION_STATUS_CODE: {code}\n")


def passed(cases=(A, B)):
    return "".join(block(case, n, len(cases), code) for n, case in enumerate(cases, 1) for code in (1, 0)) + (
        f"INSTRUMENTATION_RESULT: stream=\nTime: 1.2\n\nOK ({len(cases)} tests)\n\nINSTRUMENTATION_CODE: -1\n")


def manifest(test=False):
    body = (f'<instrumentation android:name="{reports.RUNNER}" android:targetPackage="{reports.MAIN}"/>' if test else
            f'<application android:label="Visual Workbench Test" android:debuggable="true" android:allowBackup="false">'
            f'<provider android:authorities="{reports.MAIN}.images"/></application>')
    return f'<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="{reports.TEST if test else reports.MAIN}">{body}</manifest>'


class InstrumentationCases(unittest.TestCase):
    def test_reviewed_nested_packages_pass_and_unknown_packages_refuse(self):
        for package in ("capture", "editor", "instructions", "remote"):
            case = A.replace("android.", "android." + package + ".")
            self.assertEqual(reports.instrumentation(passed((case,)), [case])["actual_passed_tests"], 1)
        for package in ("foreign", "editor.nested", "capture..", "remote.nested", "remote..", "remotely"):
            case = A.replace("android.", "android." + package + ".")
            self.assertIsNone(reports.CASE.fullmatch(case))

    def refused(self, text, expected=(A, B)):
        with self.assertRaises(reports.Rejected):
            reports.instrumentation(text, list(expected))

    def test_complete_statuses_and_summary_pass(self):
        result = reports.instrumentation(passed(), [A, B])
        self.assertEqual(result["actual_passed_tests"], 2)
        self.assertEqual(result["cases"], [A, B])

    def test_different_test_order_is_allowed(self):
        self.assertEqual(reports.instrumentation(passed((B, A)), [A, B])["cases"], [A, B])

    def test_summary_does_not_replace_actual_assertions(self):
        self.refused("INSTRUMENTATION_RESULT: stream=\nOK (2 tests)\nINSTRUMENTATION_CODE: -1\n")

    def test_all_failed_ignored_assumption_and_unknown_codes_refuse(self):
        for code in (-1, -2, -3, -4, 2):
            with self.subTest(code=code):
                self.refused(passed().replace("INSTRUMENTATION_STATUS_CODE: 0", f"INSTRUMENTATION_STATUS_CODE: {code}", 1))

    def test_duplicate_and_missing_statuses_refuse(self):
        self.refused(passed().replace(block(A, 1, 2, 1), "", 1))
        self.refused(passed().replace(block(A, 1, 2, 0), "", 1))
        self.refused(block(A, 1, 2, 1) + passed())

    def test_duplicate_test_and_inconsistent_counts_refuse(self):
        self.refused(passed((A, A)))
        self.refused(passed().replace("numtests=2", "numtests=3", 1))
        self.refused(passed().replace("current=2", "current=1", 1))

    def test_extra_missing_foreign_case_refuses(self):
        self.refused(passed((A,)))
        self.refused(passed((A, B, B.replace("second", "third"))))
        self.refused(passed().replace("com.visualworkbench.android.", "private.owner."))

    def test_final_status_and_summary_are_required_exactly_once(self):
        self.refused(passed().replace("INSTRUMENTATION_CODE: -1", "INSTRUMENTATION_CODE: 0"))
        self.refused(passed().replace("OK (2 tests)", ""))
        self.refused(passed().replace("OK (2 tests)", "OK (2 tests)\nOK (2 tests)"))
        self.refused(passed() + "INSTRUMENTATION_CODE: -1\n")

    def test_duplicate_or_error_fields_refuse(self):
        self.refused(passed().replace("INSTRUMENTATION_STATUS: id=AndroidJUnitRunner", "INSTRUMENTATION_STATUS: class=forged", 1))
        self.refused(passed().replace("INSTRUMENTATION_RESULT: stream=", "INSTRUMENTATION_RESULT: shortMsg=Process crashed"))

    def test_oversized_and_after_result_status_refuse(self):
        self.refused(passed() + "x" * 65537)
        self.refused(passed() + block(A, 1, 2, 1))

    def test_untrusted_stream_prose_is_never_copied_to_receipt(self):
        text = passed().replace("Time: 1.2", "Time: 1.2\nprivate-device serial /private/file")
        self.assertNotIn("private", json.dumps(reports.instrumentation(text, [A, B])))

    def test_failure_diagnosis_contains_only_known_case_and_status(self):
        text = block(A, 1, 2, -2) + "secret private failure detail\n"
        report = reports.failure_observation(text, [A, B])
        self.assertEqual(report["observed_statuses"], [{"case": A, "status_code": -2}])
        self.assertNotIn("secret", json.dumps(report))

    def test_failure_diagnosis_retains_only_known_test_lines(self):
        cls = A.split("#")[0]
        stem = cls.rsplit(".", 1)[-1]
        stack = ("INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: secret personal text\n"
                 f"\tat {cls}.example({stem}.kt:27)\n"
                 "\tat unknown.private.Class.call(Private.kt:81)\n")
        text = block(A, 1, 2, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
        report = reports.failure_observation(text, [A, B])
        self.assertEqual(report["observed_statuses"], [{"case": A, "status_code": -2,
            "test_source_lines": [27], "exception_kind": "java.lang.AssertionError"}])
        self.assertNotIn("personal", json.dumps(report))
        self.assertNotIn("Private", json.dumps(report))
        self.refused(text)


class ArtifactCases(unittest.TestCase):
    def test_only_isolated_manifest_pair_is_admitted(self):
        reports.manifest(manifest(), False)
        reports.manifest(manifest(True), True)

    def test_main_owner_package_and_authority_are_refused(self):
        for old, new in ((reports.MAIN, "com.visualworkbench.android"),
                         (reports.MAIN + ".images", "com.visualworkbench.android.images"),
                         ("Visual Workbench Test", "Visual Workbench")):
            with self.subTest(new=new), self.assertRaises(reports.Rejected):
                reports.manifest(manifest().replace(old, new), False)

    def test_test_target_runner_shared_uid_and_release_refused(self):
        for text, is_test in ((manifest(True).replace(reports.RUNNER, "unknown.Runner"), True),
                              (manifest(True).replace(f'targetPackage="{reports.MAIN}"', 'targetPackage="com.visualworkbench.android"'), True),
                              (manifest().replace('android:debuggable="true"', 'android:debuggable="false"'), False),
                              (manifest().replace('<manifest ', '<manifest android:sharedUserId="owner" '), False)):
            with self.subTest(text=text), self.assertRaises(reports.Rejected):
                reports.manifest(text, is_test)

    def test_manifest_entity_and_size_bombs_refuse(self):
        for text in ('<!DOCTYPE manifest [<!ENTITY x "a">]>' + manifest(), manifest() + ' ' * (1024**2)):
            with self.assertRaises(reports.Rejected):
                reports.manifest(text, False)

    def test_signer_exactly_one_sha256_required(self):
        text = "Signer #1 certificate SHA-256 digest: " + "a" * 64 + "\n"
        self.assertEqual(reports.signer(text), "a" * 64)
        for invalid in ("", text + text, text.replace("#1", "#2"), text[:-3]):
            with self.assertRaises(reports.Rejected):
                reports.signer(invalid)

    def test_case_inventory_fails_on_added_removed_and_skip_methods(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tests = root / "apps/android/src/androidTest"
            config = root / "tools/app-test"
            tests.mkdir(parents=True)
            config.mkdir(parents=True)
            (config / "inventory.json").write_text(json.dumps({"schema": 1, "expected_tests": [A]}))
            path = tests / "Synthetic.kt"
            source = 'package com.visualworkbench.android\nclass SyntheticInstrumentedTest { @Test fun first() {} }'
            path.write_text(source)
            self.assertEqual(reports.cases(root), [A])
            for invalid in (source.replace('first()', 'second()'), source.replace('@Test', '@Ignore @Test'),
                            source.replace(' {} }', ' {} @Test fun second() {} }')):
                path.write_text(invalid)
                with self.assertRaises(reports.Rejected):
                    reports.cases(root)

    def test_remote_inventory_preserves_phase2_154_and_matches_all_source_methods(self):
        root = Path(__file__).resolve().parents[2]
        declared = reports.common.bounded_json(root / "tools/app-test/inventory.json")["expected_tests"]
        reviewed_remote = ['com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#actual_callback_thread_is_retained_until_it_joins_and_old_callback_cannot_ack', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#blocked_configuration_retains_permit_and_surface_until_actual_join', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#blocked_surface_release_retains_its_io_worker_and_permit_until_join', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#borrowed_surface_survives_actual_owned_wrapper_retirement', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#failed_release_stays_owned_and_explicit_close_retries_same_engine', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#first_frame_requires_idr_and_scope_generation_must_match', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#input_bytes_are_copied_and_au_size_is_bounded', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#policy_rejects_oversize_config_odd_coded_geometry_and_unbounded_padding', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#pts_release_render_callback_and_enqueue_times_remain_distinct', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#same_config_recovery_has_fresh_owner_and_rejects_delta_start', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#stale_pts_frame_ticket_and_expired_input_refuse', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#successful_surface_callback_returns_exact_ticket_without_editor_effect_claim']
        remote = [case for case in declared if ".remote." in case]
        baseline = [case for case in declared if ".remote." not in case]
        self.assertEqual(len(declared), 166)
        self.assertEqual(remote, reviewed_remote)
        self.assertEqual(len(baseline), 154)
        self.assertEqual(hashlib.sha256(json.dumps(baseline, separators=(",", ":"), ensure_ascii=False).encode("utf-8")).hexdigest(), 'ece5b64a5fb3d6ef2abe01944fed1a19771acf479ff852a5843aa7ded2b75e72')
        self.assertEqual(reports.cases(root), declared)
        self.assertEqual(reports.instrumentation(passed(tuple(declared)), declared)["actual_passed_tests"], 166)

    def test_discovery_refuses_normal_output_and_split_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for folder, package in (("debug", reports.MAIN), ("androidTest/debug", reports.TEST)):
                out = root / "apps/android/build-hil/outputs/apk" / folder
                out.mkdir(parents=True)
                (out / "fixture.apk").write_bytes(b"synthetic-not-inspected")
                (out / "output-metadata.json").write_text(json.dumps({"applicationId": package,
                    "elements": [{"outputFile": "fixture.apk", "filters": []}]}))
            with patch.object(reports, "unchanged"):
                self.assertEqual(set(reports.discover(root, {"started_ns": 0})), {"main", "test"})
                metadata = root / "apps/android/build-hil/outputs/apk/debug/output-metadata.json"
                for value in ({"applicationId": "com.visualworkbench.android", "elements": []},
                              {"applicationId": reports.MAIN, "elements": [{"filters": ["arm64"]}]},
                              {"applicationId": reports.MAIN, "elements": [{"outputFile": "../normal.apk"}]}):
                    metadata.write_text(json.dumps(value))
                    with self.assertRaises(reports.Rejected):
                        reports.discover(root, {"started_ns": 0})


class InputBindingCases(unittest.TestCase):
    def test_native_override_must_match_pinned_directories(self):
        root = Path.cwd()
        expected = reports.binding_inputs(root, {})
        for key, value in expected.items():
            self.assertEqual(reports.binding_inputs(root, {"ORG_GRADLE_PROJECT_" + key: str(value)}), expected)
            with self.assertRaises(reports.Rejected):
                reports.binding_inputs(root, {"ORG_GRADLE_PROJECT_" + key: str(root / "unbound")})

    def test_generator_library_and_wrapper_are_bound_and_drift_refuses(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            watched = ["target/debug/vw_core.dll", "target/debug/vw-bindgen.exe", "apps/gradlew.bat"]
            for relative in watched:
                file = root / relative
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(b"reviewed synthetic input")
            def digest(file):
                return hashlib.sha256(file.read_bytes()).hexdigest() if file.is_file() else "f" * 64
            with patch.object(reports.common, "files_under", return_value=[]), patch.object(reports.common, "digest", side_effect=digest), patch.object(reports, "cases", return_value=[A]):
                source = reports.inventory(root)
                self.assertTrue(all(name in source for name in watched))
                state = {"schema": 1, "expected_tests": [A], "source_sha256": source}
                reports.unchanged(root, state)
                for relative in watched:
                    original = (root / relative).read_bytes()
                    (root / relative).write_bytes(b"changed by another build")
                    with self.assertRaises(reports.Rejected):
                        reports.unchanged(root, state)
                    (root / relative).write_bytes(original)

    def test_staged_copy_must_have_private_path_and_discovered_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            private = Path(directory)
            paths, sources = {}, {}
            for key in ("main", "test"):
                file = private / (key + ".apk")
                file.write_bytes(b"synthetic bound bytes")
                digest = hashlib.sha256(file.read_bytes()).hexdigest()
                paths[key] = {"path": str(file), "sha256": digest}
                sources[key] = {"path": str(private / "agp-output" / (key + ".apk")), "sha256": digest}
            reports.staged_paths(paths, sources, private)
            foreign = {key: dict(value) for key, value in paths.items()}
            foreign["main"]["path"] = sources["main"]["path"]
            with self.assertRaises(reports.Rejected):
                reports.staged_paths(foreign, sources, private)
            (private / "main.apk").write_bytes(b"replacement package")
            with self.assertRaises(reports.Rejected):
                reports.staged_paths(paths, sources, private)

    def test_actual_verify_binds_rust_library_with_all_generator_inputs_present(self):
        with tempfile.TemporaryDirectory() as directory:
            private = Path(directory)
            paths, sources = {}, {}
            for key in ("main", "test"):
                file = private / (key + ".apk")
                file.write_bytes(b"synthetic SDK-verified APK bytes")
                digest = hashlib.sha256(file.read_bytes()).hexdigest()
                paths[key] = {"path": str(file), "sha256": digest}
                sources[key] = {"path": "unread-mutable-agp-output", "sha256": digest}
                (private / (key + ".xml")).write_text(manifest(key == "test"))
                (private / (key + ".cert.txt")).write_text("Signer #1 certificate SHA-256 digest: " + "f" * 64 + "\n")
            (private / "source-apks.json").write_text(json.dumps(sources))
            state = {"source_sha256": {"target/android-jni/arm64-v8a/libvw_core.so": "a" * 64,
                     "target/debug/vw_core.dll": "b" * 64, "target/debug/vw-bindgen.exe": "c" * 64}}
            def native(path, required):
                return {"apk_sha256": paths["main" if required else "test"]["sha256"],
                        "libraries": [{"entry": "lib/arm64-v8a/libvw_core.so", "sha256": "a" * 64}] if required else []}
            with patch.object(reports, "unchanged"), patch.object(reports, "native_inventory", side_effect=native):
                verified = reports.verify(private, state, paths, private)
                self.assertEqual(verified["main"]["package"], reports.MAIN)
                state["source_sha256"]["target/android-jni/arm64-v8a/libvw_core.so"] = "d" * 64
                with self.assertRaises(reports.Rejected):
                    reports.verify(private, state, paths, private)

    def test_runner_pins_inputs_and_uses_locked_stage_for_sdk_and_install(self):
        source = Path(__file__).with_name("run_android.ps1").read_text()
        self.assertIn("('-PvwNativeDir='+(Join-Path $ProjectRoot 'target/debug'))", source)
        self.assertIn("('-PvwAndroidNativeDir='+(Join-Path $ProjectRoot 'target/android-jni'))", source)
        self.assertLess(source.index("$candidate=Copy-VwAppApk"), source.index("$analyzer @('manifest'"))
        self.assertIn("@('install','--user','0','-t',$apks.$key.path)", source)
        self.assertLess(source.index("@('install','--user','0','-t',$apks.$key.path)"), source.index("foreach($handle in $apkLocks.Values){try{$handle.Dispose()}catch{$processesClean=$false}}"))


if __name__ == "__main__":
    unittest.main()
