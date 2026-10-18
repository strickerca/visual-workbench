package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlin.test.*

class GiantAttachmentOwnerTest {
    private class Image : GiantImage {
        var closes=0
        override suspend fun viewport(view:GiantViewport):GiantViewportPlan=error("Unused")
        override suspend fun tile(address:GiantTileAddress,foreign:SessionResourceCharge):GiantTilePixels=error("Unused")
        override suspend fun close(){closes++}
    }
    private class Project : WorkbenchProject, WorkbenchGiantImagesProject {
        val created=mutableListOf<Image>();var onOpen:suspend()->Unit={}
        override val changes=emptyFlow<ProjectChange>()
        override suspend fun openGiantImage(binding:WorkflowBinding,diskCacheBytes:ULong,ramCacheBytes:ULong):GiantImage {
            val image=Image();created+=image
            try{onOpen();return image}catch(error:Throwable){image.close();throw error}
        }
        override suspend fun info():ProjectInfo=error("Unused")
        override suspend fun document(documentId:String):DocumentSnapshot=error("Unused")
        override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("Unused")
        override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("Unused")
        override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("Unused")
        override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("Unused")
        override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("Unused")
        override suspend fun export(options:ExportOptions):ExportResult=error("Unused")
        override suspend fun close()=Unit
    }
    private class Fixture {
        val project=Project();val events=mutableListOf<String>()
        var document=DocumentSnapshot("doc","Giant",100_000u,1000u,8u,emptyList(),RenderList(
            ProjectInfo("project","Giant","device",1u,false,false,1u,"a".repeat(64),listOf("doc")),emptyList()))
        var epoch=1L;var failClose=false
        var attachmentReads=0
        var permit:((WorkflowBinding,Long)->Boolean)?=null
        var onClose:suspend()->Unit={}
        val owner=GiantAttachmentOwner<Image>(
            {attachmentReads++;DocumentAttachment(project,document,epoch)},
            {image,_,_,_,current->permit=current;image as Image},
            {image->events+="retire:start";onClose();if(failClose)throw IllegalStateException("pipeline retained");image.close();events+="retire:end"},
            {events+="changed"},1024u,1024u,
            {object:SessionResources{override suspend fun reserveForeign(bytes:ULong):SessionResourceCharge=error("Unused")}})
        suspend fun bind():Image?=owner.bind(DocumentAttachment(project,document,epoch))
    }
    @Test fun matchingAttachmentReusesOwnerAndRevisionRebindRetiresBeforeOpeningReplacement()=runBlocking {
        val f=Fixture();val first=f.bind();assertSame(first,f.bind());assertEquals(1,f.project.created.size)
        f.document=f.document.copy(render=f.document.render.copy(revision=f.document.render.revision.copy(stateHash="b".repeat(64))))
        f.owner.observe(f.document.render.revision);assertNull(f.owner.current)
        val next=f.bind();assertNotSame(first,next);assertEquals(1,first?.closes)
        f.owner.detach();assertEquals(1,next?.closes);assertFalse(f.owner.retaining)
    }
    @Test fun failedRetirementRetainsOwnerAndPreventsAnotherNativeOpen()=runBlocking {
        val f=Fixture();f.bind();f.failClose=true
        assertTrue(runCatching{f.owner.detach()}.isFailure)
        assertTrue(f.owner.retaining);assertNull(f.owner.current)
        assertTrue(runCatching{f.bind()}.isFailure);assertEquals(1,f.project.created.size)
        f.failClose=false;f.owner.detach();assertFalse(f.owner.retaining)
    }
    @Test fun epochChangeDuringNativeOpenClosesLateImageWithoutAdoptingAView()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        f.project.onOpen={entered.complete(Unit);withContext(NonCancellable){release.await()}}
        val work=async(start=CoroutineStart.UNDISPATCHED){f.bind()}
        entered.await();f.epoch++;f.owner.invalidate();release.complete(Unit)
        assertNull(work.await());assertNull(f.owner.current);assertEquals(1,f.project.created.single().closes)
        f.owner.detach()
    }
    @Test fun orderedVisibleHashChangeFencesPendingNativeOpenBeforeRefreshAdoptsSnapshot()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        f.project.onOpen={entered.complete(Unit);withContext(NonCancellable){release.await()}}
        val work=async(start=CoroutineStart.UNDISPATCHED){f.bind()};entered.await()
        f.owner.observe(f.document.render.revision.copy(stateHash="c".repeat(64)));release.complete(Unit)
        assertNull(work.await());assertEquals(1,f.project.created.single().closes);f.owner.detach()
    }
    @Test fun canceledDetachStillJoinsRealRetirementBeforeReturning()=runBlocking {
        val f=Fixture();f.bind();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        f.onClose={entered.complete(Unit);release.await()}
        val closing=launch(start=CoroutineStart.UNDISPATCHED){f.owner.detach()};entered.await();closing.cancel()
        assertFalse(closing.isCompleted);assertTrue(f.owner.retaining)
        release.complete(Unit);closing.join();assertFalse(f.owner.retaining)
    }
    @Test fun sameRevisionButDifferentGeometryCannotReuseSourceOwner()=runBlocking {
        val f=Fixture();val first=f.bind();f.document=f.document.copy(width=100_001u)
        assertNull(f.owner.current);val next=f.bind();assertNotSame(first,next);assertEquals(1,first?.closes);f.owner.detach()
    }
    @Test fun invalidationSignalsPendingNativeOpenAndDetachStillWaitsForItsSettlement()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val canceled=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        f.project.onOpen={entered.complete(Unit);try{awaitCancellation()}finally{canceled.complete(Unit);withContext(NonCancellable){release.await()}}}
        val work=async(start=CoroutineStart.UNDISPATCHED){f.bind()};entered.await();f.owner.invalidate();canceled.await()
        val closing=launch(start=CoroutineStart.UNDISPATCHED){f.owner.detach()}
        assertFalse(closing.isCompleted);assertEquals(0,f.project.created.single().closes)
        release.complete(Unit);assertNull(work.await());closing.join();assertEquals(1,f.project.created.single().closes)
    }
    @Test fun factoryFailureKeepsItsFailedCleanupOwnerAndBlocksParentReplacement()=runBlocking {
        val f=Fixture();var quarantined:Image?=null;var failClose=true
        class Failed(val retained:Image):Exception()
        val owner=GiantAttachmentOwner<Image>(
            {DocumentAttachment(f.project,f.document,f.epoch)},
            {image,_,_,_,_->throw Failed(image as Image)},
            {if(failClose)throw IllegalStateException("retained context");it.close()}, {},1024u,1024u,
            {object:SessionResources{override suspend fun reserveForeign(bytes:ULong):SessionResourceCharge=error("Unused")}},
            {error->(error as? Failed)?.retained?.also{quarantined=it}})
        assertTrue(runCatching{owner.bind(DocumentAttachment(f.project,f.document,f.epoch))}.isFailure)
        assertTrue(owner.retaining);assertNull(owner.current);assertEquals(0,quarantined?.closes)
        assertTrue(runCatching{owner.bind(DocumentAttachment(f.project,f.document,f.epoch))}.isFailure)
        assertEquals(1,f.project.created.size);failClose=false;owner.detach()
        assertEquals(1,quarantined?.closes);assertFalse(owner.retaining)
    }

    @Test fun drawCallbackReadsPublishedPermitWithoutAccessingUiAttachment()=runBlocking {
        val f=Fixture();f.bind();val reads=f.attachmentReads
        val revision=f.document.render.revision
        val bound=WorkflowBinding(revision.projectId,f.document.documentId,revision.hostSeq,revision.stateHash)
        assertTrue(withContext(Dispatchers.Default){checkNotNull(f.permit)(bound,f.epoch)})
        assertEquals(reads,f.attachmentReads)
        f.owner.invalidate()
        assertFalse(withContext(Dispatchers.Default){checkNotNull(f.permit)(bound,f.epoch)})
        assertEquals(reads,f.attachmentReads);f.owner.detach()
    }

}
