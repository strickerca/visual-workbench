package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.Executors
import javax.imageio.ImageIO

/** Actual controller/native storage; no AWT window, clipboard or provider. */
class ImageOutputControllerTest {
    @Test fun toneApplyRefreshReopenFurtherEditAndUndoKeepTheEditingCanvasUsable()=fixture{f->
        val original=f.editor.state.value.document!!.render.revision.stateHash
        f.openPanel()
        assertTrue(f.editor.imageOutputs.selectTone(null))
        assertTrue(f.editor.imageOutputs.edit(ImageOutputForm.Tone(null,brightness="-1.0")))
        assertTrue(f.editor.imageOutputs.apply())
        f.await{f.editor.imageOutputs.state.value.ready && f.editor.imageOutputs.state.value.snapshot?.tones?.size==1}
        val saved=f.editor.state.value.document!!.render.revision.stateHash
        assertNotEquals(original,saved);assertNull(f.editor.state.value.renderIssue)
        assertTrue(f.editor.state.value.document!!.render.items.none{it.shape is Shape.Adjustment})
        f.editor.imageOutput(false);f.reopen()
        assertEquals(saved,f.editor.state.value.document!!.render.revision.stateHash)
        assertNull(f.editor.state.value.renderIssue)
        f.openPanel()
        val tone=f.editor.imageOutputs.state.value.snapshot!!.tones.single()
        assertTrue(f.editor.imageOutputs.selectTone(tone.objectId))
        val form=f.editor.imageOutputs.state.value.form as ImageOutputForm.Tone
        assertTrue(f.editor.imageOutputs.edit(form.copy(brightness="-0.5")))
        assertTrue(f.editor.imageOutputs.apply())
        f.await{f.editor.imageOutputs.state.value.ready && f.editor.imageOutputs.state.value.snapshot?.tones?.singleOrNull()?.settings?.brightness==-.5}
        assertNotEquals(saved,f.editor.state.value.document!!.render.revision.stateHash)
        f.editor.imageOutput(false);f.editor.undo(false)
        f.await{!f.editor.state.value.busy && f.editor.state.value.document?.render?.revision?.stateHash==saved}
        f.editor.undo(false)
        f.await{!f.editor.state.value.busy && f.editor.state.value.document?.render?.revision?.stateHash==original}
        assertNull(f.editor.state.value.renderIssue)
    }

    private class Fixture(val scope:CoroutineScope,val root:Path) {
        val core=workbenchCore();private val device=core.newDeviceId()
        private val path=root.resolve("project")
        lateinit var editor:EditorController
        suspend fun start(){
            val image=BufferedImage(64,64,BufferedImage.TYPE_INT_ARGB)
            for(y in 0 until 64)for(x in 0 until 64)image.setRGB(x,y,-1)
            val png=ByteArrayOutputStream().also{check(ImageIO.write(image,"png",it))}.toByteArray()
            val now=System.currentTimeMillis();fun id()=core.newId(now.toULong())
            val seed=core.create(CreateProject(path.toString(),id(),id(),id(),device,"Output fixture",png,now))
            withContext(NonCancellable){seed.close()}
            fresh()
        }
        private suspend fun fresh(){
            editor=EditorController(scope,core,object:DesktopHandoff {
                override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("No clipboard")
                override suspend fun copy(path:Path,receipt:CompletedExport):Unit=error("No clipboard")
                override suspend fun stage(path:Path,receipt:CompletedExport):DesktopDragFile=error("No drag")
            },LocationPolicy(root,true,"image output fixture"),device)
            editor.viewport(256.0,256.0);editor.open(path)
            await{!editor.state.value.busy && editor.state.value.document!=null}
            if(editor.state.value.needsColorConsent)editor.assumeSrgb()
            await{!editor.state.value.busy && editor.state.value.background!=null &&
                !editor.instructionEditor.state.value.busy && !editor.semanticEditor.state.value.busy && editor.localViews.state.value.available}
        }
        suspend fun openPanel(){
            await{!editor.state.value.busy && !editor.instructionEditor.state.value.busy && !editor.semanticEditor.state.value.busy}
            editor.imageOutput(true);await{editor.imageOutputs.state.value.ready}
        }
        suspend fun reopen(){editor.close();fresh()}
        suspend fun close(){if(::editor.isInitialized)editor.close()}
        suspend fun await(condition:()->Boolean)=withTimeout(15_000){while(!condition())delay(10)}
    }
    private fun fixture(block:suspend(Fixture)->Unit){
        Executors.newSingleThreadExecutor{r->Thread(r,"vw-output-controller-fixture").apply{isDaemon=true}}.asCoroutineDispatcher().use{dispatcher->
            runBlocking(dispatcher){
                val root=Files.createTempDirectory("vw-image-output-").toRealPath()
                val owner=SupervisorJob(coroutineContext[Job]);val scope=CoroutineScope(coroutineContext+owner);val f=Fixture(scope,root)
                try{withTimeout(60_000){f.start();block(f)}}finally{withContext(NonCancellable){
                    try{f.close()}finally{owner.cancelAndJoin()}
                    assertTrue(root.fileName.toString().startsWith("vw-image-output-"));assertTrue(root.toFile().deleteRecursively())
                }}
            }
        }
    }
}
