#!/usr/bin/env python3
"""Run the actual NativeRemoteEdit.close body at its foreign-handle boundary.

Actual borrow owner, consumer registry, publication fence, release dispatcher and
error mapping are compiled. Only the foreign handle/parent end is substituted.
This does not compile the whole adapter or exercise JNI/Windows.
"""
from pathlib import Path
import argparse, os, subprocess, tempfile

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--negative-control-ref')
args=parser.parse_args()
root=Path(__file__).resolve().parents[2]
shared=root/'apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared'
def source(name):
    return (shared/name).read_text()
native=source('NativeRemoteEdit.kt') if not args.negative_control_ref else subprocess.check_output(
    ['git','show',args.negative_control_ref+':'+str((shared/'NativeRemoteEdit.kt').relative_to(root))],cwd=root,text=True)
body=native[native.index('    override suspend fun close(){releaseNative{'):native.index('\n}\nprivate fun com.visualworkbench.bindings.core.RemoteDisplay.common()')]
body=body.replace('this@NativeRemoteEdit','this@CloseFixture')
borrow=source('NativeRemoteCompatibility.kt')
borrow=borrow[borrow.index('internal class NativeCompatibilityBorrowOwner {'):borrow.index('/** This interface belongs')]
release=source('NativeOwnership.kt')
release=release[release.index('internal suspend fun <T> releaseNative('):release.index('\nprivate class CancellationOwner')]
mapping=source('NativeSessions.kt')
mapping=mapping[mapping.index('internal fun sessionFailure('):mapping.index('\nprivate suspend fun <T> sessionCall(')]
code='''package com.visualworkbench.shared
import com.visualworkbench.bindings.core.SessionException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
'''+borrow+release+mapping+'''
private val remoteOwners=ConcurrentHashMap.newKeySet<CloseFixture>()
internal interface CloseBoundary {suspend fun close()}
internal class ForeignBoundary {
 val destroyed=AtomicBoolean();val pauses=AtomicInteger();val closes=AtomicInteger()
 val successor=AtomicBoolean();val successorPaused=AtomicBoolean()
 val pauseFailure=AtomicReference<SessionException?>(null)
 fun remotePause(reason:String) {check(reason=="owner_pause");check(!destroyed.get()){"foreign handle was destroyed"}
  pauses.incrementAndGet();if(successor.get())successorPaused.set(true)
  pauseFailure.getAndSet(null)?.let{throw it}}
 fun remoteClose(){check(!destroyed.get());closes.incrementAndGet()}
 fun destroy(){check(destroyed.compareAndSet(false,true))}
}
internal class CloseFixture(val handle:ForeignBoundary):CloseBoundary {
 private val closing=AtomicBoolean();private val retired=AtomicBoolean()
 private val closeMutex=Mutex();private var closeAttempt:CompletableDeferred<Unit>?=null
 private val compatibilityBorrows=NativeCompatibilityBorrowOwner()
 private val consumers=RemoteConsumerRegistry();private val job=SupervisorJob()
 init{remoteOwners.add(this)}
 fun retired():Boolean=retired.get()
 suspend fun borrow(body:suspend()->Unit){compatibilityBorrows.borrow(body)}
 suspend fun lockFromFinalizer(){closeMutex.withLock{}}
 fun register(consumer:RemoteConsumerOwner){consumers.register(consumer)}
 fun ownerWork(block:suspend CoroutineScope.()->Unit):Job=CoroutineScope(job+Dispatchers.Default).launch(block=block)
'''+body+'''
}
internal class ParentBoundary {
 val handle=ForeignBoundary();private val closed=AtomicBoolean()
 private val publication=RemoteOwnerPublication<CloseFixture>(closed){it.retired()}
 fun remote():CloseFixture=publication.acquire({check(!handle.destroyed.get())}){CloseFixture(handle)}
 suspend fun destroyAfterChild(){publication.seal()?.close();handle.destroy()}
}
public fun main():Unit=runBlocking {
 var passed=0;val failures=mutableListOf<String>()
 suspend fun scenario(name:String,block:suspend CoroutineScope.()->Unit){
  try {withTimeout(4_000){block()};passed++;println("PASS "+name)}
  catch(error:Throwable){failures.add(name+": "+error.javaClass.simpleName+": "+error.message);println("FAIL "+failures.last())}
 }
 scenario("repeated close performs no further foreign operation") {
  val p=ParentBoundary();val child=p.remote();child.close();val calls=p.handle.pauses.get()
  child.close();check(child.retired()&&p.handle.pauses.get()==calls&&p.handle.closes.get()==1)
 }
 scenario("retired child does not touch parent-destroyed handle") {
  val p=ParentBoundary();val child=p.remote();p.destroyAfterChild();val calls=p.handle.pauses.get()
  child.close();check(p.handle.pauses.get()==calls&&p.handle.closes.get()==1)
 }
 scenario("retired child cannot pause successor on same hub") {
  val p=ParentBoundary();val old=p.remote();old.close();val successor=p.remote();check(successor!==old)
  p.handle.successor.set(true);old.close();check(!p.handle.successorPaused.get()&&!successor.retired())
  p.handle.successor.set(false);successor.close()
 }
 scenario("borrower cleanup can acquire close mutex before joined retirement") {
  val p=ParentBoundary();val child=p.remote();val entered=CompletableDeferred<Unit>()
  val cleanup=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
  val read=launch {child.borrow {entered.complete(Unit);try{awaitCancellation()}finally{withContext(NonCancellable){
   child.lockFromFinalizer();cleanup.complete(Unit);release.await()}}}}
  entered.await();val close=async{child.close()};cleanup.await()
  check(p.handle.pauses.get()>0&&!close.isCompleted&&!child.retired()&&!p.handle.destroyed.get())
  release.complete(Unit);close.await();read.join();check(child.retired()&&p.handle.closes.get()==1)
 }
 scenario("pause failure still joins borrow and preserves retryable child") {
  val p=ParentBoundary();val child=p.remote();val entered=CompletableDeferred<Unit>()
  val cleanup=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
  val read=launch {child.borrow {entered.complete(Unit);try{awaitCancellation()}finally{withContext(NonCancellable){
   child.lockFromFinalizer();cleanup.complete(Unit);release.await()}}}}
  entered.await();p.handle.pauseFailure.set(SessionException.Worker())
  val result=async {runCatching{child.close()}};cleanup.await();check(!result.isCompleted&&!child.retired())
  release.complete(Unit);check(result.await().isFailure);read.join();check(p.handle.closes.get()==0)
  child.close();check(child.retired()&&p.handle.closes.get()==1)
 }
 if(System.getProperty("vw.close.negative")!="true")scenario("consumer and owner-job joins run outside mutex") {
  val p=ParentBoundary();val child=p.remote();val ready=CompletableDeferred<Unit>()
  child.register(object:RemoteConsumerOwner {override fun invalidate(){}
   override suspend fun retire():Boolean {child.lockFromFinalizer();return true}})
  child.ownerWork {ready.complete(Unit);try{awaitCancellation()}finally{withContext(NonCancellable){child.lockFromFinalizer()}}}
  ready.await();child.close();check(child.retired()&&p.handle.closes.get()==1)
 }
 scenario("concurrent close callers share one retirement attempt") {
  val p=ParentBoundary();val child=p.remote();val entered=CompletableDeferred<Unit>()
  val cleanup=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
  val read=launch {child.borrow {entered.complete(Unit);try{awaitCancellation()}finally{withContext(NonCancellable){
   cleanup.complete(Unit);release.await()}}}}
  entered.await();val first=async{child.close()};cleanup.await();val second=async{child.close()}
  yield();check(!first.isCompleted&&!second.isCompleted);release.complete(Unit)
  first.await();second.await();read.join();check(child.retired()&&p.handle.closes.get()==1)
 }
 scenario("failed consumer stays owned until successful retry") {
  val p=ParentBoundary();val child=p.remote();var ready=false
  child.register(object:RemoteConsumerOwner {override fun invalidate(){}
   override suspend fun retire():Boolean=ready})
  check(runCatching{child.close()}.isFailure&&!child.retired()&&p.handle.closes.get()==0)
  ready=true;child.close();check(child.retired()&&p.handle.closes.get()==1)
 }
 println("Actual remote close boundary: "+passed+" passed, "+failures.size+" failed")
 check(failures.isEmpty()){failures.joinToString("; ")}
}
'''
tools=Path(os.environ.get('VW_TOOLS_ROOT','/workspace/.vw-tools'))
cache=tools/'gradle/caches/modules-2/files-2.1'
def jar(group,name,version):
    matches=list((cache/group/name/version).glob('*/*.jar'));assert len(matches)==1,(group,name,version);return matches[0]
