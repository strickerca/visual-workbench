#!/usr/bin/env python3
"""Source/ownership audit only. This does not compile Kotlin or run Android."""
from pathlib import Path
import hashlib
import json
import re
import subprocess
import xml.etree.ElementTree as ET

ROOT = next(parent for parent in Path(__file__).resolve().parents if (parent / "AGENTS.md").is_file())
BASE = "f661eca9d2f6d6d6ebe4fc31ed2d0a5c6ec095e0"
MAIN = "apps/android/src/main/"
KOTLIN = MAIN + "kotlin/com/visualworkbench/android/"
checks = set()


def check(name, value):
    if not value:
        raise AssertionError(name)
    checks.add(name)


def previous(path):
    return subprocess.check_output(["git", "show", BASE + ":" + path], cwd=ROOT, text=True)


def code(text):
    """Mask strings/comments, preserving offsets for delimiter and body checks."""
    result = list(text)
    index = 0
    while index < len(text):
        begin = index
        if text.startswith("//", index):
            end = text.find("\n", index)
            index = len(text) if end < 0 else end
        elif text.startswith("/*", index):
            index += 2
            depth = 1
            while index < len(text) and depth:
                if text.startswith("/*", index): depth += 1; index += 2
                elif text.startswith("*/", index): depth -= 1; index += 2
                else: index += 1
            check("terminated block comment", depth == 0)
        elif text.startswith('"""', index):
            end = text.find('"""', index + 3)
            check("terminated raw string", end >= 0)
            index = end + 3
        elif text[index] in "\"'":
            quote = text[index]
            index += 1
            while index < len(text):
                if text[index] == "\\": index += 2
                elif text[index] == quote: index += 1; break
                else: index += 1
            else: raise AssertionError("unterminated literal")
        else:
            index += 1
            continue
        for position in range(begin, index):
            if result[position] != "\n": result[position] = " "
    return "".join(result)


def body(text, name):
    masked = code(text)
    found = list(re.finditer(r"\bfun\s+" + re.escape(name) + r"\s*\(", masked))
    check("one method " + name, len(found) == 1)
    index = masked.index("(", found[0].start())
    depth = 1
    index += 1
    while depth:
        if masked[index] == "(": depth += 1
        if masked[index] == ")": depth -= 1
        index += 1
    begin = masked.index("{", index)
    depth = 1
    index = begin + 1
    while depth:
        if masked[index] == "{": depth += 1
        if masked[index] == "}": depth -= 1
        index += 1
    return text[begin:index]


def run():
    changed = subprocess.check_output(["git", "diff", "--name-only", BASE], cwd=ROOT, text=True).splitlines()
    untracked = subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard"], cwd=ROOT, text=True).splitlines()
    paths = sorted(set(changed + untracked))
    exact = {KOTLIN + path for path in ["MainActivity.kt", "editor/EditorController.kt", "editor/CanvasSurface.kt",
                                      "editor/EditorPreferences.kt", "ui/WorkbenchApp.kt"]}
    exact |= {MAIN + "AndroidManifest.xml", MAIN + "res/xml/workbench_air_actions.xml", MAIN + "res/xml/air_actions.xml",
              "docs/T5_04_AIR_ACTIONS_HANDOFF_20261018.md"}
    prefixes = [KOTLIN + "airactions/", "apps/android/src/test/kotlin/com/visualworkbench/android/airactions/"]
    check("changed files stay within the exclusive Android grant", all(path in exact or any(path.startswith(p) for p in prefixes) for path in paths))
    for path in paths:
        if not path.endswith(".kt"): continue
        masked = code((ROOT / path).read_text())
        stack = []
        for token in masked:
            if token in "({[": stack.append(token)
            elif token in ")}]":
                check("balanced Kotlin delimiters " + path, bool(stack) and stack.pop() == {")": "(", "}": "{", "]": "["}[token])
        check("closed Kotlin delimiters " + path, not stack)

    manifest = ET.parse(ROOT / (MAIN + "AndroidManifest.xml")).getroot()
    old = ET.fromstring(previous(MAIN + "AndroidManifest.xml"))
    android = "{http://schemas.android.com/apk/res/android}"
    check("permission set unchanged", [item.attrib for item in manifest.findall("uses-permission")] == [item.attrib for item in old.findall("uses-permission")])
    remote = "com.samsung.android.support.REMOTE_ACTION"
    registrations = [(activity, data) for activity in manifest.findall("application/activity")
                     for data in activity.findall("meta-data") if data.get(android + "name") == remote]
    check("exactly one REMOTE_ACTION registration", len(registrations) == 1)
    activity, data = registrations[0]
    check("registration belongs to MainActivity", activity.get(android + "name") == ".MainActivity")
    check("registration names accepted resource", data.get(android + "resource") == "@xml/workbench_air_actions")
    check("matching REMOTE_ACTION intent filter", any(item.get(android + "name") == remote for item in activity.findall("intent-filter/action")))

    protected = {
        "editor/CanvasSurface.kt": ["ensureRenderer", "drawOwned", "retireRenderer", "publish", "enqueue", "retireForProject"],
        "editor/EditorController.kt": ["adoptWorkflowDocument", "refresh", "loadBackground", "loadGiantBackground", "giantFrame", "observeProject", "observeLink"],
        "MainActivity.kt": ["acceptShare", "takePhoto", "onSaveInstanceState", "onNewIntent"],
    }
    for path, methods in protected.items():
        actual = (ROOT / (KOTLIN + path)).read_text()
        frozen = previous(KOTLIN + path)
        for name in methods:
            check("preserved frozen method " + path + ":" + name, body(actual, name) == body(frozen, name))
    controller = (ROOT / (KOTLIN + "editor/EditorController.kt")).read_text()
    frozen = previous(KOTLIN + "editor/EditorController.kt")
    for name in ["closeCurrent", "detachLink", "onCleared"]:
        actual_body = body(controller, name)
        lines = [line for line in actual_body.splitlines() if not line.strip().startswith("interruptAirInput(")]
        check("retirement logic preserved " + name, "\n".join(lines) == body(frozen, name))
    check("Air command awaits native mutation without launching detached work", "scope.launch" not in body(controller, "executeAirCommand"))
    check("Air command reuses UI native commit", "commitUndoRedo(" in body(controller, "executeAirCommand") and "commitUndoRedo(" in body(controller, "undoRedo"))
    canvas = (ROOT / (KOTLIN + "editor/CanvasSurface.kt")).read_text()
    for method in ["onTouchEvent", "onHoverEvent"]:
        method_body = body(canvas, method)
        check("barrel observer precedes PrecisionButtons " + method, method_body.index("controller.observeAirBarrel(packet)") < method_body.index("buttons.accept(packet)"))

    result = {"status": "source/ownership audit passed", "base": BASE, "checks": len(checks),
              "changed_source_sha256": {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in paths},
              "limits": ["No Kotlin/Gradle/compiler execution", "New JUnit regressions unrun", "No Android runtime/device/BLE/S23/A09 acceptance"]}
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    run()
