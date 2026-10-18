package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.junit.Assert.*
import org.junit.Test

/** Integration of the exact borrowed leaves. No native/Compose/device proof. */
class EditorWorkflowsTest {
    private fun id(n:Int)="00000000-0000-7000-8000-"+n.toString().padStart(12,'0')
    private inner class Fixture(parent:CoroutineScope) {
        val scope=CoroutineScope(parent.coroutineContext+SupervisorJob(parent.coroutineContext[Job]))
        val gate=Mutex();var epoch=7L;var sequence=0uL;var idle=true;var adoptions=0;var closedProjects=0
        var layerFactories=0;var reviewFactories=0;var pageReads=0
        var currentCamera=Camera(Point(20.0,30.0),1.0,0.0,320.0,240.0)
        var savedCamera:Camera?=null
        var document=DocumentSnapshot(id(1),"Document",640u,480u,8u,listOf(LayerInfo(id(90),"Ink",true,false,1.0,"normal")),
            RenderList(ProjectInfo(id(80),"Project","installation",1u,false,false,1u,"a".repeat(64),(1..24).map(::id)),emptyList()))
        var onRead:suspend()->Unit={}
        val localBindings=mutableListOf<LocalViewsBinding>();val messages=mutableListOf<String>()
        val project=object:WorkbenchProject {
            override val changes=emptyFlow<ProjectChange>()
            override suspend fun info():ProjectInfo=this@Fixture.document.render.revision
            override suspend fun document(documentId:String):DocumentSnapshot {
                val captured=this@Fixture.document.copy(documentId=documentId);onRead();return captured
            }
            override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("unused")
            override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("unused")
            override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("coordinator must not edit canonical state")
            override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("unused")
            override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("unused")
            override suspend fun export(options:ExportOptions):ExportResult=error("unused")
            override suspend fun close(){closedProjects++}
        }
        val owner=EditorWorkflows(scope,gate,{attached()},{idle},{},
            {WorkflowMetadata(id(99),"installation",1u,1L)},{id(99)},
            {currentCamera},{currentCamera=it},{_,_->},{_,_->},
            {a,target->check(gate.isLocked);check(a.project===project);adoptions++;epoch++;document=target},
            {messages+=it},
            {object:WorkbenchLocalViews {
                override suspend fun snapshot(binding:LocalViewsBinding,memoryBudgetBytes:ULong):LocalViewsSnapshot {
                    localBindings+=binding;return LocalViewsSnapshot(binding,if(savedCamera==null)0u else 1u,savedCamera,emptyList())
                }
                override suspend fun change(binding:LocalViewsBinding,expectedRevision:ULong,operationId:String,
                    mutation:LocalViewsMutation,memoryBudgetBytes:ULong):LocalViewsSnapshot {
                    val value=when(mutation){is LocalViewsMutation.Remember->mutation.camera;else->error("unused")}
                    return LocalViewsSnapshot(binding,expectedRevision+1u,value,emptyList())
                }
            }},
            {layerFactories++;object:WorkbenchDocumentControls {
                override suspend fun snapshot(documentId:String,memoryBudgetBytes:ULong):DocumentControlSnapshot=error("unused")
                override suspend fun prepare(binding:WorkflowBinding,metadata:WorkflowMetadata,command:DocumentControlCommand,memoryBudgetBytes:ULong):WorkbenchWorkflowPlan=error("unused")
            }},
            {reviewFactories++;object:WorkbenchOfflineReview {
                override suspend fun page(afterRow:Long,limit:UInt,memoryBudgetBytes:ULong):OfflineReviewPage {
                    pageReads++;return OfflineReviewPage(document.render.revision.offlineReviewBinding(),emptyList())
                }
                override suspend fun details(conflictId:String,memoryBudgetBytes:ULong):OfflineReviewDetails=error("unused")
                override suspend fun stage(conflictId:String,expected:OfflineReviewBinding,metadata:WorkflowMetadata,choice:OfflineReviewChoice,memoryBudgetBytes:ULong):OfflineReviewMutation=error("unused")
                override suspend fun submit(conflictId:String,expected:OfflineReviewBinding,memoryBudgetBytes:ULong):OfflineReviewSubmission=error("unused")
                override suspend fun keep(conflictId:String,expected:OfflineReviewBinding,memoryBudgetBytes:ULong):OfflineReviewMutation=error("unused")
                override suspend fun cancelDraft(conflictId:String,expected:OfflineReviewBinding,memoryBudgetBytes:ULong):OfflineReviewMutation=error("unused")
            }})
        fun attached()=EditorWorkflowAttachment(project,document,epoch,sequence,"installation")
        fun bind(){owner.bind(attached())}
        fun request(target:Int=24):DocumentNavigationRequest {
            owner.activate(EditorWorkflowPanel.Documents,true)
            val state=owner.navigator.state.value
            return checkNotNull(owner.navigator.request(checkNotNull(state.binding),checkNotNull(state.attachmentEpoch),id(target)))
        }
        suspend fun close(){owner.beginDeparture();owner.close();scope.coroutineContext[Job]?.cancelAndJoin()}
    }
    private suspend fun until(test:()->Boolean){withTimeout(3000){while(!test())yield()}}

