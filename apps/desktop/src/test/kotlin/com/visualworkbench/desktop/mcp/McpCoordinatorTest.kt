package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Path
import java.util.Base64

class McpCoordinatorTest {
    private val session="a".repeat(32)
    private val hash="b".repeat(64)
    private var receiptCounter=0
    private fun receiptId()="019a3aaa-0123-789a-8abc-${(++receiptCounter).toString(16).padStart(12,'0')}"
    private fun id(value:String)=java.util.UUID.nameUUIDFromBytes(value.toByteArray(Charsets.UTF_8)).toString()
    private val binding=WorkflowBinding(id("project"),id("document"),7uL,"c".repeat(64))
    private fun info(id:String="package",target:String="generic")=PackageInfo(this.id(id),target,null,binding,"d".repeat(64),hash,"2026-10-03",1024uL,0u,emptyList(),true,false)
    private inner class Compiled(override val info:PackageInfo=info()):WorkbenchCompiledPackage{var closes=0;override suspend fun close(){closes++}}
    private inner class Catalog:WorkbenchPackageCatalog {
        val values=mutableListOf<PublishedPackage>();var closed=0;var retired=0
        var publishEntered:CompletableDeferred<Unit>?=null;var publishFinish:CompletableDeferred<Unit>?=null
        override suspend fun publish(packageValue:WorkbenchCompiledPackage):PublishedPackage {publishEntered?.complete(Unit);publishFinish?.await();return PublishedPackage(packageValue.info,"C:/private/${packageValue.info.packageId}").also{values+=it}}
        override suspend fun list()=values.toList()
        override suspend fun lookup(packageId:String,target:String,manifestSha256:String)=values.single{it.info.packageId==packageId&&it.info.target==target&&it.info.manifestSha256==manifestSha256}
        override suspend fun readFile(packageId:String,target:String,manifestSha256:String,name:String,maxBytes:UInt):ByteArray {
            require(name=="manifest.json");val value=lookup(packageId,target,manifestSha256).info
            return """{"package_id":"${value.packageId}","source":{"project_id":"${value.binding.projectId}","document_id":"${value.binding.documentId}","asset_hash":"${value.sourceAssetId}","width":256,"height":128},"compiled_for":{"target":"${value.target}","max_long_edge":2048,"scale":1},"markers":[]}""".toByteArray()
        }
        override suspend fun pendingRetirement():PackageRetirement?=null
        override suspend fun retire(packageId:String,target:String,manifestSha256:String):PackageRetirement {retired++;values.removeAll{it.info.packageId==packageId&&it.info.target==target};return PackageRetirement(packageId,target,manifestSha256,true)}
        override suspend fun close(){closed++}
    }
    private inner class Inbox:WorkbenchMcpInbox {
        val receipts=mutableListOf<McpInboxReceipt>();var closed=0;var submitting:CompletableDeferred<Unit>?=null;var settle:CompletableDeferred<Unit>?=null
        override suspend fun submit(catalog:WorkbenchPackageCatalog,submission:McpInboxSubmission):McpInboxReceipt {
            catalog.lookup(submission.packageId,submission.target,submission.manifestSha256)
            val image=McpInboxImage(1u,1u,8u,1uL,"e".repeat(64),null,1u)
            val receipt=McpInboxReceipt(submission.receiptId,"f".repeat(64),submission.packageId,submission.target,submission.manifestSha256,binding,"d".repeat(64),submission.connection,submission.createdAtMs,submission.text,submission.note,image,null,false)
            receipts+=receipt;submitting?.complete(Unit);settle?.let{withContext(NonCancellable){it.await()}}
            currentCoroutineContext().ensureActive();return receipt
        }
        override suspend fun list()=receipts.toList()
        override suspend fun readPng(receiptId:String,receiptBlake3:String,side:McpInboxSide)=error("unused")
        override suspend fun pixels(receiptId:String,receiptBlake3:String,side:McpInboxSide,region:McpInboxRegion,assumeUntaggedSrgb:Boolean,allowDepthReduction:Boolean)=error("unused")
        override suspend fun retire(receiptId:String,receiptBlake3:String)=error("unused")
        override suspend fun close(){closed++}
    }
    private inner class Connection:McpConnection {
        override val agents=MutableStateFlow(listOf(McpAgent(session,42)))
        override val port=23123
        val published=mutableListOf<String>();var sends=0;var closed=0;var revokes=0
        val revokedSessions=mutableListOf<String>();var revokeHook:suspend(String)->Unit={}
        var grantEntered:CompletableDeferred<Unit>?=null;var grantFinish:CompletableDeferred<Unit>?=null
        var unpublishEntered:CompletableDeferred<Unit>?=null;var unpublishFinish:CompletableDeferred<Unit>?=null
        override suspend fun publish(directory:Path,manifestSha256:String){published+=directory.toString()}
        override suspend fun unpublish(packageId:String,target:String,manifestSha256:String){unpublishEntered?.complete(Unit);unpublishFinish?.await()}
        override suspend fun previewClaude(packageId:String,manifestSha256:String)=McpClaudePreview(packageId,manifestSha256,"private folder","literal summary",mapOf("package_id" to packageId))
        override suspend fun pushClaude(connection:String,displayed:McpClaudePreview){sends++}
        override suspend fun grant(connection:String,selectors:List<String>,lifetimeMs:Int){grantEntered?.complete(Unit);grantFinish?.let{finish->withContext(NonCancellable){finish.await()}}}
        override suspend fun revoke(connection:String){revokes++;revokedSessions+=connection;revokeHook(connection)}
        override suspend fun close(){closed++}
    }
    private inner class Capture:McpCaptureEngine {
        var calls=0;var closed=0;var closeFailure:Throwable?=null;var enter:CompletableDeferred<Unit>?=null
        var waitForever=false
        var result=info()
        var beforeReturn:suspend()->Unit={}
        var selected=McpSelectedTarget("chosen","Owner-selected window")
        var returned:Compiled?=null
        var cleanupEntered:CompletableDeferred<Unit>?=null;var cleanupFinish:CompletableDeferred<Unit>?=null
        override suspend fun select()=selected
        override suspend fun compile(selector:String,checkGrant:suspend()->Unit):WorkbenchCompiledPackage {checkGrant();calls++;enter?.complete(Unit);try{if(waitForever)awaitCancellation();beforeReturn();return Compiled(result).also{returned=it}}finally{cleanupFinish?.let{withContext(NonCancellable){cleanupEntered?.complete(Unit);it.await()}}}}
        override suspend fun close(){closed++;closeFailure?.let{throw it}}
    }
    @Test fun failedCaptureRetirementKeepsCoordinatorOwnerUntilExactRetrySettles()=runBlocking {
        val catalog=Catalog();val inbox=Inbox();val connection=Connection();val capture=Capture()
        val owner=DesktopMcpCoordinator(catalog,inbox,connection,capture,newId={receiptId()})
        val failure=IllegalStateException("Capture project retirement uncertain")
        capture.closeFailure=failure
        suspend fun refusedClose() {
            try { owner.close(); fail("Failed retirement was reported as closed") }
            catch (error: IllegalStateException) {
                assertSame(failure, if (error === failure) error else error.cause)
            }
            assertSame(failure, capture.closeFailure)
            assertFalse(owner.state.value.closed)
        }
        refusedClose()
        assertFalse(owner.state.value.closed)
        assertEquals(1,capture.closed);assertEquals(1,catalog.closed);assertEquals(1,inbox.closed);assertEquals(1,connection.closed)
        refusedClose()
        assertEquals(2,capture.closed);assertEquals(1,catalog.closed);assertEquals(1,inbox.closed);assertEquals(1,connection.closed)
        capture.closeFailure=null
        owner.close()
        assertTrue(owner.state.value.closed)
        assertEquals(3,capture.closed);assertEquals(1,catalog.closed);assertEquals(1,inbox.closed);assertEquals(1,connection.closed)
    }
    @Test fun decoderRefusesAmbiguousOversizeAndNoncanonicalBase64(){
        assertThrows(IllegalArgumentException::class.java){decodeMcpReturn("x","eA==","")}
        assertThrows(IllegalArgumentException::class.java){decodeMcpReturn(null,"eB==","")}
        assertThrows(IllegalArgumentException::class.java){decodeMcpReturn("Ã©".repeat(17000),null,"")}
        val bytes=byteArrayOf(0,1,2,3);assertArrayEquals(bytes,decodeMcpReturn(null,Base64.getEncoder().encodeToString(bytes),"").second)
    }
    @Test fun publicationNeverSendsAndOnlyDisplayedPreviewMaySend()=runBlocking {
        val catalog=Catalog();val inbox=Inbox();val wire=Connection();val capture=Capture();val owner=DesktopMcpCoordinator(catalog,inbox,wire,capture,newId={receiptId()})
        try {
            val saved=owner.publish(Compiled(info(target="claude")));assertEquals(0,wire.sends)
            val displayed=owner.previewClaude(saved)
            try{owner.sendClaude(session,displayed.copy(content="changed"));fail("Changed preview sent")}catch(_:IllegalArgumentException){}
            assertEquals(0,wire.sends);owner.sendClaude(session,displayed);assertEquals(1,wire.sends)
        }finally{owner.close()}
    }
    @Test fun captureWaitsForDrawnIndicatorAndRevocationSettlesIt()=runBlocking {
        val catalog=Catalog();val inbox=Inbox();val wire=Connection();val capture=Capture();capture.waitForever=true;capture.enter=CompletableDeferred();val owner=DesktopMcpCoordinator(catalog,inbox,wire,capture,newId={receiptId()})
        var request:Job?=null
        try {
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            val requestNow=launch{owner.capture(session,"chosen")};request=requestNow
            val activity=withTimeout(5000){owner.state.map{it.activeCapture}.filterNotNull().first()}
            assertEquals(0,capture.calls);owner.indicatorDrawn(activity.id)
            withTimeout(5000){capture.enter!!.await();owner.revoke(session);requestNow.join()}
            assertNull(owner.state.value.activeCapture);assertTrue(wire.published.isEmpty())
        }finally{request?.cancelAndJoin();owner.close()}
    }
    @Test fun retirementDrainDoesNotLockUnrelatedPublication()=runBlocking {
        val catalog=Catalog();val inbox=Inbox();val wire=Connection();val capture=Capture();val owner=DesktopMcpCoordinator(catalog,inbox,wire,capture,newId={receiptId()})
        val entered=CompletableDeferred<Unit>();val finish=CompletableDeferred<Unit>();wire.unpublishEntered=entered;wire.unpublishFinish=finish
        var retirement:Deferred<Unit>?=null
        try {
            val first=owner.publish(Compiled(info("one")));val job=async{owner.retirePackage(first)};retirement=job
            withTimeout(5000){entered.await();owner.publish(Compiled(info("two")))}
            assertEquals(0,catalog.retired);finish.complete(Unit);withTimeout(5000){job.await()};assertEquals(1,catalog.retired)
        }finally{finish.complete(Unit);withContext(NonCancellable){retirement?.await();owner.close()}}
    }
    @Test fun closeWaitsForDurablySubmittedLateReturnWithoutDeletingIt()=runBlocking {
        val catalog=Catalog();val inbox=Inbox();val wire=Connection();val capture=Capture();val owner=DesktopMcpCoordinator(catalog,inbox,wire,capture,newId={receiptId()})
        val entered=CompletableDeferred<Unit>();val finish=CompletableDeferred<Unit>();inbox.submitting=entered;inbox.settle=finish
        var submit:Job?=null;var closing:Deferred<Unit>?=null
        try {
            owner.publish(Compiled())
            val producer=launch{owner.submitResult(session,info().packageId,hash,"literal ``` text",null,"untrusted")};submit=producer
            withTimeout(5000){entered.await()}
            val close=async(start=CoroutineStart.UNDISPATCHED){owner.close()};closing=close
            assertFalse(close.isCompleted);assertEquals(0,inbox.closed);finish.complete(Unit)
            withTimeout(5000){close.await();producer.join()}
            assertEquals(1,inbox.receipts.size);assertEquals(1,inbox.closed);assertEquals(1,catalog.closed);assertEquals(1,capture.closed);assertEquals(1,wire.closed)
            owner.close();assertEquals(1,inbox.closed)
        }finally{finish.complete(Unit);withContext(NonCancellable){submit?.cancelAndJoin();closing?.await();owner.close()}}
    }
    @Test fun ownerRevokeBypassesFourOccupiedCallbacksAndStopsFurtherCapture()=runBlocking {
        val catalog=Catalog();val inbox=Inbox();val wire=Connection();val capture=Capture();capture.waitForever=true;capture.enter=CompletableDeferred()
        val owner=DesktopMcpCoordinator(catalog,inbox,wire,capture,newId={receiptId()});val jobs=mutableListOf<Job>()
        try{
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            repeat(4){jobs+=launch(start=CoroutineStart.UNDISPATCHED){owner.capture(session,"chosen")}}
            val activity=withTimeout(5000){owner.state.map{it.activeCapture}.filterNotNull().first()};owner.indicatorDrawn(activity.id)
            withTimeout(5000){capture.enter!!.await();owner.revoke(session);jobs.joinAll()}
            assertTrue(owner.state.value.grants.isEmpty());assertNull(owner.state.value.activeCapture);assertEquals(1,wire.revokes)
            try{owner.capture(session,"chosen");fail("Revoked capture admitted")}catch(_:McpRefused){}
            assertEquals(1,capture.calls);assertTrue(wire.published.isEmpty())
        }finally{withContext(NonCancellable){jobs.forEach{it.cancel()};jobs.joinAll();owner.close()}}
    }
    @Test fun revokeInvalidatesAnInFlightGrantBeforeItsRemoteReply()=runBlocking {
        val catalog=Catalog();val inbox=Inbox();val wire=Connection();val capture=Capture();val owner=DesktopMcpCoordinator(catalog,inbox,wire,capture,newId={receiptId()})
        val entered=CompletableDeferred<Unit>();val finish=CompletableDeferred<Unit>();wire.grantEntered=entered;wire.grantFinish=finish
        var granting:Job?=null;var revoking:Deferred<Unit>?=null
        try{
            owner.selectTarget();granting=launch{try{owner.grant(session,listOf("chosen"),60_000);fail("Late grant published")}catch(_:McpRefused){}}
            entered.await();val stop=async(start=CoroutineStart.UNDISPATCHED){owner.revoke(session)};revoking=stop
            assertTrue(owner.state.value.grants.isEmpty());assertFalse(stop.isCompleted)
            finish.complete(Unit);withTimeout(5000){granting!!.join();stop.await()};assertTrue(owner.state.value.grants.isEmpty());assertEquals(1,wire.revokes)
        }finally{finish.complete(Unit);withContext(NonCancellable){granting?.cancelAndJoin();revoking?.await();owner.close()}}
    }

