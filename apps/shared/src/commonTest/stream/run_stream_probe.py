#!/usr/bin/env python3
"""Exact new common/owner sources with cached model APIs; no generated ABI stubs.

The real native adapters, generated methods, Compose and whole shared module are
not compiled here. Synthetic project/stream boundaries test common transitions;
generic child tests substitute allocation/retirement only. No Gradle resolution.
"""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[5]


def main():
    gradle = Path(os.environ.get("GRADLE_USER_HOME", "/workspace/.vw-tools/gradle"))
    compilers = sorted(gradle.glob("wrapper/dists/gradle-*/**/lib/kotlin-compiler-embeddable-*.jar"))
    if not compilers:
        raise SystemExit("Cached Kotlin compiler unavailable")
    lib = compilers[-1].parent
    def cached(pattern):
        values = sorted(gradle.glob(pattern))
        if len(values) != 1:
            raise SystemExit("Cached probe dependency missing or ambiguous: " + pattern)
        return values[0]
    coroutines = cached("caches/modules-2/files-2.1/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/*/*.jar")
    junit = cached("caches/modules-2/files-2.1/junit/junit/4.13.2/*/*.jar")
    hamcrest = cached("caches/modules-2/files-2.1/org.hamcrest/hamcrest-core/1.3/*/*.jar")
    models = ROOT / "apps/shared/build/libs/shared-desktop.jar"
    if not models.is_file():
        raise SystemExit("Cached shared model API unavailable; no replacement generated bindings")
    cp = os.pathsep.join(map(str, [next(lib.glob("kotlin-stdlib-*.jar")), coroutines, models]))
    shared = ROOT / "apps/shared/src"
    common = shared / "commonMain/kotlin/com/visualworkbench/shared"
    jvm = shared / "jvmMain/kotlin/com/visualworkbench/shared"
    tests = shared / "jvmTest/kotlin/com/visualworkbench/shared"
    java = str(Path(os.environ["JAVA_HOME"]) / "bin/java")
    with tempfile.TemporaryDirectory(prefix="vw-stream-probe-") as directory:
        temp = Path(directory)
        # Actual declarations only; the rest of NativeSessions needs fresh ABI.
        declarations = temp / "SessionOwnershipDeclarations.kt"
        sessions = (jvm / "NativeSessions.kt").read_text()
        owned = next(line for line in sessions.splitlines() if line.startswith("internal interface OwnedSession"))
        session_api = (common / "SessionApi.kt").read_text()
        a, b = session_api.index("public enum class SessionFailureKind"), session_api.index("public enum class SessionCarrier")
        declarations.write_text("package com.visualworkbench.shared\n" + session_api[a:b] + owned + "\n")
        sources = [common / (name + ".kt") for name in ["CaptureStreamApi", "CaptureDraftApi", "CaptureAnnotationSession"]]
        sources += [jvm / "CaptureStreamOwnership.kt", jvm / "CaptureChildren.kt", declarations]
        compile_cmd = [java, "-Xmx384m", "-cp", str(lib / "*"), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
                       "-no-stdlib", "-no-reflect", "-Werror", "-jvm-target", "21", "-d", str(temp / "classes")]
        subprocess.run(compile_cmd + ["-Xexplicit-api=strict", "-classpath", cp] + list(map(str, sources)), check=True, timeout=45)
        runtime = os.pathsep.join([str(temp / "classes"), cp, str(junit), str(hamcrest)])
        classes = ["CaptureAnnotationSessionTest", "CaptureStreamOwnershipTest", "CaptureChildrenTest"]
        subprocess.run(compile_cmd + ["-Xfriend-paths=" + str(temp / "classes"), "-classpath", runtime] + [str(tests / (name + ".kt")) for name in classes], check=True, timeout=45)
        subprocess.run([java, "-Xmx256m", "-cp", runtime, "org.junit.runner.JUnitCore"] +
                       ["com.visualworkbench.shared." + name for name in classes], check=True, timeout=40)


if __name__ == "__main__":
    main()
