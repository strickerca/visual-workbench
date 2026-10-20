#!/usr/bin/env python3
"""Small offline source/compiler probe; never builds Gradle/JNI or uses a device.

Uses actual stream/common state logic, Android SDK signatures and input adapter.
Unrelated expect factories and the existing editor renderer are substituted.
Compose/app/JNI compilation and real Surface/Bitmap behavior remain unverified.
"""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile

repo = next(path for path in Path(__file__).resolve().parents if (path / "AGENTS.md").is_file())
tools = Path(os.environ.get("VW_TOOL_ROOT", "/workspace/.vw-tools"))
cache = tools / "gradle/caches/modules-2/files-2.1"


def jar(pattern):
    return next(cache.glob(pattern))


compiler = jar("org.jetbrains.kotlin/kotlin-compiler-embeddable/2.4.20/*/*.jar")
stdlib = jar("org.jetbrains.kotlin/kotlin-stdlib/2.4.20/*/*.jar")
coroutines = jar("org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/*/*.jar")
distro = next((tools / "gradle/wrapper/dists").glob("gradle-9.7.0-bin/*/gradle-9.7.0/lib"))
annotations = distro / "annotations-24.0.1.jar"
android = tools / "android-sdk/platforms/android-37.0/android.jar"
java = next((tools / "jdk-root").rglob("bin/java"))
output = Path(tempfile.mkdtemp(prefix="vw-android-stream-probe-"))
common = repo / "apps/shared/src/commonMain/kotlin/com/visualworkbench/shared"
sources = []
for name in ("CoreApi.kt", "SessionApi.kt", "WorkflowApi.kt", "CaptureImportsApi.kt", "CaptureStreamApi.kt",
             "CaptureDraftApi.kt", "CaptureAnnotationSession.kt", "SessionResourcesApi.kt"):
    source = common / name
    text = "\n".join(line for line in source.read_text().splitlines() if " expect " not in line)
    target = output / name
    target.write_text(text)
    sources.append(target)

boundary = output / "EditorBoundary.kt"
boundary.write_text('''package com.visualworkbench.android.editor
import android.graphics.Bitmap
import android.graphics.Canvas
import com.visualworkbench.shared.*
internal data class DrawObject(val item:RenderItem)
internal data class CanvasScene(val document:DocumentSnapshot,val background:Bitmap?,val camera:Camera,
 val objects:List<DrawObject>,val provisional:List<DrawObject> = emptyList(),
 val giantMode:Boolean=false,val documentFrame:Any?=null,
 val aiResults:Map<String,Any> = emptyMap(),val resultDisplayReady:Boolean=true)
internal object EditorDrawing { fun draw(canvas:Canvas,scene:CanvasScene) {} }
''')
factory = output / "FactoryBoundary.kt"
factory.write_text('''package com.visualworkbench.shared
fun sessionResources():SessionResources=error("no JNI in offline probe")
''')
main = output / "ProbeMain.kt"
main.write_text('''package com.visualworkbench.android.stream
fun main(){println("lifecycle cases: ${runStreamLifecycleContract()}")
println("input cases: ${runStreamInputContract()}")
println("router/signal boundary cases: ${runStreamBoundaryRepairContract()}")
println("closed lifecycle cases: ${runStreamClosedLifecycleContract()}")}
''')
sources += [boundary, factory, main]
platform = repo / "apps/android/src/main/kotlin/com/visualworkbench/android"
sources += sorted((platform / "stream").glob("*.kt"))
sources = [path for path in sources if path.name != "AndroidStreamPanel.kt"]
sources += [platform / "input/CanvasGestureRouter.kt", platform / "input/StylusInput.kt"]
sources += [Path(__file__).with_name("StreamLifecycleContract.kt"), Path(__file__).with_name("StreamInputContract.kt")]
sources += [Path(__file__).with_name("StreamBoundaryContract.kt")]
snapshots = []
manifest = []
for index, source in enumerate(sources):
    content = source.read_bytes()
    snapshot = output / "snapshots" / str(index) / source.name
    snapshot.parent.mkdir(parents=True, exist_ok=True)
    snapshot.write_bytes(content)
    snapshots.append(snapshot)
    manifest.append(f"{hashlib.sha256(content).hexdigest()} {source}")
(output / "sources.sha256").write_text("\n".join(manifest) + "\n")
compiler_cp = [compiler] + [path for path in distro.glob("*.jar") if "kotlin-compiler-embeddable-" not in path.name]
runtime_cp = [stdlib, coroutines, annotations, android]
artifact = output / "probe.jar"
command = [str(java), "-Xmx512m", "-cp", os.pathsep.join(map(str, compiler_cp)),
           "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler", "-no-stdlib", "-no-reflect", "-jvm-target", "17",
           "-classpath", os.pathsep.join(map(str, runtime_cp)), "-d", str(artifact)] + list(map(str, snapshots))
compile_result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
(output / "compile.log").write_text(compile_result.stdout)
print(f"probe evidence: {output}")
print(compile_result.stdout, end="")
if compile_result.returncode:
    raise SystemExit(compile_result.returncode)
run = subprocess.run([str(java), "-cp", os.pathsep.join(map(str, [artifact] + runtime_cp)),
                      "com.visualworkbench.android.stream.ProbeMainKt"], text=True,
                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
(output / "run.log").write_text(run.stdout)
print(run.stdout, end="")
raise SystemExit(run.returncode)
