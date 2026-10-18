#!/usr/bin/env python3
"""Cheap current-source Kotlin routing/interaction probe; no Gradle/native build.

Uses exact shared declarations, current production coordinator/workflow/leaves
and existing package result owner. JNI factory is deliberately unavailable:
workflow tests inject a backend through the production dependency seam.
JUnit/common test bodies run with tiny ordinary assertion facades in temp files.
This does not validate Compose, bindings, native catalog bytes or real captures.
"""
from pathlib import Path
import argparse
import os
import re
import subprocess
import tempfile

ROOT = next(p for p in Path(__file__).resolve().parents if (p / 'AGENTS.md').is_file())
TOOLS = Path(os.environ.get('VW_TOOLS_ROOT', '/workspace/.vw-tools'))
GRADLE = Path(os.environ.get('GRADLE_USER_HOME', TOOLS / 'gradle'))
compilers = sorted(GRADLE.glob('wrapper/dists/gradle-*/**/lib/kotlin-compiler-embeddable-*.jar'))
if not compilers:
    raise SystemExit('Cached compiler unavailable; no installation attempted')
lib = compilers[-1].parent
stdlib = next(lib.glob('kotlin-stdlib-*.jar'))
coroutines = list(GRADLE.glob('caches/modules-2/files-2.1/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/*/*.jar'))
if len(coroutines) != 1:
    raise SystemExit('Pinned cached coroutines unavailable')
classpath = os.pathsep.join(map(str, (stdlib, coroutines[0])))
java = Path(os.environ.get('JAVA_HOME', TOOLS / 'jdk-root/usr/lib/jvm/java-21-openjdk-amd64')) / 'bin/java'
shared = ROOT / 'apps/shared/src/commonMain/kotlin/com/visualworkbench/shared'
jvm = ROOT / 'apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared'
production = ROOT / 'apps/desktop/src/main/kotlin/com/visualworkbench/desktop/mcp'
tests = Path(__file__).parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--legacy-failure-budget-negative-control', action='store_true',
                    help='Restore only the temporary shared128MiB default; failure caller regression must fail')
args = parser.parse_args()

