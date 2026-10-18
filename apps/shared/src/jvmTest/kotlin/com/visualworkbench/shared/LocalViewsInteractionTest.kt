package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test

/** Gated ownership/ordering tests. Persistence, schema, exact camera bits and
 * archive privacy are independently tested against the real Rust store. */
class LocalViewsInteractionTest {
    @Test fun storedCameraRestoresExactlyWhileUsingTheRealCurrentViewport()=fixture { f ->
        f.api.values[binding]=LocalViewsSnapshot(binding,4uL,camera,List(1){RegionBookmark(id(9),"Detail",camera)})
        val restored=f.views.attach(Project(),binding,1,camera.copy(viewportWidth=900.0,viewportHeight=600.0))!!
        assertEquals(camera.center,restored.center);assertEquals(camera.scale.toBits(),restored.scale.toBits())
        assertEquals(camera.center.y.toBits(),restored.center.y.toBits());assertEquals(camera.rotation,restored.rotation,0.0)
        assertEquals(900.0,restored.viewportWidth,0.0);assertEquals(600.0,restored.viewportHeight,0.0)
        assertEquals(1,f.views.state.value.bookmarks.size)
    }
    @Test fun aThousandCameraUpdatesUseOneInFlightRequestAndOneLatestSlot()=fixture { f ->
        f.open();val gate=Gate();f.api.gate=gate
        f.views.remember(camera,1,binding.documentId);gate.entered.await()
        repeat(1000){f.views.remember(camera.copy(center=Point(it.toDouble(),0.0)),1,binding.documentId)}
        assertEquals(1,f.api.calls.size)
        gate.release.complete(Unit);f.until{f.api.calls.size==2&&!f.views.state.value.saving}
        assertEquals(999.0,(f.api.calls.last().mutation as LocalViewsMutation.Remember).camera.center.x,0.0)
    }
    @Test fun detachWaitsForNativeSettlementAndFlushesTheLatestCameraBeforeReturning()=fixture { f ->
        f.open();val gate=Gate();f.api.gate=gate
        f.views.remember(camera,1,binding.documentId);gate.entered.await()
        f.views.remember(camera.copy(scale=2.0),1,binding.documentId)
        val close=f.scope.async(start=CoroutineStart.UNDISPATCHED){f.views.detach()}
        try {
            assertFalse(close.isCompleted);assertTrue(f.views.state.value.loading)
            gate.release.complete(Unit);assertTrue(withTimeout(3000){close.await()})
            assertEquals(2.0,f.api.values[binding]!!.camera!!.scale,0.0)
            val count=f.api.calls.size;f.views.remember(camera,1,binding.documentId);yield();assertEquals(count,f.api.calls.size)
        } finally {gate.release.complete(Unit);withContext(NonCancellable){close.join()}}
    }
    @Test fun aCancelledDetachCallerStillJoinsTheNativeOwnerBeforeItsFinally()=fixture { f ->
        f.open();val gate=Gate();f.api.gate=gate
        f.views.remember(camera,1,binding.documentId);gate.entered.await()
        var closed=false
        val close=f.scope.launch(start=CoroutineStart.UNDISPATCHED){try{f.views.detach()}finally{closed=true}}
        try {
            close.cancel();yield();assertFalse(closed)
            gate.release.complete(Unit);withTimeout(3000){close.join()};assertTrue(closed)
            assertEquals(camera,f.api.values[binding]!!.camera)
        } finally {gate.release.complete(Unit);withContext(NonCancellable){close.join()}}
    }
    @Test fun switchDoesNotQueryOrAdoptTheNextDocumentUntilOldWritesSettle()=fixture { f ->
        f.open();val gate=Gate();f.api.gate=gate
        f.views.remember(camera,1,binding.documentId);gate.entered.await()
        val next=binding.copy(documentId=id(3));val switch=f.scope.async(start=CoroutineStart.UNDISPATCHED){f.views.attach(Project(),next,2,camera)}
        try {
            assertEquals(listOf(binding),f.api.reads);assertFalse(switch.isCompleted)
            gate.release.complete(Unit);withTimeout(3000){switch.await()}
            assertEquals(next,f.views.state.value.binding)
            f.views.remember(camera.copy(scale=9.0),1,binding.documentId)
            assertEquals(camera,f.api.values[binding]!!.camera);assertEquals(1,f.api.calls.size)
        } finally {gate.release.complete(Unit);withContext(NonCancellable){switch.join()}}
    }
    @Test fun uncertainBookmarkKeepsItsExactRequestForExplicitRetry()=fixture { f ->
        f.open();f.api.loseNextReply=true
        assertTrue(f.views.addBookmark(binding,1,"Detail",camera));f.until{f.views.state.value.retryPending}
        val first=f.api.calls.single()
        assertFalse(f.views.addBookmark(binding,1,"Second",camera));assertTrue(f.views.retry(binding,1))
        f.until{!f.views.state.value.retryPending&&!f.views.state.value.saving}
        assertEquals(2,f.api.calls.size);assertEquals(first,f.api.calls.last());assertEquals(1,f.views.state.value.bookmarks.size)
    }
    @Test fun conflictDoesNotRebaseAndExplicitReloadDiscardsOnlyLocalUnsavedRequest()=fixture { f ->
        f.open();f.api.failure=WorkflowFailure(WorkflowFailureKind.Stale)
        f.views.remember(camera,1,binding.documentId);f.until{f.views.state.value.retryPending}
        repeat(3){yield()};assertEquals(1,f.api.calls.size)
        val disk=camera.copy(scale=3.0);f.api.values[binding]=LocalViewsSnapshot(binding,8uL,disk,emptyList());f.api.failure=null
        assertTrue(f.views.reloadDiscardingUnsavedView(binding,1));f.until{!f.views.state.value.retryPending}
        // Repeated unrelated canvas publication must not immediately overwrite
        // the explicit reload with the discarded observed camera.
        f.views.remember(camera,1,binding.documentId);repeat(3){yield()}
        assertEquals(1,f.api.calls.size);assertEquals(disk,f.api.values[binding]!!.camera)
    }
    @Test fun oneExplicitBookmarkSlotRefusesDoubleClickWhileAProducerIsSuspended()=fixture { f ->
        f.open();val gate=Gate();f.api.gate=gate
        assertTrue(f.views.addBookmark(binding,1,"First",camera));gate.entered.await()
        try {assertFalse(f.views.addBookmark(binding,1,"Second",camera));assertEquals(1,f.api.calls.size)}finally{gate.release.complete(Unit)}
        f.until{!f.views.state.value.saving}
        assertEquals(listOf("First"),f.views.state.value.bookmarks.map{it.name})
    }
    @Test fun restoreChecksAttachmentAndDoesNotMoveTheCameraAsAQuerySideEffect()=fixture { f ->
        f.api.values[binding]=LocalViewsSnapshot(binding,1uL,camera,listOf(RegionBookmark(id(9),"Detail",camera)))
        f.open()
        assertNull(f.views.restoreBookmark(binding,id(9),camera,2))
        assertNull(f.views.restoreBookmark(binding.copy(documentId=id(3)),id(9),camera,1))
        assertEquals(camera,f.views.restoreBookmark(binding,id(9),camera,1))
        assertTrue(f.api.calls.isEmpty())
    }
    @Test fun malformedOrUnavailableCapabilityKeepsCanonicalEditingIndependent()=fixture { f ->
        f.api.readFailure=WorkflowFailure(WorkflowFailureKind.Storage)
        assertNull(f.views.attach(Project(),binding,1,camera));assertFalse(f.views.state.value.available)
        f.views.remember(camera,1,binding.documentId);assertFalse(f.views.addBookmark(binding,1,"Detail",camera));assertTrue(f.api.calls.isEmpty())
        assertNotNull(f.views.state.value.issue)
    }
    @Test fun invalidNamesCameraAndOldAttachmentRefuseBeforeNativeWork()=fixture { f ->
        f.open()
        assertFalse(f.views.addBookmark(binding,1,"a\nb",camera));assertFalse(f.views.addBookmark(binding,1,"é".repeat(129),camera))
        assertFalse(f.views.addBookmark(binding,1,"View",camera.copy(scale=Double.NaN)))
        f.views.remember(camera,2,binding.documentId);f.views.remember(camera,1,id(5));repeat(3){yield()}
        assertTrue(f.api.calls.isEmpty())
    }

