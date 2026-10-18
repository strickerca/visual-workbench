"""Source checks only. Runs no compiler, Kotlin test, native code, GUI or device."""
from pathlib import Path
import datetime
import hashlib
import json
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
BASE = "f661eca9d2f6d6d6ebe4fc31ed2d0a5c6ec095e0"
SOURCE = "490ed8ba471fe13391c542fa389a7261f6d21af3"
GRANT = "30b77615fe0ce79b2b1a044a0704ae2c7d186c14"
OUTPUT = ROOT / "evidence/T3_01_DESKTOP_CALLER_SOURCE_20261018.json"
MAIN = "apps/desktop/src/main/kotlin/com/visualworkbench/desktop/Main.kt"
CAPTURE = "apps/desktop/src/main/kotlin/com/visualworkbench/desktop/capture/"
TESTS = "apps/desktop/src/test/kotlin/com/visualworkbench/desktop/capture/"
SHARED = "apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/CaptureRegionEditor.kt"
SHARED_TEST = "apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/CaptureRegionEditorTest.kt"


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def old(commit, path):
    return git("show", f"{commit}:{path}").decode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def public_api(text):
    return [line.strip().split("{")[0].split(" = ")[0].strip()
            for line in text.splitlines()
            if line.strip().startswith(("public class ", "public fun ", "public val "))]


def lexical_delimiters(text):
    # Deliberately NOT a Kotlin parser or PSI. Ignores quoted strings/comments;
    # it does not parse interpolation, types, overloads or Compose semantics.
    stack, index, quote, comment = [], 0, None, 0
    while index < len(text):
        char, pair = text[index], text[index:index + 2]
        if quote:
            if char == "\\":
                index += 2
                continue
            if char == quote:
                quote = None
            index += 1
            continue
        if comment:
            if pair == "/*":
                comment += 1
                index += 2
                continue
            if pair == "*/":
                comment -= 1
                index += 2
                continue
            index += 1
            continue
        if pair == "//":
            end = text.find("\n", index)
            index = len(text) if end == -1 else end + 1
            continue
        if pair == "/*":
            comment = 1
            index += 2
            continue
        if char in ("\"", "'"):
            quote = char
            index += 1
            continue
        if char in "({[":
            stack.append(char)
        if char in ")}]":
            assert stack and "({["[")}]".index(char)] == stack.pop(), index
        index += 1
    assert not stack and not quote and not comment