helpers = '''
private fun assertTrue(value:Boolean){check(value)}
private fun assertTrue(message:String,value:Boolean){check(value){message}}
private fun assertFalse(value:Boolean){check(!value)}
private fun assertEquals(expected:Any?,actual:Any?){check(expected==actual){"Expected=$expected actual=$actual"}}
private fun assertSame(expected:Any?,actual:Any?){check(expected===actual)}
private fun assertNull(value:Any?){check(value==null)}
private fun fail(message:String):Nothing=error(message)
private inline fun <reified T:Throwable> assertFailsWith(noinline block:()->Unit):T {
    try{block()}catch(error:Throwable){check(error is T);return error};error("Expected failure")
}
'''
with tempfile.TemporaryDirectory(prefix='vw-verification-workflow-') as scratch:
    scratch = Path(scratch)
    files = []
    sources = {}
    core = (shared / 'CoreApi.kt').read_text()
    sources['CoreApi.kt'] = '\n'.join(line for line in core.splitlines() if ' expect fun ' not in line)
    workflow = (shared / 'WorkflowApi.kt').read_text()
    binding = re.findall(r'^public data class WorkflowBinding\([^\n]+\)$', workflow, re.MULTILINE)
    assert len(binding) == 1
    sources['WorkflowBinding.kt'] = 'package com.visualworkbench.shared\n' + binding[0]
    for name in ('PackagesApi.kt', 'McpInboxApi.kt', 'VerificationApi.kt'):
        sources[name] = (shared / name).read_text().split('public expect')[0]
    if args.legacy_failure_budget_negative_control:
        current = 'memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL'
        assert sources['VerificationApi.kt'].count(current) == 1
        sources['VerificationApi.kt'] = sources['VerificationApi.kt'].replace(current,
            'memoryBudgetBytes: ULong = 128uL * 1024uL * 1024uL')
        print('NEGATIVE CONTROL: only temporary shared default restored to128MiB; default caller must fail', flush=True)
    sources['SemanticsApi.kt'] = (shared / 'SemanticsApi.kt').read_text().split('public interface WorkbenchSemanticCollection')[0]
    sources['UnavailableNative.kt'] = '''package com.visualworkbench.shared
@Suppress("UNUSED_PARAMETER")
public fun verificationWorkflows(original:WorkbenchProject,after:WorkbenchProject,catalog:WorkbenchPackageCatalog,inbox:WorkbenchMcpInbox):WorkbenchVerification=error("JNI intentionally unavailable in focused probe")
'''
    host = (production / 'DesktopMcpHost.kt').read_text()
    sources['OwnerTypes.kt'] = 'package com.visualworkbench.desktop.mcp\n' + host[host.index('/** These callbacks'):host.index('/** Terminate')]
    sources['Connection.kt'] = (production / 'McpConnection.kt').read_text().split('internal class NativeMcpConnection')[0]
    sources['Coordinator.kt'] = (production / 'DesktopMcpCoordinator.kt').read_text()
    suites = [
        ROOT / 'apps/shared/src/commonTest/kotlin/com/visualworkbench/shared/VerificationInteractionTest.kt',
        ROOT / 'apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/VerificationOwnershipTest.kt',
    ]
    calls = []
    for suite in suites:
        text = suite.read_text()
        names = re.findall(r'@Test fun (\w+)\(', text)
        assert names
        text = '\n'.join(line for line in text.splitlines() if not line.startswith(('import kotlin.test.', 'import org.junit.'))).replace('@Test ', '')
        sources[suite.name] = text + helpers
        calls += [suite.stem + '().' + name + '()\nprintln("PASS ' + name + '")' for name in names]
    original_coordinator_tests = (tests / 'McpCoordinatorTest.kt').read_text()
    selected_tests = original_coordinator_tests[:original_coordinator_tests.index('    @Test fun decoder')]
    selected_tests += original_coordinator_tests[original_coordinator_tests.index('    private suspend fun <T> drawCapture'):]
    coordinator_names = re.findall(r'@Test fun (\w+)\(', selected_tests)
    assert len(coordinator_names) == 6
    selected_tests = '\n'.join(line for line in selected_tests.splitlines() if not line.startswith('import org.junit.')).replace('@Test ', '')
    sources['CoordinatorSelectedTests.kt'] = selected_tests + helpers
    registration_tests = (tests / 'McpImageRegistrationTest.kt').read_text()
    registration_names = re.findall(r'@Test fun (\w+)\(', registration_tests)
    assert len(registration_names) == 13
    registration_tests = '\n'.join(line for line in registration_tests.splitlines() if not line.startswith('import org.junit.')).replace('@Test ', '')
    sources['RegistrationSelectedTests.kt'] = registration_tests + helpers
    coordinator_calls = ['McpCoordinatorTest().' + name + '()\nprintln("PASS ' + name + '")' for name in coordinator_names]
    coordinator_calls += ['McpImageRegistrationTest().' + name + '()\nprintln("PASS ' + name + '")' for name in registration_names]
    sources['CoordinatorProbe.kt'] = 'package com.visualworkbench.desktop.mcp\nfun main(){\n' + '\n'.join(coordinator_calls) + '\n}\n'
    sources['SharedProbe.kt'] = 'package com.visualworkbench.shared\nfun main(){\n' + '\n'.join(calls) + '\n}\n'
    for name, text in sources.items():
        path = scratch / name
        path.write_text(text)
        files.append(str(path))
    files += [str(shared / 'VerificationInteraction.kt'), str(jvm / 'PackageOwnership.kt')]
    files += [str(production / name) for name in ('McpJson.kt','McpVerificationTargets.kt','McpMarkerAssessments.kt','McpSemanticMarkerAlignment.kt','McpImageRegistration.kt','McpVerificationWorkflow.kt')]
    files += [str(tests / 'McpVerificationWorkflowProbe.kt')]
    output = scratch / 'classes'
    command = [str(java), '-Xmx384m', '-cp', str(lib / '*'), 'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler', '-no-stdlib', '-no-reflect', '-Werror', '-jvm-target', '21', '-classpath', classpath, '-d', str(output), *files]
    print('Compiling current coordinator/workflow + exact shared API + native-owner-independent tests: ' + compilers[-1].name, flush=True)
    subprocess.run(command, check=True, timeout=55)
    for main in ('com.visualworkbench.shared.SharedProbeKt', 'com.visualworkbench.desktop.mcp.McpVerificationWorkflowProbe', 'com.visualworkbench.desktop.mcp.CoordinatorProbeKt'):
        subprocess.run([str(java), '-Xmx128m', '-cp', os.pathsep.join((str(output), classpath)), main], check=True, timeout=30)
print('44 focused Kotlin cases passed; temporary classes/declarations disposed. No Gradle/native/Compose/ABI validation claimed.')