compiler=[jar('org.jetbrains.kotlin',name,'2.4.20') for name in ['kotlin-compiler-embeddable','kotlin-build-tools-api','kotlin-stdlib','kotlin-script-runtime','kotlin-daemon-embeddable']]
compiler += [jar('org.jetbrains.kotlin','kotlin-reflect','1.6.10'),jar('org.jetbrains.kotlinx','kotlinx-coroutines-core-jvm','1.8.0'),jar('org.jetbrains','annotations','13.0')]
runtime=[jar('org.jetbrains.kotlin','kotlin-stdlib','2.4.20'),jar('org.jetbrains.kotlinx','kotlinx-coroutines-core-jvm','1.9.0'),
    root/'apps/shared/build/libs/shared-desktop.jar',root/'apps/bindings-core/build/libs/bindings-core.jar',jar('net.java.dev.jna','jna','5.19.1')]
java=tools/'jdk-root/usr/lib/jvm/java-21-openjdk-amd64/bin/java'
with tempfile.TemporaryDirectory(prefix='vw-remote-close-') as directory:
    temporary=Path(directory);path=temporary/'CloseFixture.kt';path.write_text(code);output=temporary/'classes'
    subprocess.run([str(java),'-Xmx384m','-cp',os.pathsep.join(map(str,compiler)),'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler',
        '-no-stdlib','-no-reflect','-Werror','-jvm-target','21','-classpath',os.pathsep.join(map(str,runtime)),'-d',str(output),
        str(path),str(shared/'RemoteConsumerOwnership.kt'),str(shared/'RemoteOwnerPublication.kt')],check=True,timeout=55)
    result=subprocess.run([str(java),'-Dvw.close.negative='+str(bool(args.negative_control_ref)).lower(),'-Xmx128m','-cp',os.pathsep.join(map(str,[output,*runtime])),
        'com.visualworkbench.shared.CloseFixtureKt'],timeout=55)
    if args.negative_control_ref:
        assert result.returncode!=0,'Old close body unexpectedly passed'
        print('Expected old-source failures reproduce PRT1; the blocking old mutex-join case is omitted.')
    else:
        result.check_returncode()
        print('Actual close body passed with substituted foreign handle; whole adapter/ABI/Windows remain unrun.')