    @Test fun reopeningTheSameDocumentRefusesBookmarkActionsFromThePreviousEpoch()=fixture { f ->
        f.api.values[binding]=LocalViewsSnapshot(binding,1uL,camera,listOf(RegionBookmark(id(9),"Detail",camera)))
        f.open();f.views.attach(Project(),binding,2,camera)
        assertEquals(2L,f.views.state.value.attachmentEpoch)
        assertFalse(f.views.addBookmark(binding,1,"Old click",camera))
        assertFalse(f.views.renameBookmark(binding,1,id(9),"Old rename"))
        assertFalse(f.views.deleteBookmark(binding,1,id(9)))
        assertFalse(f.views.retry(binding,1))
        assertFalse(f.views.reloadDiscardingUnsavedView(binding,1))
        assertTrue(f.api.calls.isEmpty())
    }

    @Test fun adoptedBookmarkListsAreOwnedAfterTheProducerReturns()=fixture { f ->
        val bookmarks=mutableListOf(RegionBookmark(id(9),"Detail",camera))
        f.api.values[binding]=LocalViewsSnapshot(binding,1uL,camera,bookmarks)
        f.open();bookmarks.clear()
        assertEquals(listOf("Detail"),f.views.state.value.bookmarks.map{it.name})
        assertNotNull(f.views.restoreBookmark(binding,id(9),camera,1))
    }