subprocess.run(["git", "merge-base", "--is-ancestor", BASE, SOURCE], cwd=ROOT, check=True)
paths = git("diff", "--name-only", BASE, SOURCE).decode().splitlines()
assert len(paths) == 15 and all(
    path in (MAIN, SHARED, SHARED_TEST) or path.startswith((CAPTURE, TESTS)) for path in paths
)
hashes = {}
inventory = {}
for path in paths:
    data = (ROOT / path).read_bytes()
    assert data == git("show", f"{SOURCE}:{path}"), f"Unreviewed source change: {path}"
    text = data.decode()
    assert text.endswith("\n") and "\r" not in text
    assert all(line == line.rstrip() for line in text.splitlines())
    lexical_delimiters(text)
    hashes[path] = digest(data)
    names = re.findall(r"@Test\s+fun\s+(\w+)", text)
    if names:
        assert len(names) == len(set(names))
        baseline = []
        exists = subprocess.run(["git", "cat-file", "-e", f"{BASE}:{path}"], cwd=ROOT,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if exists.returncode == 0:
            baseline = re.findall(r"@Test\s+fun\s+(\w+)", old(BASE, path))
        assert set(baseline) <= set(names)
        inventory[path] = {"baseline_written": len(baseline), "current_written": len(names),
                           "new_written": len(names) - len(baseline), "names": names, "executed": False}

shared = (ROOT / SHARED).read_text()
assert public_api(shared) == public_api(old(BASE, SHARED))
assert not any(value in shared for value in ("java.", "javax.", "android."))
main = (ROOT / MAIN).read_text()
mark = "@Composable private fun EditorShell"
renderer = main[main.index(mark):]
assert renderer == old(BASE, MAIN)[old(BASE, MAIN).index(mark):]
assert main.count("openDesktopCaptureService(") == 2
assert "rememberUpdatedState(::localCaptureAdmission)" in main
assert "rememberUpdatedState(::pickerAdmission)" in main
assert "lifetime.owner?.state?.value" in main
quit_body = main[main.index("        fun quit()"):main.index("        LaunchedEffect(appOwner)")]
assert quit_body.index("diagnosticsLifecycle.closeParentsAfterRetirement") < quit_body.index("withRetiredCaptureCallers")
assert quit_body.index("withRetiredCaptureCallers") < quit_body.index("controller.close()") < quit_body.index("nativeRuntime.close()")
assert "capturePicker.shutdown();true" not in quit_body
assert "capturePickerState.cleanupPending,onClick=::retryPickerCleanup" in main
assert "McpVerificationTargets" not in main

# These untouched files bind CaptureDelivery's narrow exception interpretation
# to the actual destination and NativeProject close implementation at the freeze.
bound_paths = [
    "apps/desktop/src/main/kotlin/com/visualworkbench/desktop/EditorController.kt",
    "apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeCore.kt",
    "apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeOwnership.kt",
    "apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeGiantImages.kt",
    "apps/desktop/src/main/kotlin/com/visualworkbench/desktop/DesktopDiagnosticsLifecycle.kt",
]
bound_hashes = {}
for path in bound_paths:
    data = (ROOT / path).read_bytes()
    assert data == git("show", f"{BASE}:{path}")
    bound_hashes[path] = digest(data)

# Independent fixture arithmetic only: does not execute CaptureRegionEditor,
# its native mapper, Compose pointer conversion or Windows capture geometry.
fixtures = []
for density in (1.0, 1.25, 1.5, 2.0, 3.0):
    scale = 2 * density
    got = [(40 * density - 120 * density) / scale + 50,
           (40 * density - 80 * density) / scale + 30,
           (100 * density - 120 * density) / scale + 50,
           (100 * density - 80 * density) / scale + 30]
    assert got == [10, 10, 40, 40]
    fixtures.append({"density": density, "expected_edges": got})
assert [(100.5 - 120) / 2 + 50, (100.25 - 80) / 2 + 30,
        (40.25 - 120) / 2 + 50, (40.5 - 80) / 2 + 30] == [40.25, 40.125, 10.125, 10.25]
assert [(0 - 64) / (1 / 256) + 16384, (128 - 64) / (1 / 256) + 16384] == [0, 32768]
subprocess.run(["git", "diff", "--check", BASE, SOURCE], cwd=ROOT, check=True)

receipt = {
    "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "evidence_kind": "SOURCE_ONLY; no compilation or Kotlin test execution",
    "base": BASE, "source_commit": SOURCE, "grant_commit": GRANT,
    "grant_sha256": digest(git("show", f"{GRANT}:docs/CALLER_PARALLEL_GRANTS_20261018.md")),
    "branch": "task/t3.01-desktop-capture-callers-20261018",
    "source_sha256": hashes,
    "unchanged_bound_contract_sha256": bound_hashes,
    "checks": {"exclusive_source_paths": "PASS", "source_matches_frozen_candidate": "PASS",
               "whitespace_and_newlines": "PASS", "git_diff_check": "PASS",
               "lexical_delimiters_only": "PASS; not parsing, PSI or compilation",
               "shared_public_declaration_signatures": "UNCHANGED",
               "shared_platform_neutral_imports": "PASS",
               "existing_editor_shell_and_rendering_suffix_sha256": digest(renderer.encode()),
               "caller_hook_presence_and_retirement_order_source_assertions": "PASS; not runtime proof",
               "independent_fixture_arithmetic_only": fixtures},
    "test_inventory": inventory,
    "written_test_totals": {key: sum(item[key] for item in inventory.values())
                            for key in ("baseline_written", "current_written", "new_written")},
    "executed_kotlin_tests": 0,
    "not_run": ["all compiler/build processes including Kotlin/JUnit/PSI/Gradle/cargo/rustc",
                "new and existing Kotlin/JVM tests", "generated binding/app compatibility builds",
                "Windows WGC/UIA/hotkey/exclusion/picker runtime", "physical mixed-DPI/negative-origin region mapping",
                "actual preview/GPU presentation and stale-target lifecycle exercises", "device or physical input/performance acceptance"],
    "build_slot": "coordinator owns heavy slot; no compiler started by this lane",
    "runtime_gates_passed": [],
    "installations_downloads_security_or_device_actions": False,
    "screenshots_created": False,
    "source_bound_delivery_contract": "Actual NativeProject and frozen EditorController destination only; retain escaped non-cancellation close failure without retrying destruction",
    "workspace_identity_limit": "Normal token-owned deletion works with null fileKey; an unmarked directory retry without an exact key remains held",
    "windows_provider_reference": "https://raw.githubusercontent.com/openjdk/jdk17u/master/src/java.base/windows/classes/sun/nio/fs/WindowsFileAttributes.java",
}
OUTPUT.write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps({"result": "SOURCE CHECKS PASS", "source_commit": SOURCE,
                  "written_tests": receipt["written_test_totals"], "executed_kotlin_tests": 0,
                  "receipt": str(OUTPUT.relative_to(ROOT))}, indent=2))
