@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import com.visualworkbench.desktop.stream.DesktopStreamContact
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Path

class DesktopStreamRootTest {
    @Test fun cancelledBeforeAcquisitionAllocatesNothing()=rootFixture{f->
        f.editor.openCaptureStream()
        f.editor.closeCaptureStream()
        assertEquals(0,f.api.opens)
        assertFalse(f.editor.captureStreamBlocksParent)
        assertEquals(0,f.project.closes)
        assertDesktopStreamsRetired()
    }
    @Test fun lateReceiverIsRetainedAndJoinedBeforeCancellationCompletes()=rootFixture{f->
        val entered=CompletableDeferred<Unit>();val release=f.gate()
        f.api.onOpen={entered.complete(Unit);release.await()}
        f.editor.openCaptureStream();entered.await()
        val closing=async(start=CoroutineStart.UNDISPATCHED){f.editor.closeCaptureStream()}
        assertFalse(closing.isCompleted)
        assertTrue(f.editor.captureStreamBlocksParent)
        assertEquals(0,f.api.stream.closes)
        release.complete(Unit);closing.await()
        assertEquals(1,f.api.stream.closes)
        assertFalse(f.editor.captureStreamBlocksParent)
        assertEquals(0,f.project.closes)
        assertEquals(0,f.link.closed)
        assertDesktopStreamsRetired()
    }
    @Test fun sameLinkReconnectBeforeUiObservationRejectsTheLateReceiver()=rootFixture{f->
        val entered=CompletableDeferred<Unit>();val release=f.gate()
        f.api.onOpen={entered.complete(Unit);release.await()}
        f.editor.openCaptureStream();entered.await()
        f.api.epoch=2uL
        release.complete(Unit)
        f.await{!f.editor.captureStreamBlocksParent}
        assertEquals(1,f.api.stream.closes)
        assertEquals(0,f.api.annotations)
        assertEquals(SyncStatus.Synced,f.editor.state.value.sync?.status)
        assertEquals(0,f.link.closed)
        assertDesktopStreamsRetired()
    }
    @Test fun admittedReceiverBlocksOrdinaryMutationVerificationAndReplacement()=rootFixture{f->
        f.open()
        assertTrue(f.editor.captureStreamBlocksParent)
        f.editor.create(Shape.Rectangle(Rect(1.0,1.0,3.0,3.0)))
        f.editor.open(Path.of("unrelated-project"))
        assertTrue(runCatching{f.editor.withCurrentVerificationOriginal{}}.exceptionOrNull() is PackageFailure)
        yield()
        assertEquals(0,f.project.edits)
        assertEquals(1,f.core.opens)
        f.editor.closeCaptureStream();f.idle()
        f.editor.create(Shape.Rectangle(Rect(1.0,1.0,3.0,3.0)))
        f.await{f.project.edits==1&&!f.editor.state.value.busy}
        assertEquals(1,f.api.stream.closes)
        assertEquals(listOf("renderer-close","stream-close"),f.order)
        assertDesktopStreamsRetired()
    }
    @Test fun failedRendererRetirementKeepsRootAndPreventsProjectCloseUntilRetry()=rootFixture{f->
        f.api.failRenderer=true;f.open()
        assertTrue(runCatching{f.editor.close()}.isFailure)
        assertTrue(f.editor.captureStreamBlocksParent)
        assertTrue(f.editor.captureStream.value.cleanupPending)
        assertEquals(0,f.project.closes)
        assertEquals(0,f.link.closed)
        assertEquals(0,f.api.stream.closes)
        assertTrue(runCatching{assertDesktopStreamsRetired()}.isFailure)
        f.api.failRenderer=false
        f.editor.close()
        assertEquals(1,f.project.closes)
        assertEquals(1,f.link.closed)
        assertEquals(1,f.api.stream.closes)
        assertDesktopStreamsRetired()
    }
    @Test fun independentBusyParentEraseRefusesReceiverBeforeAllocation()=rootFixture{f->
        val eraser=f.editor.erasers.interaction
        eraser.tool(EraserMode.Object)
        val entered=CompletableDeferred<Unit>();val release=f.gate()
        f.project.editGate={entered.complete(Unit);release.await()}
        assertTrue(eraser.begin(Point(10.0,10.0),f.editor.state.value.view.camera))
        assertTrue(eraser.finish())
        entered.await()
        assertFalse(f.editor.state.value.busy)
        assertFalse(f.editor.canOpenCaptureStream)
        f.editor.openCaptureStream()
        assertEquals(0,f.api.opens)
        release.complete(Unit)
        eraser.state.first{!it.busy}
    }
}

