package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.Executors
import javax.imageio.ImageIO
import kotlin.math.cos
import kotlin.math.sin

/** Actual editor input/controller and native store/undo, with an owned synthetic
 * PNG project and no AWT window, real input, clipboard, provider or network. */
class ObjectRotationControllerTest {
    @Test fun groupRotationUsesTheFinalPointerAndCurrentShiftInOneUndoableTransaction()=fixture { f ->
        val before=f.document();val handle=f.handle();val preview=f.turned(handle,13.0);val end=f.turned(handle,26.0)
        f.editor.pointerDown(handle.knob,false,false)
        f.editor.pointerMove(preview,snapRotation=true)
        assertEquals(before.render.revision.stateHash,f.project.info().stateHash)
        f.editor.pointerUp(end,snapRotation=false);f.editor.pointerUp(end,snapRotation=true)
        f.committed(before.render.revision.hostSeq+1uL)
        val changed=f.document()
        assertEquals(2,changed.render.items.size)
        for(item in changed.render.items){assertEquals(cos(26.0*Math.PI/180.0),item.transform.a,1e-12);assertEquals(sin(26.0*Math.PI/180.0),item.transform.b,1e-12)}
        f.editor.undo(false)
        f.await{!f.editor.state.value.busy&&f.document().render.revision.stateHash==before.render.revision.stateHash}
        assertEquals(before.render.items.map{it.transform},f.document().render.items.map{it.transform})
    }

    @Test fun cancellationAndAChangedCameraRetirePrivateGeometryWithoutAnEdit()=fixture { f ->
        val before=f.document();val handle=f.handle();val end=f.turned(handle,45.0)
        f.editor.pointerDown(handle.knob,false,false);f.editor.pointerMove(end);f.editor.cancelInput();f.editor.pointerUp(end)
        assertEquals(before.render.revision.stateHash,f.project.info().stateHash)
        assertTrue(f.editor.state.value.preview.isEmpty())
        f.editor.pointerDown(handle.knob,false,false);f.editor.pointerMove(end);f.editor.zoom(1.5);f.editor.pointerUp(end)
        assertEquals(before.render.revision.stateHash,f.project.info().stateHash)
        assertTrue(f.editor.state.value.preview.isEmpty())
    }

    @Test fun anAcceptedConcurrentRevisionRefusesTheOldContactAndTheNextContactCanCommit()=fixture { f ->
        val before=f.document();val handle=f.handle();val end=f.turned(handle,35.0)
        f.editor.pointerDown(handle.knob,false,false);f.editor.pointerMove(end)
        val revision=f.project.info();val objectId=f.document().render.items.first().objectId
        val accepted=f.project.edit(EditOptions(f.id(),f.document().documentId,revision.deviceId,revision.nextLamport,System.currentTimeMillis()),
            listOf(EditCommand.SetStyle(objectId,ObjectStyle(0xff0000ffu,1.0,fill=0xff000080u))))
        f.editor.pointerUp(end)
        f.committed(accepted.hostSeq)
        assertTrue(f.document().render.items.all{it.transform==Transform()})
        assertEquals(before.render.revision.hostSeq+1uL,f.project.info().hostSeq)
        f.prepare();val fresh=f.handle();val finish=f.turned(fresh,13.0)
        f.editor.pointerDown(fresh.knob,false,false);f.editor.pointerUp(finish,snapRotation=true)
        f.committed(accepted.hostSeq+1uL)
        assertTrue(f.document().render.items.all{kotlin.math.abs(it.transform.b-sin(Math.PI/12.0))<1e-12})
    }

