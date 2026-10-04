@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.combine
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Path
import java.util.concurrent.Executors
import java.util.ArrayDeque
import kotlin.coroutines.CoroutineContext

class DesktopCoherenceTest {
    private class HeldDispatch(private val delegate: CoroutineDispatcher) : CoroutineDispatcher() {
        private val lock = Any(); private val pending = ArrayDeque<Pair<CoroutineContext, Runnable>>(); private var held = false
        override fun dispatch(context: CoroutineContext, block: Runnable) {
            val queue = synchronized(lock) { if (held) { pending.addLast(context to block); true } else false }
            if (!queue) delegate.dispatch(context, block)
        }
        fun hold() { synchronized(lock) { check(!held); held = true } }
        fun releaseOne() { val next = synchronized(lock) { check(held); pending.removeFirst() }; delegate.dispatch(next.first, next.second) }
        fun releaseAll() { val jobs = synchronized(lock) { held = false; pending.toList().also { pending.clear() } }; jobs.forEach { delegate.dispatch(it.first, it.second) } }
    }
    @Test fun queuedProjectCloseRechecksAiSendAdmissionBeforeRetiringTheProject() {
        Executors.newSingleThreadExecutor { r -> Thread(r, "vw-ai-close-test").apply { isDaemon = true } }.asCoroutineDispatcher().use { dispatcher ->
            runBlocking(dispatcher) {
                val held = HeldDispatch(dispatcher); val scope = CoroutineScope(coroutineContext + SupervisorJob() + held)
                val f = ControllerFixture(scope); var closes = 0
                try {
                    withTimeout(10000) {
                        f.start(); f.editor.instructionEditor.state.first { !it.busy }
                        f.project.beforeClose = { closes++ }
                        val before = checkNotNull(f.editor.state.value.document)
                        held.hold()
                        // Real controller admission passes before any Send; the
                        // queued close cannot acquire its edit mutex yet.
                        f.editor.closeProject(); assertFalse(f.editor.state.value.busy)
                        val send = checkNotNull(f.editor.ai.send("never-sent-fixture", 0uL, false))
                        assertTrue(f.editor.ai.needsDecision())
                        assertEquals(AiStage.Sending, f.editor.ai.state.value.busy)
                        held.releaseOne() // close runs; actual Send remains held.
                        f.editor.state.first { !it.busy && it.message.orEmpty().contains("Save the paid Result") }
                        assertEquals(before, f.editor.state.value.document); assertEquals(0, closes)
                        assertFalse(send.isCompleted)
                        // No request/credentials/provider exists in this fixture.
                        // Its held Send later refuses, after the close decision.
                        held.releaseAll(); send.join()
                        f.editor.closeProject(); f.editor.state.first { !it.busy && it.document == null }
                        assertEquals(1, closes)
                    }
                } finally {
                    held.releaseAll()
                    withContext(NonCancellable) { try { f.editor.close() } finally { scope.cancel() } }
                }
            }
        }
    }
    @Test fun focusedControlsAndDialogTextKeepOrdinaryKeys(){
        // Tab/Enter/arrows/letters are ordinary: controls own them before any
        // editor shortcut; dialogs own modified keys too (e.g. Ctrl+A in text).
        assertFalse(shortcutAllowed(false,false,false,false))
        assertTrue(shortcutAllowed(true,false,false,false))
        assertTrue(shortcutAllowed(false,false,true,false))
        assertTrue(shortcutAllowed(false,false,false,true))
        for(canvas in listOf(false,true))for(modified in listOf(false,true))for(function in listOf(false,true))assertFalse(shortcutAllowed(canvas,true,modified,function))
    }