    @Test fun revokeAllRemovesBothAuthoritiesBeforeFirstNativeCleanupFinishes() = runBlocking {
        checkBatchRevoke(failFirst = false)
    }
    @Test fun revokeAllSettlesOtherReceiptsAfterTheFirstRemoteFailure() = runBlocking {
        checkBatchRevoke(failFirst = true)
    }
    private suspend fun checkBatchRevoke(failFirst: Boolean) = coroutineScope {
        val second = "e".repeat(32)
        val wire = Connection().apply { agents.value = listOf(McpAgent(session, 42), McpAgent(second, 43)) }
        val capture = Capture().apply {
            waitForever = true; enter = CompletableDeferred()
            cleanupEntered = CompletableDeferred(); cleanupFinish = CompletableDeferred()
        }
        val owner = DesktopMcpCoordinator(Catalog(), Inbox(), wire, capture,newId={receiptId()})
        val requests = mutableListOf<Job>(); var batch: Deferred<Boolean>? = null
        try {
            owner.selectTarget(); owner.grant(session, listOf("chosen"), 60_000); owner.grant(second, listOf("chosen"), 60_000)
            requests += launch { owner.capture(session, "chosen") }
            val activity = withTimeout(5000) { owner.state.map { it.activeCapture }.filterNotNull().first() }
            owner.indicatorDrawn(activity.id); withTimeout(5000) { checkNotNull(capture.enter).await() }
            requests += launch(start = CoroutineStart.UNDISPATCHED) { owner.capture(second, "chosen") }
            wire.revokeHook = { if (failFirst && it == session) throw McpRefused() }
            val pending = async(start = CoroutineStart.UNDISPATCHED) {
                try { owner.revokeAll(); false } catch (_: McpRefused) { true }
            }; batch = pending
            withTimeout(5000) { checkNotNull(capture.cleanupEntered).await() }
            assertFalse(pending.isCompleted); assertTrue(owner.state.value.grants.isEmpty())
            assertNotNull(owner.state.value.activeCapture)
            try { owner.grant(second, listOf("chosen"), 60_000); fail("Grant admitted during retiring batch") } catch (_: McpRefused) { }
            assertEquals(1, capture.calls)
            checkNotNull(capture.cleanupFinish).complete(Unit)
            assertEquals(failFirst, withTimeout(5000) { pending.await() })
            withTimeout(5000) { requests.joinAll() }
            assertEquals(setOf(session, second), wire.revokedSessions.toSet())
            assertEquals(2, wire.revokedSessions.size); assertTrue(owner.state.value.grants.isEmpty())
            assertNull(owner.state.value.activeCapture); assertTrue(wire.published.isEmpty())
        } finally {
            checkNotNull(capture.cleanupFinish).complete(Unit)
            withContext(NonCancellable) { requests.forEach { it.cancel() }; requests.joinAll(); batch?.await(); owner.close() }
        }
    }
    @Test fun duplicateStartDuringHeldStopPreservesTheActualIndicatorOwner() = runBlocking {
        val owner = DesktopMcpCoordinator(Catalog(), Inbox(), Connection(), Capture(),newId={receiptId()})
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val lifetime = DesktopMcpLifetime {
            object : McpDesktopServiceOwner {
                override val coordinator = owner
                override suspend fun close() {
                    entered.complete(Unit); release.await(); coordinator.close()
                }
            }
        }
        var stopping: Deferred<Unit>? = null
        try {
            lifetime.start(); owner.selectTarget(); owner.grant(session, listOf("chosen"), 60_000)
            val stop = async { lifetime.stop() }; stopping = stop
            withTimeout(5000) { entered.await() }
            assertSame(owner, lifetime.state.value.owner)
            assertTrue(mcpGrantIndicatorVisible(owner.state.value))
            try { lifetime.start(); fail("Duplicate start admitted during stop") } catch (_: McpRefused) { }
            assertSame(owner, lifetime.state.value.owner)
            assertTrue(mcpGrantIndicatorVisible(owner.state.value)); assertFalse(stop.isCompleted)
            release.complete(Unit); withTimeout(5000) { stop.await() }
            assertNull(lifetime.state.value.owner)
        } finally { release.complete(Unit); withContext(NonCancellable) { stopping?.await(); lifetime.close() } }
    }

