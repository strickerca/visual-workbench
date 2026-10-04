package com.visualworkbench.desktop.mcp

import com.visualworkbench.desktop.NativeFileGuard
import com.visualworkbench.desktop.NativeResources
import java.nio.file.*
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class McpStartupTest {
    @Test fun modalCommandFencesPendingPeerFocusBeforeCompositionAndSharesChooserLease(){
        var fenced=false;var epoch=0;var acquired=0;var released=0
        val modal=McpModalOwner<String>{fenced=true;epoch++;acquired++;AutoCloseable{fenced=false;epoch++;released++}}
        val pendingEpoch=epoch
        val pendingFocus={ !fenced && epoch==pendingEpoch }
        try{
            // This is the exact synchronous setter used by Main's menu, error
            // and duplicate-start actions; no composition has run yet.
            modal.settings();assertFalse(pendingFocus());assertTrue(fenced)
            modal.compare("exact-receipt");assertEquals(1,acquired)
            val chooser=modal.retain()
            try{modal.dismiss();assertTrue(fenced);assertEquals(0,released);modal.settings();assertEquals(1,acquired);modal.dismiss()}
            finally{chooser.close()}
            assertFalse(fenced);assertEquals(1,released);assertFalse(pendingFocus())
        }finally{modal.close()}
    }
    @Test fun modalShutdownSealsNewAdmissionButRetainsAnOwnedNativeChooser(){
        var releases=0;val modal=McpModalOwner<String>{AutoCloseable{releases++}}
        modal.settings();val chooser=modal.retain()
        try{modal.close();assertEquals(0,releases);try{modal.settings();fail("closed admission")}catch(_:IllegalStateException){}}
        finally{chooser.close();modal.close()}
        assertEquals(1,releases)
    }
    private val guard=object:NativeFileGuard{
        override fun check(path:Path){check(!Files.isSymbolicLink(path))}
        override fun pin(path:Path)=AutoCloseable{}
        override fun pinDirectory(path:Path)=AutoCloseable{}
    }
    private fun fails(body:()->Unit){try{body();fail("Expected refusal")}catch(_:IllegalArgumentException){}catch(_:McpRefused){}}
    private fun hex(bytes:ByteArray)=MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(""){"%02x".format(it.toInt() and 255)}
    private val requiredRuntime=listOf("vw-mcp-host.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs","vw-codex-host.exe","mcp/codex/main.mjs","mcp/codex/client.mjs","mcp/codex/package.mjs","mcp/codex/process.mjs","mcp/codex/schema.mjs","mcp/codex/schema-worker.mjs","mcp/codex/image-profiles.json")
    private fun manifest(extra:String=""):ByteArray=(requiredRuntime.joinToString(""){"${hex(byteArrayOf(1))} 1 $it\n"}+extra).toByteArray()
    @Test fun runtimeManifestRequiresCompleteUniqueBoundedPaths(){
        assertEquals(requiredRuntime.size,DesktopMcpBundle.parse(manifest()).size)
        for(path in listOf("../x","mcp//x","mcp/NUL.txt","mcp/x.","NODE.EXE"))fails{DesktopMcpBundle.parse(manifest("${"a".repeat(64)} 1 $path\n"))}
        fails{DesktopMcpBundle.parse(manifest("${"a".repeat(64)} 268435457 huge\n"))}
    }
    @Test fun verifiedBundleRefusesUnknownOrChangedBytesAndKeepsPriorFiles(){
        val dir=Files.createTempDirectory("vw-mcp-bundle-").toRealPath()
        val resources=NativeResources{name->when{name==DesktopMcpBundle.MANIFEST->manifest().inputStream();name.startsWith("mcp-server/")->byteArrayOf(1).inputStream();else->null}}
        try{
            val first=DesktopMcpBundle.prepare(dir,resources,guard);val folder=first.runtime.root;first.close()
            Files.write(folder.resolve("node.exe"),byteArrayOf(2))
            fails{DesktopMcpBundle.prepare(dir,resources,guard)};assertArrayEquals(byteArrayOf(2),Files.readAllBytes(folder.resolve("node.exe")))
            Files.write(folder.resolve("node.exe"),byteArrayOf(1));Files.write(folder.resolve("injected.dll"),byteArrayOf(1))
            fails{DesktopMcpBundle.prepare(dir,resources,guard)};assertTrue(Files.exists(folder.resolve("injected.dll")))
        }finally{deleteFixture(dir)}
    }
    @Test fun appMarkerIsExactBoundedNonsecretCommand(){
        val nonce=UUID.randomUUID().toString();val bytes=DesktopAppOwner.encode(AppStartRequest.Mcp,nonce,1000)
        assertEquals(AppRequest(AppStartRequest.Mcp,nonce,1000),DesktopAppOwner.decode(bytes))
        for(value in listOf(bytes+byteArrayOf(10),"VWAPP1\nSend\n$nonce\n1000\n".toByteArray(),ByteArray(161){65},"VWAPP1\nMcp\n$nonce\n01000\n".toByteArray()))fails{DesktopAppOwner.decode(value)}
    }
    @Test fun realWindowsOwnerLockPreventsDuplicateAndReleasesExactly(){
        val dir=Files.createTempDirectory("vw-mcp-owner-").toRealPath();var first:DesktopAppOwner?=null;var next:DesktopAppOwner?=null
        try{first=DesktopAppOwner.claim(dir,guard,WindowsAppMarkerIo());assertNotNull(first);assertNull(DesktopAppOwner.claim(dir,guard,WindowsAppMarkerIo()));first!!.close();first=null
            next=DesktopAppOwner.claim(dir,guard,WindowsAppMarkerIo());assertNotNull(next)
        }finally{next?.close();first?.close();deleteFixture(dir)}
    }
    @Test fun markerDeletionBindsValidatedHandleAndMalformedDataSurvives(){
        val dir=Files.createTempDirectory("vw-mcp-marker-").toRealPath();val io=WindowsAppMarkerIo();val path=dir.resolve("request")
        try{
            Files.write(path,"foreign".toByteArray());fails{io.consume(path){DesktopAppOwner.decode(it)}};assertEquals("foreign",Files.readString(path));Files.delete(path)
            val bytes=DesktopAppOwner.encode(AppStartRequest.Show,UUID.randomUUID().toString(),1000);Files.write(path,bytes)
            assertEquals(AppStartRequest.Show,io.consume(path){DesktopAppOwner.decode(it).kind});assertFalse(Files.exists(path))
        }finally{deleteFixture(dir)}
    }
    @Test fun lateStartupResultAndEveryConcurrentCloseWaitForActualSettlement()=runBlocking{
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();val closing=CompletableDeferred<Unit>();val finish=CompletableDeferred<Unit>();val closed=AtomicInteger()
        val lifetime=McpStartLifetime{entered.complete(Unit);withContext(NonCancellable){release.await()};object:McpOwnedService{override suspend fun close(){closing.complete(Unit);finish.await();closed.incrementAndGet()}}}
        val start=async{lifetime.start()};entered.await();val a=async{lifetime.close()};val b=async{lifetime.close()}
        try{yield();assertFalse(a.isCompleted);release.complete(Unit);closing.await();assertFalse(a.isCompleted);assertFalse(b.isCompleted);finish.complete(Unit);a.await();b.await();assertEquals(1,closed.get())}
        finally{release.complete(Unit);finish.complete(Unit);start.cancelAndJoin();lifetime.close()}
    }
    @Test fun failedStartupCanRetryWithoutLeakingAnOwner()=runBlocking{
        val count=AtomicInteger();val closed=AtomicInteger();val lifetime=McpStartLifetime{if(count.incrementAndGet()==1)throw McpRefused();object:McpOwnedService{override suspend fun close(){closed.incrementAndGet()}}}
        try{try{lifetime.start();fail("first start")}catch(_:McpRefused){};lifetime.start();assertEquals(2,count.get())}finally{lifetime.close()};assertEquals(1,closed.get())
    }
    @Test fun callerCancellationDoesNotDiscardApplicationOwnedStartup()=runBlocking{
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();val closed=AtomicInteger();val lifetime=McpStartLifetime{entered.complete(Unit);release.await();object:McpOwnedService{override suspend fun close(){closed.incrementAndGet()}}}
        val caller=async{lifetime.start()}
        try{entered.await();caller.cancelAndJoin();release.complete(Unit);lifetime.start();assertEquals(0,closed.get())}finally{release.complete(Unit);lifetime.close()};assertEquals(1,closed.get())
    }
    @Test fun stopFencesRestartAndEveryCloserJoinsBeforeNewOwner()=runBlocking {
        val closing=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();val made=AtomicInteger()
        val cycles=McpServiceCycles{val id=made.incrementAndGet();object:McpOwnedService{override suspend fun close(){if(id==1){closing.complete(Unit);release.await()}}}}
        try{cycles.start();val first=async{cycles.stop()};withTimeout(5000){closing.await()};val second=async{cycles.stop()}
            yield();assertFalse(first.isCompleted);assertFalse(second.isCompleted)
            try{cycles.start();fail("restart during stop")}catch(_:IllegalStateException){}
            release.complete(Unit);withTimeout(5000){first.await();second.await()};cycles.start();assertEquals(2,made.get())
        }finally{release.complete(Unit);cycles.close()}
    }
    @Test fun lateResourceCleanupFailureIsReportedToEveryCloser()=runBlocking {
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        val lifetime=McpStartLifetime{entered.complete(Unit);withContext(NonCancellable){release.await()};object:McpOwnedService{override suspend fun close(){throw McpRefused()}}}
        val start=launch{try{lifetime.start()}catch(_:Exception){}}
        try{withTimeout(5000){entered.await()};val closing=async{try{lifetime.close();false}catch(_:McpRefused){true}}
            yield();release.complete(Unit);assertTrue(withTimeout(5000){closing.await()});try{lifetime.close();fail("missing cleanup failure")}catch(_:McpRefused){}
        }finally{release.complete(Unit);start.cancelAndJoin();try{lifetime.close()}catch(_:McpRefused){}}
    }
    @Test fun expiredAndFutureOwnerMarkersRetireWithoutStartingService(){
        val directory=Files.createTempDirectory("vw-mcp-marker-age-").toRealPath()
        var bytes:ByteArray?=null
        val marker=object:AppMarkerIo{
            override fun peek(path:Path)=checkNotNull(bytes)
            override fun <T> consume(path:Path,validate:(ByteArray)->T):T?{val current=bytes?:return null;val value=validate(current);bytes=null;return value}
        }
        val owner=checkNotNull(DesktopAppOwner.claim(directory,guard,marker))
        try{
            bytes=DesktopAppOwner.encode(AppStartRequest.Mcp,UUID.randomUUID().toString(),1000);assertNull(owner.poll(1000+DesktopAppOwner.TTL+1));assertNull(bytes)
            bytes=DesktopAppOwner.encode(AppStartRequest.Mcp,UUID.randomUUID().toString(),2000);assertNull(owner.poll(1999));assertNull(bytes)
            bytes=DesktopAppOwner.encode(AppStartRequest.Show,UUID.randomUUID().toString(),3000);assertEquals(AppStartRequest.Show,owner.poll(3001));assertNull(owner.poll(3001))
        }finally{owner.close();deleteFixture(directory)}
    }
    private fun deleteFixture(root:Path){Files.walk(root).use{paths->paths.sorted(Comparator.reverseOrder()).forEach{Files.delete(it)}}}
}