    @Test fun framesNeverMixAcrossRevisionsAttachmentsOrFailedPreparation(){
        val first=snapshot(7u,"hash-7");val next=snapshot(8u,"hash-8")
        val state=EditorState(document=first,projectEpoch=1)
        val frame=GeometryFrame(first.binding(1),first,mapOf("object" to "old geometry"))
        assertNotNull(frame.current(state))
        assertNull(frame.current(state.copy(document=next)))
        assertNull(frame.current(state.copy(projectEpoch=2)))
        assertNull(frame.current(state.copy(renderIssue="preparation failed")))
        assertEquals(SnapshotDecision.Stale,snapshotDecision(next,first))
        assertEquals(SnapshotDecision.Duplicate,snapshotDecision(first,first.copy(title="same revision receipt")))
        assertEquals(SnapshotDecision.Conflict,snapshotDecision(first,snapshot(7u,"different")))
        assertEquals(SnapshotDecision.Newer,snapshotDecision(first,next))
    }

    @Test fun delayedOlderSnapshotCannotOverwriteANewerAcceptedEdit()=fixture { f ->
        f.project.bump(10.0)
        val gate=DocumentGate();f.project.nextDocumentGate=gate
        f.project.announce();gate.captured.await()
        f.project.bump(20.0)
        f.editor.nudge(1.0,0.0)
        f.idle(10u)
        assertEquals(21.0,f.editor.state.value.document!!.render.items.single().transform.e,0.0)
        gate.release.complete(Unit);gate.delivered.await();yield()
        assertEquals(10uL,f.editor.state.value.document!!.render.revision.hostSeq)
        assertEquals(21.0,f.editor.state.value.document!!.render.items.single().transform.e,0.0)
    }

    @Test fun duplicateReceiptLeavesDragPreviewIntactAndOneGestureCommitsOnce()=fixture { f ->
        f.project.bump(10.0)
        val gate=DocumentGate();f.project.nextDocumentGate=gate
        f.project.announce();gate.captured.await()
        // A separate read reaches the same revision while its older observer
        // response is delayed; no mutation is necessary for this refresh.
        f.editor.clearSelection();f.editor.nudge(0.0,0.0);f.idle(8u)
        f.ready();f.editor.select("object")
        f.editor.pointerDown(Point(25.0,15.0),false,false)
        f.editor.pointerMove(Point(28.0,19.0))
        val preview=f.editor.state.value.preview
        assertEquals(13.0,preview.getValue("object").e,0.0)
        gate.release.complete(Unit);gate.delivered.await();yield()
        assertEquals(preview,f.editor.state.value.preview)
        f.editor.pointerUp();f.idle(9u)
        assertEquals(1,f.project.accepted.size)
        val command=f.project.accepted.single().single() as EditCommand.SetTransform
        assertEquals(13.0,command.transform.e,0.0);assertEquals(4.0,command.transform.f,0.0)
    }

    @Test fun peerEditBetweenInfoAndCommitRefusesTheCapturedAbsoluteDrag()=fixture { f ->
        f.editor.pointerDown(Point(15.0,15.0),false,false)
        f.editor.pointerMove(Point(25.0,15.0))
        f.project.beforeNextEdit={f.project.bump(50.0)}
        f.editor.pointerUp();f.idle(8u)
        assertTrue(f.project.accepted.isEmpty())
        assertEquals(50.0,f.project.current.render.items.single().transform.e,0.0)
        assertTrue(f.editor.state.value.preview.isEmpty())
        assertTrue(f.editor.state.value.message.orEmpty().contains("document changed"))
    }

    @Test fun remoteRevisionKeepsTextDialogAndRejectedTextRetainsItsDraft()=fixture { f ->
        f.editor.tool(Tool.Text);f.editor.pointerDown(Point(14.0,18.0),false,false)
        val anchor=f.editor.state.value.textAnchor
        f.project.bump(10.0);f.project.announce();f.idle(8u)
        assertEquals(anchor,f.editor.state.value.textAnchor)
        f.project.beforeNextEdit={f.project.bump(20.0)}
        f.editor.acceptText("Keep this typed draft");f.idle(9u)
        assertEquals(anchor,f.editor.state.value.textAnchor);assertTrue(f.project.accepted.isEmpty())
        f.editor.acceptText("Keep this typed draft");f.idle(10u)
        assertNull(f.editor.state.value.textAnchor)
        val create=f.project.accepted.single().single() as EditCommand.Create
        assertEquals("Keep this typed draft",(create.shape as Shape.Text).text)
    }