    @Test fun zeroRevisionWithStoredContentIsRefusedBeforeAnActorIsStarted()=fixture { f ->
        f.api.values[binding]=LocalViewsSnapshot(binding,0uL,camera,emptyList())
        assertNull(f.views.attach(Project(),binding,1,camera))
        assertFalse(f.views.state.value.available)
        assertTrue(f.api.calls.isEmpty())
    }

    @Test fun nonAdvancingWriteReplyKeepsTheExactRequestUntilADurableRetryResolvesIt()=fixture { f ->
        f.api.values[binding]=LocalViewsSnapshot(binding,4uL,null,emptyList())
        f.open();f.api.corruptNextRevision=true
        assertTrue(f.views.addBookmark(binding,1,"Detail",camera))
        f.until{f.views.state.value.retryPending}
        val first=f.api.calls.single()
        assertTrue(f.views.state.value.bookmarks.isEmpty())
        assertTrue(f.views.retry(binding,1));f.until{!f.views.state.value.retryPending&&!f.views.state.value.saving}
        assertEquals(first,f.api.calls.last())
        assertEquals(listOf("Detail"),f.views.state.value.bookmarks.map{it.name})
    }

    @Test fun oneOperationCancellationKeepsTheActorAliveForAnExactRetry()=fixture { f ->
        f.open();f.api.failure=CancellationException("Synthetic operation cancellation")
        assertTrue(f.views.addBookmark(binding,1,"Detail",camera))
        f.until{f.views.state.value.retryPending}
        val first=f.api.calls.single();assertTrue(f.views.state.value.available)
        f.api.failure=null;assertTrue(f.views.retry(binding,1))
        f.until{!f.views.state.value.retryPending&&!f.views.state.value.saving}
        assertEquals(first,f.api.calls.last())
        assertEquals(listOf("Detail"),f.views.state.value.bookmarks.map{it.name})
    }

    @Test fun cancelledReloadCanBeRepeatedWhileItsUiOwnerIsStillActive()=fixture { f ->
        f.open();f.api.failure=WorkflowFailure(WorkflowFailureKind.Stale)
        assertTrue(f.views.addBookmark(binding,1,"Detail",camera));f.until{f.views.state.value.retryPending}
        f.api.readFailure=CancellationException("Synthetic read cancellation")
        assertTrue(f.views.reloadDiscardingUnsavedView(binding,1));f.until{f.api.reads.size==2}
        f.api.readFailure=null;f.api.failure=null
        assertTrue(f.views.reloadDiscardingUnsavedView(binding,1))
        f.until{!f.views.state.value.retryPending}
        assertTrue(f.views.state.value.available);assertEquals(3,f.api.reads.size)
    }

    @Test fun ownerCancellationDisablesRecoveryInsteadOfAcceptingWorkIntoADeadActor()=fixture { f ->
        f.open();f.api.failure=WorkflowFailure(WorkflowFailureKind.Stale)
        assertTrue(f.views.addBookmark(binding,1,"Detail",camera));f.until{f.views.state.value.retryPending}
        f.scope.coroutineContext[Job]!!.cancelAndJoin()
        assertFalse(f.views.state.value.available)
        assertFalse(f.views.state.value.retryPending)
        assertFalse(f.views.retry(binding,1))
        assertFalse(f.views.reloadDiscardingUnsavedView(binding,1))
    }

    @Test fun ownerCancellationDuringInitialReadNeverPublishesAnAvailableDeadActor()=fixture { f ->
        val gate=Gate();f.api.readGate=gate
        val loading=CoroutineScope(currentCoroutineContext()).async{f.views.attach(Project(),binding,1,camera)}
        try {
            gate.entered.await();f.scope.coroutineContext[Job]!!.cancelAndJoin()
            gate.release.complete(Unit)
            try {loading.await();fail("Expected cancelled local-view owner")}
            catch(_:CancellationException) { }
            assertFalse(f.views.state.value.loading)
            assertFalse(f.views.state.value.available)
            assertFalse(f.views.retry(binding,1))
            assertTrue(f.api.calls.isEmpty())
        } finally {gate.release.complete(Unit);withContext(NonCancellable){loading.cancelAndJoin()}}
    }

