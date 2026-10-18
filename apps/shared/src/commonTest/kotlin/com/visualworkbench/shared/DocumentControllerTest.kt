package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlin.test.*

class DocumentControllerTest {
    private class Core:WorkbenchCore {
        override fun newId(unixMs:ULong):String="id"
        override fun newDeviceId():String="device"
        override fun cameraMatrix(camera:Camera,inverse:Boolean):Transform=Transform()
        override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>):List<Point> = points
        override suspend fun create(options:CreateProject):WorkbenchProject=error("unused")
        override suspend fun open(path:String):WorkbenchProject=error("unused")
        override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("unused")
    }
    private class Project:WorkbenchProject,WorkbenchDocumentsProject {
        var revision=ProjectInfo("project","title","device",1uL,false,false,1uL,"b".repeat(64),listOf("doc"))
        var descriptor=EditableDescriptor("doc","a".repeat(64),EditableKind.Pdf,List(2){DocumentPage(it.toUInt(),100.0,100.0,listOf(0.0,0.0,100.0,100.0),0u,"layer-$it")},revision)
        var sourceFailure:CoreFailureKind?=null
        var page=0u
        override val changes=emptyFlow<ProjectChange>()
        override suspend fun info():ProjectInfo=revision
        override suspend fun document(documentId:String):DocumentSnapshot=documentPage(documentId,page)
        override suspend fun documentPage(documentId:String,page:UInt):DocumentSnapshot=DocumentSnapshot("doc","title",100u,100u,8u,listOf(LayerInfo("layer-$page","Annotations",true,false,1.0,"normal")),RenderList(revision,emptyList()))
        override suspend fun selectDocumentPage(documentId:String,page:UInt):DocumentSnapshot{this.page=page;return documentPage(documentId,page)}
        override suspend fun documentDescriptor(documentId:String):EditableDescriptor=descriptor
        override suspend fun documentSource(documentId:String,expected:ProjectInfo):EditableSource {sourceFailure?.let{throw CoreFailure(it)};return EditableSource(descriptor,byteArrayOf(1))}
        override suspend fun documentOverlays(documentId:String,page:UInt,expected:ProjectInfo):List<DocumentOverlay> = emptyList()
        override suspend fun publishDocumentCopy(documentId:String,destination:String,copy:DocumentExport):String=error("publication must not occur")
        override fun documentPassword(documentId:String):String?=null
        override fun documentPassword(documentId:String,password:String?):Unit=Unit
        override fun selectedDocumentPage(documentId:String):UInt=page
        override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("unused")
        override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("unused")
        override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("unused")
        override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("unused")
        override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("unused")
        override suspend fun export(options:ExportOptions):ExportResult=error("unused")
        override suspend fun close():Unit=Unit
    }
    private class Jobs:DocumentJobs {
        override val pcRedaction=false
        val entered=CompletableDeferred<Unit>()
        var release:CompletableDeferred<Unit>?=null
        var calls=0
        var notices=listOf("foreign_object_ignored","resource_denied")
        var sourceHash="a".repeat(64)
        var kind=EditableKind.Pdf
        override suspend fun run(request:DocumentRequest,source:ByteArray):DocumentReply {
            calls++;entered.complete(Unit)
            release?.let{withContext(NonCancellable){it.await()}}
            if(request.operation==DocumentOperation.PdfInfo || request.operation==DocumentOperation.SvgInfo)
                return DocumentReply(sourceHash,"c".repeat(64),DocumentMetadata.Info(kind,if(kind==EditableKind.Pdf)2u else 1u,100.0,100.0,false,notices),byteArrayOf())
            val tile=(request.operation as DocumentOperation.Tile)
            return DocumentReply("a".repeat(64),"c".repeat(64),DocumentMetadata.Tile(tile.page,tile.tile.origin,tile.tile.scale.toFloat().toDouble(),tile.tile.width,tile.tile.height),ByteArray(tile.tile.width.toInt()*tile.tile.height.toInt()*4))
        }
        override suspend fun close():Unit=Unit
    }
    private class Fixture {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val project=Project();val jobs=Jobs()
        var attached:DocumentAttachment?=null
        var admitted=true
        val controller=DocumentController(scope,Core(),jobs,{attached},{admitted},{a,page->attached=DocumentAttachment(a.project,project.selectDocumentPage("doc",page),a.epoch)})
        val camera=Camera(Point(0.0,0.0),1.0,0.0,2.0,2.0)
        suspend fun bind(epoch:Long=1){attached=DocumentAttachment(project,project.document("doc"),epoch);controller.bind(project.descriptor,project.page)}
        suspend fun close(){jobs.release?.complete(Unit);controller.close();scope.cancel()}
    }
    @Test fun importAndReopenRefreshSvgFidelityNoticesWithoutPasswordSubmission():Unit=runBlocking {
        val f=Fixture();try{
            f.project.descriptor=f.project.descriptor.copy(kind=EditableKind.Svg,pages=f.project.descriptor.pages.take(1));f.jobs.kind=EditableKind.Svg
            f.bind();f.controller.scheduleNotices()
            assertEquals(f.jobs.notices,f.controller.state.value.notices);assertEquals(1,f.jobs.calls)
            f.controller.close();f.bind(2);f.controller.scheduleNotices()
            assertEquals(f.jobs.notices,f.controller.state.value.notices);assertEquals(2,f.jobs.calls)
        }finally{f.close()}
    }
    @Test fun sameSourceEditAndPageSwitchKeepValidatedFidelityNotices():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.controller.scheduleNotices();val notices=f.controller.state.value.notices
            f.project.revision=f.project.revision.copy(hostSeq=2uL,stateHash="d".repeat(64));f.project.descriptor=f.project.descriptor.copy(revision=f.project.revision)
            f.bind();assertEquals(notices,f.controller.state.value.notices);f.controller.scheduleNotices();assertEquals(1,f.jobs.calls)
            f.controller.page(1u);assertEquals(notices,f.controller.state.value.notices)
        }finally{f.close()}
    }
    @Test fun replacementSourceDropsWarningsAndRejectsLateOldSourceNotices():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.controller.scheduleNotices();assertNotNull(f.controller.state.value.notices.firstOrNull())
            f.jobs.release=CompletableDeferred();f.controller.refreshNotices()
            f.project.descriptor=f.project.descriptor.copy(sourceAssetId="d".repeat(64));f.bind()
            assertTrue(f.controller.state.value.notices.isEmpty());f.jobs.release!!.complete(Unit);yield()
            assertTrue(f.controller.state.value.notices.isEmpty())
        }finally{f.close()}
    }
    @Test fun noticesWaitForCallerAdmissionBeforeBorrowingSource():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.admitted=false;f.controller.scheduleNotices();assertEquals(0,f.jobs.calls)
            f.admitted=true;withTimeout(2000){while(f.controller.state.value.notices.isEmpty())delay(5)}
            assertEquals(f.jobs.notices,f.controller.state.value.notices)
        }finally{f.close()}
    }
    @Test fun aDifferentSourceCannotSupplyFidelityNotices():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.jobs.sourceHash="d".repeat(64);f.controller.scheduleNotices()
            assertTrue(f.controller.state.value.notices.isEmpty());assertNotNull(f.controller.state.value.message)
        }finally{f.close()}
    }
    @Test fun ordinaryViewportAdoptionCarriesAllAuthorityAndItsOwnGeneration():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.controller.viewport(f.camera);val frame=assertNotNull(f.controller.state.value.frame)
            assertEquals("a".repeat(64),frame.binding.sourceAssetId);assertEquals(1L,frame.binding.attachmentEpoch);assertEquals(0u,frame.binding.page);assertTrue(frame.binding.viewportGeneration>0);assertEquals(1,frame.tiles.size)
        }finally{f.close()}
    }
    @Test fun sameRevisionAndIdenticalViewportCannotReviveAnOldAttachment():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.jobs.release=CompletableDeferred();f.controller.viewport(f.camera);f.jobs.entered.await();f.bind(2);f.jobs.release!!.complete(Unit);yield();assertNull(f.controller.state.value.frame)
        }finally{f.close()}
    }
    @Test fun pageSwitchChangesPrivateViewWithoutInventingACanonicalRevision():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.controller.page(1u);assertEquals(1u,f.controller.state.value.page);assertEquals(f.project.revision,f.attached!!.snapshot.render.revision)
            f.controller.viewport(f.camera);assertEquals(1u,assertNotNull(f.controller.state.value.frame).binding.page)
        }finally{f.close()}
    }
    @Test fun cancellationStaysBusyUntilTheBorrowingWorkerActuallyRetires():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.jobs.release=CompletableDeferred();f.controller.viewport(f.camera);f.jobs.entered.await();f.controller.cancel();assertTrue(f.controller.state.value.busy);assertNull(f.controller.state.value.frame)
            f.jobs.release!!.complete(Unit);yield();assertFalse(f.controller.state.value.busy);assertNull(f.controller.state.value.frame)
        }finally{f.close()}
    }
    @Test fun closeJoinsLateWorkerBeforeClearingTheAttachmentState():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.jobs.release=CompletableDeferred();f.controller.viewport(f.camera);f.jobs.entered.await();val closing=async{f.controller.close()};yield();assertFalse(closing.isCompleted)
            f.jobs.release!!.complete(Unit);closing.await();assertNull(f.controller.state.value.frame);assertNull(f.controller.state.value.descriptor)
        }finally{f.close()}
    }
    @Test fun copyPublicationWaitsForAnEmbeddedRevisionAndSourceCarrier():Unit=runBlocking {
        val f=Fixture();try{f.bind();assertFalse(f.controller.exportPublicationAvailable);f.controller.export(DocumentCopy.Annotated);assertNull(f.controller.checkedExport());assertEquals(0,f.jobs.calls);assertNotNull(f.controller.state.value.message)
        }finally{f.close()}
    }
    @Test fun unsupportedDocumentSourceKeepsItsCapabilityGuidance():Unit=runBlocking {
        val f=Fixture();try{f.bind();f.project.sourceFailure=CoreFailureKind.Unsupported;f.controller.refreshNotices()
            assertEquals(documentFailureText(DocumentFailure(DocumentFailureKind.Unsupported)),f.controller.state.value.message);assertEquals(0,f.jobs.calls)
        }finally{f.close()}
    }

}
