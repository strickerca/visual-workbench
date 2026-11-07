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
        /** Checked manifest source.capture.platform; null omits the capture object. */
        var platform:String?=null
        var publishEntered:CompletableDeferred<Unit>?=null;var publishFinish:CompletableDeferred<Unit>?=null
        override suspend fun publish(packageValue:WorkbenchCompiledPackage):PublishedPackage {publishEntered?.complete(Unit);publishFinish?.await();return PublishedPackage(packageValue.info,"C:/private/${packageValue.info.packageId}").also{values+=it}}
        override suspend fun list()=values.toList()
        override suspend fun lookup(packageId:String,target:String,manifestSha256:String)=values.single{it.info.packageId==packageId&&it.info.target==target&&it.info.manifestSha256==manifestSha256}
        override suspend fun readFile(packageId:String,target:String,manifestSha256:String,name:String,maxBytes:UInt):ByteArray {
            require(name=="manifest.json");val value=lookup(packageId,target,manifestSha256).info
            val capture=platform?.let{",\"capture\":{\"platform\":\"$it\",\"captured_at\":\"2026-11-04T00:00:00Z\"}"}.orEmpty()
            return """{"package_id":"${value.packageId}","source":{"project_id":"${value.binding.projectId}","document_id":"${value.binding.documentId}","asset_hash":"${value.sourceAssetId}","width":256,"height":128$capture},"compiled_for":{"target":"${value.target}","max_long_edge":2048,"scale":1},"markers":[]}""".toByteArray()
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
        /** Same identity rule as the native engine: one selector per route object. */
        private val routes=linkedMapOf<String,CoordinatorRoute>();private var nextRoute=0
        var runtime=true
        override fun phoneRuntimeAvailable()=runtime
        override suspend fun retainPhoneRoute(route:McpPhoneScreenRoute):McpSelectedTarget {
            val exact=route as? CoordinatorRoute?:throw McpRecaptureUnavailable(McpRecaptureRefusal.SourceUnavailable)
            exact.checkAvailable()
            val selector=routes.entries.singleOrNull{it.value===exact}?.key?:"phone-${nextRoute++}".also{routes[it]=exact}
            return McpSelectedTarget(selector,"Phone screen | ${exact.description}",McpVerificationTargetKind.Tunnel)
        }
        override suspend fun pruneRetiredRoutes():Set<String> {
            val dead=routes.filterValues{it.retired}.keys.toSet();dead.forEach{routes.remove(it)};return dead
        }
        override suspend fun forgetPhoneRoute(selector:String){routes.remove(selector)}
        /** Retained phone routes, each holding one of the shared target slots. */
        val routeCount:Int get()=routes.size
        override suspend fun compileVerification(selector:String,profile:McpCaptureProfile,checkGrant:suspend()->Unit):WorkbenchCompiledPackage {
            if(!selector.startsWith("phone-"))return super.compileVerification(selector,profile,checkGrant)
            val route=routes[selector]?:throw McpRecaptureUnavailable(McpRecaptureRefusal.PhoneDisconnected)
            return route.capture{checkGrant();calls++;Compiled(result).also{returned=it}}
        }
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
        // An agent note may not carry an owner-written recapture source label.
        for(kind in McpVerificationTargetKind.entries.filter{it!=McpVerificationTargetKind.Window})
            assertThrows(IllegalArgumentException::class.java){decodeMcpReturn(null,Base64.getEncoder().encodeToString(bytes),mcpRecaptureNote(kind))}
        assertEquals("x",decodeMcpReturn("x",null,mcpRecaptureNote(McpVerificationTargetKind.Phone)+" ").first)
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

    private val afterBinding=WorkflowBinding(id("after-project"),id("after-document"),1uL,"e".repeat(64))

    @Test fun tunnelCaptureRouteIsGrantGatedAndRecapturesThroughTheSameSession()=runBlocking {
        val catalog=Catalog();val capture=Capture();val wire=Connection();val route=CoordinatorRoute()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
        try {
            owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),route))
            owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),route))
            val phone=owner.state.value.targets.single()
            assertEquals(McpVerificationTargetKind.Tunnel,phone.kind)
            val original=owner.publish(Compiled(info("tunnel-original"))).info
            assertNull(owner.state.value.message);assertTrue(owner.state.value.phoneUnbound.isEmpty())
            assertEquals(setOf("${original.packageId}/${original.target}"),owner.state.value.phoneScreens)
            // No grant for that phone screen: neither the indicator nor the phone is touched.
            try {owner.recaptureOriginal(session,original);fail("Ungranted phone recapture")}
            catch(_:McpRefused) { }
            assertEquals(0,route.captures);assertNull(owner.state.value.activeCapture)
            owner.grant(session,listOf(phone.selector),60_000)
            capture.result=info("tunnel-after").copy(binding=afterBinding)
            val pair=drawCapture(owner){owner.recaptureOriginal(session,original)}
            assertEquals(McpVerificationTargetKind.Tunnel,pair.kind)
            assertEquals(McpVerificationPackage.of(original),pair.original)
            assertEquals(McpVerificationPackage.of(capture.result),pair.recaptured)
            assertEquals(1,route.captures);assertEquals(1,capture.calls);assertEquals(2,wire.published.size)
            assertNull(owner.state.value.activeCapture)
        } finally {owner.close()}
    }

    @Test fun unavailablePhoneRouteRefusesWithItsReasonAndPublishesNothing()=runBlocking {
        for(reason in listOf(McpRecaptureRefusal.TunnelRuntimeUnavailable,McpRecaptureRefusal.PhoneDisconnected,
            McpRecaptureRefusal.FeedbackLoop,McpRecaptureRefusal.SourceBusy)) {
            val catalog=Catalog();val capture=Capture();val wire=Connection();val route=CoordinatorRoute()
            val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
            try {
                owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),route))
                val selector=owner.state.value.targets.single().selector
                val original=owner.publish(Compiled(info("unavailable-original"))).info
                owner.grant(session,listOf(selector),60_000)
                route.refusal=reason;capture.result=info("unavailable-after").copy(binding=afterBinding)
                val actual=try {drawCapture(owner){owner.recaptureOriginal(session,original)};null}
                    catch(error:McpRecaptureUnavailable) {error.reason}
                assertEquals(reason,actual)
                assertEquals(0,route.captures);assertEquals(0,capture.calls)
                assertEquals(1,catalog.values.size);assertEquals(1,wire.published.size)
                assertNull(owner.state.value.activeCapture)
            } finally {owner.close()}
        }
    }

    @Test fun aTunnelCaptureWhoseSessionIsGoneStaysATunnelCaptureNeedingRebind()=runBlocking {
        val catalog=Catalog().apply{platform="android"};val capture=Capture()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),Connection(),capture,newId={receiptId()})
        try {
            val route=CoordinatorRoute().apply{refusal=McpRecaptureRefusal.PhoneDisconnected}
            owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),route))
            assertTrue(owner.state.value.targets.isEmpty())
            val saved=owner.publish(Compiled(info("closed-session")))
            val itemKey="${saved.info.packageId}/${saved.info.target}"
            // Not silently reclassified as a phone-app capture, even with an android manifest.
            assertTrue(checkNotNull(owner.state.value.message).contains("Tunnel capture saved"))
            assertEquals(setOf(itemKey),owner.state.value.phoneUnbound);assertEquals(setOf(itemKey),owner.state.value.phoneScreens)
            // Ungranted agents learn nothing about the route state.
            try {owner.recaptureOriginal(session,saved.info);fail("Coded refusal reached an ungranted agent")}
            catch(_:McpRefused) { }
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            val disconnected=try {owner.recaptureOriginal(session,saved.info);null} catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.PhoneDisconnected,disconnected)
            capture.runtime=false
            val runtime=try {owner.recaptureOriginal(session,saved.info);null} catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.TunnelRuntimeUnavailable,runtime)
            assertEquals(0,route.captures);assertEquals(0,capture.calls);assertNull(owner.state.value.activeCapture)
        } finally {owner.close()}
    }

    @Test fun captureWindowNeverReachesAPhoneSelector()=runBlocking {
        val catalog=Catalog();val capture=Capture();val route=CoordinatorRoute()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),Connection(),capture,newId={receiptId()})
        try {
            owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),route))
            val phone=owner.state.value.targets.single()
            owner.grant(session,listOf(phone.selector),60_000)
            try {owner.capture(session,phone.selector);fail("capture_window reached the phone")}
            catch(_:McpRefused) { }
            assertEquals(0,route.captures);assertEquals(0,capture.calls)
            assertNull(owner.state.value.activeCapture);assertTrue(catalog.values.isEmpty())
        } finally {owner.close()}
    }

    @Test fun ownerAttestedPairingRefusesAnObviouslyDifferentScreenSize()=runBlocking {
        fun sized(name:String,width:UInt,height:UInt,source:WorkflowBinding=binding)=info(name).copy(binding=source,
            images=listOf(PackageImageInfo("clean","clean_source","images/clean.png",width,height,1uL)))
        val catalog=Catalog().apply{platform="android"};val capture=Capture();val wire=Connection();val route=CoordinatorRoute()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
        try {
            val published=owner.publish(Compiled(sized("sized-original",100u,200u)))
            owner.bindPhoneScreen(published,route)
            val phone=owner.state.value.targets.single()
            owner.grant(session,listOf(phone.selector),60_000)
            capture.result=sized("other-phone",300u,300u,afterBinding)
            val refused=try {drawCapture(owner){owner.recaptureOriginal(session,published.info)};null}
                catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.PhoneMismatch,refused)
            assertEquals(1,checkNotNull(capture.returned).closes)
            assertEquals(1,catalog.values.size);assertEquals(1,wire.published.size)
            // A rotation of the same screen is not a mismatch.
            capture.result=sized("rotated",200u,100u,afterBinding)
            val pair=drawCapture(owner){owner.recaptureOriginal(session,published.info)}
            assertEquals(McpVerificationTargetKind.Phone,pair.kind)
            assertEquals(2,route.captures)
        } finally {owner.close()}
    }

    @Test fun aRetiredTunnelRouteIsPrunedAndItsOriginReportsDisconnected()=runBlocking {
        val catalog=Catalog();val capture=Capture();val route=CoordinatorRoute()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),Connection(),capture,newId={receiptId()})
        try {
            owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),route))
            val phone=owner.state.value.targets.single()
            val saved=owner.publish(Compiled(info("pruned-original")))
            owner.grant(session,listOf(phone.selector),60_000)
            route.retired=true
            val refused=try {owner.recaptureOriginal(session,saved.info);null} catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.PhoneDisconnected,refused)
            assertTrue(owner.state.value.targets.isEmpty())
            assertEquals(setOf("${saved.info.packageId}/${saved.info.target}"),owner.state.value.phoneUnbound)
            assertEquals(0,route.captures);assertNull(owner.state.value.activeCapture)
            // The freed slot is usable again and the origin can be re-bound to a live session.
            val next=CoordinatorRoute()
            owner.bindPhoneScreen(saved,next)
            assertEquals(1,owner.state.value.targets.size);assertTrue(owner.state.value.phoneUnbound.isEmpty())
        } finally {owner.close()}
    }

    @Test fun tunnelDeliveryWaitsOnlyBoundedlyForABusyCaptureGate()=runBlocking {
        val catalog=Catalog();val capture=Capture();val route=CoordinatorRoute()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),Connection(),capture,newId={receiptId()})
        try {
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            capture.waitForever=true;capture.enter=CompletableDeferred()
            val blocked=async{runCatching{owner.capture(session,"chosen")}}
            val activity=withTimeout(5000){owner.state.map{it.activeCapture}.filterNotNull().first()}
            owner.indicatorDrawn(activity.id)
            withTimeout(5000){checkNotNull(capture.enter).await()}
            val started=System.nanoTime()
            withTimeout(10_000){owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),route))}
            assertTrue(System.nanoTime()-started<8_000_000_000L)
            blocked.cancel();withContext(NonCancellable){blocked.join()}
            capture.waitForever=false
            val saved=owner.publish(Compiled(info("busy-gate")))
            assertTrue("${saved.info.packageId}/${saved.info.target}" in owner.state.value.phoneUnbound)
            assertTrue(checkNotNull(owner.state.value.message).contains("Tunnel capture saved"))
        } finally {owner.close()}
    }

    @Test fun phoneAppCaptureNeedsAnExplicitOwnerBindingBeforeRecapture()=runBlocking {
        val catalog=Catalog().apply{platform="android"};val capture=Capture();val wire=Connection();val route=CoordinatorRoute()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
        try {
            val published=owner.publish(Compiled(info("phone-original")))
            val itemKey="${published.info.packageId}/${published.info.target}"
            assertEquals(setOf(itemKey),owner.state.value.phoneUnbound)
            assertTrue(checkNotNull(owner.state.value.message).contains("Bind the connected USB phone"))
            assertTrue(owner.state.value.targets.isEmpty())
            // Grant first: an ungranted agent gets only the ordinary refusal, on both tools.
            try {owner.recaptureOriginal(session,published.info);fail("Coded refusal reached an ungranted agent")}
            catch(_:McpRefused) { }
            try {owner.markerComparison(session,published.info.packageId,published.info.target,published.info.manifestSha256,receiptId(),"f".repeat(64),1u);fail("Comparison admitted")}
            catch(_:McpRefused) { }
            owner.selectTarget();owner.grant(session,listOf("chosen"),60_000)
            val unbound=try {owner.recaptureOriginal(session,published.info);null} catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.PhoneRouteUnbound,unbound)
            val compared=try {owner.markerComparison(session,published.info.packageId,published.info.target,published.info.manifestSha256,receiptId(),"f".repeat(64),1u);null}
                catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.PhoneRouteUnbound,compared)
            // Without the compiled tunnel runtime, the honest reason is the runtime, for agent and owner.
            capture.runtime=false
            val agentRuntime=try {owner.recaptureOriginal(session,published.info);null} catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.TunnelRuntimeUnavailable,agentRuntime)
            val ownerRuntime=try {owner.bindPhoneScreen(published,route);null} catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.TunnelRuntimeUnavailable,ownerRuntime)
            capture.runtime=true
            assertNull(owner.state.value.activeCapture)
            val disconnected=try {owner.bindPhoneScreen(published,null);null} catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.PhoneDisconnected,disconnected)
            route.refusal=McpRecaptureRefusal.TunnelRuntimeUnavailable
            val runtime=try {owner.bindPhoneScreen(published,route);null} catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.TunnelRuntimeUnavailable,runtime)
            assertEquals(setOf(itemKey),owner.state.value.phoneUnbound)
            assertTrue(owner.state.value.targets.none{it.kind==McpVerificationTargetKind.Tunnel})
            route.refusal=null
            owner.bindPhoneScreen(published,route)
            assertTrue(owner.state.value.phoneUnbound.isEmpty())
            assertTrue(checkNotNull(owner.state.value.message).contains("Grant that phone screen"))
            val phone=owner.state.value.targets.single{it.kind==McpVerificationTargetKind.Tunnel}
            // Binding alone grants nothing: the existing grant names only the window.
            try {owner.recaptureOriginal(session,published.info);fail("Binding minted a grant")}
            catch(_:IllegalArgumentException) { }
            assertEquals(0,route.captures)
            owner.grant(session,listOf(phone.selector),60_000)
            owner.bindPhoneScreen(published,route) // Same route: unchanged, but the message reflects the live grant.
            assertTrue(checkNotNull(owner.state.value.message).contains("already covers"))
            capture.result=info("phone-after").copy(binding=afterBinding)
            val pair=drawCapture(owner){owner.recaptureOriginal(session,published.info)}
            assertEquals(McpVerificationTargetKind.Phone,pair.kind)
            assertEquals(McpVerificationPackage.of(capture.result),pair.recaptured)
            assertEquals(1,route.captures);assertEquals(2,wire.published.size)
            assertTrue(owner.state.value.phoneUnbound.isEmpty())
        } finally {owner.close()}
    }

    @Test fun ownerMayRebindATunnelOriginalAfterItsSessionWasReplaced()=runBlocking {
        val catalog=Catalog();val capture=Capture();val wire=Connection();val first=CoordinatorRoute();val second=CoordinatorRoute()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),wire,capture,newId={receiptId()})
        try {
            owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),first))
            val published=owner.publish(Compiled(info("rebind-original")))
            assertEquals(setOf("${published.info.packageId}/${published.info.target}"),owner.state.value.phoneScreens)
            val firstSelector=owner.state.value.targets.single().selector
            owner.grant(session,listOf(firstSelector),60_000)
            first.refusal=McpRecaptureRefusal.PhoneDisconnected
            val stale=try {drawCapture(owner){owner.recaptureOriginal(session,published.info)};null}
                catch(error:McpRecaptureUnavailable) {error.reason}
            assertEquals(McpRecaptureRefusal.PhoneDisconnected,stale)
            owner.bindPhoneScreen(published,second)
            val secondSelector=owner.state.value.targets.single{it.selector!=firstSelector}.selector
            // The earlier grant names only the replaced session.
            try {owner.recaptureOriginal(session,published.info);fail("Old grant reached the new session")}
            catch(_:IllegalArgumentException) { }
            owner.grant(session,listOf(secondSelector),60_000)
            capture.result=info("rebind-after").copy(binding=afterBinding)
            val pair=drawCapture(owner){owner.recaptureOriginal(session,published.info)}
            assertEquals(McpVerificationTargetKind.Phone,pair.kind)
            assertEquals(0,first.captures);assertEquals(1,second.captures)
        } finally {owner.close()}
    }

    @Test fun onlyAndroidPlatformPackagesBecomePhoneOriginsAndRetirementClearsThePrompt()=runBlocking {
        val catalog=Catalog().apply{platform="windows"};val capture=Capture()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),Connection(),capture,newId={receiptId()})
        try {
            owner.publish(Compiled(info("windows-unrouted")))
            assertTrue(owner.state.value.phoneUnbound.isEmpty())
            assertTrue(checkNotNull(owner.state.value.message).contains("unavailable for verification"))
            catalog.platform="android"
            val phone=owner.publish(Compiled(info("phone-retired")))
            assertEquals(1,owner.state.value.phoneUnbound.size);assertEquals(1,owner.state.value.phoneScreens.size)
            owner.retirePackage(phone)
            assertTrue(owner.state.value.phoneUnbound.isEmpty());assertTrue(owner.state.value.phoneScreens.isEmpty())
            val refused=try {owner.bindPhoneScreen(phone,CoordinatorRoute());false} catch(_:Exception) {true}
            assertTrue(refused)
            // The refused binding released the route it retained: no hidden slot.
            assertTrue(owner.state.value.targets.isEmpty());assertEquals(0,capture.routeCount)
        } finally {owner.close()}
    }

    @Test fun aRefusedTunnelRegistrationReleasesItsRouteSlot()=runBlocking {
        val catalog=Catalog();val capture=Capture();val closed=CoordinatorRoute();val live=CoordinatorRoute()
        val owner=DesktopMcpCoordinator(catalog,Inbox(),Connection(),capture,newId={receiptId()})
        try {
            // The first delivery found its session closed: an unbound tunnel origin.
            closed.refusal=McpRecaptureRefusal.PhoneDisconnected
            owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),closed))
            assertTrue(owner.state.value.targets.isEmpty())
            // A later live session cannot silently rebind that origin; its retained
            // route is forgotten and the capture stays an unbound tunnel capture.
            owner.rememberPhoneCapture(McpPhoneCaptureOrigin(binding,"d".repeat(64),live))
            assertTrue(owner.state.value.targets.isEmpty());assertEquals(0,capture.routeCount)
            val saved=owner.publish(Compiled(info("refused-registration")))
            assertTrue("${saved.info.packageId}/${saved.info.target}" in owner.state.value.phoneUnbound)
            // The owner can still bind it explicitly to the live session.
            owner.bindPhoneScreen(saved,live)
            assertEquals(1,owner.state.value.targets.size);assertEquals(1,capture.routeCount)
            assertTrue(owner.state.value.phoneUnbound.isEmpty())
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

/** Stands in for one exact owner-opened tunnel session. The coordinator sees
 * only availability/identity; this fake engine performs its pretend capture. */
private class CoordinatorRoute:McpPhoneScreenRoute {
    var refusal:McpRecaptureRefusal?=null;var captures=0
    override var retired=false
    override val description="USB phone fixture"
    override fun checkAvailable(){refusal?.let{throw McpRecaptureUnavailable(it)}}
    suspend fun <T> capture(body:suspend()->T):T {checkAvailable();captures++;return body()}
}