    @Test fun savedCameraUsesInstallationIdentityAndCurrentViewportWithoutOwningTheProject()=runBlocking {
        val f=Fixture(this)
        try {
            f.savedCamera=Camera(Point(12.0,15.0),2.0,.3,10.0,10.0);f.bind();f.owner.enter(f.attached())
            assertEquals(f.savedCamera?.copy(viewportWidth=320.0,viewportHeight=240.0),f.currentCamera)
            assertEquals("installation",f.localBindings.single().deviceId)
            assertEquals(0,f.layerFactories);assertEquals(0,f.reviewFactories);assertEquals(0,f.closedProjects)
        }finally{f.close()}
        assertEquals(0,f.closedProjects)
    }
    @Test fun orderedSameSequenceHashChangeFencesWorkflowPanelsUntilTheNewSnapshotIsAdmitted()=runBlocking {
        val f=Fixture(this)
        try {
            f.bind();assertFalse(f.owner.blocksInput())
            val changed=f.document.render.revision.copy(stateHash="b".repeat(64))
            f.owner.observe(changed,1u)
            // Ordinary editor work retains its existing revision/CAS admission.
            assertFalse(f.owner.blocksInput())
            f.owner.activate(EditorWorkflowPanel.Documents,true)
            assertEquals(EditorWorkflowPanel.None,f.owner.panel.value)
            assertNull(f.owner.navigator.state.value.binding)
            f.document=f.document.copy(render=f.document.render.copy(revision=changed));f.sequence=1u;f.bind()
            assertFalse(f.owner.blocksInput());assertEquals(24,f.owner.navigator.state.value.documentIds.size)
        }finally{f.close()}
    }
    @Test fun unchangedOrderedReceiptStillRebindsTheCurrentCatalog()=runBlocking {
        val f=Fixture(this)
        try{
            f.bind();f.owner.observe(f.document.render.revision,1uL);assertNull(f.owner.navigator.state.value.binding)
            f.sequence=1uL;f.bind();assertNotNull(f.owner.navigator.state.value.binding)
            assertEquals(24,f.owner.navigator.state.value.documentIds.size);assertTrue(f.owner.navigator.state.value.ready)
        }finally{f.close()}
    }
    @Test fun documentSwitchRetainsOneProjectAndAdvancesItsAttachmentEpoch()=runBlocking {
        val f=Fixture(this)
        try {
            f.bind();f.owner.enter(f.attached());f.owner.choose(f.request())
            until{f.adoptions==1&&f.owner.panel.value==EditorWorkflowPanel.None&&!f.owner.blocksInput()}
            assertEquals(id(24),f.document.documentId);assertEquals(8L,f.epoch)
            assertEquals(listOf(id(1),id(24)),f.localBindings.map{it.documentId})
            assertEquals(0,f.closedProjects)
        }finally{f.close()}
    }
    @Test fun supersededNativeReadNeverAdoptsItsOldSnapshot()=runBlocking {
        val f=Fixture(this);val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.bind();f.onRead={entered.complete(Unit);withContext(NonCancellable){release.await()}}
            f.owner.choose(f.request());entered.await()
            val changed=f.document.render.revision.copy(stateHash="b".repeat(64))
            f.owner.observe(changed,1u);f.document=f.document.copy(render=f.document.render.copy(revision=changed));f.sequence=1u;f.bind()
            release.complete(Unit);until{!f.owner.navigator.state.value.busy}
            assertEquals(0,f.adoptions);assertEquals(id(1),f.document.documentId);assertEquals(0,f.closedProjects)
        }finally{release.complete(Unit);f.close()}
    }
    @Test fun detachUnderHeldEditGateCancelsAQueuedSwitchWithoutReacquiringThatGate()=runBlocking {
        val f=Fixture(this)
        try {
            f.bind();f.gate.lock();f.owner.choose(f.request());yield()
            withTimeout(3000){f.owner.detach()}
            assertTrue(f.gate.isLocked);assertEquals(0,f.adoptions);assertEquals(0,f.closedProjects)
        }finally{if(f.gate.isLocked)f.gate.unlock();f.close()}
    }
    @Test fun departureSignalsBeforeWaitingForGateAndJoinsActualNativeReadCleanup()=runBlocking {
        val f=Fixture(this);val entered=CompletableDeferred<Unit>();val canceled=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.bind();f.onRead={entered.complete(Unit);try{awaitCancellation()}finally{canceled.complete(Unit);withContext(NonCancellable){release.await()}}}
            f.owner.choose(f.request());entered.await();f.owner.beginDeparture();canceled.await()
            val closing=launch{f.gate.withLock{f.owner.detach()}}
            yield();assertFalse(closing.isCompleted);assertEquals(0,f.closedProjects)
            release.complete(Unit);withTimeout(3000){closing.join()};assertEquals(0,f.adoptions)
        }finally{release.complete(Unit);f.close()}
    }
    @Test fun reviewResumesAfterBorrowedDocumentDetachesWithoutSubmittingAnything()=runBlocking {
        val f=Fixture(this)
        try {
            f.bind();f.owner.detach();assertFalse(f.owner.blocksInput());f.bind();f.owner.activate(EditorWorkflowPanel.OfflineReview,true)
            until{f.pageReads==1&&!f.owner.review.state.value.busy}
            assertEquals(0,f.closedProjects);assertEquals(0,f.adoptions)
        }finally{f.close()}
    }
    @Test fun pagedDocumentsAndCompetingModalsDoNotAcquireImageOnlyCapabilities()=runBlocking {
        val f=Fixture(this)
        try {
            f.document=f.document.copy(page=0u);f.bind();f.owner.enter(f.attached())
            f.owner.activate(EditorWorkflowPanel.Layers,true);f.owner.activate(EditorWorkflowPanel.SavedViews,true);f.owner.activate(EditorWorkflowPanel.Tones,true)
            assertEquals(EditorWorkflowPanel.None,f.owner.panel.value);assertTrue(f.localBindings.isEmpty());assertEquals(0,f.layerFactories)
            f.idle=false;f.owner.activate(EditorWorkflowPanel.OfflineReview,true);assertEquals(0,f.reviewFactories)
        }finally{f.close()}
    }
}