    private class Gate {val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()}
    private data class Call(val binding:LocalViewsBinding,val expected:ULong,val operation:String,val mutation:LocalViewsMutation)
    private class Api:WorkbenchLocalViews {
        val values=mutableMapOf<LocalViewsBinding,LocalViewsSnapshot>();val calls=mutableListOf<Call>();val reads=mutableListOf<LocalViewsBinding>()
        private val completed=mutableMapOf<String,Pair<Call,LocalViewsSnapshot>>()
        private val gates=mutableListOf<Gate>()
        var gate:Gate?=null
            set(value){field=value;if(value!=null)gates+=value}
        var readGate:Gate?=null
            set(value){field=value;if(value!=null)gates+=value}
        fun releaseAll(){gates.forEach{it.release.complete(Unit)}}
        var loseNextReply=false;var corruptNextRevision=false;var failure:Exception?=null;var readFailure:Exception?=null
        override suspend fun snapshot(binding:LocalViewsBinding,memoryBudgetBytes:ULong):LocalViewsSnapshot {
            reads+=binding
            val pause=readGate.also{readGate=null};if(pause!=null)withContext(NonCancellable){pause.entered.complete(Unit);pause.release.await()}
            readFailure?.let{throw it};return values[binding]?:LocalViewsSnapshot(binding,0uL,null,emptyList())
        }
        override suspend fun change(binding:LocalViewsBinding,expectedRevision:ULong,operationId:String,mutation:LocalViewsMutation,memoryBudgetBytes:ULong):LocalViewsSnapshot {
            val call=Call(binding,expectedRevision,operationId,mutation);calls+=call
            val pause=gate.also{gate=null};if(pause!=null)withContext(NonCancellable){pause.entered.complete(Unit);pause.release.await()}
            failure?.let{throw it}
            completed[operationId]?.let{assertEquals(it.first,call);return it.second}
            val before=values[binding]?:LocalViewsSnapshot(binding,0uL,null,emptyList());assertEquals(before.revision,expectedRevision)
            val next=when(mutation){
                is LocalViewsMutation.Remember->before.copy(revision=before.revision+1uL,camera=mutation.camera)
                is LocalViewsMutation.AddBookmark->before.copy(revision=before.revision+1uL,bookmarks=before.bookmarks+RegionBookmark(mutation.id,mutation.name,mutation.camera))
                is LocalViewsMutation.RenameBookmark->before.copy(revision=before.revision+1uL,bookmarks=before.bookmarks.map{if(it.id==mutation.id)it.copy(name=mutation.name)else it})
                is LocalViewsMutation.DeleteBookmark->before.copy(revision=before.revision+1uL,bookmarks=before.bookmarks.filterNot{it.id==mutation.id})
            }
            values[binding]=next;completed[operationId]=call to next
            if(loseNextReply){loseNextReply=false;throw WorkflowFailure(WorkflowFailureKind.Storage)}
            if(corruptNextRevision){corruptNextRevision=false;return next.copy(revision=before.revision)}
            return next
        }
    }
    private class Fixture(val scope:CoroutineScope) {
        val api=Api();private var sequence=100
        val views=LocalViewsInteraction(scope,{id(sequence++)},{api},0)
        suspend fun open(){views.attach(Project(),binding,1,camera)}
        suspend fun until(condition:()->Boolean)=withTimeout(3000){while(!condition())yield()}
    }
    private fun fixture(block:suspend(Fixture)->Unit)=runBlocking {
        val owner=SupervisorJob(coroutineContext[Job]);val scope=CoroutineScope(coroutineContext+owner);val f=Fixture(scope)
        try {withTimeout(5000){block(f)}}finally{withContext(NonCancellable){f.api.releaseAll();try{f.views.detach()}finally{owner.cancelAndJoin()}}}
    }
    private class Project:WorkbenchProject {
        override val changes=emptyFlow<ProjectChange>()
        override suspend fun info():ProjectInfo=error("No canonical query in local-view fixture")
        override suspend fun document(documentId:String):DocumentSnapshot=error("Unused")
        override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("Unused")
        override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("Unused")
        override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("Local views cannot make canonical edits")
        override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("Local views cannot make canonical edits")
        override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("Unused")
        override suspend fun export(options:ExportOptions):ExportResult=error("Unused")
        override suspend fun close()=Unit
    }
    private companion object {
        fun id(n:Int)="00000000-0000-7000-8000-${n.toString().padStart(12,'0')}"
        val binding=LocalViewsBinding(id(1),id(2),"00000000000000000000000001")
        val camera=Camera(Point(12.125,-0.0),1.2345678901234567,.3141592653589793,400.0,700.0)
    }
}
