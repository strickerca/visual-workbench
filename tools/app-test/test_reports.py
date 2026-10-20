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
        for package in ("capture", "editor", "instructions", "remote", "stream"):
            case = A.replace("android.", "android." + package + ".")
            self.assertEqual(reports.instrumentation(passed((case,)), [case])["actual_passed_tests"], 1)
        for package in ("foreign", "editor.nested", "capture..", "remote.nested", "remote..", "remotely", "stream.nested", "stream..", "streaming"):
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

    def test_failure_diagnosis_retains_only_finite_coroutine_and_creation_kinds(self):
        known = ("java.util.concurrent.CancellationException",
                 "kotlinx.coroutines.TimeoutCancellationException", "kotlinx.coroutines.JobCancellationException",
                 "com.visualworkbench.android.editor.GiantAndroidCreationFailure")
        for kind in known:
            stack = f"INSTRUMENTATION_STATUS: stack={kind}: secret private runtime text\n"
            text = block(A, 1, 2, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
            report = reports.failure_observation(text, [A, B])
            with self.subTest(kind=kind):
                self.assertEqual(report["observed_statuses"], [{"case": A, "status_code": -2, "exception_kind": kind}])
                self.assertNotIn("secret", json.dumps(report))
                self.assertNotIn("private", json.dumps(report))
                self.refused(text)
        for kind in ("kotlinx.coroutines.PrivateFailure", known[0] + "Private", known[-1] + "Private"):
            stack = f"INSTRUMENTATION_STATUS: stack={kind}: secret private runtime text\n"
            text = block(A, 1, 2, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
            self.assertEqual(reports.failure_observation(text, [A])["observed_statuses"], [{"case": A, "status_code": -2}])

    def test_surface_timeout_retains_only_case_bound_fixed_phase(self):
        for case in reports.SURFACE_AWAIT_CASES:
            for phase in reports.SURFACE_AWAIT_PHASES:
                stack = (f"INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in {phase}: "
                         "busy=true message=secret project text blocked=/private/path\n")
                text = block(case, 1, 1, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
                with self.subTest(case=case, phase=phase):
                    report = reports.failure_observation(text, [case])
                    self.assertEqual(report["observed_statuses"], [{"case": case, "status_code": -2,
                        "exception_kind": "java.lang.AssertionError", "await_phase": phase}])
                    self.assertNotIn("secret", json.dumps(report))
                    self.assertNotIn("private", json.dumps(report))
                    self.refused(text, (case,))

    def test_sync_timeout_keeps_original_kind_and_first_fixed_phase(self):
        case = reports.SYNC_AWAIT_CASE
        for phase in reports.SYNC_AWAIT_PHASES:
            stack = ("INSTRUMENTATION_STATUS: stack=kotlinx.coroutines.TimeoutCancellationException: secret timeout text\n"
                     f"\tSuppressed: com.visualworkbench.android.EditorSyncInstrumentedTest$TimeoutPhase: VW_SYNC_AWAIT_PHASE={phase}\n"
                     "\tSuppressed: kotlinx.coroutines.TimeoutCancellationException: private cleanup detail\n"
                     "\t\tSuppressed: com.visualworkbench.android.EditorSyncInstrumentedTest$TimeoutPhase: VW_SYNC_AWAIT_PHASE=FixtureCloseRetained\n")
            text = block(case, 1, 1, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
            with self.subTest(phase=phase):
                report = reports.failure_observation(text, [case])
                self.assertEqual(report["observed_statuses"], [{"case": case, "status_code": -2,
                    "exception_kind": "kotlinx.coroutines.TimeoutCancellationException", "await_phase": phase}])
                self.assertNotIn("secret", json.dumps(report))
                self.assertNotIn("private", json.dumps(report))
                self.refused(text, (case,))

    def test_timeout_phase_rejects_unknown_cases_suffixes_and_wrong_primary_kind(self):
        surface = sorted(reports.SURFACE_AWAIT_CASES)[0]
        marker = "\tSuppressed: com.visualworkbench.android.EditorSyncInstrumentedTest$TimeoutPhase: VW_SYNC_AWAIT_PHASE="
        trials = (
            (A, "INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in save-text: secret\n"),
            (surface, "INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in save-text-private: secret\n"),
            (surface, "INSTRUMENTATION_STATUS: stack=java.lang.IllegalStateException: Timed out in save-text: secret\n"),
            (A, "INSTRUMENTATION_STATUS: stack=kotlinx.coroutines.TimeoutCancellationException: secret\n" + marker + "PageOneAdopted\n"),
            (reports.SYNC_AWAIT_CASE, "INSTRUMENTATION_STATUS: stack=kotlinx.coroutines.TimeoutCancellationException: secret\n" + marker + "PageOneAdoptedPrivate\n"),
            (reports.SYNC_AWAIT_CASE, "INSTRUMENTATION_STATUS: stack=kotlinx.coroutines.TimeoutCancellationException: secret\n" + marker + "PageOneAdopted private\n"),
            (reports.SYNC_AWAIT_CASE, "INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: secret\n" + marker + "PageOneAdopted\n"),
        )
        for case, stack in trials:
            text = block(case, 1, 1, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
            with self.subTest(case=case, stack_kind=stack.split(":", 2)[1]):
                report = reports.failure_observation(text, [case])
                self.assertNotIn("await_phase", report["observed_statuses"][0])
                self.assertNotIn("surface_timeout_state", report["observed_statuses"][0])
                self.assertNotIn("secret", json.dumps(report))
                self.assertNotIn("private", json.dumps(report))

    def test_timeout_phase_cannot_leak_into_next_case_or_success(self):
        surface = sorted(reports.SURFACE_AWAIT_CASES)[0]
        stack = "INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in save-text: secret\n"
        text = block(surface, 1, 2, 0).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
        text += block(A, 2, 2, -2)
        report = reports.failure_observation(text, [surface, A])
        self.assertEqual(report["observed_statuses"], [{"case": surface, "status_code": 0}, {"case": A, "status_code": -2}])


    def test_surface_primary_timeout_retains_only_finite_state(self):
        case = sorted(reports.SURFACE_AWAIT_CASES)[0]
        for message, kind in dict(reports.SURFACE_NAVIGATION_MESSAGES, null="none").items():
            stack = ("INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in close-owned-project: "
                     f"busy=false pending=0 message={message} blocked=null\n")
            text = block(case, 1, 1, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
            observation = reports.failure_observation(text, [case])["observed_statuses"][0]
            self.assertEqual(observation["surface_timeout_state"], {"busy": False, "pending_nonzero": False,
                "message_kind": kind, "blocked_present": False})
            self.assertNotIn(message, json.dumps(observation))
            self.refused(text, (case,))
        stack = ("INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in close-owned-project: "
                 "busy=true pending=987654 message=secret /private/project blocked=private block details\n")
        text = block(case, 1, 1, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
        report = reports.failure_observation(text, [case])
        self.assertEqual(report["observed_statuses"][0]["surface_timeout_state"],
            {"busy": True, "pending_nonzero": True, "message_kind": "unknown", "blocked_present": True})
        for private in ("secret", "private", "987654", "block details"):
            self.assertNotIn(private, json.dumps(report))

    def test_surface_state_omits_malformed_or_ambiguous_details(self):
        case = sorted(reports.SURFACE_AWAIT_CASES)[0]
        prefix = "INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in close-owned-project: "
        for detail in ("busy=true message=null blocked=null", "busy=True pending=0 message=null blocked=null",
                       "busy=false pending=00 message=null blocked=null", "busy=false pending=2147483648 message=null blocked=null",
                       "busy=true pending=0 message=private blocked=forged blocked=null",
                       "busy=true pending=0 message=private message=forged blocked=null",
                       "busy=true pending=0 message=" + "x" * 1025 + " blocked=null"):
            text = block(case, 1, 1, -2).replace("INSTRUMENTATION_STATUS_CODE:", prefix + detail + "\nINSTRUMENTATION_STATUS_CODE:")
            report = reports.failure_observation(text, [case])
            self.assertNotIn("surface_timeout_state", report["observed_statuses"][0])
            self.assertNotIn("private", json.dumps(report))

    def test_surface_primary_header_excludes_suppressed_and_duplicate_cleanup(self):
        case = sorted(reports.SURFACE_AWAIT_CASES)[0]
        primary = ("INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in save-text: "
                   "busy=true pending=1 message=null blocked=null\n")
        close = primary.replace("save-text", "close-owned-project")
        suppressed = "\tSuppressed: " + close.removeprefix("INSTRUMENTATION_STATUS: stack=")
        for stack, phase in (("INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: primary private\n" + suppressed, None),
                             (primary + suppressed, "save-text"), (primary + close, None), (primary + primary, None)):
            text = block(case, 1, 1, -2).replace("INSTRUMENTATION_STATUS_CODE:", stack + "INSTRUMENTATION_STATUS_CODE:")
            observation = reports.failure_observation(text, [case])["observed_statuses"][0]
            self.assertEqual(observation.get("await_phase"), phase)
            if phase is None:
                self.assertNotIn("surface_timeout_state", observation)
            else:
                self.assertTrue(observation["surface_timeout_state"]["busy"])
            self.assertNotIn("private", json.dumps(observation))



    def close_snapshot_failure(self, suffix, case=None, code=-2):
        case = case or sorted(reports.SURFACE_AWAIT_CASES)[0]
        header = ("INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: Timed out in close-owned-project: "
                  "busy=false pending=0 message=null blocked=null" + suffix + "\n")
        text = block(case, 1, 1, code).replace("INSTRUMENTATION_STATUS_CODE:", header + "INSTRUMENTATION_STATUS_CODE:")
        return text, reports.failure_observation(text, [case])["observed_statuses"][0]

    def test_surface_close_snapshot_preserves_existing_finite_state_and_sentinel(self):
        text, observation = self.close_snapshot_failure(
            " close_attempt=0 close_stage=None close_status=None close_failed_stage=None")
        self.assertEqual(observation["surface_timeout_state"], {
            "busy": False, "pending_nonzero": False, "message_kind": "none", "blocked_present": False,
            "close_progress": {"attempt": 0, "stage": "None", "status": "None", "failed_stage": "None"}})
        self.assertEqual(observation["await_phase"], "close-owned-project")
        self.refused(text, (observation["case"],))

    def test_surface_close_snapshot_accepts_exact_finite_stages_statuses_and_first_failure(self):
        stages = (
            "Requested", "Queued", "Navigation", "CaptureStream", "Assisted", "MutationGate", "MutationAcquired",
            "ProjectClose", "ImageFrames", "Workflows", "Rotation", "DocumentOwner", "Giant", "Erasers", "Selections",
            "InstructionFocus", "Semantics", "Instructions", "Ai", "ChangeObserver", "RefreshObserver", "Link",
            "ProjectStorage", "ProjectsCatalog", "NavigationInstructionCleanup", "Finished",
        )
        statuses = ("Running", "Settled", "Failed", "Completed", "Declined")
        self.assertEqual(reports.SURFACE_CLOSE_STAGES, frozenset(stages))
        self.assertEqual(reports.SURFACE_CLOSE_STATUSES, frozenset(statuses))
        for stage in stages:
            for status in statuses:
                with self.subTest(stage=stage, status=status):
                    _, observation = self.close_snapshot_failure(
                        f" close_attempt=1 close_stage={stage} close_status={status} close_failed_stage=None")
                    self.assertEqual(observation["surface_timeout_state"]["close_progress"],
                        {"attempt": 1, "stage": stage, "status": status, "failed_stage": "None"})
        for failed in stages:
            _, observation = self.close_snapshot_failure(
                f" close_attempt=2147483647 close_stage=Workflows close_status=Failed close_failed_stage={failed}")
            self.assertEqual(observation["surface_timeout_state"]["close_progress"]["failed_stage"], failed)

    def test_surface_close_snapshot_rejects_noncanonical_overflow_and_mismatched_sentinel(self):
        live = " close_attempt=1 close_stage=Workflows close_status=Running close_failed_stage=None"
        invalid = [live.replace("attempt=1", "attempt=" + value) for value in
                   ("00", "01", "+1", "-1", "1.0", "2147483648", "99999999999", "private", "")]
        invalid += [live.replace("attempt=1", "attempt=0"),
                    live.replace("stage=Workflows", "stage=None"),
                    live.replace("status=Running", "status=None"),
                    " close_attempt=0 close_stage=None close_status=None close_failed_stage=Workflows"]
        for suffix in invalid:
            with self.subTest(suffix=suffix):
                _, observation = self.close_snapshot_failure(suffix)
                self.assertNotIn("surface_timeout_state", observation)
                self.assertNotIn("private", json.dumps(observation))

    def test_surface_close_snapshot_rejects_unknown_duplicate_reordered_and_private_tails(self):
        live = " close_attempt=1 close_stage=Workflows close_status=Running close_failed_stage=None"
        for suffix in (live.replace("Workflows", "Private"), live.replace("Running", "Private"),
                       live.replace("failed_stage=None", "failed_stage=Private"),
                       live + " close_attempt=2", live + " close_stage=Finished", live + live,
                       live + " private /secret/project", live + " close_private=secret",
                       live.replace("close_attempt=1 close_stage=Workflows", "close_stage=Workflows close_attempt=1"),
                       live.replace("close_status=Running", "close_Status=Running"),
                       live.replace("close_status=Running", "close_status=RunningPrivate"),
                       live.replace("close_stage=Workflows", "close_stage=" + "x" * 2561)):
            with self.subTest(suffix=suffix[:100]):
                _, observation = self.close_snapshot_failure(suffix)
                self.assertNotIn("surface_timeout_state", observation)
                self.assertNotIn("secret", json.dumps(observation))
                self.assertNotIn("Private", json.dumps(observation))

    def test_surface_close_snapshot_is_case_bound_and_never_establishes_acceptance(self):
        suffix = " close_attempt=1 close_stage=Workflows close_status=Running close_failed_stage=None"
        for case in reports.SURFACE_AWAIT_CASES:
            text, observation = self.close_snapshot_failure(suffix, case)
            self.assertIn("close_progress", observation["surface_timeout_state"])
            self.refused(text, (case,))
            self.assertFalse(reports.failure_observation(text, [case])["assertions_passed"])
        for case, code in ((A, -2), (None, 0), (None, 1)):
            _, observation = self.close_snapshot_failure(suffix, case, code)
            self.assertNotIn("surface_timeout_state", observation)

    def test_surface_close_snapshot_ignores_suppressed_wrong_kind_and_duplicate_headers(self):
        suffix = " close_attempt=1 close_stage=Workflows close_status=Running close_failed_stage=None"
        text, observation = self.close_snapshot_failure(suffix)
        header = next(line for line in text.splitlines() if line.startswith("INSTRUMENTATION_STATUS: stack="))
        for replacement in (header.replace("java.lang.AssertionError", "java.lang.IllegalStateException"),
                            "INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: primary private\n\tSuppressed: " +
                            header.removeprefix("INSTRUMENTATION_STATUS: stack="),
                            header + "\n" + header,
                            header + "\n" + header.replace("attempt=1", "attempt=2")):
            changed = text.replace(header, replacement)
            report = reports.failure_observation(changed, [observation["case"]])
            self.assertNotIn("surface_timeout_state", report["observed_statuses"][0])
            self.assertNotIn("private", json.dumps(report))

    def test_surface_close_snapshot_resets_between_cases_and_keeps_private_fields_redacted(self):
        suffix = " close_attempt=1 close_stage=Workflows close_status=Failed close_failed_stage=ImageFrames"
        text, observation = self.close_snapshot_failure(suffix)
        text = text.replace("message=null blocked=null", "message=secret personal project blocked=/private/file")
        text += block(A, 2, 2, -2)
        report = reports.failure_observation(text, [observation["case"], A])
        state = report["observed_statuses"][0]["surface_timeout_state"]
        self.assertEqual(state["message_kind"], "unknown")
        self.assertTrue(state["blocked_present"])
        self.assertEqual(state["close_progress"]["failed_stage"], "ImageFrames")
        self.assertEqual(report["observed_statuses"][1], {"case": A, "status_code": -2})
        for private in ("secret", "personal", "private", "/file"):
            self.assertNotIn(private, json.dumps(report))


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

    def test_reviewed_inventory_preserves_phase2_154_and_remote_12_with_exact_current_sources(self):
        root = Path(__file__).resolve().parents[2]
        declared = reports.common.bounded_json(root / "tools/app-test/inventory.json")["expected_tests"]
        reviewed_remote = ['com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#actual_callback_thread_is_retained_until_it_joins_and_old_callback_cannot_ack', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#blocked_configuration_retains_permit_and_surface_until_actual_join', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#blocked_surface_release_retains_its_io_worker_and_permit_until_join', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#borrowed_surface_survives_actual_owned_wrapper_retirement', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#failed_release_stays_owned_and_explicit_close_retries_same_engine', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#first_frame_requires_idr_and_scope_generation_must_match', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#input_bytes_are_copied_and_au_size_is_bounded', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#policy_rejects_oversize_config_odd_coded_geometry_and_unbounded_padding', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#pts_release_render_callback_and_enqueue_times_remain_distinct', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#same_config_recovery_has_fresh_owner_and_rejects_delta_start', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#stale_pts_frame_ticket_and_expired_input_refuse', 'com.visualworkbench.android.remote.RemoteDecoderInstrumentedTest#successful_surface_callback_returns_exact_ticket_without_editor_effect_claim']
        # Explicit reviewed extensions; never derive this allowlist from current source.
        reviewed_additions = {
            'AiEditorOwnershipInstrumentedTest': (
                'cancelledUnpublishedPreviewIsAbandonedBeforeDiscardReturns',
                'detachClosesEveryGridOwnerButRetainsFailedCloseForJoinedRetry',
                'eachGridReviewKeepsItsExactConfigurationAndOnlyCurrentConfigurationCanSend',
                'failedDiscardRetainsPaidOwnerAndRetryOnlyClosesSelectedRequest',
                'gridBoundRefusesFifthBeforeCreatingAnotherOwnerAndDiscardFreesOneSlot',
                'gridNeverReplacesPaidOwnerWhenNextPreparationFailsOrSourceProofChanges',
                'gridRetainsIndependentExplicitSendOwnersAndExactSelection',
                'identicalPixelCandidateIdsRemainDistinctRequestOwners',
                'largeCandidateGridPreviewRemainsBoundedWhenFullDetailIsRefused',
                'mismatchedSaveReceiptCannotReplaceTheExactRetainedSaveTransaction',
                'partialMaskAndExactSaveRetryStayWithTheirSelectedRequest',
                'publishedGridPreviewFollowsRendererLifetimeWhenItsOwnerIsDiscarded',
                'refusedOutsideProofCannotEnterGridOrBeAppliedAndRecoveryDoesNotResend',
                'reviewedGlobalInstructionStaysWithEachGridEntry',
                'staleGridKeepsComparisonsButCannotSendApplyOrAcceptAndCannotPrepareNewGroup',
                'switchingGridEntryWaitsForItsCancelledPixelProducer',
                'unsupportedProviderReadinessCannotConsumeAnAttemptEvenThroughDirectControllerCall',
            ),
            'AiServiceControllerInstrumentedTest': (
                'failedCloseRetainsOwnerForASecondRealRetirement',
                'failedLateDeliveryRetirementRemainsOwnedUntilCloseRetries',
            ),
            'EditorSyncInstrumentedTest': (
                'oldPageRefreshCannotAdoptOrRebindAfterSameRevisionNavigation',
            ),
            'PolygonEditorInstrumentedTest': (
                'canceledContactAndChangedViewportCannotReuseEarlierPolygonVertices',
                'separateDownUpContactsKeepVerticesAndFinishPublishesOneUndoableTransaction',
            ),
            'PrecisionInstrumentedTest': (
                'acceptedAndPredictedRoutingPreservesEveryNonCoordinateField',
                'actualResizeCornerAdmissionStaysTwentyFourPhysicalPixelsAtTwoFourAndEightTimes',
                'actualSelectionAndTextHitSlopStaysEightPhysicalPixelsAtEveryLensMagnification',
                'inspectionOnAndOffPreservesAllAcceptedSampleBits',
                'lensVisibleGeometryOutsideTheBaseViewportRemainsSelectable',
                'physicalHoldAcrossHoverToContactDoesNotBecomeAShortClick',
                'rendererClipsTheInsetAndUsesActualImmutableSourcePixels',
                'shortReleaseBeforeUpQueuesExactlyOneToggleAndCancellationNeverClicks',
                'snapCalibrationCapturesLensPhysicalPixelsBeforeQueuedClose',
            ),
            'RasterTransfersInstrumentedTest': (
                'receiptCallbackRunsBeforeWriterAndCanCancelWithoutPublishing',
                'refusedShareReceiptNeverCreatesLease',
                'staleNativeReceiptIsRejectedBeforeOpeningProviderWriter',
                'unsupportedRawAndAiNeverReachAnImageDecoderOrOpenSource',
            ),
            'SessionLifecycleInstrumentedTest': (
                'acceptedQrOwnsBytesAndCopiesOnlyChosenRouteAfterSuccessfulPairing',
                'backgroundDuringPeerRefreshRejectsLateKnownEmptyOrLoadedResult',
                'friendlyNamesStayLocalAndCertificateBoundWithoutChangingIdentityOrRoutes',
                'loadedTrustedPeersDoNotStartScannerOrReplaceChosenRoutesOnEntry',
                'pairingConnectionHintRejectsChangedCertificateAndPendingCodeConfirmation',
                'trustedPeersRemainLoadedWhenOptionalDeviceDetailsFail',
            ),
            'StylusTiltInstrumentedTest': (
                'absentAxesRemainUnavailableAndCannotBeEnabledBySupplyingSigns',
                'horizontalFloatAxesAndQuarterTurnProduceOnlyTheKnownCardinalComponent',
                'reportedHistoricalAnglesStayRawWhileProjectionUsesAnExplicitCalibration',
            ),
            'editor.GiantAndroidCanvasInstrumentedTest': (
                'actualFrameworkReleaseCallbackPrecedesBitmapAndChargeRelease',
                'cancelledFactorySettlesLateSurfaceReservationBeforeReturningCancellation',
                'closeSealsBeforeReleaseAndKeepsBitmapAndBothReservationsUntilAck',
                'composedModeUsesEditingTilesAndRefusesASourceFrameAtDraw',
                'failedFactoryCleanupReturnsTheExactOwnerForRootQuarantine',
                'failedPipelineRetirementKeepsTheBitmapAndBothChargesQuarantined',
                'failedSurfaceChargeReleaseRetriesWithoutReclosingNativeOrGraphics',
                'fullBindingEpochViewportAndCurrentOwnerFenceEveryFrame',
                'invalidAlphaAndDimensionsRefuseBeforeAnImageCanBePresented',
                'parentCancellationNeverReopensTheRetiredSurface',
                'rgbaBytesUseOpaqueSrgbAndActualStorageFitsTheTileReservation',
                'softwareDrawUsesOnlyDocumentRectanglesAndTheSuppliedCanvasTransform',
            ),
            'stream.AndroidStreamPixelsInstrumentedTest': (
                'admissionPrecedesRowsAndCopiesExactOpaquePixelsInBoundedBatches',
                'cancellationWhileReadingRowsJoinsPartialBitmapCleanup',
                'malformedRowsReleaseOnlyThePartialBitmapAndItsCharge',
                'refusedForeignReservationNeverReadsPixels',
            ),
        }
        additions = sorted(f"com.visualworkbench.android.{owner}#{method}"
                           for owner, methods in reviewed_additions.items() for method in methods)
        # The old sticky-close-failure contract was replaced by two retained-owner retry cases.
        retired = ['com.visualworkbench.android.AiServiceControllerInstrumentedTest#laterCloseCallersObserveTheSameSettledFailure']
        remote = [case for case in declared if ".remote." in case]
        surviving_baseline = [case for case in declared if case not in additions and ".remote." not in case]
        baseline = sorted(surviving_baseline + retired)
        self.assertEqual(len(declared), 225)
        self.assertEqual(remote, reviewed_remote)
        self.assertEqual(len(additions), 60)
        self.assertTrue(set(additions).issubset(declared))
        self.assertTrue(set(retired).isdisjoint(declared))
        self.assertEqual(len(surviving_baseline), 153)
        self.assertEqual(len(baseline), 154)
        self.assertEqual(hashlib.sha256(json.dumps(baseline, separators=(",", ":"), ensure_ascii=False).encode("utf-8")).hexdigest(), 'ece5b64a5fb3d6ef2abe01944fed1a19771acf479ff852a5843aa7ded2b75e72')
        self.assertEqual(reports.cases(root), declared)
        self.assertEqual(reports.instrumentation(passed(tuple(declared)), declared)["actual_passed_tests"], 225)

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
