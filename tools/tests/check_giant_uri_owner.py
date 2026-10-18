#!/usr/bin/env python3
"""Cheap actual-source URI holder tests and fixed Android JNI-name compile probe."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--tools-root", type=Path, required=True)
parser.add_argument("--receipt-dir", type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
tools = args.tools_root.resolve()
output = args.receipt_dir.resolve()
output.mkdir(exist_ok=False)
cache = tools / "gradle/caches/modules-2/files-2.1"


def jar(group, name, version):
    paths = list((cache / group / name / version).glob("*/*.jar"))
    if len(paths) != 1:
        raise RuntimeError(f"exact cached tool unavailable: {group}/{name}/{version}")
    return paths[0]


compiler = [jar("org.jetbrains.kotlin", name, "2.4.20") for name in
            ["kotlin-compiler-embeddable", "kotlin-build-tools-api", "kotlin-stdlib",
             "kotlin-script-runtime", "kotlin-daemon-embeddable"]]
compiler += [jar("org.jetbrains.kotlin", "kotlin-reflect", "1.6.10"),
             jar("org.jetbrains.kotlinx", "kotlinx-coroutines-core-jvm", "1.8.0"),
             jar("org.jetbrains", "annotations", "13.0")]
stdlib = jar("org.jetbrains.kotlin", "kotlin-stdlib", "2.4.20")
coroutines = jar("org.jetbrains.kotlinx", "kotlinx-coroutines-core-jvm", "1.9.0")
junit = jar("junit", "junit", "4.13.2")
hamcrest = jar("org.hamcrest", "hamcrest-core", "1.3")
shared = root / "apps/shared/build/libs/shared-desktop.jar"
bindings = root / "apps/bindings-core/build/libs/bindings-core.jar"
android = tools / "android-sdk/platforms/android-37.0/android.jar"
java = tools / "jdk-root/usr/lib/jvm/java-21-openjdk-amd64/bin/java"
javap = java.parent / "javap"
base = root / "apps/shared/src"
api = base / "commonMain/kotlin/com/visualworkbench/shared/GiantImportsApi.kt"
owner = base / "jvmMain/kotlin/com/visualworkbench/shared/GiantUriReaderOwner.kt"
runtime = base / "androidMain/kotlin/com/visualworkbench/shared/giant/AndroidGiantUriRuntime.kt"
tests = base / "jvmTest/kotlin/com/visualworkbench/shared/GiantUriReaderOwnerTest.kt"
commands = []


def run(name, command):
    process = subprocess.run([str(x) for x in command], cwd=root,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, timeout=60)
    (output / (name + ".log")).write_text(process.stdout)
    commands.append({"name": name, "exit_code": process.returncode})
    print(name, process.returncode, process.stdout[-2000:], flush=True)
    if process.returncode:
        raise RuntimeError(name)
    return process.stdout


try:
    with tempfile.TemporaryDirectory(prefix="vw-giant-uri-holder-") as directory:
        directory = Path(directory)
        production = directory / "production.jar"
        cases = directory / "cases.jar"
        dependencies = [stdlib, coroutines, shared, bindings, android]
        compile_base = [java, "-Xmx512m", "-cp", os.pathsep.join(map(str, compiler)),
                        "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler", "-no-stdlib", "-no-reflect",
                        "-jvm-target", "21", "-Werror"]
        run("production-explicit-api", compile_base + ["-Xexplicit-api=strict", "-classpath",
            os.pathsep.join(map(str, dependencies)), "-d", production, api, owner, runtime])
        run("test-compile", compile_base + ["-Xfriend-paths=" + str(production), "-classpath",
            os.pathsep.join(map(str, [production, *dependencies, junit, hamcrest])), "-d", cases, tests])
        result = run("holder-junit", [java, "-Xmx256m", "-cp",
            os.pathsep.join(map(str, [cases, production, stdlib, shared, junit, hamcrest])),
            "org.junit.runner.JUnitCore", "com.visualworkbench.shared.GiantUriReaderOwnerTest"])
        if "OK (7 tests)" not in result:
            raise RuntimeError("missing exact seven-case result")
        names = run("actual-jni-methods", [javap, "-p", "-s", "-classpath", production,
            "com.visualworkbench.shared.giant.AndroidGiantUriRuntime",
            "com.visualworkbench.shared.giant.AndroidGiantUriReader"])
        for signature in ["readerForNative(java.lang.String)", "initializeNative()",
                          "openForNative()", "readForNative(byte[])", "closeForNative()"]:
            if signature not in names:
                raise RuntimeError("missing fixed JVM method: " + signature)
        for descriptor in ["()V", "([B)I", "()I",
                           "(Ljava/lang/String;)Lcom/visualworkbench/shared/giant/AndroidGiantUriReader;"]:
            if "descriptor: " + descriptor not in names:
                raise RuntimeError("missing JNI descriptor: " + descriptor)
    (output / "receipt.json").write_text(json.dumps({
        "status": "focused source holder/JNI-name probe passed", "junit_cases": 7,
        "compiler": "Kotlin2.4.20", "android_compile_api": 37, "commands": commands,
        "source_sha256": {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
                          for p in [api, owner, runtime, tests]},
        "temporary_compiler_outputs_disposed": True,
        "limits": ["Actual InputStream holder behavior exercised on JVM, no Android replacement classes",
                   "Actual Android fixed bridge compiled against SDK37, JNI names inspected",
                   "Cached predecessor shared/bindings used only for unchanged project/error types",
                   "New Rust URI ownership regressions and Android JNI target not compiled/executed",
                   "Thin native URI facade requires final ABI; app picker and device journey remain open"]
    }, indent=2) + "\n")
except Exception:
    (output / "failure.json").write_text(json.dumps({"status": "failed; preserve logs",
                                                    "commands": commands}, indent=2) + "\n")
    raise