    @Test fun optimisticSameHostSequenceUsesVisibleEventOrderWithoutMovingTheCamera()=fixture { f ->
        val camera=f.editor.state.value.view.camera
        f.project.optimistic(23.0,"pending-one");f.project.announce()
        withTimeout(5000){f.editor.state.first{it.document?.render?.revision?.stateHash=="pending-one"}}
        assertNull(f.editor.state.value.renderIssue)
        f.project.optimistic(41.0,"pending-two");f.project.announce()
        withTimeout(5000){f.editor.state.first{it.document?.render?.revision?.stateHash=="pending-two"}}
        f.project.changes.emit(ProjectChange(1u,ChangeKind.Committed,snapshot(7u,"pending-one").render.revision))
        yield()
        assertEquals("pending-two",f.editor.state.value.document!!.render.revision.stateHash)
        assertEquals(camera,f.editor.state.value.view.camera)
        assertEquals(7uL,f.editor.state.value.document!!.render.revision.hostSeq)
    }

    @Test fun sameSequenceStaleFetchCannotOverwriteTheNewerOptimisticState()=fixture { f ->
        f.project.optimistic(10.0,"pending-one")
        val gate=DocumentGate();f.project.nextDocumentGate=gate;f.project.announce();gate.captured.await()
        f.project.optimistic(20.0,"pending-two")
        f.editor.clearSelection();f.editor.nudge(0.0,0.0)
        withTimeout(5000){f.editor.state.first{!it.busy&&it.document?.render?.revision?.stateHash=="pending-two"}}
        gate.release.complete(Unit);gate.delivered.await();yield()
        assertEquals("pending-two",f.editor.state.value.document!!.render.revision.stateHash)
        assertNull(f.editor.state.value.renderIssue)
    }

    @Test fun projectCloseSettlesLinkBeforeReleasingTheProject()=fixture { f ->
        val service=SessionFixture().apply{device="owner"}
        val sessions=DesktopSessionController(f.scope,"owner",{service})
        try{
            sessions.requireService()
            f.editor.connectSession(sessions,"peer",listOf(SessionEndpoint(SessionCarrier.QuicWifi,"127.0.0.1:44242")),true,null)
            withTimeout(5000){f.editor.state.first{!it.busy&&it.sync!=null}}
            f.project.beforeClose={assertEquals(1,service.link.closed)}
            f.editor.closeProject()
            withTimeout(5000){f.editor.state.first{!it.busy&&it.document==null}}
            assertNull(f.editor.state.value.peerFrame)
        }finally{sessions.close()}
    }

    @Test fun handlePreviewAndCommitShareIdentityAndCancelCommitsNothing()=fixture { f ->
        val service=SessionFixture().apply{device="owner"}
        val sessions=DesktopSessionController(f.scope,"owner",{service})
        try{
            sessions.requireService();f.editor.connectSession(sessions,"peer",listOf(SessionEndpoint(SessionCarrier.QuicWifi,"127.0.0.1:44242")),true,null)
            withTimeout(5000){f.editor.state.first{!it.busy&&it.sync!=null}}
            f.editor.pointerDown(Point(15.0,15.0),false,false);f.editor.pointerMove(Point(20.0,18.0))
            val id=service.link.objects.single().gestureId
            assertTrue(f.project.accepted.isEmpty())
            f.editor.pointerUp();f.idle(8u)
            assertEquals(id,f.project.lastOptions?.gestureId)
            withTimeout(5000){while(service.link.finished.isEmpty())yield()}
            assertEquals(id to false,service.link.finished.single())
            f.ready();f.editor.pointerDown(Point(20.0,18.0),false,false);f.editor.pointerMove(Point(22.0,19.0));f.editor.cancelInput()
            withTimeout(5000){while(service.link.finished.size<2)yield()}
            assertTrue(service.link.finished.last().second)
            assertEquals(1,f.project.accepted.size)
        }finally{f.editor.disconnectSession();withTimeout(5000){f.editor.state.first{!it.busy&&it.sync==null}};sessions.close()}
    }

