#!/usr/bin/env python3
"""Focused production capture callers and delivery/retirement regression tests.

Uses cached predecessor ABI jars for unchanged symbols. Main/Compose, actual
WGC/UIA/JNI, full app shutdown and device journeys remain aggregate validation.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--tools-root', required=True, type=Path)
parser.add_argument('--receipt-dir', required=True, type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
tools = args.tools_root.resolve()
output = args.receipt_dir.resolve(); output.mkdir(exist_ok=False)
cache = tools / 'gradle/caches/modules-2/files-2.1'
def jar(group, name, version):
    matches = list((cache / group / name / version).glob('*/*.jar'))
    assert len(matches) == 1, (group, name, version)
    return matches[0]
compiler = [jar('org.jetbrains.kotlin', name, '2.4.20') for name in [
    'kotlin-compiler-embeddable', 'kotlin-build-tools-api', 'kotlin-stdlib',
    'kotlin-script-runtime', 'kotlin-daemon-embeddable']]
compiler += [jar('org.jetbrains.kotlin', 'kotlin-reflect', '1.6.10'),
             jar('org.jetbrains.kotlinx', 'kotlinx-coroutines-core-jvm', '1.8.0'),
             jar('org.jetbrains', 'annotations', '13.0')]
base = [jar('org.jetbrains.kotlin', 'kotlin-stdlib', '2.4.20'),
        jar('org.jetbrains.kotlinx', 'kotlinx-coroutines-core-jvm', '1.9.0'),
        root / 'apps/shared/build/libs/shared-desktop.jar',
        root / 'apps/bindings-core/build/libs/bindings-core.jar',
        root / 'apps/bindings-host/build/libs/bindings-host.jar',
        jar('junit', 'junit', '4.13.2'), jar('org.hamcrest', 'hamcrest-core', '1.3'),
        jar('net.java.dev.jna', 'jna', '5.19.1')]
java = tools / 'jdk-root/usr/lib/jvm/java-21-openjdk-amd64/bin/java'
capture = root / 'apps/desktop/src/main/kotlin/com/visualworkbench/desktop/capture'
sources = [capture / (name + '.kt') for name in [
    'CaptureDelivery', 'CaptureCallerRetirement', 'CaptureAdmission',
    'PickerWorkspace', 'PickerOwnership', 'PickerDisplay',
    'CapturePickerController', 'DesktopCaptureController']]
sources += [root / 'apps/desktop/src/main/kotlin/com/visualworkbench/desktop/DesktopDiagnosticsLifecycle.kt']
tests = sorted((root / 'apps/desktop/src/test/kotlin/com/visualworkbench/desktop/capture').glob('*Test.kt'))
commands = []
def run(name, command):
    result = subprocess.run([str(x) for x in command], cwd=root, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, timeout=60)
    (output / (name + '.log')).write_text(result.stdout)
    commands.append({'name': name, 'exit_code': result.returncode})
    print(name, result.returncode, result.stdout[-2200:], flush=True)
    assert result.returncode == 0, name
    return result.stdout
try:
    with tempfile.TemporaryDirectory(prefix='vw-capture-verification-') as temp:
        destination = Path(temp) / 'cases.jar'
        run('callers-compile', [java, '-Xmx512m', '-cp', os.pathsep.join(map(str, compiler)),
            'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler', '-no-stdlib', '-no-reflect',
            '-jvm-target', '21', '-Werror', '-classpath', os.pathsep.join(map(str, base)),
            '-d', destination, *sources, *tests])
        result = run('capture-junit', [java, '-Xmx256m', '-cp', os.pathsep.join(map(str, [destination, *base])),
            'org.junit.runner.JUnitCore', *['com.visualworkbench.desktop.capture.' + p.stem for p in tests]])
    receipt = {'status': 'focused current capture caller compile and regressions passed',
        'junit_cases': int(re.search(r'OK \((\d+) tests?\)', result).group(1)), 'commands': commands,
        'source_sha256': {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in [*sources, *tests, Path(__file__).resolve()]},
        'predecessor_abi_sha256': {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in base[2:5]},
        'limits': ['Both actual current capture controller sources compiled without substituted factories',
            'Main/Compose and real controller/MCP wiring not assembled or executed',
            'Cached predecessor jars supply unchanged core/shared/host signatures',
            'No actual WGC/UIA/native callback, whole-app shutdown, device or hardware approval'],
        'temporary_compiler_outputs_disposed': True}
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
except Exception:
    (output / 'failure.json').write_text(json.dumps({'status': 'failed; preserve logs', 'commands': commands}, indent=2) + '\n')
    raise