private fun rootFixture(body:suspend CoroutineScope.(ReceiverRootFixture)->Unit)=runBlocking{
    withTimeout(10_000){
        val f=ReceiverRootFixture(this)
        try{f.initialize();body(f)}finally{withContext(NonCancellable){
            f.gates.forEach{it.complete(Unit)};f.api.failRenderer=false
            if(f.project.closes==0)f.editor.close();f.sessions.close();f.job.cancelAndJoin()
        }}
    }
}
private class ReceiverRootFixture(parent:CoroutineScope){
    val job=SupervisorJob(parent.coroutineContext[Job])
    val scope=CoroutineScope(parent.coroutineContext+job)
    val gates=mutableListOf<CompletableDeferred<Unit>>()
    val order=mutableListOf<String>()
    val project=ReceiverRootProject()
    val core=ReceiverRootCore(project)
    val link=LinkFixture().apply{initialStatus=sessionStatus(1uL,SyncStatus.Synced,SessionCarrier.QuicWifi)}
    val api=ReceiverRootApi(link,order)
    val service=SessionFixture().also{it.link=link;it.device="owner"}
    val sessions=DesktopSessionController(scope,"owner",{service})
    val editor=EditorController(scope,core,object:DesktopHandoff{
        override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("unused")
        override suspend fun copy(path:Path,receipt:CompletedExport)=error("unused")
        override suspend fun stage(path:Path,receipt:CompletedExport):DesktopDragFile=error("unused")
    },LocationPolicy(Path.of("receiver-root-memory-only").toAbsolutePath(),false,"memory-only"),"owner",streamApi=api)
    fun gate()=CompletableDeferred<Unit>().also{gates+=it}
    suspend fun await(predicate:()->Boolean){withTimeout(5000){while(!predicate()){yield();delay(1)}}}
    suspend fun idle(){await{!editor.state.value.busy&&!editor.instructionEditor.state.value.busy&&!editor.semanticEditor.state.value.busy}}
    suspend fun initialize(){
        editor.open(Path.of("receiver-root-memory-only-project"));await{editor.state.value.document!=null};idle()
        editor.connectSession(sessions,"peer",link.endpoints,true,null)
        await{editor.state.value.sync?.status==SyncStatus.Synced};idle()
        await{editor.canOpenCaptureStream}
    }
    suspend fun open(){editor.openCaptureStream();await{editor.captureStream.value.session!=null&&!editor.captureStream.value.opening}}
}
private class ReceiverRootApi(private val link:ProjectLink,private val order:MutableList<String>):DesktopStreamReceiverApi{
    var epoch=1uL;var opens=0;var annotations=0;var failRenderer=false
    var onOpen:suspend()->Unit={}
    val stream=ReceiverRootStream(order)
    override fun connectionEpoch(link:ProjectLink):ULong{check(link===this.link);return epoch}
    override suspend fun open(link:ProjectLink):WorkbenchCaptureStream{opens++;onOpen();return stream}
    override fun annotation(link:ProjectLink,target:WorkbenchProject,documentId:String,stream:WorkbenchCaptureStream,
        newId:()->String,metadata:suspend(ProjectInfo)->WorkflowMetadata,current:()->Boolean):CaptureAnnotationSession{
        annotations++
        return CaptureAnnotationSession(target,documentId,link,stream,newId,metadata,current,{_,_,_,_->error("No contact in this fixture")})
    }
    override fun renderer(scope:CoroutineScope,core:WorkbenchCore,stream:WorkbenchCaptureStream,current:()->Boolean)=object:DesktopStreamRenderPort{
        override suspend fun stopForContact(viewPointLogical:Point):DesktopStreamContact?=error("unused")
        override suspend fun adoptLossless(annotation:CaptureAnnotationSession)=error("unused")
        override suspend fun close(){if(failRenderer)error("retained renderer");order+="renderer-close"}
    }
}
private class ReceiverRootStream(private val order:MutableList<String>):WorkbenchCaptureStream{
    private val value=CaptureStreamSignal(1uL,1uL,1uL,0uL,CaptureStreamStage.AwaitingSource,0u,0u,0uL,null)
    override val signals=MutableStateFlow(value)
    var closes=0
    override fun signal()=value
    override suspend fun beginDraw():CaptureStreamDraw?=error("unused")
    override suspend fun frozenBackground():CaptureFrozenBackground?=error("unused")
    override suspend fun firstContact(draftId:String):CaptureFreezeReceipt?=error("unused")
    override suspend fun resumeAfterDraftDisposed():String?=error("unused")
    override suspend fun close(){closes++;order+="stream-close"}
}
private class ReceiverRootProject:WorkbenchProject{
    var closes=0;var edits=0
    var editGate:suspend()->Unit={}
    private val revision=ProjectInfo("root-project","Root","owner",10uL,true,false,1uL,"root-hash",listOf("root-document"))
    private val item=RenderItem("root-object","root-layer",1.0,"normal",Rect(1.0,1.0,20.0,20.0),
        Contours(longArrayOf(),longArrayOf(),uintArrayOf()),Transform(),ObjectStyle(0x000000ffu,2.0,fill=0x000000ffu),
        Shape.Rectangle(Rect(1.0,1.0,20.0,20.0)),false)
    private val doc=DocumentSnapshot("root-document","Root",64u,64u,8u,
        listOf(LayerInfo("root-layer","Annotations",true,false,1.0,"normal")),RenderList(revision,listOf(item)))
    override val changes=emptyFlow<ProjectChange>()
    override suspend fun info()=revision
    override suspend fun document(documentId:String)=doc
    override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=throw CoreFailure(CoreFailureKind.Backpressure)
    override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("unused")
    override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo{editGate();edits++;return revision}
    override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("unused")
    override suspend fun render(documentId:String,rectangle:Rect?)=doc.render
    override suspend fun export(options:ExportOptions):ExportResult=error("unused")
    override suspend fun close(){closes++}
}
private class ReceiverRootCore(private val project:WorkbenchProject):WorkbenchCore{
    var opens=0;private var ids=0
    override fun newId(unixMs:ULong)="root-id-${ids++}"
    override fun newDeviceId()=error("No identity changes")
    override fun cameraMatrix(camera:Camera,inverse:Boolean)=Transform()
    override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>)=points
    override suspend fun create(options:CreateProject):WorkbenchProject=error("unused")
    override suspend fun open(path:String):WorkbenchProject{opens++;return project}
    override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("unused")
}