    private suspend fun <T> drawCapture(owner:DesktopMcpCoordinator,body:suspend()->T):T = supervisorScope {
        val pending=async{body()}
        val activity=withTimeout(5000){owner.state.map{it.activeCapture}.filterNotNull().first()}
        owner.indicatorDrawn(activity.id)
        withTimeout(5000){pending.await()}
    }

    @Test fun verificationRecaptureUsesOnlyTheCheckedOriginalAndJoinedAfterPackage()=runBlocking {
        val catalog=Catalog();val capture=Capture();val wire=Connection()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
        try {
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            drawCapture(owner){owner.capture(session,"chosen")}
            val original=catalog.values.single().info
            capture.result=info("after")
            val pair=drawCapture(owner){owner.recaptureOriginal(session,original)}
            assertEquals(McpVerificationPackage.of(original),pair.original)
            assertEquals(McpVerificationPackage.of(capture.result),pair.recaptured)
            assertEquals(2,capture.calls);assertEquals(2,wire.published.size)
            assertNull(owner.state.value.activeCapture)
        } finally {owner.close()}
    }

    @Test fun verificationRefusesMissingCurrentOriginalBeforeStartingAnotherCapture()=runBlocking {
        val catalog=Catalog();val capture=Capture()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),Connection(),capture,newId={receiptId()})
        try {
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            drawCapture(owner){owner.capture(session,"chosen")}
            val original=catalog.values.single().info
            catalog.values.clear()
            try {owner.recaptureOriginal(session,original);fail("Missing original admitted")}
            catch(_:NoSuchElementException) { }
            assertEquals(1,capture.calls);assertNull(owner.state.value.activeCapture)
        } finally {owner.close()}
    }

    @Test fun verificationRechecksActualOriginalCatalogAfterTheRecaptureSettles()=runBlocking {
        val catalog=Catalog();val capture=Capture();val wire=Connection()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
        try {
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            drawCapture(owner){owner.capture(session,"chosen")}
            val original=catalog.values.single().info
            capture.result=info("after-original-disappeared")
            capture.beforeReturn={catalog.values.removeAll{it.info.packageId==original.packageId}}
            try {drawCapture(owner){owner.recaptureOriginal(session,original)};fail("Stale original accepted")}
            catch(_:NoSuchElementException) { }
            assertEquals(2,capture.calls)
            // The actual after package is durable, but no checked pair escapes.
            assertEquals(capture.result,catalog.values.single().info)
            assertEquals(2,wire.published.size);assertNull(owner.state.value.activeCapture)
        } finally {owner.close()}
    }

    @Test fun verificationRegistrySaturationCannotFailAnOrdinaryDurablePublication()=runBlocking {
        val catalog=Catalog();val capture=Capture();val wire=Connection()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
        try {
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            drawCapture(owner){owner.capture(session,"chosen")}
            repeat(63){owner.publish(Compiled(info("marked-$it")))}
            val unbound=info("registry-full")
            val saved=owner.publish(Compiled(unbound))
            assertEquals(unbound,saved.info);assertEquals(65,catalog.values.size)
            assertEquals(65,wire.published.size)
            assertTrue(checkNotNull(owner.state.value.message).contains("unavailable for verification"))
            try {owner.recaptureOriginal(session,unbound);fail("Unregistered target recaptured")}
            catch(_:IllegalStateException) { }
            assertEquals(1,capture.calls);assertNull(owner.state.value.activeCapture)
        } finally {owner.close()}
    }

    @Test fun identicalCaptureBytesCannotInheritAnotherSelectedWindowsVerificationRoute()=runBlocking {
        val catalog=Catalog();val capture=Capture()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),Connection(),capture,newId={receiptId()})
        try {
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            drawCapture(owner){owner.capture(session,"chosen")}
            capture.selected=McpSelectedTarget("other","Another owner-selected window")
            owner.selectTarget();owner.grant(session,listOf("chosen","other"),60_000)
            capture.result=info("identical-other-window")
            drawCapture(owner){owner.capture(session,"other")}
            val unbound=catalog.values.single{it.info.packageId==capture.result.packageId}
            assertTrue(checkNotNull(owner.state.value.message).contains("unavailable for verification"))
            try {owner.recaptureOriginal(session,unbound.info);fail("Inherited another OS target")}
            catch(_:IllegalStateException) { }
            assertEquals(2,capture.calls);assertNull(owner.state.value.activeCapture)
        } finally {owner.close()}
    }

    @Test fun originalRetirementDuringJoinedRecapturePreventsPublicationAndClosesCompiledOwner()=runBlocking {
        val catalog=Catalog();val capture=Capture();val wire=Connection()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
        val release=CompletableDeferred<Unit>();var pending:Deferred<Boolean>?=null
        try {
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            drawCapture(owner){owner.capture(session,"chosen")}
            val original=catalog.values.single()
            capture.result=info("retired-original-after")
            capture.enter=CompletableDeferred();capture.beforeReturn={release.await()}
            val request=async {
                try {drawCapture(owner){owner.recaptureOriginal(session,original.info)};false}
                catch(_:IllegalStateException) {true}
            };pending=request
            withTimeout(5000){checkNotNull(capture.enter).await()}
            owner.retirePackage(original)
            assertFalse(request.isCompleted)
            release.complete(Unit)
            assertTrue(withTimeout(5000){request.await()})
            assertEquals(1,checkNotNull(capture.returned).closes)
            assertEquals(1,wire.published.size);assertTrue(catalog.values.isEmpty())
            assertNull(owner.state.value.activeCapture)
        } finally {release.complete(Unit);withContext(NonCancellable){pending?.await();owner.close()}}
    }

}
