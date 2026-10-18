package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Path
import java.util.concurrent.Executors

/** Actual controller/refresh ownership with injected document and file APIs.
 * No AWT window, clipboard, device, disk project, or network is started. */
class DocumentSwitchTest {
    @Test fun switchingKeepsEachCameraWithoutClosingTheProject()=fixture { f ->
        f.editor.pan(12.0,7.0);f.editor.zoom(1.5)
        val first=f.editor.state.value.view.camera
        assertNotNull(f.editor.selectDocument("second"));f.idle("second")
        f.editor.pan(-8.0,13.0);f.editor.zoom(.7)
        val second=f.editor.state.value.view.camera
        assertNotNull(f.editor.selectDocument("first"));f.idle("first")
        assertEquals(first,f.editor.state.value.view.camera)
        assertNotNull(f.editor.selectDocument("second"));f.idle("second")
        assertEquals(second,f.editor.state.value.view.camera)
        assertEquals(0,f.project.closed)
    }

    @Test fun anOldDocumentQueryCannotOverwriteTheSelectedDocumentEvenAtTheSameProjectRevision()=fixture { f ->
        val gate=Gate();f.project.nextFirst=gate;f.project.advance()
        f.project.announce();gate.entered.await()
        try {
            assertNotNull(f.editor.selectDocument("second"));f.idle("second")
            val selected=f.editor.state.value.document
            gate.release.complete(Unit);gate.delivered.await();yield()
            assertEquals(selected,f.editor.state.value.document)
            assertEquals("second",f.editor.state.value.document!!.documentId)
            assertEquals(0,f.project.closed)
        } finally { gate.release.complete(Unit) }
    }

    private class Gate {
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();val delivered=CompletableDeferred<Unit>()
    }
    private class Project:WorkbenchProject {
        override val changes=MutableSharedFlow<ProjectChange>(extraBufferCapacity=8)
        var revision=ProjectInfo("project","Many documents","device",1uL,false,false,0uL,"state-0",listOf("first","second"))
        var nextFirst:Gate?=null;var closed=0
        override suspend fun info()=revision
        override suspend fun document(documentId:String):DocumentSnapshot {
            require(documentId in revision.documentIds)
            val result=DocumentSnapshot(documentId,documentId,64u,64u,8u,listOf(LayerInfo("layer-$documentId","Annotations",true,false,1.0,"normal")),RenderList(revision,emptyList()))
            val gate=if(documentId=="first")nextFirst.also{nextFirst=null}else null
            if(gate!=null){gate.entered.complete(Unit);gate.release.await();gate.delivered.complete(Unit)}
            return result
        }
        override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean)=BackgroundImage(1u,1u,byteArrayOf(-1,-1,-1,-1),"source",8u,byteArrayOf())
        override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("no drawing in fixture")
        override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("no edit in fixture")
        override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("no history edit in fixture")
        override suspend fun render(documentId:String,rectangle:Rect?)=document(documentId).render
        override suspend fun export(options:ExportOptions):ExportResult=error("no export in fixture")
        override suspend fun close(){closed++}
        fun advance(){revision=revision.copy(hostSeq=1uL,stateHash="state-1",nextLamport=2uL)}
        suspend fun announce(){changes.emit(ProjectChange(1uL,ChangeKind.Committed,revision))}
    }
    private class Core(private val project:Project):WorkbenchCore {
        override fun newId(unixMs:ULong)="unused"
        override fun newDeviceId()="device"
        override suspend fun create(options:CreateProject):WorkbenchProject=error("no file creation")
        override suspend fun open(path:String):WorkbenchProject=project
        override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("no text")
        override fun cameraMatrix(camera:Camera,inverse:Boolean)=Transform()
        override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>)=points
    }
    private class Fixture(val scope:CoroutineScope) {
        val project=Project()
        val editor=EditorController(scope,Core(project),object:DesktopHandoff {
            override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("no clipboard")
            override suspend fun copy(path:Path,receipt:CompletedExport):Unit=error("no clipboard")
            override suspend fun stage(path:Path,receipt:CompletedExport):DesktopDragFile=error("no drag")
        },LocationPolicy(Path.of("fixture-document-controls"),true,"fixture"),"device")
        suspend fun start(){editor.viewport(100.0,100.0);editor.open(Path.of("fixture-document-controls"));idle("first");project.changes.subscriptionCount.first{it>0}}
        suspend fun idle(document:String){withTimeout(5000){editor.state.first{!it.busy&&it.document?.documentId==document&&it.background!=null}}}
    }
    private fun fixture(block:suspend(Fixture)->Unit) {
        Executors.newSingleThreadExecutor{r->Thread(r,"vw-document-controls-test").apply{isDaemon=true}}.asCoroutineDispatcher().use { dispatcher ->
            runBlocking(dispatcher) {
                val scope=CoroutineScope(coroutineContext+SupervisorJob());val f=Fixture(scope)
                try { withTimeout(15_000){f.start();block(f)} }
                finally { withContext(NonCancellable){f.project.nextFirst?.release?.complete(Unit);f.editor.close();scope.cancel()} }
                assertEquals(1,f.project.closed)
            }
        }
    }
}