    @Test fun newShapePreviewAndFinalCreateUseTheSameDGeometryStyleAndGesture()=fixture { f ->
        val service=SessionFixture().apply{device="owner"};val sessions=DesktopSessionController(f.scope,"owner",{service})
        try{
            sessions.requireService();f.editor.connectSession(sessions,"peer",listOf(SessionEndpoint(SessionCarrier.QuicWifi,"127.0.0.1:44242")),true,null)
            withTimeout(5000){f.editor.state.first{!it.busy&&it.sync!=null}}
            f.editor.tool(Tool.Rectangle);f.editor.pointerDown(Point(10.0,12.0),false,false);f.editor.pointerMove(Point(40.0,60.0))
            val preview=service.link.newObjects.single()
            f.editor.pointerUp();f.idle(8u)
            val create=f.project.accepted.single().single() as EditCommand.Create
            assertEquals(preview.shape,create.shape);assertEquals(preview.style,create.style);assertEquals(Transform(),create.transform)
            assertEquals(preview.objectId,create.objectId);assertEquals(preview.gestureId,f.project.lastOptions?.gestureId)
            assertEquals(preview.createdAtMs,f.project.lastOptions?.createdAtMs)
            assertFalse(create.style.screenConstantWidth)
        }finally{f.editor.disconnectSession();withTimeout(5000){f.editor.state.first{!it.busy&&it.sync==null}};sessions.close()}
    }

    @Test fun cancelledWidthSliderPublishesNoDurableEdit()=fixture { f ->
        val service=SessionFixture().apply{device="owner"};val sessions=DesktopSessionController(f.scope,"owner",{service})
        try{
            sessions.requireService();f.editor.connectSession(sessions,"peer",listOf(SessionEndpoint(SessionCarrier.QuicWifi,"127.0.0.1:44242")),true,null)
            withTimeout(5000){f.editor.state.first{!it.busy&&it.sync!=null}}
            f.editor.width(12.0,false)
            assertEquals(PreviewKind.Slider,service.link.objects.single().kind)
            assertEquals(12.0,f.editor.state.value.previewStyles.getValue("object").width,0.0)
            f.editor.cancelInput()
            withTimeout(5000){while(service.link.finished.isEmpty())yield()}
            assertTrue(service.link.finished.single().second)
            assertTrue(f.editor.state.value.previewStyles.isEmpty());assertTrue(f.project.accepted.isEmpty())
        }finally{f.editor.disconnectSession();withTimeout(5000){f.editor.state.first{!it.busy&&it.sync==null}};sessions.close()}
    }

    @Test fun oldPeerOverlayCannotCrossARevisionOrAttachment() {
        val doc=snapshot(7u,"hash-7");val replacement=doc.render.items.single().copy(transform=Transform(e=90.0))
        val peer=PeerFrame(doc.binding(1),4,PeerPreviews(12u,listOf("gesture"),listOf(replacement)))
        val state=EditorState(document=doc,projectEpoch=1,peerFrame=peer)
        assertEquals(listOf(replacement),overlayItems(state))
        assertTrue(overlayItems(state.copy(projectEpoch=2)).isEmpty())
        assertTrue(overlayItems(state.copy(document=snapshot(7u,"pending-hash"))).isEmpty())
        assertTrue(overlayItems(state.copy(peerFrame=null)).isEmpty())
    }

