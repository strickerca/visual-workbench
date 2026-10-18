#!/usr/bin/env python3
"""Bounded Kotlin leaf probe; no Gradle/native build, installation or fake DTOs.

Compile exact, unmodified production DTO declarations without their unrelated
expect/actual service implementations. This verifies these leaves and their DTO
usage, not the full application, generated bindings or framework integration.
"""
from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main() -> None:
    tools = Path(os.environ.get("VW_TOOLS_ROOT", "/workspace/.vw-tools"))
    gradle = Path(os.environ.get("GRADLE_USER_HOME", tools / "gradle"))
    compilers = sorted(gradle.glob("wrapper/dists/gradle-*/**/lib/kotlin-compiler-embeddable-*.jar"))
    if not compilers:
        raise SystemExit("Cached Kotlin compiler unavailable; no installation attempted")
    lib = compilers[-1].parent
    stdlib = next(lib.glob("kotlin-stdlib-*.jar"))
    java = Path(os.environ.get("JAVA_HOME", tools / "jdk-root/usr/lib/jvm/java-21-openjdk-amd64")) / "bin/java"
    if not java.is_file():
        raise SystemExit("Configured Java unavailable; no installation attempted")
    shared = ROOT / "apps/shared/src/commonMain/kotlin/com/visualworkbench/shared"
    production = ROOT / "apps/desktop/src/main/kotlin/com/visualworkbench/desktop/mcp"
    print(f"Focused compiler: {compilers[-1].name}; production DTO declaration slices; no Gradle build", flush=True)
    with tempfile.TemporaryDirectory(prefix="vw-t501-leaf-") as scratch:
        scratch = Path(scratch)
        files = []
        workflow = (shared / "WorkflowApi.kt").read_text()
        bindings = re.findall(r"^public data class WorkflowBinding\([^\n]+\)$", workflow, re.MULTILINE)
        if len(bindings) != 1:
            raise SystemExit("WorkflowBinding declaration changed; review probe source admission")
        geometry = re.findall(r"^public data class (?:Point|Rect)\([^\n]+\)$", (shared / "CoreApi.kt").read_text(), re.MULTILINE)
        if len(geometry) != 2:
            raise SystemExit("Point/Rect declarations changed; review probe source admission")
        sources = {
            "WorkflowBinding.kt": "package com.visualworkbench.shared\n" + bindings[0] + "\n",
            "Geometry.kt": "package com.visualworkbench.shared\n" + "\n".join(geometry) + "\n",
            "Semantics.kt": (shared / "SemanticsApi.kt").read_text().split("public interface WorkbenchSemanticCollection")[0],
            "Packages.kt": (shared / "PackagesApi.kt").read_text().split("public interface WorkbenchCompiledPackage")[0],
            "Inbox.kt": (shared / "McpInboxApi.kt").read_text().split("public interface WorkbenchMcpInbox")[0],
        }
        for name, content in sources.items():
            path = scratch / name
            path.write_text(content)
            files.append(str(path))
        output = scratch / "classes"
        command = [str(java), "-Xmx384m", "-cp", str(lib / "*"), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
                   "-no-stdlib", "-no-reflect", "-Werror", "-jvm-target", "21", "-classpath", str(stdlib),
                   "-d", str(output), *files,
                   str(production / "McpVerificationTargets.kt"), str(production / "McpMarkerAssessments.kt"),
                   str(production / "McpSemanticMarkerAlignment.kt"),
                   str(production / "McpJson.kt"),
                   str(ROOT / "tools/tests/T501VerificationLeafProbe.kt")]
        subprocess.run(command, check=True, timeout=45)
        subprocess.run([str(java), "-Xmx128m", "-cp", os.pathsep.join((str(output), str(stdlib))),
                        "com.visualworkbench.desktop.mcp.T501VerificationLeafProbe"], check=True, timeout=15)
    print("Temporary Kotlin classes and exact DTO slices disposed.")


if __name__ == "__main__":
    main()
