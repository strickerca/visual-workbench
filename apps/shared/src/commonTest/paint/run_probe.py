#!/usr/bin/env python3
"""Compile actual Paint/palette leaves and run their synthetic lifecycle cases.

No Gradle/native/Compose build, network, installation or dependency resolution.
Uses the setup owner's cached Kotlin compiler and pinned coroutine artifact.
"""
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[5]


def main() -> None:
    tools = Path(os.environ.get("VW_TOOLS_ROOT", "/workspace/.vw-tools"))
    gradle = Path(os.environ.get("GRADLE_USER_HOME", tools / "gradle"))
    compilers = sorted(gradle.glob("wrapper/dists/gradle-*/**/lib/kotlin-compiler-embeddable-*.jar"))
    coroutines = sorted(gradle.glob("caches/modules-2/files-2.1/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/*/*.jar"))
    if not compilers or len(coroutines) != 1:
        raise SystemExit("Cached compiler/pinned coroutines unavailable; no installation attempted")
    lib = compilers[-1].parent
    stdlib = next(lib.glob("kotlin-stdlib-*.jar"))
    classpath = os.pathsep.join(map(str, (stdlib, coroutines[0])))
    java = Path(os.environ.get("JAVA_HOME", tools / "jdk-root/usr/lib/jvm/java-21-openjdk-amd64")) / "bin/java"
    production = ROOT / "apps/shared/src/commonMain/kotlin/com/visualworkbench/shared"
    cases = ROOT / "apps/shared/src/commonTest/kotlin/com/visualworkbench/shared/RemotePaintPaletteCases.kt"
    sources = [production / (name + ".kt") for name in (
        "RemoteEditApi", "RemoteEditorCompatibility", "RemoteEditorPalette", "RemoteEditorProfileEvidence",
    )]
    print(f"Cached leaf compiler: {compilers[-1].name}; actual production sources, no stubs", flush=True)
    with tempfile.TemporaryDirectory(prefix="vw-paint-palette-") as temp:
        classes = Path(temp) / "classes"
        command = [str(java), "-Xmx384m", "-cp", str(lib / "*"),
                   "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler", "-no-stdlib", "-no-reflect",
                   "-Werror", "-jvm-target", "21", "-d", str(classes)]
        subprocess.run(command + ["-Xexplicit-api=strict", "-classpath", classpath] + list(map(str, sources)),
                       check=True, timeout=45)
        runtime = str(classes) + os.pathsep + classpath
        subprocess.run(command + ["-classpath", runtime, str(cases)], check=True, timeout=45)
        subprocess.run([str(java), "-Xmx128m", "-cp", runtime,
                        "com.visualworkbench.shared.RemotePaintPaletteCasesKt"], check=True, timeout=15)
    print("Exact production leaves passed explicit API / warnings denied; temporary classes disposed.")


if __name__ == "__main__":
    main()