    @Test fun acceptedStrokeRefreshesAndRetiresWetGeometryWhenPreviewCloseIsCancelled()=fixture { f ->
        val service=SessionFixture().apply {
            device="owner"
            link=object:LinkFixture(){override suspend fun finishPreview(gestureId:String,cancel:Boolean){
                if(!cancel)throw CancellationException("Synthetic preview close cancellation")
                super.finishPreview(gestureId,cancel)
            }}
        }
        val sessions=DesktopSessionController(f.scope,"owner",{service})
        val contours=Contours(longArrayOf(2560,2816,2560),longArrayOf(2560,2560,2816),uintArrayOf(3u))
        f.project.strokeFactory={ options -> object:WorkbenchStroke {
            override suspend fun append(batch:SampleBatch)=InkUpdate(batch.sequence,0u,batch.x.size.toULong(),contours)
            override suspend fun predict(batch:SampleBatch):InkUpdate=error("Prediction must not run")
            override suspend fun commit():ProjectInfo {
                val item=RenderItem(options.objectId,"layer",1.0,"normal",Rect(10.0,10.0,1.0,1.0),contours,
                    Transform(),ObjectStyle(options.rgba,options.width),Shape.Stroke("pen"),false)
                // No event is emitted: the explicit accepted receipt must drive
                // this refresh even when volatile close fails immediately.
                f.project.current=snapshot(8u,"hash-8",items=f.project.current.render.items+item)
                return f.project.current.render.revision
            }
            override fun cancel()=Unit
            override suspend fun dispose()=Unit
        }}
        try {
            sessions.requireService();f.editor.connectSession(sessions,"peer",listOf(SessionEndpoint(SessionCarrier.QuicWifi,"127.0.0.1:44242")),true,null)
            withTimeout(5000){f.editor.state.first{!it.busy&&it.sync!=null}}
            f.editor.tool(Tool.Pen);f.editor.pointerDown(Point(10.0,10.0),false,false,1000);f.editor.pointerUp(Point(11.0,11.0),1010)
            f.idle(8u)
            assertNotNull(f.editor.state.value.wetStroke)
            assertEquals(2,f.editor.state.value.document!!.render.items.size)
            f.ready()
            assertNull(f.editor.state.value.wetStroke)
        } finally { f.editor.disconnectSession();withTimeout(5000){f.editor.state.first{!it.busy&&it.sync==null}};sessions.close() }
    }
}

