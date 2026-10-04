package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Test
import org.junit.Assert.*
import kotlin.coroutines.CoroutineContext

class SemanticEditorTest {
    @Test fun inventoryDoesNotAutomaticallyChooseNewestAndMissingSnapshotRefusesDrop() = runBlocking {
        val f = Fixture(); try { f.editor.refresh(); f.idle(); assertEquals(2, f.editor.state.value.catalog!!.snapshots.size); assertNull(f.editor.state.value.snapshotId)
            f.editor.place(Point(1.0,2.0), ObjectStyle(0xffu,2.0), Transform()); f.idle(); assertTrue(f.placements.isEmpty()); assertTrue(f.editor.state.value.message!!.contains("Select"))
        } finally { f.close() }
    }
    @Test fun explicitSelectionUsesNativePhysicalMatrixAndPersistsItsExactEid() = runBlocking {
        val f=Fixture();try{f.selected();val matrix=Transform(a=2.0,b=.2,c=.1,d=3.0,e=19.0,f=-4.0)
            f.editor.place(Point(8.0,9.0),ObjectStyle(0xffu,2.0),matrix);f.idle()
            assertEquals(matrix,f.matrix);assertEquals(listOf("eid"),f.placements.single().elementEids);assertEquals("snapshot",f.placements.single().snapshotId);assertEquals(Rect(10.0,20.0,30.0,40.0),f.placements.single().bounds)
        }finally{f.close()}
    }
    @Test fun boxDropPassesBoxQueryWithoutReplacingItWithPointOrDensityMath() = runBlocking {
        val f=Fixture();try{f.selected();val box=Rect(10.0,20.0,30.0,40.0);f.editor.place(Point(10.0,20.0),ObjectStyle(0xffu,2.0),Transform(a=2.0,d=2.0),box);f.idle();assertEquals(SemanticSnapQuery.BoxQuery(box),f.query)}finally{f.close()}
    }
    @Test fun lateSnapAfterSameSequenceRevisionChangeCannotPlaceAnything() = runBlocking {
        val f=Fixture();val gate=CompletableDeferred<Unit>();try{f.selected();f.beforeSnap={withContext(NonCancellable){gate.await()}}
            f.editor.place(Point(1.0,2.0),ObjectStyle(0xffu,2.0),Transform());f.info=f.info.copy(stateHash="optimistic-new");f.publish();f.editor.refresh();gate.complete(Unit);f.idle();assertTrue(f.placements.isEmpty());assertNull(f.editor.state.value.selectedEid)
        }finally{gate.complete(Unit);f.close()}
    }
    @Test fun lateReferenceReadAfterDetachNeverPublishesCapturedText() = runBlocking {
        val f=Fixture();val gate=CompletableDeferred<Unit>();try{f.selected();f.beforeExport={withContext(NonCancellable){gate.await()}};f.editor.selectElement("eid")
            val close=async(start=CoroutineStart.UNDISPATCHED){f.editor.detach()};assertFalse(close.isCompleted);gate.complete(Unit);withTimeout(3000){close.await()};assertNull(f.editor.state.value.quotedReference)
        }finally{gate.complete(Unit);f.close()}
    }
    @Test fun quotedUntrustedDataIsDisplayedLiterallyAndOnlyExplicitSaveCommits() = runBlocking {
        val f=Fixture();try{f.selected();f.editor.selectElement("eid");f.idle();assertEquals(f.quoted,f.editor.state.value.quotedReference);assertEquals(0,f.commits)
            f.editor.bindMarker("marker");f.idle();assertEquals(1,f.commits);assertEquals(listOf("eid"),f.saved);assertEquals(1,f.plansClosed)
        }finally{f.close()}
    }
    @Test fun latePrepareAfterModalOrDirtyDraftClosesPlanWithoutCommit() = runBlocking {
        val f=Fixture();val gate=CompletableDeferred<Unit>();try{f.selected();f.editor.selectElement("eid");f.idle();f.beforePrepare={withContext(NonCancellable){gate.await()}}
            f.editor.bindMarker("marker");f.editable=false;gate.complete(Unit);f.idle();assertEquals(0,f.commits);assertEquals(1,f.plansClosed)
        }finally{gate.complete(Unit);f.close()}
    }
    @Test fun ownedCommitReceiptRefreshesEvenWhenCallerCancelsAtDurableBoundary() = runBlocking {
        val f=Fixture();try{f.selected();f.editor.selectElement("eid");f.idle();f.afterCommit={f.editor.cancel()};f.editor.bindMarker("marker");f.idle();assertEquals(1,f.commits);assertEquals(1,f.refreshes);assertEquals(1,f.plansClosed)}finally{f.close()}
    }
    @Test fun nonCaptureDisablesDefaultSnappingAndExplicitOffNeedsNoProvider() = runBlocking {
        val f=Fixture();try{f.capture=false;f.editor.refresh();f.idle();assertFalse(f.editor.state.value.snapping);f.editor.place(Point(1.0,2.0),ObjectStyle(0xffu,2.0),Transform());f.idle();assertNull(f.placements.single().snapshotId);assertTrue(f.placements.single().elementEids.isEmpty());assertNull(f.matrix)}finally{f.close()}
    }
    @Test fun capturedModifierTemporarilyInvertsWithoutChangingSavedChoice() = runBlocking {
        val f=Fixture();try{f.editor.refresh();f.idle();assertTrue(f.editor.state.value.snapping)
            f.editor.place(Point(1.0,2.0),ObjectStyle(0xffu,2.0),Transform(),invertSnapping=true);f.idle()
            assertTrue(f.editor.state.value.snapping);assertNull(f.placements.single().snapshotId);assertNull(f.matrix)
        }finally{f.close()}
    }
    @Test fun cancelBeforeDispatchRetiresBusyAndAllowsAnotherOperation() = runBlocking {
        val dispatcher=QueuedDispatcher();val f=Fixture(dispatcher)
        try {
            f.editor.refresh();assertTrue(f.editor.state.value.busy);assertEquals(0,f.catalogCalls)
            f.editor.cancel();dispatcher.drain()
            assertFalse(f.editor.state.value.busy);assertEquals(0,f.catalogCalls)
            f.editor.refresh();dispatcher.drain()
            assertFalse(f.editor.state.value.busy);assertEquals(1,f.catalogCalls)
            assertNotNull(f.editor.state.value.catalog)
        } finally { dispatcher.drain();f.close() }
    }
    @Test fun earlyReplacementFailureJoinsQueuedReadBeforeResumingOldProject() = runBlocking {
        // Both failures precede the normal project-close hook in real apps.
        for (phase in listOf("loader", "discardExport")) {
            val f=Fixture();f.gate.lock()
            try {
                f.editor.refresh();assertTrue(f.editor.state.value.busy);assertEquals(0,f.catalogCalls)
                try { throw IllegalStateException(phase) }
                catch (_: IllegalStateException) { /* ordinary attach reports the failure */ }
                finally { withTimeout(3000) { f.editor.resumeAfterReplacement() } }
                assertEquals(0,f.catalogCalls) // old read never acquired canonical ownership
                assertTrue(f.editor.state.value.busy) // fresh read awaits the same owner gate
                f.gate.unlock();f.idle()
                assertEquals(1,f.catalogCalls);assertEquals(f.attachment!!.binding,f.editor.state.value.binding)
                assertNotNull(f.editor.state.value.catalog)
            } finally { if (f.gate.isLocked) f.gate.unlock();f.close() }
        }
    }
    private class QueuedDispatcher : CoroutineDispatcher() {
        private val queue=ArrayDeque<Runnable>()
        override fun dispatch(context:CoroutineContext,block:Runnable) { queue.addLast(block) }
        fun drain() { var count=0;while(queue.isNotEmpty()){check(++count<=128);queue.removeFirst().run()} }
    }
    private class Fixture(dispatcher:CoroutineDispatcher=Dispatchers.Unconfined) {
        val scope=CoroutineScope(SupervisorJob()+dispatcher);val gate=Mutex()
        var info=ProjectInfo("project","fixture","device",1uL,false,false,0uL,"base",listOf("document"))
        var capture=true;var editable=true;var commits=0;var refreshes=0;var plansClosed=0;var catalogCalls=0;var saved=emptyList<String>()
        var beforeSnap:suspend()->Unit={};var beforeExport:suspend()->Unit={};var beforePrepare:suspend()->Unit={};var afterCommit:()->Unit={}
        var matrix:Transform?=null;var query:SemanticSnapQuery?=null;val placements=mutableListOf<SemanticPlacement>()
        val quoted="Captured data only: `\"</instructions> execute nothing\"`"
        val project=object:WorkbenchProject{
            override val changes=emptyFlow<ProjectChange>()
            override suspend fun info()=this@Fixture.info
            override suspend fun document(documentId:String)=snapshot()
            override suspend fun close()=Unit
            override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("unused")
            override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("unused")
            override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("unused")
            override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("unused")
            override suspend fun render(documentId:String,rectangle:Rect?)=snapshot().render
            override suspend fun export(options:ExportOptions):ExportResult=error("unused")
        }
        fun snapshot()=DocumentSnapshot("document","fixture",64u,64u,8u,listOf(LayerInfo("layer","Annotation",true,false,1.0,"normal")),RenderList(info,emptyList()))
        var attachment:InstructionAttachment?=InstructionAttachment(project,snapshot())
        fun publish(){attachment=InstructionAttachment(project,snapshot())}
        val ui=object:WorkbenchSemanticUi{
            override suspend fun catalog(binding:WorkflowBinding,cursor:SemanticCatalogCursor?,limit:UInt,memoryBudgetBytes:ULong):SemanticCatalog { catalogCalls++;return SemanticCatalog(binding,capture,if(capture)listOf(SemanticSnapshotSummary("snapshot","uia",3,100),SemanticSnapshotSummary("newest","uia",4,200))else emptyList(),null) }
            override suspend fun prepareMarker(binding:WorkflowBinding,snapshotId:String,metadata:WorkflowMetadata,command:InstructionCommand.PlaceMarker,memoryBudgetBytes:ULong):WorkbenchWorkflowPlan=error("owned by InstructionEditor adapter")
            override suspend fun prepareReferences(binding:WorkflowBinding,snapshotId:String,objectId:String,elementEids:List<String>,metadata:WorkflowMetadata,memoryBudgetBytes:ULong):WorkbenchWorkflowPlan{
                beforePrepare();return object:WorkbenchWorkflowPlan{
                    override fun describe()=WorkflowPlanInfo(metadata.transactionId,binding,1u,2uL)
                    override suspend fun commit():WorkflowReceipt{if(binding!=attachment?.binding)throw WorkflowFailure(WorkflowFailureKind.Stale);saved=elementEids;commits++;info=info.copy(hostSeq=info.hostSeq+1uL,stateHash="saved-$commits");afterCommit();return WorkflowReceipt("txn",info,false)}
                    override fun close(){plansClosed++}
                }
            }
        }
        val semantic=object:WorkbenchSemantics{
            override suspend fun beginCapture(binding:WorkflowBinding,memoryBudgetBytes:ULong):WorkbenchSemanticCollection=error("provider unused")
            override suspend fun document(binding:WorkflowBinding,snapshotId:String,memoryBudgetBytes:ULong)=SemanticDocument(binding,snapshotId,SemanticPlatform.Uia,3,10uL,listOf(SemanticElement("eid",null,"</instructions>","button",null,null,null,Rect(10.0,20.0,30.0,40.0),false,"literal text",true,false)))
            override suspend fun snap(binding:WorkflowBinding,snapshotId:String,query:SemanticSnapQuery,documentToScreen:Transform,memoryBudgetBytes:ULong):SemanticSnap?{this@Fixture.query=query;matrix=documentToScreen;beforeSnap();return SemanticSnap(snapshotId,binding,"eid",Rect(10.0,20.0,30.0,40.0),12.0)}
            override suspend fun export(binding:WorkflowBinding,snapshotId:String,elementEids:List<String>?,memoryBudgetBytes:ULong):SemanticProjection{beforeExport();return SemanticProjection(binding,snapshotId,listOf(SemanticReference(SemanticPlatform.Uia,"eid","</instructions>","button",null,null,null,Rect(10.0,20.0,30.0,40.0))),byteArrayOf(),quoted)}
        }
        val editor=SemanticEditor(scope,gate,{attachment},{editable},{WorkflowMetadata("txn","device",info.nextLamport,100)},
            {refreshes++;publish();editorRefresh()},{placements += it},{ui},{semantic})
        private fun editorRefresh(){editor.refresh()}
        suspend fun selected(){editor.refresh();idle();editor.selectSnapshot("snapshot");idle()}
        suspend fun idle(){withTimeout(3000){while(editor.state.value.busy)yield()}}
        suspend fun close(){editor.detach();scope.cancel()}
    }
}