    private class Fixture(val scope:CoroutineScope,val root:Path) {
        private val actual=workbenchCore()
        lateinit var project:WorkbenchProject
        private val core=object:WorkbenchCore by actual {
            override suspend fun open(path:String):WorkbenchProject=actual.open(path).also{project=it}
        }
        val editor=EditorController(scope,core,object:DesktopHandoff {
            override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("No clipboard in fixture")
            override suspend fun copy(path:Path,receipt:CompletedExport):Unit=error("No clipboard in fixture")
            override suspend fun stage(path:Path,receipt:CompletedExport):DesktopDragFile=error("No drag in fixture")
        },LocationPolicy(root,true,"rotation fixture"),actual.newDeviceId())
        fun id()=actual.newId(System.currentTimeMillis().toULong())
        fun document():DocumentSnapshot=checkNotNull(editor.state.value.document)
        suspend fun start() {
            val image=BufferedImage(64,64,BufferedImage.TYPE_INT_ARGB)
            for(y in 0 until 64)for(x in 0 until 64)image.setRGB(x,y,-1)
            val png=ByteArrayOutputStream().also{assertTrue(ImageIO.write(image,"png",it))}.toByteArray()
            val doc=id();val layer=id();val device=actual.newDeviceId();val path=root.resolve("project")
            val seed=actual.create(CreateProject(path.toString(),id(),doc,layer,device,"Rotation",png,System.currentTimeMillis()))
            try {
                val options=EditOptions(id(),doc,device,seed.info().nextLamport,System.currentTimeMillis())
                seed.edit(options,listOf(
                    EditCommand.Create(id(),layer,Shape.Rectangle(Rect(10.0,20.0,14.0,16.0)),ObjectStyle(0x102030ffu,1.0,fill=0x10203080u)),
                    EditCommand.Create(id(),layer,Shape.Rectangle(Rect(36.0,24.0,14.0,16.0)),ObjectStyle(0x405060ffu,1.0,fill=0x40506080u))))
            } finally { withContext(NonCancellable){seed.close()} }
            editor.viewport(400.0,400.0);editor.open(path)
            await{!editor.state.value.busy&&editor.state.value.document!=null}
            if(editor.state.value.needsColorConsent)editor.assumeSrgb()
            await{!editor.state.value.busy&&editor.state.value.background!=null&&!editor.semanticEditor.state.value.busy}
            prepare()
        }
        fun prepare(){editor.tool(Tool.Select);editor.selectAll();editor.canvasPrepared(document().binding(editor.state.value.projectEpoch))}
        fun handle():ObjectRotationHandle=checkNotNull(desktopRotationHandle(core,editor.state.value.view.camera,document().render.items))
        fun turned(handle:ObjectRotationHandle,degrees:Double):Point {
            val camera=editor.state.value.view.camera
            val values=core.mapPoints(camera,true,listOf(handle.center,handle.knob));val center=values[0]
            val x=values[1].x-center.x;val y=values[1].y-center.y;val angle=degrees*Math.PI/180.0
            return core.mapPoints(camera,false,listOf(Point(center.x+cos(angle)*x-sin(angle)*y,center.y+sin(angle)*x+cos(angle)*y))).single()
        }
        suspend fun committed(sequence:ULong){await{!editor.state.value.busy&&document().render.revision.hostSeq==sequence&&!editor.semanticEditor.state.value.busy}}
        suspend fun await(condition:()->Boolean)=withTimeout(10_000){while(!condition())delay(10)}
    }
    private fun fixture(block:suspend(Fixture)->Unit) {
        Executors.newSingleThreadExecutor{r->Thread(r,"vw-rotation-fixture").apply{isDaemon=true}}.asCoroutineDispatcher().use { dispatcher ->
            runBlocking(dispatcher) {
                val root=Files.createTempDirectory("vw-object-rotation-").toRealPath()
                val owner=SupervisorJob(coroutineContext[Job]);val scope=CoroutineScope(coroutineContext+owner);val f=Fixture(scope,root)
                try { withTimeout(30_000){f.start();block(f)} }
                finally { withContext(NonCancellable){
                    try { f.editor.close() } finally { owner.cancelAndJoin() }
                    // Reached only after successful native close; never remove
                    // files beneath a project whose close was uncertain.
                    assertTrue(root.fileName.toString().startsWith("vw-object-rotation-"))
                    assertTrue(root.toFile().deleteRecursively())
                } }
            }
        }
    }
}
