package com.visualworkbench.android

import android.app.Application
import android.graphics.Bitmap
import androidx.lifecycle.ViewModelStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File

/** Actual editor -> shared local owner -> native settings. Synthetic camera
 * methods only; no screenshot, real input, clipboard, network or provider. */
@RunWith(AndroidJUnit4::class)
class LocalViewsInstrumentedTest {
    @Test fun finalUndebouncedCameraSurvivesViewModelClosureAndRestoresIntoTheCurrentViewport() = runBlocking {
        fixture { f ->
            val before=withContext(Dispatchers.Main){checkNotNull(f.editor.info)}
            val saved=withContext(Dispatchers.Main){
                f.editor.pan(17.0,-11.0)
                f.editor.pinch(100.0,80.0,109.0,83.0,1.375,0.21)
                checkNotNull(f.editor.scene).camera
            }
            // No delay for the 400ms debounce: final close must flush/join it.
            f.reopen(320,240)
            withContext(Dispatchers.Main){
                sameWorld(saved,checkNotNull(f.editor.scene).camera)
                assertEquals(320.0,f.editor.scene!!.camera.viewportWidth,0.0)
                assertEquals(240.0,f.editor.scene!!.camera.viewportHeight,0.0)
                assertEquals(before.hostSeq,f.editor.info!!.hostSeq)
                assertEquals(before.stateHash,f.editor.info!!.stateHash)
            }
        }
    }

    @Test fun namedRegionRestoresAfterReopenAndAStalePanelBindingCannotEditIt() = runBlocking {
        fixture { f ->
            val saved=withContext(Dispatchers.Main){f.editor.pan(-23.0,8.0);f.editor.localViews(true);f.editor.scene!!.camera}
            val binding=withContext(Dispatchers.Main){checkNotNull(f.editor.localViews.state.value.binding)}
            withContext(Dispatchers.Main){f.editor.saveLocalView(binding,"Synthetic close detail")}
            waitUntil{f.editor.localViews.state.value.bookmarks.size==1 && !f.editor.localViews.state.value.saving}
            val id=withContext(Dispatchers.Main){f.editor.localViews.state.value.bookmarks.single().id}
            withContext(Dispatchers.Main){f.editor.localViews(false);f.editor.pan(46.0,-5.0)}
            f.reopen(280,220)
            withContext(Dispatchers.Main){
                assertEquals("Synthetic close detail",f.editor.localViews.state.value.bookmarks.single().name)
                val wrong=binding.copy(projectId=f.core.newId(System.currentTimeMillis().toULong()))
                val before=f.editor.scene!!.camera
                f.editor.restoreLocalView(wrong,id)
                assertEquals(before,f.editor.scene!!.camera)
                assertFalse(f.editor.localViews.deleteBookmark(wrong,id))
                f.editor.localViews(true);f.editor.restoreLocalView(binding,id)
                sameWorld(saved,f.editor.scene!!.camera)
                assertEquals(280.0,f.editor.scene!!.camera.viewportWidth,0.0)
                assertFalse(f.editor.localViews.state.value.active)
            }
        }
    }

    private class Fixture(val app:Application) {
        private val actual=workbenchCore()
        private val handles=mutableListOf<WorkbenchProject>()
        lateinit var owned:File
        private lateinit var store:ViewModelStore
        lateinit var editor:EditorController
        val core:WorkbenchCore=object:WorkbenchCore by actual {
            override suspend fun create(options:CreateProject):WorkbenchProject {
                owned=File(options.path).canonicalFile
                val bitmap=Bitmap.createBitmap(64,64,Bitmap.Config.ARGB_8888)
                val png=try {bitmap.eraseColor(-1);ByteArrayOutputStream().also{assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG,100,it))}.toByteArray()} finally {bitmap.recycle()}
                return actual.create(options.copy(source=png)).also{handles+=it}
            }
            override suspend fun open(path:String):WorkbenchProject=actual.open(path).also{
                if(::owned.isInitialized && File(path).canonicalFile==owned)handles+=it
            }
        }
        private suspend fun fresh(width:Int,height:Int) {
            withContext(Dispatchers.Main){store=ViewModelStore();editor=EditorController(app,core);store.put("local-views",editor)}
            waitUntil{!editor.busy}
            withContext(Dispatchers.Main){editor.viewport(width,height,1f)}
        }
        suspend fun start(){fresh(240,200);withContext(Dispatchers.Main){editor.newCanvas()};ready()}
        suspend fun reopen(width:Int,height:Int){release();fresh(width,height);withContext(Dispatchers.Main){assertNotNull(editor.open(owned.name))};ready()}
        private suspend fun ready(){
            waitUntil{!editor.busy && editor.document!=null}
            withContext(Dispatchers.Main){if(editor.needsColorConsent)editor.assumeSrgb()}
            waitUntil{!editor.busy && editor.scene?.background!=null && editor.localViews.state.value.available && !editor.semanticEditor.state.value.busy}
        }
        suspend fun release()=withContext(NonCancellable){
            if(::editor.isInitialized){withContext(Dispatchers.Main){store.clear()};withTimeout(15_000){checkNotNull(editor.scope.coroutineContext[Job]).join()}}
            // Observe native completion too; a failed close preserves its files.
            for(handle in handles)withTimeout(15_000){handle.close()}
            handles.clear()
        }
        suspend fun clean(){
            release()
            if(::owned.isInitialized){
                assertEquals(File(app.filesDir,"projects").canonicalFile,owned.parentFile)
                assertTrue(owned.name.matches(Regex("[0-9a-fA-F-]{36}")))
                assertTrue(!owned.exists()||owned.deleteRecursively())
            }
        }
    }
    private suspend fun fixture(block:suspend(Fixture)->Unit){
        val app=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val f=Fixture(app)
        try {withTimeout(45_000){f.start();block(f)}} finally {withContext(NonCancellable){f.clean()}}
    }
    private companion object {
        suspend fun waitUntil(condition:()->Boolean)=withTimeout(10_000){while(!withContext(Dispatchers.Main){condition()})delay(10)}
        fun sameWorld(a:Camera,b:Camera){
            assertEquals(a.center.x.toBits(),b.center.x.toBits());assertEquals(a.center.y.toBits(),b.center.y.toBits())
            assertEquals(a.scale.toBits(),b.scale.toBits());assertEquals(a.rotation.toBits(),b.rotation.toBits())
        }
    }
}
