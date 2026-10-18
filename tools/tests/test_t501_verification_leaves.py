#!/usr/bin/env python3
"""Bounded Kotlin leaf probe; no Gradle/native build, installation or fake DTOs.

Compile exact, unmodified production DTO declarations without their unrelated
expect/actual service implementations. This verifies these leaves and their DTO
usage, not the full application, generated bindings or framework integration.
"""
from __future__ import annotations

import os
import argparse
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--negative-publication-control", action="store_true",
                        help="Remove only the proposal's inside-gate guard; race probe must fail")
    args = parser.parse_args()
    tools = Path(os.environ.get("VW_TOOLS_ROOT", "/workspace/.vw-tools"))
    gradle = Path(os.environ.get("GRADLE_USER_HOME", tools / "gradle"))
    compilers = sorted(gradle.glob("wrapper/dists/gradle-*/**/lib/kotlin-compiler-embeddable-*.jar"))
    if not compilers:
        raise SystemExit("Cached Kotlin compiler unavailable; no installation attempted")
    lib = compilers[-1].parent
    stdlib = next(lib.glob("kotlin-stdlib-*.jar"))
    coroutines = sorted(gradle.glob("caches/modules-2/files-2.1/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/*/*.jar"))
    if len(coroutines) != 1:
        raise SystemExit("Cached pinned coroutines unavailable; no installation attempted")
    classpath = os.pathsep.join((str(stdlib), str(coroutines[0])))
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
            "Packages.kt": (shared / "PackagesApi.kt").read_text().split("public expect")[0],
            "Inbox.kt": (shared / "McpInboxApi.kt").read_text().split("public expect")[0],
        }
        host = (production / "DesktopMcpHost.kt").read_text()
        sources["OwnerTypes.kt"] = "package com.visualworkbench.desktop.mcp\n" + host[host.index("/** These callbacks"):host.index("/** Terminate")]
        sources["Connection.kt"] = (production / "McpConnection.kt").read_text().split("internal class NativeMcpConnection")[0]
        for name, content in sources.items():
            path = scratch / name
            path.write_text(content)
            files.append(str(path))
        # Apply the owned proposal only to an owned temporary copy. Production
        # coordinator, dispatch, schema and registrations remain unchanged.
        relative = Path("apps/desktop/src/main/kotlin/com/visualworkbench/desktop/mcp/DesktopMcpCoordinator.kt")
        candidate = scratch / relative
        candidate.parent.mkdir(parents=True)
        candidate.write_text((ROOT / relative).read_text())
        subprocess.run(["git", "apply", "--unsafe-paths", "--directory", str(scratch),
                        str(ROOT / "integration/t5-01/private-recapture-owner.patch")],
                       cwd=ROOT, check=True, timeout=5)
        body = candidate.read_text()
        scope = "private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)"
        if body.count(scope) != 1:
            raise SystemExit("Coordinator scope changed; review deterministic probe admission")
        # Test-only dispatcher substitution lets UNDISPATCHED callers establish
        # exact Mutex waiter order without sleeps. Every owner function, guard,
        # grant, lock, cancellation and publication body is the proposal's code.
        body = body.replace(scope, scope.replace("Dispatchers.Default", "Dispatchers.Unconfined"))
        if args.negative_publication_control:
            if body.count("        beforePublish()\n") != 1:
                raise SystemExit("Inside-gate guard changed; review negative control")
            body = body.replace("        beforePublish()\n", "", 1)
            print("NEGATIVE CONTROL: inside-catalogGate guard removed; queued-publication race must fail", flush=True)
        candidate.write_text(body)
        files.append(str(candidate))
        print("Coordinator proposal: temporary postimage; deterministic scope-dispatcher substitution only", flush=True)
        output = scratch / "classes"
        command = [str(java), "-Xmx384m", "-cp", str(lib / "*"), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
                   "-no-stdlib", "-no-reflect", "-Werror", "-jvm-target", "21", "-classpath", classpath,
                   "-d", str(output), *files,
                   str(production / "McpVerificationTargets.kt"), str(production / "McpMarkerAssessments.kt"),
                   str(production / "McpSemanticMarkerAlignment.kt"),
                   str(production / "McpJson.kt"),
                   str(ROOT / "tools/tests/T501VerificationLeafProbe.kt"),
                   str(ROOT / "tools/tests/T501RecapturePublicationProbe.kt")]
        subprocess.run(command, check=True, timeout=45)
        for probe in ("T501VerificationLeafProbe", "T501RecapturePublicationProbe"):
            subprocess.run([str(java), "-Xmx128m", "-cp", os.pathsep.join((str(output), classpath)),
                            "com.visualworkbench.desktop.mcp." + probe], check=True, timeout=30)
    print("Temporary Kotlin classes and exact DTO slices disposed.")


if __name__ == "__main__":
    main()