private fun snapshot(sequence:ULong,hash:String,transform:Transform=Transform(),items:List<RenderItem>?=null):DocumentSnapshot {
    val info=ProjectInfo("project","fixture","owner",sequence+1u,true,false,sequence,hash,listOf("document"))
    val item=RenderItem("object","layer",1.0,"normal",Rect(10+transform.e,10+transform.f,40.0,40.0),Contours(longArrayOf(),longArrayOf(),uintArrayOf()),transform,ObjectStyle(0xffffffffu,3.0),Shape.Rectangle(Rect(10.0,10.0,40.0,40.0)),false)
    return DocumentSnapshot("document","fixture",64u,64u,8u,listOf(LayerInfo("layer","Annotations",true,false,1.0,"normal")),RenderList(info,items?:listOf(item)))
}
private class DocumentGate {
    val captured=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();val delivered=CompletableDeferred<Unit>()
}
private class FixtureProject:WorkbenchProject {
    var current=snapshot(7u,"hash-7");var nextDocumentGate:DocumentGate?=null;var beforeNextEdit:(()->Unit)?=null
    var beforeClose:(()->Unit)?=null;var lastOptions:EditOptions?=null;private var eventSequence=0uL
    var strokeFactory:((StrokeOptions)->WorkbenchStroke)?=null
    val accepted=mutableListOf<List<EditCommand>>()
    override val changes=MutableSharedFlow<ProjectChange>(extraBufferCapacity=16)
    fun bump(x:Double){val next=current.render.revision.hostSeq+1u;current=snapshot(next,"hash-$next",Transform(e=x))}
    fun optimistic(x:Double,hash:String){current=snapshot(current.render.revision.hostSeq,hash,Transform(e=x))}
    suspend fun announce(){changes.emit(ProjectChange(++eventSequence,ChangeKind.Committed,current.render.revision))}
    override suspend fun info():ProjectInfo=current.render.revision
    override suspend fun document(documentId:String):DocumentSnapshot{
        val captured=current;val gate=nextDocumentGate;nextDocumentGate=null
        if(gate!=null){gate.captured.complete(Unit);gate.release.await();gate.delivered.complete(Unit)}
        return captured
    }
    override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean)=BackgroundImage(64u,64u,ByteArray(64*64*4){if(it%4==3)255.toByte()else 60},"source",8u,byteArrayOf())
    override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo {
        beforeNextEdit?.also{beforeNextEdit=null;it()}
        // Behaves like the actual atomic worker precondition. Missing desktop
        // bindings would allow the stale command, making the race test fail.
        if(options.expectedHostSeq!=null&&(options.expectedHostSeq!=current.render.revision.hostSeq||options.expectedStateHash!=current.render.revision.stateHash))throw CoreFailure(CoreFailureKind.Invalid)
        accepted+=commands
        lastOptions=options
        val next=current.render.revision.hostSeq+1u
        val transform=commands.filterIsInstance<EditCommand.SetTransform>().lastOrNull()?.transform?:current.render.items.first().transform
        current=snapshot(next,"hash-$next",transform)
        return current.render.revision
    }
    override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=strokeFactory?.invoke(options)?:error("not used by fixture")
    override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("not used by fixture")
    override suspend fun render(documentId:String,rectangle:Rect?):RenderList=current.render
    override suspend fun export(options:ExportOptions):ExportResult=error("not used by fixture")
    override suspend fun close(){beforeClose?.invoke()}
}
private class FixtureCore(private val project:FixtureProject):WorkbenchCore {
    private var serial=0
    override fun newId(unixMs:ULong):String="transaction-${serial++}"
    override fun newDeviceId():String="owner"
    override suspend fun create(options:CreateProject):WorkbenchProject=error("not used by fixture")
    override suspend fun open(path:String):WorkbenchProject=project
    override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("not used by fixture")
    override fun cameraMatrix(camera:Camera,inverse:Boolean):Transform=Transform()
    override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>):List<Point> = points
}
private class ControllerFixture(val scope:CoroutineScope){
    val project=FixtureProject()
    val editor=EditorController(scope,FixtureCore(project),object:DesktopHandoff{
        override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("clipboard must not run in fixture")
        override suspend fun copy(path:Path,receipt:CompletedExport):Unit=error("clipboard must not run in fixture")
        override suspend fun stage(path:Path,receipt:CompletedExport):DesktopDragFile=error("drag must not run in fixture")
    },LocationPolicy(Path.of("fixture"),true,"fixture"),"owner")
    suspend fun start(){editor.viewport(100.0,100.0);editor.open(Path.of("fixture"));idle(7u);withTimeout(5000){editor.state.first{it.background!=null};project.changes.subscriptionCount.first{it>0}};ready();editor.select("object")}
    suspend fun idle(sequence:ULong){withTimeout(5000){editor.state.first{!it.busy&&it.document?.render?.revision?.hostSeq==sequence}}}
    suspend fun ready(){
        val state=withTimeout(5000){
            combine(editor.state,editor.instructionEditor.state,editor.semanticEditor.state){view,instructions,semantics->
                view.takeIf{!it.busy&&!instructions.busy&&!semantics.busy&&it.document!=null&&it.background!=null}
            }.first{it!=null}
        }
        val current=checkNotNull(state)
        editor.canvasPrepared(checkNotNull(current.document).binding(current.projectEpoch))
    }
}
private fun fixture(block:suspend(ControllerFixture)->Unit){
    Executors.newSingleThreadExecutor{r->Thread(r,"vw-desktop-test").apply{isDaemon=true}}.asCoroutineDispatcher().use{dispatcher->
        runBlocking(dispatcher){val scope=CoroutineScope(coroutineContext+SupervisorJob());val fixture=ControllerFixture(scope);try{withTimeout(15000){fixture.start();block(fixture)}}finally{fixture.editor.close();scope.cancel()}}
    }
}
