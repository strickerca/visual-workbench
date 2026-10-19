package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.util.Comparator
import java.util.concurrent.Executors
import javax.imageio.ImageIO

/** Actual controller/precision/FFI journal, with synthetic host contacts. */
class RotationEditorTest {
    @Test fun handleUpCommitsOneTransactionAndOrdinaryUndoRestoresItsOriginalTransform(){
        Executors.newSingleThreadExecutor().asCoroutineDispatcher().use{dispatcher->runBlocking(dispatcher){
            val root=Files.createTempDirectory("vw-rotation-editor-");val scope=CoroutineScope(coroutineContext+SupervisorJob())
            var editor:EditorController?=null
            try{
                val core=workbenchCore();val now=System.currentTimeMillis();val device=core.newDeviceId()
                fun id()=core.newId(now.toULong())
                val document=id();val layer=id();val objectId=id();val path=root.resolve("project")
                val source=ByteArrayOutputStream().also{check(ImageIO.write(BufferedImage(64,64,BufferedImage.TYPE_INT_RGB),"png",it))}.toByteArray()
                val project=core.create(CreateProject(path.toString(),id(),document,layer,device,"Rotation fixture",source,now))
                try{
                    project.edit(EditOptions(id(),document,device,project.info().nextLamport,now),listOf(
                        EditCommand.Create(objectId,layer,Shape.Rectangle(Rect(12.0,20.0,24.0,12.0)),ObjectStyle(0x112233ffu,1.0))))
                }finally{project.close()}
                val owner=EditorController(scope,core,object:DesktopHandoff {
                    override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("unused")
                    override suspend fun copy(path:java.nio.file.Path,receipt:CompletedExport)=error("unused")
                    override suspend fun stage(path:java.nio.file.Path,receipt:CompletedExport):DesktopDragFile=error("unused")
                },LocationPolicy(root,true,"owned synthetic fixture"),device)
                editor=owner;owner.viewport(200.0,200.0);owner.open(path)
                until{!owner.state.value.busy&&(owner.state.value.background!=null||owner.state.value.needsColorConsent)}
                if(owner.state.value.needsColorConsent)owner.assumeSrgb()
                until{!owner.state.value.busy&&owner.state.value.background!=null&&!owner.instructionEditor.state.value.busy&&!owner.semanticEditor.state.value.busy}
                owner.tool(Tool.Select);owner.select(objectId)
                val before=checkNotNull(owner.state.value.document).render.revision
                owner.canvasPrepared(checkNotNull(owner.state.value.document).binding(owner.state.value.projectEpoch))
                val handle=checkNotNull(owner.rotationHandle(owner.state.value));val camera=owner.state.value.view.camera
                val from=core.mapPoints(camera,false,listOf(handle.point)).single()
                val end=Point(handle.pivot.x-(handle.point.y-handle.pivot.y),handle.pivot.y+(handle.point.x-handle.pivot.x))
                val to=core.mapPoints(camera,false,listOf(end)).single()
                owner.precisionDown(from,false,false,100)
                assertTrue(owner.rotation.state.value.active)
                owner.precisionMove(to,120,snapRotation15=true);owner.precisionUp(to,140,snapRotation15=true)
                owner.finishPointerContact()
                until{!owner.rotation.state.value.busy&&owner.state.value.preview.isEmpty()&&owner.state.value.document?.render?.revision?.hostSeq==before.hostSeq+1u}
                assertNotEquals(Transform(),owner.state.value.document?.render?.items?.single()?.transform)
                owner.undo(false);until{owner.state.value.document?.render?.revision?.stateHash==before.stateHash}
                assertEquals(Transform(),owner.state.value.document?.render?.items?.single()?.transform)
            }finally{
                editor?.close();scope.coroutineContext[Job]?.cancelAndJoin()
                Files.walk(root).use{paths->paths.sorted(Comparator.reverseOrder()).forEach{Files.delete(it)}}
            }
        }}
    }
    private suspend fun until(test:()->Boolean){withTimeout(10_000){while(!test())delay(5)}}
}
