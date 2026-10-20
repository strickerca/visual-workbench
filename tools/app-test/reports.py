"""Bounded, path-free assertions for the isolated Android app HIL runner.

No Gradle, adb, app, or subprocess runs here. APK signatures are checked by the
SDK apksigner in the parent runner; its successful exit is independently required.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ffi-test"))
import shared_reports as common
import check_apk

MAIN = "com.visualworkbench.android.hil"
TEST = MAIN + ".test"
RUNNER = "androidx.test.runner.AndroidJUnitRunner"
CASE = re.compile(r"com\.visualworkbench\.android(?:\.(?:capture|editor|instructions|remote|stream))?\.[A-Za-z][A-Za-z0-9_]*#[A-Za-z][A-Za-z0-9_]*\Z")
MAX_OUTPUT = 8 * 1024 * 1024
SURFACE_AWAIT_CASES = frozenset({
    "com.visualworkbench.android.EditorSurfaceInstrumentedTest#everyDrawingToolPersistsAndCancellationCreatesNoObject",
    "com.visualworkbench.android.EditorSurfaceInstrumentedTest#textSelectionMoveResizeEraserUndoAndActivityRecreationUseRealCore",
})
SURFACE_AWAIT_PHASES = frozenset({
    "startup", "new-project-and-background", "finish-owned-edits", "close-owned-project",
    "closed-for-reopen", "reopen-document", "save-text", "delete-selected", "undo-delete",
    "renderer-after-recreation",
}) | frozenset(f"{prefix}-{tool}" for prefix in ("input-ready", "gesture") for tool in (
    "Pen", "Highlighter", "Marker", "Line", "Arrow", "Rectangle", "Ellipse", "Select", "Text", "Eraser",
))
SURFACE_CLOSE_STAGES = frozenset({
    "Requested", "Queued", "Navigation", "CaptureStream", "Assisted", "MutationGate", "MutationAcquired",
    "ProjectClose", "ImageFrames", "Workflows", "Rotation", "DocumentOwner", "Giant", "Erasers", "Selections",
    "InstructionFocus", "Semantics", "Instructions", "Ai", "ChangeObserver", "RefreshObserver", "Link",
    "ProjectStorage", "ProjectsCatalog", "NavigationInstructionCleanup", "Finished",
})
SURFACE_CLOSE_STATUSES = frozenset({"Running", "Settled", "Failed", "Completed", "Declined"})
SYNC_AWAIT_CASE = "com.visualworkbench.android.EditorSyncInstrumentedTest#oldPageRefreshCannotAdoptOrRebindAfterSameRevisionNavigation"
SYNC_AWAIT_PHASES = frozenset({
    "InitialPdfPage", "OldReadEntered", "PageOneAdopted", "OldReadDelivered", "EraserEdit",
    "FixtureStartup", "FixtureCanvasReady", "FixtureCloseEntered", "FixtureCloseRetained",
})
SURFACE_NAVIGATION_MESSAGES = {
    "Close the current controls or finish their operation first.": "controls_blocked",
    "Save or discard the instruction draft before closing this project.": "instruction_unsaved",
    "Finish or cancel the image transfer before leaving this screen.": "transfer",
    "Finishing the current edit. Try returning to projects in a moment.": "pending_edit",
    "Save the paid Result or explicitly discard its review before replacing or closing this project.": "ai_decision",
    "Apply or explicitly discard the private tone changes before leaving this document.": "tone_changes",
    "Finish the layer edit or explicitly discard its retry handle before leaving this document.": "layer_edit",
    "Finish or cancel the document switch first.": "document_switch",
}
require = common.require
Rejected = common.Rejected


def read_text(path: Path, limit: int) -> str:
    common.plain(path)
    require(path.stat().st_size <= limit, "text_limit")
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    require(len(raw) <= limit and b"\0" not in raw, "text_limit")
    return raw.decode("utf-8-sig")


def cases(root: Path) -> list[str]:
    declared = common.bounded_json(root / "tools/app-test/inventory.json")
    expected = declared.get("expected_tests")
    require(declared.get("schema") == 1 and isinstance(expected, list) and
            1 <= len(expected) <= 1024 and all(isinstance(c, str) and CASE.fullmatch(c) for c in expected),
            "case_inventory")
    require(expected == sorted(set(expected)), "case_inventory_duplicate")
    actual = []
    for path in common.files_under(root / "apps/android/src/androidTest", 1024):
        require(path.suffix == ".kt", "unreviewed_test_source")
        text = read_text(path, 1024 * 1024)
        require(not re.search(r"@(?:Ignore|Parameterized)|\bAssume\.|\bassume(?:True|False|NoException)\s*\(", text),
                "test_skip_annotation")
        package = re.findall(r"(?m)^package ([A-Za-z0-9_.]+)\s*$", text)
        classes = re.findall(r"\bclass ([A-Za-z0-9_]+InstrumentedTest)\b", text)
        methods = re.findall(r"@Test\s+fun\s+([A-Za-z0-9_]+)\s*\(", text)
        require(len(package) == 1 and len(classes) == 1 and methods and text.count("@Test") == len(methods),
                "test_source_shape")
        actual.extend(f"{package[0]}.{classes[0]}#{m}" for m in methods)
    require(sorted(actual) == expected, "test_inventory_drift")
    return expected


def binding_inputs(root: Path, environment: dict | None = None) -> dict[str, Path]:
    """Only the canonical prebuilt inputs are accepted; CLI -P also pins them."""
    environment = os.environ if environment is None else environment
    inputs = {"vwNativeDir": root / "target/debug", "vwAndroidNativeDir": root / "target/android-jni"}
    for name, expected in inputs.items():
        override = environment.get("ORG_GRADLE_PROJECT_" + name)
        require(override is None or Path(override).absolute() == expected.absolute(), "native_override_refused")
    return inputs


def inventory(root: Path) -> dict[str, str]:
    binding_inputs(root)
    required = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.ps1", "tools/hil-test.ps1",
                "tools/process.psm1", "tools/android-device.psm1", "tools/mcp-build.ps1", "tools/check_npm_licenses.py",
                "mcp/package.json", "mcp/package-lock.json", "third_party/LICENSES", "apps/build.gradle.kts",
                "apps/settings.gradle.kts", "apps/gradle.properties", "apps/gradlew.bat", "apps/android/build.gradle.kts",
                "apps/android/gradle.lockfile", "apps/shared/build.gradle.kts", "apps/shared/gradle.lockfile",
                "target/android-jni/arm64-v8a/libvw_core.so", "target/debug/vw_core.dll", "target/debug/vw-bindgen.exe"]
    paths = {root / p for p in required}
    for tree in ("core/crates", "contracts", "apps/android/src", "apps/shared/src", "apps/gradle",
                 "apps/bindings-core", "tools/app-test", "tools/ffi-test", "third_party/notices", "tools/licenses"):
        for path in common.files_under(root / tree):
            if path.suffix not in (".pyc", ".log") and "build-hil" not in path.parts:
                paths.add(path)
    return {p.relative_to(root).as_posix(): common.digest(p) for p in sorted(paths)}


def begin(root: Path) -> dict:
    return {"schema": 1, "started_ns": time.time_ns(), "expected_tests": cases(root),
            "source_sha256": inventory(root)}


def unchanged(root: Path, state: dict) -> None:
    require(state.get("schema") == 1 and state.get("expected_tests") == cases(root), "state_inventory")
    require(state.get("source_sha256") == inventory(root), "source_drift")


def discover(root: Path, state: dict) -> dict:
    unchanged(root, state)
    result = {}
    base = root / "apps/android/build-hil/outputs/apk"
    for key, folder, package in (("main", "debug", MAIN), ("test", "androidTest/debug", TEST)):
        directory = common.plain(base / folder, True)
        value = common.bounded_json(directory / "output-metadata.json")
        require(value.get("applicationId") == package, "apk_output_package")
        elements = value.get("elements", [])
        require(len(elements) == 1 and not elements[0].get("filters"), "apk_split")
        name = elements[0].get("outputFile", "")
        require(re.fullmatch(r"[A-Za-z0-9_.-]+\.apk", name) is not None, "apk_output_name")
        path = common.plain(directory / name)
        require(path.stat().st_mtime_ns >= state["started_ns"], "stale_apk")
        result[key] = {"path": str(path), "sha256": common.digest(path, 2 * 1024**3)}
    return result


def manifest(text: str, test: bool) -> None:
    doc = common.xml_document(text.encode("utf-8"))
    ns = "{http://schemas.android.com/apk/res/android}"
    require(doc.tag == "manifest" and doc.get("package") == (TEST if test else MAIN), "manifest_package")
    require(doc.get(ns + "sharedUserId") is None, "shared_uid_refused")
    entries = doc.findall("instrumentation")
    if test:
        require(len(entries) == 1 and entries[0].get(ns + "name") == RUNNER and
                entries[0].get(ns + "targetPackage") == MAIN, "manifest_runner")
    else:
        require(not entries, "main_instrumentation")
        applications = doc.findall("application")
        require(len(applications) == 1, "manifest_application")
        app = applications[0]
        require(app.get(ns + "label") == "Visual Workbench Test" and
                app.get(ns + "debuggable") == "true" and app.get(ns + "allowBackup") == "false",
                "manifest_isolation")
        authorities = [p.get(ns + "authorities") for p in app.findall("provider")]
        require(MAIN + ".images" in authorities and all(a and a.startswith(MAIN + ".") for a in authorities),
                "provider_isolation")


def signer(text: str) -> str:
    require(len(text) <= 256 * 1024, "signer_output_limit")
    values = re.findall(r"(?m)^Signer #([0-9]+) certificate SHA-256 digest: ([0-9a-fA-F]{64})\s*$", text)
    require(len(values) == 1 and values[0][0] == "1", "signer_inventory")
    return values[0][1].lower()


def native_inventory(path: Path, required: bool) -> dict:
    if required:
        return check_apk.verify_apk(path)
    # The instrumentation APK normally has no JNI payload; validate every library
    # if AGP packages one without imposing the main APK's mandatory Rust/JNA pair.
    limits = check_apk.Limits()
    with common.plain(path).open("rb") as stream:
        before = os.fstat(stream.fileno())
        require(22 <= before.st_size <= limits.max_apk_bytes, "test_apk_size")
        entries, cd_at = check_apk.directory(stream, before.st_size, limits)
        positions = check_apk.local_ranges(stream, entries, cd_at, limits)
        natives = [e for e in entries if e.library]
        require(len(natives) <= limits.max_libraries, "test_native_count")
        libs = []
        for entry in natives:
            offset = positions[entry.name]
            loads = check_apk.elf_headers(stream, offset, entry.size, limits)
            sha, crc = check_apk.hash_region(stream, offset, entry.size)
            require(crc == entry.crc, "test_native_crc")
            libs.append({"entry": entry.name, "bytes": entry.size, "sha256": sha, "loads": loads,
                         "zip_data_offset": offset, "zip_alignment_bytes": 16384})
        sha, _ = check_apk.hash_region(stream, 0, before.st_size)
        after = os.fstat(stream.fileno())
        require((before.st_size, before.st_mtime_ns) == (after.st_size, after.st_mtime_ns), "apk_changed")
        return {"apk_sha256": sha, "libraries": libs, "runtime_16k_verified": False}


def staged_paths(paths: dict, sources: dict, private: Path) -> None:
    require(set(paths) == set(sources) == {"main", "test"}, "apk_inventory")
    common.plain(private, True)
    for key in ("main", "test"):
        require(set(paths[key]) == {"path", "sha256"} and set(sources[key]) == {"path", "sha256"}, "apk_inventory")
        expected = private / (key + ".apk")
        require(Path(paths[key]["path"]) == expected and re.fullmatch(r"[0-9a-f]{64}", paths[key]["sha256"]) is not None,
                "staged_apk_path")
        require(paths[key]["sha256"] == sources[key]["sha256"] == common.digest(common.plain(expected), 2 * 1024**3),
                "staged_apk_changed")


def verify(root: Path, state: dict, paths: dict, private: Path) -> dict:
    unchanged(root, state)
    staged_paths(paths, common.bounded_json(private / "source-apks.json"), private)
    reports = {}
    for key in ("main", "test"):
        manifest(read_text(private / (key + ".xml"), 1024**2), key == "test")
        cert = signer(read_text(private / (key + ".cert.txt"), 256 * 1024))
        native = native_inventory(Path(paths[key]["path"]), key == "main")
        require(native["apk_sha256"] == paths[key]["sha256"], "apk_changed")
        reports[key] = {"package": MAIN if key == "main" else TEST, "sha256": paths[key]["sha256"],
                        "signer_sha256": cert, "native": native}
    require(reports["main"]["signer_sha256"] == reports["test"]["signer_sha256"], "apk_signer_mismatch")
    rust = next(p for p in reports["main"]["native"]["libraries"] if p["entry"] == "lib/arm64-v8a/libvw_core.so")
    require(rust["sha256"] == state["source_sha256"]["target/android-jni/arm64-v8a/libvw_core.so"],
            "native_source_binding")
    return reports


def instrumentation(text: str, expected: list[str]) -> dict:
    require(len(text.encode("utf-8")) <= MAX_OUTPUT and all(len(line) <= 65536 for line in text.splitlines()),
            "instrumentation_output_limit")
    require(expected and len(expected) == len(set(expected)), "case_inventory")
    wanted = set(expected)
    completed = []
    active = None
    fields = {}
    final = []
    summaries = []
    in_result = False
    last_field = None
    for line in text.splitlines():
        if line.startswith("INSTRUMENTATION_STATUS: "):
            require(not in_result, "status_after_result")
            item = line[len("INSTRUMENTATION_STATUS: "):]
            require("=" in item, "status_field")
            key, value = item.split("=", 1)
            require(key not in fields, "duplicate_status_field")
            fields[key] = value
            last_field = key
        elif line.startswith("INSTRUMENTATION_STATUS_CODE: "):
            require(not in_result, "status_after_result")
            code = line[len("INSTRUMENTATION_STATUS_CODE: "):]
            require(code in ("0", "1"), "test_failed_skipped_or_ignored")
            require(set(fields) <= {"class", "test", "current", "numtests", "id", "stream"} and
                    all(k in fields for k in ("class", "test", "current", "numtests")), "status_fields")
            case = fields["class"] + "#" + fields["test"]
            require(case in wanted and fields["numtests"] == str(len(expected)) and
                    fields["current"] == str(len(completed) + 1), "test_identity_or_count")
            if code == "1":
                require(active is None and case not in completed, "duplicate_test_start")
                active = case
            else:
                require(active == case, "missing_test_start")
                completed.append(case)
                active = None
            fields = {}
            last_field = None
        elif line.startswith("INSTRUMENTATION_RESULT: "):
            require(not fields and active is None and not final, "premature_result")
            require(line.startswith("INSTRUMENTATION_RESULT: stream="), "runner_result_error")
            require(not in_result, "duplicate_result")
            in_result = True
        elif line.startswith("INSTRUMENTATION_CODE: "):
            require(in_result and not final, "final_code_order")
            final.append(line[len("INSTRUMENTATION_CODE: "):])
        elif re.fullmatch(r"OK \([0-9]+ tests?\)", line.strip()):
            require(in_result and not final, "summary_outside_result")
            summaries.append(int(re.search(r"[0-9]+", line).group()))
        elif line.startswith("INSTRUMENTATION_") or "FAILURES!!!" in line:
            raise Rejected("runner_error")
        else:
            # Only the documented stream values may have continuation lines.
            # Ignore their prose, never use a printed PASS as test authority.
            require(not line.strip() or last_field in ("stream", "stack") or in_result and not final,
                    "unexpected_instrumentation_output")
    require(not fields and active is None and set(completed) == wanted and len(completed) == len(expected),
            "incomplete_test_run")
    require(final == ["-1"] and summaries == [len(expected)], "instrumentation_final")
    return {"actual_passed_tests": len(completed), "cases": sorted(completed), "skipped": 0,
            "instrumentation_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest()}


def surface_timeout_state(header: str) -> dict | None:
    # The optional close snapshot is captured once on Main by the fixture.
    # Split only its exact ordered suffix; never absorb malformed close fields
    # into the private blocked-message tail or infer values from suppressed text.
    if len(header) > 2560:
        return None
    close = None
    if " close_" in header:
        keys = (" close_attempt=", " close_stage=", " close_status=", " close_failed_stage=")
        if header.count(" close_") != 4 or any(header.count(key) != 1 for key in keys):
            return None
        suffix = re.fullmatch(r"([^\r\n]+) close_attempt=(0|[1-9][0-9]{0,9}) close_stage=([A-Za-z]{1,32}) close_status=([A-Za-z]{1,32}) close_failed_stage=([A-Za-z]{1,32})", header)
        if not suffix:
            return None
        header, attempt, stage, status, failed = suffix.groups()
        attempt = int(attempt)
        if attempt == 0:
            if (stage, status, failed) != ("None", "None", "None"):
                return None
        elif not (attempt <= 2147483647 and stage in SURFACE_CLOSE_STAGES and
                  status in SURFACE_CLOSE_STATUSES and (failed == "None" or failed in SURFACE_CLOSE_STAGES)):
            return None
        close = {"attempt": attempt, "stage": stage, "status": status, "failed_stage": failed}
    # The unescaped fixture fields must each occur exactly once. Ambiguous,
    # oversized or malformed details contribute no flags and no private text.
    if len(header) > 2304 or any(header.count(token) != 1 for token in (" pending=", " message=", " blocked=")):
        return None
    match = re.fullmatch(r"INSTRUMENTATION_STATUS: stack=java\.lang\.AssertionError: Timed out in [A-Za-z0-9-]{1,64}: busy=(true|false) pending=(0|-?[1-9][0-9]{0,9}) message=([^\r\n]{1,1024}) blocked=([^\r\n]{1,1024})", header)
    if not match or not -2147483648 <= int(match[2]) <= 2147483647:
        return None
    result = {"busy": match[1] == "true", "pending_nonzero": match[2] != "0",
              "message_kind": "none" if match[3] == "null" else SURFACE_NAVIGATION_MESSAGES.get(match[3], "unknown"),
              "blocked_present": match[4] != "null"}
    if close is not None:
        result["close_progress"] = close
    return result


def failure_observation(text: str, expected: list[str]) -> dict:
    """Whitelisted diagnostics even when no passing result can be established."""
    require(len(text.encode("utf-8")) <= MAX_OUTPUT, "instrumentation_output_limit")
    known = set(expected)
    fields = {}
    observed = []
    source_frames = []
    exception_kind = None
    surface_phase = None
    surface_state = None
    stack_headers = 0
    sync_phase = None
    for line in text.splitlines():
        # Retain only an exact known test class's Kotlin line numbers, never
        # exception messages, assertion values, local paths or arbitrary frames.
        frame = re.fullmatch(r"\s*at (com\.visualworkbench\.[A-Za-z0-9_.$]+)\.[A-Za-z0-9_$<>]+\(([A-Za-z0-9_]+)\.kt:([0-9]{1,6})\)", line)
        if frame and len(source_frames) < 16:
            source_frames.append((frame[1].split("$")[0], frame[2], int(frame[3])))
        kind = re.match(r"INSTRUMENTATION_STATUS: stack=(java\.lang\.(?:AssertionError|IllegalArgumentException|IllegalStateException)|org\.junit\.ComparisonFailure|java\.util\.concurrent\.CancellationException|kotlinx\.coroutines\.(?:TimeoutCancellationException|JobCancellationException)|com\.visualworkbench\.android\.editor\.GiantAndroidCreationFailure)(?::|$)", line)
        if kind:
            exception_kind = kind[1]
        # A sole primary header may contribute a fixed phase and bounded flags.
        # Suppressed frames cannot supply it; duplicate headers omit all derived
        # Surface fields rather than selecting one potentially ambiguous header.
        if line.startswith("INSTRUMENTATION_STATUS: stack="):
            stack_headers += 1
            surface_phase = None
            surface_state = None
            surface_timeout = re.match(r"INSTRUMENTATION_STATUS: stack=java\.lang\.AssertionError: Timed out in ([A-Za-z0-9-]{1,64}):", line)
            if stack_headers == 1 and surface_timeout and surface_timeout[1] in SURFACE_AWAIT_PHASES:
                surface_phase = surface_timeout[1]
                surface_state = surface_timeout_state(line)
        sync_timeout = re.fullmatch(r"\s*Suppressed: com\.visualworkbench\.android\.EditorSyncInstrumentedTest\$TimeoutPhase: VW_SYNC_AWAIT_PHASE=([A-Za-z]{1,32})", line)
        if sync_phase is None and sync_timeout and sync_timeout[1] in SYNC_AWAIT_PHASES:
            sync_phase = sync_timeout[1]
        if line.startswith("INSTRUMENTATION_STATUS: ") and "=" in line:
            key, value = line[len("INSTRUMENTATION_STATUS: "):].split("=", 1)
            if key in ("class", "test"):
                fields[key] = value
        elif line.startswith("INSTRUMENTATION_STATUS_CODE: "):
            identity = fields.get("class", "") + "#" + fields.get("test", "")
            code = line[len("INSTRUMENTATION_STATUS_CODE: "):]
            if identity in known and code in ("-4", "-3", "-2", "-1", "0", "1") and len(observed) < 2048:
                observation = {"case": identity, "status_code": int(code)}
                if int(code) < 0:
                    cls = fields["class"]
                    lines = sorted({n for owner, stem, n in source_frames
                                    if owner == cls and stem == cls.rsplit(".", 1)[-1]})
                    if lines:
                        observation["test_source_lines"] = lines
                    if exception_kind:
                        observation["exception_kind"] = exception_kind
                    if identity in SURFACE_AWAIT_CASES and exception_kind == "java.lang.AssertionError" and surface_phase:
                        observation["await_phase"] = surface_phase
                        if surface_state is not None:
                            observation["surface_timeout_state"] = surface_state
                    if identity == SYNC_AWAIT_CASE and exception_kind == "kotlinx.coroutines.TimeoutCancellationException" and sync_phase:
                        observation["await_phase"] = sync_phase
                observed.append(observation)
            fields = {}
            source_frames = []
            exception_kind = None
            surface_phase = None
            surface_state = None
            stack_headers = 0
            sync_phase = None
    return {"raw_output_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
            "observed_statuses": observed, "assertions_passed": False}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("begin", "discover", "verify", "collect", "diagnose"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--private", type=Path)
    args = parser.parse_args()
    try:
        root = common.plain(args.root.absolute(), True)
        if args.command == "begin":
            common.write_json(args.state, begin(root))
        else:
            require(args.output is not None, "output_argument")
            state = common.bounded_json(args.state)
            if args.command == "diagnose":
                require(args.private is not None, "private_argument")
                report = failure_observation(read_text(args.private / "instrumentation.txt", MAX_OUTPUT), state["expected_tests"])
            elif args.command == "discover":
                report = discover(root, state)
            else:
                require(args.private is not None, "private_argument")
                paths = common.bounded_json(args.private / "apks.json")
                reports = verify(root, state, paths, args.private)
                report = {"artifacts": reports}
                if args.command == "collect":
                    parsed = instrumentation(read_text(args.private / "instrumentation.txt", MAX_OUTPUT), state["expected_tests"])
                    report.update({"schema": 1, "assertions_passed": True, "tests": parsed,
                                   "source_sha256": state["source_sha256"], "isolated_application": True,
                                   "normal_app_tested": False, "runtime_16k_verified": False})
            common.write_json(args.output, report)
        print("APP_HIL_REPORT_OK")
        return 0
    except (Rejected, check_apk.Rejected) as failure:
        print("APP_HIL_REPORT_REJECTED:" + str(failure), file=sys.stderr)
    except Exception:
        print("APP_HIL_REPORT_REJECTED:invalid_input", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
