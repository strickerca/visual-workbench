"""Strict, path-redacting evidence checks for the shared JVM/Android FFI run.

Only the parent validation lane invokes this tool. No adb, Gradle or native code
is executed here. A passing XML report proves its assertions ran, not S Pen,
performance, production pairing, or provenance of an earlier compiler run.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import sys
import time
import xml.etree.ElementTree as ET
import zipfile

PACKAGE = "com.visualworkbench.shared.test"
RUNNER = "androidx.test.runner.AndroidJUnitRunner"
EXPECTED = {
    "desktop": ("com.visualworkbench.shared.DesktopCoreSmokeTest", "rustGoldenAndBoundary[desktop]"),
    "android": ("com.visualworkbench.shared.AndroidCoreSmokeTest", "rustGoldenAndBoundary"),
}
REPORT_ROOTS = {
    "desktop": "apps/shared/build/test-results/desktopTest",
    "android": "apps/shared/build/outputs/androidTest-results/connected",
}
REQUIRED = (
    "Cargo.lock", "Cargo.toml", "rust-toolchain.toml", "apps/shared/build.gradle.kts",
    "apps/shared/gradle.lockfile", "build.ps1", "tools/hil-test.ps1",
    "apps/build.gradle.kts", "apps/settings.gradle.kts", "apps/gradle.properties",
    "apps/bindings-core/build.gradle.kts", "apps/bindings-host/build.gradle.kts",
    "apps/gradle/libs.versions.toml", "apps/gradle/wrapper/gradle-wrapper.properties",
    "tools/process.psm1", "tools/android-device.psm1", "tools/ffi-test/check_apk.py",
    "tools/mcp-build.ps1", "tools/check_npm_licenses.py", "mcp/package.json", "mcp/package-lock.json", "third_party/LICENSES",
    "tools/ffi-test/fixtures/ffi-golden.properties", "tools/ffi-test/fixtures/ffi-golden.json",
    "target/debug/vw_core.dll", "target/debug/vw_host.dll", "target/debug/vw-bindgen.exe",
    "target/android-jni/arm64-v8a/libvw_core.so",
)
TREES = (
    "core/crates", "host-win/crates", "contracts", "apps/shared/src", "tools/ffi-test",
    # Include module lockfiles as well as their required build scripts. A lock
    # appearing, disappearing or changing after begin also changes the inventory.
    "apps/bindings-core", "apps/bindings-host", "tools/licenses", "third_party/notices",
)
HASH = re.compile(r"[0-9a-f]{64}\Z")


class Rejected(ValueError):
    """Static refusal code, never an OS exception, XML value, serial or path."""


def require(ok: bool, code: str) -> None:
    if not ok:
        raise Rejected(code)


def plain(path: Path, directory: bool = False) -> Path:
    path = path.absolute()
    for item in (path, *path.parents):
        info = item.lstat()
        require(not stat.S_ISLNK(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
                "redirected_path")
    require(path.is_dir() if directory else path.is_file(), "path_type")
    return path


def digest(path: Path, limit: int = 512 * 1024**2) -> str:
    plain(path)
    before = path.stat()
    require(0 <= before.st_size <= limit, "file_size")
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        total = 0
        while chunk := stream.read(1024**2):
            total += len(chunk)
            require(total <= limit, "file_size")
            hasher.update(chunk)
    after = path.stat()
    require(total == before.st_size == after.st_size and before.st_mtime_ns == after.st_mtime_ns,
            "file_changed_during_read")
    return hasher.hexdigest()


def files_under(root: Path, maximum: int = 10000) -> list[Path]:
    if not root.exists():
        return []
    plain(root, True)
    result: list[Path] = []
    visited = 0
    for parent, directories, files in os.walk(root, followlinks=False):
        directories[:] = [name for name in directories if name not in ("__pycache__", "build", "target", ".gradle", ".git")]
        for name in directories:
            plain(Path(parent) / name, True)
        visited += len(directories) + len(files)
        require(visited <= maximum, "inventory_limit")
        for name in files:
            path = Path(parent) / name
            plain(path)
            result.append(path)
    return sorted(result)


def bounded_json(path: Path) -> dict:
    plain(path)
    require(path.stat().st_size <= 4 * 1024**2, "json_size")
    def unique(pairs):
        out = {}
        for key, value in pairs:
            require(key not in out, "duplicate_json_key")
            out[key] = value
        return out
    value = json.loads(path.read_text(encoding="utf-8-sig"), object_pairs_hook=unique)
    require(isinstance(value, dict), "json_object")
    return value


def write_json(path: Path, value: dict, *, replace: bool = False) -> None:
    # Caller supplies one owned private directory, never a project data path.
    plain(path.parent, True)
    if replace:
        plain(path)
    with path.open("w" if replace else "x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def source_inventory(root: Path) -> dict[str, str]:
    plain(root, True)
    paths = {root / relative for relative in REQUIRED}
    for tree in TREES:
        paths.update(p for p in files_under(root / tree) if p.suffix not in (".pyc", ".log"))
    for path in files_under(root / "apps/gradle"):
        if path.suffix in (".toml", ".properties", ".lockfile"):
            paths.add(path)
    return {path.relative_to(root).as_posix(): digest(path) for path in sorted(paths)}


def report_inventory(root: Path) -> dict[str, dict]:
    result = {}
    for path in files_under(root, 4096):
        if path.name.startswith("TEST-") and path.suffix == ".xml":
            require(len(result) < 64, "report_count")
            result[path.relative_to(root).as_posix()] = {
                "sha256": digest(path, 1024**2), "mtime_ns": path.stat().st_mtime_ns,
            }
    return result


def xml_document(data: bytes) -> ET.Element:
    require(0 < len(data) <= 1024**2, "xml_size")
    # Also reject UTF-16/32, where an ASCII DTD check would be bypassed.
    require(b"\x00" not in data and b"<!DOCTYPE" not in data.upper() and b"<!ENTITY" not in data.upper(), "xml_declaration")
    element = ET.fromstring(data)
    nodes = [(element, 0)]
    count = 0
    while nodes:
        node, depth = nodes.pop()
        count += 1
        require(count <= 20000 and depth <= 24, "xml_complexity")
        require(all(len(k) <= 256 and len(v) <= 8192 for k, v in node.attrib.items()), "xml_attribute")
        nodes.extend((child, depth + 1) for child in node)
    return element


def count_attribute(node: ET.Element, name: str, default: int | None = None) -> int:
    value = node.get(name)
    if value is None and default is not None:
        return default
    require(isinstance(value, str) and re.fullmatch(r"0|[1-9][0-9]{0,5}", value) is not None, "xml_counter")
    return int(value)


def parse_junit(data: bytes) -> list[tuple[str, str]]:
    root = xml_document(data)
    require(root.tag in ("testsuite", "testsuites"), "xml_root")
    suites = [root] if root.tag == "testsuite" else list(root)
    require(bool(suites) and all(s.tag == "testsuite" for s in suites), "xml_suites")
    passed = []
    for suite in suites:
        require(not suite.findall("testsuite"), "nested_suite")
        cases = suite.findall("testcase")
        require(count_attribute(suite, "tests") == len(cases), "test_count_mismatch")
        require(count_attribute(suite, "failures", 0) == 0 and count_attribute(suite, "errors", 0) == 0,
                "reported_failure")
        require(count_attribute(suite, "skipped", 0) == 0 and count_attribute(suite, "disabled", 0) == 0,
                "reported_skip")
        for case in cases:
            require(not any(case.find(tag) is not None for tag in ("failure", "error", "skipped")), "case_not_passed")
            require(case.get("status", "run") == "run" and case.get("result", "completed") == "completed",
                    "case_not_run")
            identity = (case.get("classname", ""), case.get("name", ""))
            require(all(identity), "case_identity")
            require(identity not in passed, "duplicate_case")
            passed.append(identity)
    return passed


def fresh_results(root: Path, before: dict, started_ns: int, expected: tuple[str, str],
                  required_cases: set[tuple[str, str]] | None = None) -> dict:
    current = report_inventory(root)
    all_passed: set[tuple[str, str]] = set()
    hashes = []
    for relative, record in current.items():
        if before.get(relative) == record:
            continue
        require(record["mtime_ns"] >= started_ns, "stale_report")
        path = root / relative
        passed = parse_junit(path.read_bytes())
        require(not all_passed.intersection(passed), "duplicate_case")
        all_passed.update(passed)
        hashes.append(record["sha256"])
    require(expected in all_passed, "required_golden_not_passed")
    if required_cases is not None:
        require(all_passed == required_cases, "required_case_inventory")
    require(bool(all_passed), "zero_actual_tests")
    # Never copy suite properties, filenames, output, stack traces or device IDs.
    return {"actual_passed_tests": len(all_passed), "required_golden": ".".join(expected),
            "report_sha256": sorted(hashes), "fresh_reports": len(hashes)}


def begin(root: Path) -> dict:
    return {"schema": 1, "source_sha256": source_inventory(root), "phases": {}}


def mark(root: Path, state: dict, phase: str) -> dict:
    require(phase in EXPECTED and phase not in state["phases"], "phase_duplicate")
    require(source_inventory(root) == state["source_sha256"], "source_drift")
    state["phases"][phase] = {"before": report_inventory(root / REPORT_ROOTS[phase]), "started_ns": time.time_ns()}
    return state


def collect(root: Path, state: dict) -> dict:
    require(state.get("schema") == 1 and set(state.get("phases", {})) == set(EXPECTED), "missing_phase")
    require(source_inventory(root) == state["source_sha256"], "source_drift")
    android_cases = android_inventory(root)
    result = {phase: fresh_results(root / REPORT_ROOTS[phase], state["phases"][phase]["before"],
              state["phases"][phase]["started_ns"], expected,
              android_cases if phase == "android" else None) for phase, expected in EXPECTED.items()}
    properties = (root / "tools/ffi-test/fixtures/ffi-golden.properties").read_text(encoding="ascii")
    fixture = {}
    for key in ("state_hash", "export_blake3"):
        values = re.findall(r"^" + key + r"=([0-9a-f]{64})\s*$", properties, re.MULTILINE)
        require(len(values) == 1, "fixture_hash")
        fixture[key] = values[0]
    return {"schema": 1, "assertions_passed": True, "model": "IN2019", "platforms": result,
            "golden": fixture, "source_sha256": state["source_sha256"],
            "claim": "Both fresh Kotlin wrappers passed the same Rust-generated state/PNG golden and boundary assertions.",
            "limits": ["No S23/S Pen or target performance acceptance", "No UI foreground or provider acceptance",
                       "Source and binary hashes were co-observed; this is not a compiler provenance receipt",
                       "Pass requires the runner's separate process, package and temporary-work cleanup checks"]}


def android_inventory(root: Path) -> set[tuple[str, str]]:
    value = bounded_json(root / "tools/ffi-test/android-inventory.json")
    expected = value.get("expected_tests")
    require(value.get("schema") == 1 and isinstance(expected, list) and 1 <= len(expected) <= 1024,
            "android_case_inventory")
    require(all(isinstance(row, list) and len(row) == 2 and all(isinstance(part, str) for part in row) and
                re.fullmatch(r"com\.visualworkbench\.shared\.[A-Za-z][A-Za-z0-9_]*Test", row[0]) and
                re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", row[1]) for row in expected), "android_case_identity")
    require(expected == sorted(map(list, set(map(tuple, expected)))), "android_case_duplicate")
    actual = set()
    # Match all three explicit device source roots in shared/build.gradle.kts.
    # A missing, added or duplicated case must not silently pass the receipt.
    for source_set in ("androidDeviceTest", "jvmTest", "commonTest"):
        for path in files_under(root / "apps/shared/src" / source_set, 1024):
            require(path.suffix == ".kt", "android_test_source")
            text = path.read_text(encoding="utf-8-sig")
            owners = re.findall(r"\bclass ([A-Za-z0-9_]+Test)\b", text)
            methods = re.findall(r"@Test\s+(?:public\s+)?fun\s+([A-Za-z0-9_]+)\s*\(", text)
            if not owners and "@Test" not in text:
                continue  # Shared fixture helpers have no test declarations.
            require(len(owners) == 1 and methods and text.count("@Test") == len(methods) and
                    re.search(r"(?m)^package com\.visualworkbench\.shared\s*$", text) is not None and
                    not re.search(r"@(?:Ignore|Parameterized)|\bAssume\.", text), "android_test_declaration")
            cases = [("com.visualworkbench.shared." + owners[0], method) for method in methods]
            require(len(set(cases)) == len(cases) and not actual.intersection(cases), "android_test_duplicate")
            actual.update(cases)
    require(actual == set(map(tuple, expected)), "android_test_inventory_drift")
    return actual


def apk_info(root: Path, since_ns: int) -> dict:
    outputs = root / "apps/shared/build/outputs/apk"
    candidates = []
    for metadata in files_under(outputs, 1024):
        if metadata.name != "output-metadata.json" or metadata.stat().st_mtime_ns < since_ns:
            continue
        value = bounded_json(metadata)
        if value.get("applicationId") != PACKAGE:
            continue
        elements = value.get("elements", [])
        require(len(elements) == 1 and not elements[0].get("filters"), "split_test_apk")
        filename = elements[0].get("outputFile", "")
        require(re.fullmatch(r"[A-Za-z0-9_.-]+\.apk", filename) is not None, "apk_name")
        apk = plain(metadata.parent / filename)
        require(apk.stat().st_mtime_ns >= since_ns, "stale_test_apk")
        candidates.append(apk)
    require(len(candidates) == 1, "test_apk_inventory")
    # Library instrumentation must be self-targeting: no starter or other app.
    manifests = []
    namespace = "{http://schemas.android.com/apk/res/android}"
    for path in files_under(root / "apps/shared/build/intermediates", 20000):
        if path.name != "AndroidManifest.xml" or path.stat().st_size > 1024**2:
            continue
        data = path.read_bytes()
        if not data.removeprefix(b"\xef\xbb\xbf").lstrip().startswith(b"<"):
            continue
        doc = xml_document(data)
        if doc.tag != "manifest" or doc.get("package") != PACKAGE:
            continue
        instrumentation = doc.findall("instrumentation")
        require(len(instrumentation) == 1, "instrumentation_count")
        require(instrumentation[0].get(namespace + "targetPackage") == PACKAGE and
                instrumentation[0].get(namespace + "name") == RUNNER, "instrumentation_target")
        manifests.append(path)
    require(bool(manifests), "self_target_manifest_missing")
    return {"path": str(candidates[0]), "package": PACKAGE, "sha256": digest(candidates[0], 2 * 1024**3)}


def bind_apk(root: Path, apk: Path) -> dict:
    # Invoke only after check_apk.py's bounded ZIP32/ELF validation passes.
    bindings = {"lib/arm64-v8a/libvw_core.so": root / "target/android-jni/arm64-v8a/libvw_core.so",
                "ffi-golden.properties": root / "tools/ffi-test/fixtures/ffi-golden.properties"}
    with plain(apk).open("rb") as stream:
        before = os.fstat(stream.fileno())
        # Bound ZipFile's central-directory allocation independently of the
        # preceding process: the artifact may have changed between invocations.
        require(22 <= before.st_size <= 2 * 1024**3, "apk_size")
        tail_size = min(before.st_size, 65535 + 22)
        stream.seek(before.st_size - tail_size)
        tail = stream.read(tail_size)
        offset = tail.rfind(b"PK\x05\x06")
        require(offset >= 0 and offset + 22 <= len(tail), "apk_end_record")
        _, disk, cd_disk, count_disk, count, cd_size, cd_at, comment = struct.unpack_from("<4s4H2IH", tail, offset)
        require(offset + 22 + comment == len(tail), "apk_end_record")
        require(disk == cd_disk == 0 and count_disk == count and count < 65535,
                "apk_zip32_required")
        require(count * 46 <= cd_size <= 32 * 1024**2 and cd_at + cd_size == before.st_size - tail_size + offset,
                "apk_directory_bound")
        stream.seek(0)
        with zipfile.ZipFile(stream) as archive:
            bind_members(archive, bindings)
        stream.seek(0)
        apk_hash = hashlib.sha256()
        while chunk := stream.read(1024**2):
            apk_hash.update(chunk)
        after = os.fstat(stream.fileno())
        require((before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                (after.st_size, after.st_mtime_ns, after.st_ctime_ns), "apk_changed_during_read")
    require(digest(apk, 2 * 1024**3) == apk_hash.hexdigest(), "apk_changed_during_read")
    return {"apk_sha256": apk_hash.hexdigest(), "native_and_golden_match": True}


def bind_members(archive: zipfile.ZipFile, bindings: dict[str, Path]) -> None:
    names = archive.namelist()
    require(len(names) == len(set(names)) and len(names) < 65535, "apk_duplicate_entries")
    for member, source in bindings.items():
        info = archive.getinfo(member)
        require(info.file_size == source.stat().st_size and info.file_size <= 512 * 1024**2, "apk_binding_size")
        with archive.open(info) as stream:
            hasher = hashlib.sha256()
            total = 0
            while chunk := stream.read(1024**2):
                total += len(chunk)
                require(total <= info.file_size, "apk_binding_size")
                hasher.update(chunk)
        require(total == info.file_size and hasher.hexdigest() == digest(source), "apk_binding_mismatch")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("begin", "mark", "collect", "apk-info", "apk-bind"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--state", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--phase", choices=tuple(EXPECTED))
    parser.add_argument("--since-ns", type=int)
    parser.add_argument("--apk", type=Path)
    args = parser.parse_args()
    try:
        root = plain(args.root.absolute(), True)
        if args.command == "begin":
            require(args.state is not None, "state_argument")
            write_json(args.state, begin(root))
        elif args.command == "mark":
            require(args.state is not None and args.phase is not None, "phase_argument")
            write_json(args.state, mark(root, bounded_json(args.state), args.phase), replace=True)
        elif args.command == "collect":
            require(args.state is not None and args.output is not None, "collect_argument")
            write_json(args.output, collect(root, bounded_json(args.state)))
        elif args.command == "apk-info":
            require(args.since_ns is not None and args.output is not None, "apk_argument")
            write_json(args.output, apk_info(root, args.since_ns))
        else:
            require(args.apk is not None and args.output is not None, "apk_argument")
            write_json(args.output, bind_apk(root, args.apk))
        print("SHARED_FFI_REPORT_OK")
        return 0
    except Rejected as failure:
        print("SHARED_FFI_REPORT_REJECTED:" + str(failure), file=sys.stderr)
        return 1
    except (OSError, ValueError, KeyError, TypeError, ET.ParseError, zipfile.BadZipFile):
        print("SHARED_FFI_REPORT_REJECTED", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
