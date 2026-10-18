#!/usr/bin/env python3
"""Bounded actual-router/motion-observer/Window-key checks; no Gradle/native/device run.
Run only with the coordinator's focused compiler slot or cheap-test authorization.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools-root", required=True, type=Path)
    parser.add_argument("--receipt-dir", required=True, type=Path)
    args = parser.parse_args()
    root = next(p for p in Path(__file__).resolve().parents if (p / "AGENTS.md").is_file())
    output = args.receipt_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    tools = args.tools_root.resolve()

    def one(paths):
        values = sorted(paths)
        if len(values) != 1: raise RuntimeError(f"Expected one cached tool; found {len(values)}")
        return values[0]

    java = one(tools.glob("jdk-root/usr/lib/jvm/*/bin/java"))
    compiler = one(tools.glob("gradle/wrapper/dists/gradle-*/**/lib/kotlin-compiler-embeddable-*.jar"))
    stdlib = one(compiler.parent.glob("kotlin-stdlib-[0-9]*.jar"))
    coroutines = one(tools.glob("gradle/caches/modules-2/files-2.1/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/*/*.jar"))
    android = one(tools.glob("android-sdk/platforms/android-37*/android.jar"))
    shared = root / "apps/shared/src/commonMain/kotlin/com/visualworkbench/shared"
    caller = root / "apps/android/src/main/kotlin/com/visualworkbench/android/airactions"
    sources = [shared / name for name in ["AirActions.kt", "AirActionRouter.kt", "EditorSupport.kt"]]
    sources += [caller / name for name in ["AirActionBarrelCorrelation.kt", "AirActionRemoteMotion.kt", "AirActionRemoteMotionEvent.kt", "AirActionWindowKeyHook.kt"]]
    sources += [Path(__file__).resolve().with_name("AirActionRemoteMotionContract.kt")]
    sources += [Path(__file__).resolve().with_name("AirActionWindowKeyContract.kt")]
    commands = []

    def run(name, command):
        result = subprocess.run([str(arg) for arg in command], cwd=root, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        (output / (name + ".log")).write_text(result.stdout)
        commands.append({"name": name, "exit": result.returncode})
        if result.returncode: raise RuntimeError(name + " failed:\n" + result.stdout)
        return result.stdout

    try:
        with tempfile.TemporaryDirectory(prefix="vw-air-remote-motion-") as folder:
            scratch = Path(folder)
            runner = scratch / "RemoteMotionCheck.kt"
            runner.write_text('''import com.visualworkbench.android.airactions.AirActionRemoteMotionContract
import com.visualworkbench.android.airactions.AirActionWindowKeyContract
fun main() {
    val scenarios = AirActionRemoteMotionContract.scenarios() + AirActionWindowKeyContract.scenarios()
    for ((name, scenario) in scenarios) { scenario(); println("PASS: " + name) }
    println("SCENARIOS=" + scenarios.size)
}
''')
            classes = scratch / "remote-motion.jar"
            compiler_cp = ":".join(str(p) for p in sorted(compiler.parent.glob("*.jar")))
            run("compile", [java, "-Xmx512m", "-cp", compiler_cp, "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
                            "-no-stdlib", "-no-reflect", "-jvm-target", "21", "-Werror", "-classpath",
                            f"{stdlib}:{coroutines}:{android}", "-d", classes, *sources, runner])
            result = run("host-contracts", [java, "-Xmx256m", "-cp", f"{stdlib}:{coroutines}:{classes}", "RemoteMotionCheckKt"])
            count = int(result.strip().split("SCENARIOS=")[-1])
        receipt = {"status": "focused motion and Window-key contracts passed", "compiler": compiler.name,
                   "scenarios": count, "commands": commands,
                   "source_sha256": {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources},
                   "temporary_artifacts_disposed": True,
                   "limits": ["Real router and production observer; native readiness/ACK is a simulated admission flag",
                              "MotionEvent adapter and public Window callback compiled against API 37; framework methods are not executed",
                              "No full Compose/EditorController compilation or JUnit runner",
                              "No Android runtime, native input, device/BLE/S23/A09 acceptance"]}
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps({"status": receipt["status"], "scenarios": count, "receipt": str(output / "receipt.json")}))
    except Exception:
        (output / "failure.json").write_text(json.dumps({"status": "failed", "commands": commands}, indent=2) + "\n")
        raise


if __name__ == "__main__":
    main()
