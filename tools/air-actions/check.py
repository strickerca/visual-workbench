#!/usr/bin/env python3
"""Cheap Air Actions leaf checks using existing cached tools, without Gradle,
native compilation, downloads or installation. Not an app build or JUnit/device run.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools-root", type=Path, required=True)
    parser.add_argument("--receipt-dir", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    output = args.receipt_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    tools = args.tools_root.resolve()

    def one(paths):
        found = sorted(paths)
        if len(found) != 1:
            raise RuntimeError(f"Expected one cached tool, found {len(found)}")
        return found[0]

    java = one(tools.glob("jdk-root/usr/lib/jvm/*/bin/java"))
    compiler = one(tools.glob("gradle/wrapper/dists/gradle-*/**/lib/kotlin-compiler-embeddable-*.jar"))
    compiler_lib = compiler.parent
    stdlib = one(compiler_lib.glob("kotlin-stdlib-[0-9]*.jar"))
    coroutines = one(tools.glob("gradle/caches/modules-2/files-2.1/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/*/*.jar"))
    android = one(tools.glob("android-sdk/platforms/android-37*/android.jar"))
    aapt = tools / "android-sdk/build-tools/36.1.0/aapt2"
    shared = root / "apps/shared/src/commonMain/kotlin/com/visualworkbench/shared"
    platform = root / "apps/android/src/main/kotlin/com/visualworkbench/android/airactions"
    sources = [shared / "AirActions.kt", shared / "AirActionRouter.kt", shared / "EditorSupport.kt",
               platform / "AirActionKeys.kt", platform / "AirActionsPanel.kt",
               root / "apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/AirActionContract.kt",
               root / "apps/android/src/test/kotlin/com/visualworkbench/android/airactions/AirActionKeyContract.kt"]
    commands = []

    def run(name, command):
        result = subprocess.run([str(arg) for arg in command], cwd=root, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        (output / f"{name}.log").write_text(result.stdout)
        commands.append({"name": name, "exit": result.returncode})
        if result.returncode:
            raise RuntimeError(f"{name} failed:\n{result.stdout}")
        return result.stdout

    try:
        with tempfile.TemporaryDirectory(prefix="vw-air-actions-") as scratch_name:
            scratch = Path(scratch_name)
            common_jar = scratch / "air-shared.jar"
            android_jar = scratch / "air-android.jar"
            tests_jar = scratch / "air-contracts.jar"
            compiler_cp = ":".join(str(path) for path in sorted(compiler_lib.glob("*.jar")))
            base_cp = f"{stdlib}:{coroutines}"

            def compile_kotlin(name, inputs, target, classpath, *flags):
                run(name, [java, "-Xmx512m", "-cp", compiler_cp, "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
                           "-no-stdlib", "-no-reflect", "-jvm-target", "21", "-Werror", "-classpath", classpath,
                           *flags, "-d", target, *inputs])

            compile_kotlin("shared-explicit-api", sources[:3], common_jar, base_cp, "-Xexplicit-api=strict")
            compile_kotlin("android-leaves", sources[3:5], android_jar, f"{base_cp}:{common_jar}:{android}")
            runner = scratch / "AirCheck.kt"
            runner.write_text('''import com.visualworkbench.shared.AirActionContract
import com.visualworkbench.android.airactions.AirActionKeyContract
fun main() {
    val scenarios = AirActionContract.scenarios() + AirActionKeyContract.scenarios()
    for ((name, scenario) in scenarios) { scenario(); println("PASS: " + name) }
    println("SCENARIOS=" + scenarios.size)
}
''')
            compile_kotlin("contract-sources", sources[5:] + [runner], tests_jar,
                           f"{base_cp}:{common_jar}:{android_jar}:{android}", f"-Xfriend-paths={android_jar}")
            result = run("host-contracts", [java, "-Xmx256m", "-cp", f"{base_cp}:{common_jar}:{android_jar}:{tests_jar}", "AirCheckKt"])
            count = int(result.strip().split("SCENARIOS=")[-1])

            res = root / "apps/android/src/main/res"
            declaration = res / "xml/workbench_air_actions.xml"
            labels = res / "values/air_actions.xml"
            actions = ET.parse(declaration).getroot()
            strings = {item.attrib["name"] for item in ET.parse(labels).getroot()}
            expected = ["click", "double_click", "swipe_left", "swipe_right", "swipe_up", "swipe_down", "circle_cw", "circle_ccw"]
            assert actions.tag == "remote-actions" and actions.attrib == {"version": "1.2"}
            assert len(actions) == 8 and len({item.attrib["id"] for item in actions}) == 8
            for index, item in enumerate(actions):
                assert item.attrib["priority"] == str(index + 1)
                assert item.attrib["trigger_key"] == f"F{index + 1}"
                assert item.attrib["label"].removeprefix("@string/") in strings
                assert len(item) == 1 and item[0].attrib == {"name": "gesture", "value": expected[index]}
                assert "repeatable" not in item.attrib
            task_res = scratch / "res"
            for source in (declaration, labels):
                dest = task_res / source.relative_to(res)
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, dest)
            resource_zip = scratch / "resources.zip"
            run("resource-compile", [aapt, "compile", "--dir", task_res, "-o", resource_zip])
            manifest = scratch / "AndroidManifest.xml"
            manifest.write_text('<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="com.visualworkbench.airactionscheck"><application /></manifest>')
            run("resource-link", [aapt, "link", "-I", android, "--manifest", manifest, "-o", scratch / "resources.apk", resource_zip])
            run("common-imports", ["python3", root / "tools/ffi-test/check_imports.py", shared])
            receipt = {"status": "focused checks passed", "compiler": compiler.name, "coroutines": coroutines.name,
                       "host_contract_scenarios": count, "commands": commands,
                       "source_sha256": {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sources + [declaration, labels]},
                       "limits": ["No full app/native/Gradle build or JUnit runner", "No Android Handler/View runtime test", "No Activity/manifest/controller composition", "No S23 stylus/Bluetooth/action acceptance"],
                       "temporary_artifacts_disposed": True}
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps({"status": receipt["status"], "host_contract_scenarios": count, "receipt": str(output / "receipt.json")}))
    except Exception:
        (output / "failure.json").write_text(json.dumps({"commands": commands, "status": "failed; see logs"}, indent=2) + "\n")
        raise


if __name__ == "__main__":
    main()
