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

/** Native settings and actual editor open/close. No AWT window or OS clipboard. */
class LocalViewsControllerTest {
    @Test fun immediateCloseFlushesTheFinalCameraAndBookmarksSurviveAnotherEditor()=fixture { f ->
        val initial=f.project.info()
        f.editor.pan(17.0,-11.0);f.editor.zoom(1.375)
        val saved=f.editor.state.value.view.camera
        f.editor.localViews(true)
        val binding=checkNotNull(f.editor.localViews.state.value.binding)
        f.editor.saveLocalView(binding,"Synthetic region")
        f.await{f.editor.localViews.state.value.bookmarks.size==1&&!f.editor.localViews.state.value.saving}
        val mark=f.editor.localViews.state.value.bookmarks.single().id
        f.editor.localViews(false);f.editor.pan(-22.0,10.0)
        val final=f.editor.state.value.view.camera
        // Do not wait for debounce; actual close joins the native settings write.
        f.reopen(420.0,360.0)
        sameWorld(final,f.editor.state.value.view.camera)
        assertEquals(420.0,f.editor.state.value.view.camera.viewportWidth,0.0)
        assertEquals(initial.stateHash,f.project.info().stateHash)
        assertEquals(initial.hostSeq,f.project.info().hostSeq)
        assertEquals("Synthetic region",f.editor.localViews.state.value.bookmarks.single().name)
        f.editor.localViews(true);f.editor.restoreLocalView(binding,mark)
        sameWorld(saved,f.editor.state.value.view.camera)
        assertEquals(360.0,f.editor.state.value.view.camera.viewportHeight,0.0)
        assertFalse(f.editor.localViews.state.value.active)
    }

    @Test fun aSecondInstallationGetsItsOwnViewAndOldPanelBindingCannotMutateIt()=fixture { f ->
        val originalDevice=f.device
        f.editor.pan(25.0,12.0);f.editor.zoom(1.75)
        val saved=f.editor.state.value.view.camera
        f.editor.localViews(true)
        val binding=f.editor.localViews.state.value.binding!!
        f.editor.saveLocalView(binding,"Private installation region")
        f.await{f.editor.localViews.state.value.bookmarks.size==1&&!f.editor.localViews.state.value.saving}
        val mark=f.editor.localViews.state.value.bookmarks.single().id
        f.reopen(300.0,240.0,f.core.newDeviceId())
        assertTrue(f.editor.localViews.state.value.bookmarks.isEmpty())
        val second=f.editor.state.value.view.camera
        assertNotEquals(saved.scale,second.scale)
        f.editor.restoreLocalView(binding,mark)
        assertEquals(second,f.editor.state.value.view.camera)
        assertFalse(f.editor.localViews.deleteBookmark(binding,mark))
        f.reopen(300.0,240.0,originalDevice)
        sameWorld(saved,f.editor.state.value.view.camera)
        assertEquals(mark,f.editor.localViews.state.value.bookmarks.single().id)
    }

    private class Fixture(val scope:CoroutineScope,val root:Path) {
        private val actual=workbenchCore()
        lateinit var project:WorkbenchProject
        val core:WorkbenchCore=object:WorkbenchCore by actual {
            override suspend fun open(path:String):WorkbenchProject=actual.open(path).also{project=it}
        }
        var device=core.newDeviceId();private set
        lateinit var editor:EditorController
        private val path=root.resolve("project")
        suspend fun start(){
            val image=BufferedImage(64,64,BufferedImage.TYPE_INT_ARGB)
            for(y in 0 until 64)for(x in 0 until 64)image.setRGB(x,y,-1)
            val png=ByteArrayOutputStream().also{assertTrue(ImageIO.write(image,"png",it))}.toByteArray()
            val now=System.currentTimeMillis();fun id()=core.newId(now.toULong())
            val seed=core.create(CreateProject(path.toString(),id(),id(),id(),device,"Local views",png,now))
            withContext(NonCancellable){seed.close()}
            fresh(300.0,240.0)
        }
        private suspend fun fresh(width:Double,height:Double){
            editor=EditorController(scope,core,object:DesktopHandoff {
                override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("No clipboard")
                override suspend fun copy(path:Path,receipt:CompletedExport):Unit=error("No clipboard")
                override suspend fun stage(path:Path,receipt:CompletedExport):DesktopDragFile=error("No drag")
            },LocationPolicy(root,true,"local view fixture"),device)
            editor.viewport(width,height);editor.open(path)
            await{!editor.state.value.busy&&editor.state.value.document!=null}
            if(editor.state.value.needsColorConsent)editor.assumeSrgb()
            await{!editor.state.value.busy&&editor.state.value.background!=null&&editor.localViews.state.value.available&&!editor.semanticEditor.state.value.busy}
        }
        suspend fun reopen(width:Double,height:Double,nextDevice:String=device){editor.close();device=nextDevice;fresh(width,height)}
        suspend fun close(){if(::editor.isInitialized)editor.close();if(::project.isInitialized)project.close()}
        suspend fun await(condition:()->Boolean)=withTimeout(10_000){while(!condition())delay(10)}
    }
    private fun fixture(block:suspend(Fixture)->Unit){
        Executors.newSingleThreadExecutor{r->Thread(r,"vw-local-views-fixture").apply{isDaemon=true}}.asCoroutineDispatcher().use {dispatcher->
            runBlocking(dispatcher){
                val root=Files.createTempDirectory("vw-local-views-").toRealPath()
                val owner=SupervisorJob(coroutineContext[Job]);val scope=CoroutineScope(coroutineContext+owner);val f=Fixture(scope,root)
                try {withTimeout(45_000){f.start();block(f)}} finally {withContext(NonCancellable){
                    try {f.close()} finally {owner.cancelAndJoin()}
                    assertTrue(root.fileName.toString().startsWith("vw-local-views-"))
                    assertTrue(root.toFile().deleteRecursively())
                }}
            }
        }
    }
    private companion object {
        fun sameWorld(a:Camera,b:Camera){
            assertEquals(a.center.x.toBits(),b.center.x.toBits());assertEquals(a.center.y.toBits(),b.center.y.toBits())
            assertEquals(a.scale.toBits(),b.scale.toBits());assertEquals(a.rotation.toBits(),b.rotation.toBits())
        }
    }
}
