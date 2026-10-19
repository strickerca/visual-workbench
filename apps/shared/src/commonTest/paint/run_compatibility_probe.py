#!/usr/bin/env python3
"""Exact importer/owner/parser leaves and Desktop file guards; no Gradle/native build.

No fake generated bindings or framework types. Desktop guard functions are
unchanged source slices; Compose UI and NativeRemoteEdit require final app checks.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[5]


def main():
    arguments = argparse.ArgumentParser(description=__doc__)
    arguments.add_argument("--check-adapter", action="store_true",
                           help="Also compile the complete actual NativeRemoteEdit source against cached, older shared/binding jars; no ABI regeneration")
    options = arguments.parse_args()
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
    sources = [common / (name + ".kt") for name in ["RemoteEditApi", "RemoteEditorCompatibility", "RemoteEditorPalette", "RemoteEditorProfileEvidence", "RemoteCompatibilityImport", "RemoteDrawingCheck"]]
    sources += [jvm / "RemoteNativeJson.kt", jvm / "NativeRemoteCompatibility.kt",
                jvm / "RemoteConsumerOwnership.kt", jvm / "NativeRemoteDrawingOwner.kt"]
    with tempfile.TemporaryDirectory(prefix="vw-compatibility-probe-") as temp:
        temp = Path(temp)
        # Exact common exception declarations; the remainder of SessionApi pulls
        # unrelated project/session APIs. No generated bindings are substituted.
        session = (common / "SessionApi.kt").read_text()
        start, end = session.index("public enum class SessionFailureKind"), session.index("public enum class SessionCarrier")
        failures = temp / "SessionFailure.kt"
        failures.write_text("package com.visualworkbench.shared\n" + session[start:end])
        sources += [failures]
        out = temp / "classes"
        compile = [java, "-Xmx384m", "-cp", str(lib / "*"), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
                   "-no-stdlib", "-no-reflect", "-Werror", "-jvm-target", "21", "-d", str(out)]
        subprocess.run(compile + ["-Xexplicit-api=strict", "-classpath", cp] + list(map(str, sources)), check=True, timeout=45)
        runtime = str(out) + os.pathsep + cp
        if options.check_adapter:
            cached = [ROOT / "apps/shared/build/libs/shared-desktop.jar",
                      ROOT / "apps/bindings-core/build/libs/bindings-core.jar"]
            if not all(path.is_file() for path in cached):
                raise SystemExit("Cached shared/binding jars unavailable; no build attempted")
            adapter_out = temp / "adapter"
            adapter_cp = runtime + os.pathsep + os.pathsep.join(map(str, cached))
            adapter_compile = compile.copy()
            adapter_compile[adapter_compile.index(str(out))] = str(adapter_out)
            subprocess.run(adapter_compile + ["-Xexplicit-api=strict", "-Xfriend-paths=" + ",".join(map(str, [out, cached[0]])),
                "-classpath", adapter_cp, str(jvm / "NativeRemoteEdit.kt")], check=True, timeout=45)
            print("Complete actual NativeRemoteEdit source compiled against cached older shared/binding jars; ABI/apps/runtime unrun.", flush=True)
        subprocess.run(compile + ["-Xfriend-paths=" + str(out), "-classpath", runtime,
            str(shared / "jvmTest/kotlin/com/visualworkbench/shared/RemoteCompatibilityImportCases.kt"),
            str(shared / "jvmTest/kotlin/com/visualworkbench/shared/RemoteDrawingCheckCases.kt"),
            str(shared / "jvmTest/kotlin/com/visualworkbench/shared/NativeDrawingOwnerCases.kt")], check=True, timeout=45)
        subprocess.run([java, "-Xmx128m", "-cp", runtime, "com.visualworkbench.shared.RemoteCompatibilityImportCasesKt"], check=True, timeout=20)
        subprocess.run([java, "-Xmx128m", "-cp", runtime, "com.visualworkbench.shared.RemoteDrawingCheckCasesKt"], check=True, timeout=30)
        subprocess.run([java, "-Xmx128m", "-cp", runtime, "com.visualworkbench.shared.NativeDrawingOwnerCasesKt"], check=True, timeout=30)
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
