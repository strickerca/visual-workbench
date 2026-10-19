#!/usr/bin/env python3
"""Compile the actual NativeProject.close body at its foreign-handle boundary.

Only the handle and unadopted-child drain are substituted. The production close
body, error mapping, release dispatcher and giant owner are compiled unchanged.
This is deliberately not a whole-module, regenerated-ABI or real-JNI test.
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
parser.add_argument('--tools-root', type=Path, required=True)
parser.add_argument('--receipt-dir', type=Path, required=True)
parser.add_argument('--negative-control-ref')
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
tools = args.tools_root.resolve()
output = args.receipt_dir.resolve()
output.mkdir(exist_ok=False)
cache = tools / 'gradle/caches/modules-2/files-2.1'

def jar(group, artifact, version):
    paths = list((cache / group / artifact / version).glob('*/*.jar'))
    assert len(paths) == 1, (group, artifact, version, len(paths))
    return paths[0]

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
        jar('junit', 'junit', '4.13.2'), jar('org.hamcrest', 'hamcrest-core', '1.3'),
        jar('net.java.dev.jna', 'jna', '5.19.1')]
java = tools / 'jdk-root/usr/lib/jvm/java-21-openjdk-amd64/bin/java'
shared = base[2]
native = root / 'apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeCore.kt'
ownership = root / 'apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeOwnership.kt'
giant = root / 'apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/GiantImportOwnership.kt'
api = root / 'apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/GiantImportsApi.kt'
commands = []

def run(name, command, expected=0):
    result = subprocess.run([str(x) for x in command], cwd=root, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, timeout=60)
    (output / (name + '.log')).write_text(result.stdout)
    commands.append({'name': name, 'exit_code': result.returncode})
    print(name, result.returncode, result.stdout[-1400:], flush=True)
    assert (result.returncode == 0) if expected == 0 else (result.returncode != 0), name
    return result.stdout

def subject(source):
    project = source.index('private class NativeProject(')
    close = source[source.index('    override suspend fun close(): Unit', project):source.index('\n}\nprivate class NativeStroke', project)]
    mapping = source[source.index('internal fun mapFailure('):source.index('\nprivate suspend fun <T> native(')]
    fields = source[source.index('    private val closed =', project):source.index('    private suspend fun <T> call(', project)]
    fields += '\n'.join(re.findall(r'^    private val document(?:Passwords|Pages) = .*$', source, re.M)) + '\n'
    release = ownership.read_text()
    release = release[release.index('internal suspend fun <T> releaseNative('):release.index('\nprivate class CancellationOwner')]
    return '''package com.visualworkbench.shared
import com.visualworkbench.bindings.core.CoreException
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
''' + mapping + '\n' + release + '''
internal interface CloseBoundary { suspend fun close() }
internal class CloseBoundaryHandle(
    var shutdown: suspend () -> Unit = {}, var disposal: () -> Unit = {},
) {
    var closes = 0; var destroys = 0
    suspend fun closeSession() { closes++; shutdown() }
    fun destroy() { destroys++; disposal() }
}
internal class NativeProjectCloseBoundary(
    val handle: CloseBoundaryHandle,
    private val giantOwner: GiantImportOwner<CloseBoundaryHandle>? = null,
    private val drain: suspend () -> Unit = {},
) : CloseBoundary {
''' + fields + '''
    private suspend fun drainUnadoptedGiants() = drain()
''' + close + '\n}\n'

tests = r'''package com.visualworkbench.shared
import com.visualworkbench.bindings.core.CoreException
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
class NativeProjectCloseBoundaryTest {
    private suspend fun failure(block: suspend () -> Unit): Throwable {
        try { block() } catch (error: Throwable) { return error }
        throw AssertionError("Expected close failure")
    }
    @Test fun ordinaryShutdownFailureIsMappedAndStickyWithoutNativeRetries() = runBlocking {
        val nativeError = CoreException.Storage()
        val handle = CloseBoundaryHandle(shutdown = { throw nativeError })
        val project = NativeProjectCloseBoundary(handle)
        val error = failure { project.close() }
        assertTrue(error is CoreFailure); assertSame(nativeError, error.cause)
        assertSame(error, failure { project.close() })
        assertEquals(1, handle.closes); assertEquals(1, handle.destroys)
    }
    @Test fun destroyFailureIsStickyWithoutDestroyRetry() = runBlocking {
        val error = IllegalStateException("synthetic dispose failure")
        val handle = CloseBoundaryHandle(disposal = { throw error })
        val project = NativeProjectCloseBoundary(handle)
        repeat(2) { assertSame(error, failure { project.close() }) }
        assertEquals(1, handle.closes); assertEquals(1, handle.destroys)
    }
    @Test fun shutdownRemainsPrimaryAndDistinctDestroyFailureIsSuppressedOnce() = runBlocking {
        val primary = IllegalStateException("synthetic shutdown failure")
        val secondary = IllegalArgumentException("synthetic dispose failure")
        val handle = CloseBoundaryHandle({ throw primary }, { throw secondary })
        val project = NativeProjectCloseBoundary(handle)
        repeat(2) { assertSame(primary, failure { project.close() }) }
        assertArrayEquals(arrayOf(secondary), primary.suppressed)
        assertEquals(1, handle.closes); assertEquals(1, handle.destroys)
    }
    @Test fun unadoptedChildFailureBeforeShutdownRemainsRetryable() = runBlocking {
        var drains = 0
        val childError = IllegalStateException("synthetic child drain failure")
        val handle = CloseBoundaryHandle()
        val project = NativeProjectCloseBoundary(handle, drain = { if (++drains == 1) throw childError })
        assertSame(childError, failure { project.close() })
        assertEquals(0, handle.closes); assertEquals(0, handle.destroys)
        project.close(); project.close()
        assertEquals(2, drains); assertEquals(1, handle.closes); assertEquals(1, handle.destroys)
    }
    @Test fun successfulCloseIsIdempotent() = runBlocking {
        val handle = CloseBoundaryHandle(); val project = NativeProjectCloseBoundary(handle)
        project.close(); project.close()
        assertEquals(1, handle.closes); assertEquals(1, handle.destroys)
    }
    @Test fun concurrentCallersJoinShutdownAndReceiveSameStickyResult() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val settle = CompletableDeferred<Unit>()
        val error = IllegalStateException("synthetic joined shutdown failure")
        val handle = CloseBoundaryHandle(shutdown = { entered.complete(Unit); settle.await(); throw error })
        val project = NativeProjectCloseBoundary(handle)
        val first = async { failure { project.close() } }; entered.await()
        val second = async(start = CoroutineStart.UNDISPATCHED) { failure { project.close() } }
        assertFalse(first.isCompleted); assertFalse(second.isCompleted)
        assertEquals(1, handle.closes); assertEquals(0, handle.destroys)
        settle.complete(Unit)
        assertSame(error, first.await()); assertSame(error, second.await())
        assertEquals(1, handle.closes); assertEquals(1, handle.destroys)
    }
    @Test fun joinedGiantOwnerRetainsItsExistingRetryContract() = runBlocking {
        val retirements = GiantImportRetirements()
        var attempts = 0
        val error = IllegalStateException("synthetic unsettled giant retirement")
        val handle = CloseBoundaryHandle(shutdown = { if (++attempts == 1) throw error })
        val owner = GiantImportOwner(handle, { it.closeSession() }, { it.destroy() }, retirements)
        val project = NativeProjectCloseBoundary(handle, owner)
        assertSame(error, failure { project.close() })
        assertEquals(1, retirements.retainedCount()); assertEquals(0, handle.destroys)
        project.close(); project.close()
        assertEquals(0, retirements.retainedCount())
        assertEquals(2, handle.closes); assertEquals(1, handle.destroys)
    }
}
'''

try:
    with tempfile.TemporaryDirectory(prefix='vw-close-boundary-') as temp:
        temp = Path(temp)
        test_file = temp / 'NativeProjectCloseBoundaryTest.kt'; test_file.write_text(tests)
        probe = temp / 'NativeProjectCloseBoundary.kt'
        compiler_cmd = [java, '-Xmx512m', '-cp', os.pathsep.join(map(str, compiler)),
                        'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler', '-no-stdlib', '-no-reflect',
                        '-jvm-target', '21', '-Werror', '-classpath', os.pathsep.join(map(str, base)),
                        '-Xfriend-paths=' + str(shared)]
        def check(label, source, expected=0):
            probe.write_text(subject(source))
            destination = temp / (label + '.jar')
            run(label + '-compile', compiler_cmd + ['-d', destination, api, giant, probe, test_file])
            return run(label + '-junit', [java, '-Xmx256m', '-cp', os.pathsep.join(map(str, [destination, *base])),
                'org.junit.runner.JUnitCore', 'com.visualworkbench.shared.NativeProjectCloseBoundaryTest'], expected)
        current = check('current', native.read_text())
        if args.negative_control_ref:
            baseline = subprocess.run(['git', 'show', args.negative_control_ref + ':' + str(native.relative_to(root))],
                cwd=root, check=True, stdout=subprocess.PIPE, text=True).stdout
            check('prior-source-negative', baseline, expected=1)
        receipt = {'status': 'actual close-body boundary regression passed',
            'junit_cases': int(re.search(r'OK \((\d+) tests?\)', current).group(1)),
            'negative_control_ref': args.negative_control_ref, 'commands': commands,
            'source_sha256': {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in [native, ownership, giant, api, Path(__file__).resolve()]},
            'temporary_compiler_outputs_disposed': True,
            'limits': ['Actual production close body/error mapping/release dispatcher/giant owner compiled unchanged',
                'Foreign handle and unadopted-child drain substituted at their call boundary',
                'NativeProject whole class/new ABI/real JNI/app lifecycle not executed',
                'Cached predecessor shared/bindings jars used for unchanged core types; no device evidence']}
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
except Exception:
    (output / 'failure.json').write_text(json.dumps({'status': 'failed; preserve logs', 'commands': commands}, indent=2) + '\n')
    raise
