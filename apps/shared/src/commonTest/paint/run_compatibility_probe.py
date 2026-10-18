#!/usr/bin/env python3
"""Exact importer/owner/parser leaves and Desktop file guards; no Gradle/native build.

No fake generated bindings or framework types. Desktop guard functions are
unchanged source slices; Compose UI and NativeRemoteEdit require final app checks.
"""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[5]


def main():
    gradle = Path(os.environ.get("GRADLE_USER_HOME", "/workspace/.vw-tools/gradle"))
    compilers = sorted(gradle.glob("wrapper/dists/gradle-*/**/lib/kotlin-compiler-embeddable-*.jar"))
    coroutines = sorted(gradle.glob("caches/modules-2/files-2.1/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/*/*.jar"))
    if not compilers or len(coroutines) != 1:
        raise SystemExit("Cached compiler/coroutines unavailable; no dependency resolution attempted")
    lib = compilers[-1].parent
    cp = os.pathsep.join(map(str, [next(lib.glob("kotlin-stdlib-*.jar")), coroutines[0]]))
    java = str(Path(os.environ["JAVA_HOME"]) / "bin/java")
    shared = ROOT / "apps/shared/src"
    common = shared / "commonMain/kotlin/com/visualworkbench/shared"
    jvm = shared / "jvmMain/kotlin/com/visualworkbench/shared"
    sources = [common / (name + ".kt") for name in ["RemoteEditApi", "RemoteEditorCompatibility", "RemoteEditorPalette", "RemoteEditorProfileEvidence", "RemoteCompatibilityImport"]]
    sources += [jvm / "RemoteNativeJson.kt", jvm / "NativeRemoteCompatibility.kt"]
    with tempfile.TemporaryDirectory(prefix="vw-compatibility-probe-") as temp:
        temp = Path(temp)
        out = temp / "classes"
        compile = [java, "-Xmx384m", "-cp", str(lib / "*"), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
                   "-no-stdlib", "-no-reflect", "-Werror", "-jvm-target", "21", "-d", str(out)]
        subprocess.run(compile + ["-Xexplicit-api=strict", "-classpath", cp] + list(map(str, sources)), check=True, timeout=45)
        runtime = str(out) + os.pathsep + cp
        subprocess.run(compile + ["-Xfriend-paths=" + str(out), "-classpath", runtime,
            str(shared / "jvmTest/kotlin/com/visualworkbench/shared/RemoteCompatibilityImportCases.kt")], check=True, timeout=45)
        subprocess.run([java, "-Xmx128m", "-cp", runtime, "com.visualworkbench.shared.RemoteCompatibilityImportCasesKt"], check=True, timeout=20)
        desktop = ROOT / "apps/desktop/src/main/kotlin/com/visualworkbench/desktop/remote/RemoteCompatibilityImport.kt"
        text = desktop.read_text()
        start, end = text.index("internal fun sameImportTarget("), text.index("private suspend fun chooseCompatibilityReceipt(")
        fragment = temp / "RemoteCompatibilityFileGuard.kt"
        fragment.write_text("package com.visualworkbench.desktop.remote\nimport com.visualworkbench.shared.*\nimport java.nio.ByteBuffer\nimport java.nio.channels.FileChannel\nimport java.nio.file.*\nimport java.nio.file.attribute.BasicFileAttributes\n" + text[start:end])
        cases = ROOT / "apps/desktop/src/test/kotlin/com/visualworkbench/desktop/remote/RemoteCompatibilityImportCases.kt"
        subprocess.run(compile + ["-classpath", runtime, str(fragment), str(cases)], check=True, timeout=45)
        subprocess.run([java, "-Xmx128m", "-cp", runtime, "com.visualworkbench.desktop.remote.RemoteCompatibilityImportCasesKt"], check=True, timeout=20)
    print("Exact importer leaves / Desktop guard slices passed; temporary outputs disposed.")


if __name__ == "__main__":
    main()
