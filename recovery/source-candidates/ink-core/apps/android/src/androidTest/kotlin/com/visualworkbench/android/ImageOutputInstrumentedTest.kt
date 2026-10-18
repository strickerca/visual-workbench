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

/** Actual editor -> shared output owner -> native transaction/preview/export
 * preflight. Only synthetic owned pixels; no device input, clipboard or provider. */
@RunWith(AndroidJUnit4::class)
class ImageOutputInstrumentedTest {
    @Test fun committedOutputChangesExportDimensionsWhileOriginalAndUndoRemainExact()=runBlocking {
        fixture{editor->
            val before=withContext(Dispatchers.Main){editor.info!!.stateHash}
            open(editor)
            withContext(Dispatchers.Main){
                assertTrue(editor.imageOutputs.edit(ImageOutputForm.Frame("4","8","40","20",90u,"100","200")))
                assertTrue(editor.imageOutputs.apply())
            }
            waitUntil{editor.imageOutputs.state.value.ready && editor.imageOutputs.state.value.snapshot?.output?.settings?.outputWidth==100u}
            withContext(Dispatchers.Main){
                assertEquals(64u,editor.document!!.width);assertEquals(64u,editor.document!!.height)
                editor.imageOutput(false);editor.imageOutputs.requestExportContext()
            }
            waitUntil{editor.imageOutputs.exportSnapshot(editor.document!!)!=null}
            withContext(Dispatchers.Main){editor.prepareExport()}
            waitUntil{!editor.busy && editor.exportPreflight!=null}
            withContext(Dispatchers.Main){
                assertEquals(100u,editor.exportPreflight!!.width);assertEquals(200u,editor.exportPreflight!!.height)
                editor.imageOutputs.endExportContext();editor.undo()
            }
            waitUntil{editor.pending==0 && editor.info?.stateHash==before}
            open(editor)
            withContext(Dispatchers.Main){assertNull(editor.imageOutputs.state.value.snapshot!!.output)}
        }
    }
    @Test fun previewUsesCommittedSettingsAndPrivateDraftBlocksSwitchUntilDiscarded()=runBlocking {
        fixture{editor->
            open(editor)
            val before=withContext(Dispatchers.Main){editor.info!!.stateHash}
            withContext(Dispatchers.Main){
                val form=editor.imageOutputs.state.value.form as ImageOutputForm.Frame
                assertTrue(editor.imageOutputs.edit(form.copy(outputWidth="10",outputHeight="12")))
                assertFalse(editor.mayLeaveImageOutput())
                assertTrue(editor.imageOutputs.preview(0u,0u,false,true))
            }
            waitUntil{!editor.imageOutputs.state.value.previewLoading && editor.imageOutputs.state.value.preview!=null}
            withContext(Dispatchers.Main){
                val shown=editor.imageOutputs.state.value.preview!!
                assertEquals(64u,shown.outputWidth);assertEquals(64u,shown.outputHeight)
                assertEquals(before,editor.info!!.stateHash);assertNull(editor.exportPreflight)
                assertTrue(editor.imageOutputs.discardFormAndReload())
            }
            waitUntil{editor.imageOutputs.state.value.ready}
            withContext(Dispatchers.Main){assertTrue(editor.mayLeaveImageOutput());editor.imageOutput(false)}
        }
    }
    @Test fun savedTonePreviewIsQueriedThroughTheRealOutputCapability()=runBlocking {
        fixture{editor->
            open(editor)
            withContext(Dispatchers.Main){
                assertTrue(editor.imageOutputs.selectTone(null))
                assertTrue(editor.imageOutputs.edit(ImageOutputForm.Tone(null,brightness="-1.0")))
                assertTrue(editor.imageOutputs.apply())
            }
            waitUntil{editor.imageOutputs.state.value.ready && editor.imageOutputs.state.value.snapshot?.tones?.size==1}
            withContext(Dispatchers.Main){assertTrue(editor.imageOutputs.preview(0u,0u,false,true))}
            waitUntil{!editor.imageOutputs.state.value.previewLoading && editor.imageOutputs.state.value.preview!=null}
            withContext(Dispatchers.Main){
                val rgba=editor.imageOutputs.state.value.preview!!.rgba
                assertEquals(64*64*4,rgba.size)
                assertEquals(0,rgba[0].toInt() and 255);assertEquals(0,rgba[1].toInt() and 255);assertEquals(0,rgba[2].toInt() and 255)
                assertEquals(255,rgba[3].toInt() and 255)
            }
        }
    }
    @Test fun toneApplyRefreshReopenFurtherEditAndUndoKeepTheEditingCanvasUsable()=runBlocking {
        fixture{editor->
            val original=withContext(Dispatchers.Main){editor.info!!.stateHash}
            open(editor)
            withContext(Dispatchers.Main){
                assertTrue(editor.imageOutputs.selectTone(null))
                assertTrue(editor.imageOutputs.edit(ImageOutputForm.Tone(null,brightness="-1.0")))
                assertTrue(editor.imageOutputs.apply())
            }
            waitUntil{editor.imageOutputs.state.value.ready && editor.imageOutputs.state.value.snapshot?.tones?.size==1 &&
                editor.scene?.document?.render?.revision?.stateHash==editor.info?.stateHash}
            val saved=withContext(Dispatchers.Main){
                assertTrue(editor.document!!.render.items.none{it.shape is Shape.Adjustment})
                editor.imageOutput(false);editor.info!!
            }
            val reopened=withContext(Dispatchers.Main){editor.open(saved.projectId)}
            checkNotNull(reopened).join()
            waitUntil{!editor.busy && editor.scene?.background!=null && editor.info?.stateHash==saved.stateHash}
            open(editor)
            withContext(Dispatchers.Main){
                val tone=editor.imageOutputs.state.value.snapshot!!.tones.single()
                assertTrue(editor.imageOutputs.selectTone(tone.objectId))
                val form=editor.imageOutputs.state.value.form as ImageOutputForm.Tone
                assertTrue(editor.imageOutputs.edit(form.copy(brightness="-0.5")))
                assertTrue(editor.imageOutputs.apply())
            }
            waitUntil{editor.imageOutputs.state.value.ready && editor.imageOutputs.state.value.snapshot?.tones?.singleOrNull()?.settings?.brightness==-.5 && editor.info?.stateHash!=saved.stateHash}
            withContext(Dispatchers.Main){editor.imageOutput(false);editor.undo()}
            waitUntil{editor.pending==0 && editor.info?.stateHash==saved.stateHash}
            withContext(Dispatchers.Main){editor.undo()}
            waitUntil{editor.pending==0 && editor.info?.stateHash==original}
            assertTrue(withContext(Dispatchers.Main){editor.document!!.render.items.none{it.shape is Shape.Adjustment}})
        }
    }
    private suspend fun open(editor:EditorController) {
        waitUntil{!editor.busy && !editor.semanticEditor.state.value.busy && !editor.instructionEditor.state.value.busy}
        withContext(Dispatchers.Main){editor.imageOutput(true)}
        waitUntil{editor.imageOutputs.state.value.ready}
    }
    private suspend fun fixture(block:suspend(EditorController)->Unit) {
        val app=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val actual=workbenchCore();val bitmap=Bitmap.createBitmap(64,64,Bitmap.Config.ARGB_8888)
        val source=try{bitmap.eraseColor(android.graphics.Color.WHITE);ByteArrayOutputStream().also{assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG,100,it))}.toByteArray()}finally{bitmap.recycle()}
        var owned:File?=null
        val core=object:WorkbenchCore by actual {
            override suspend fun create(options:CreateProject):WorkbenchProject {
                owned=File(options.path).canonicalFile
                return actual.create(options.copy(source=source))
            }
        }
        val store=ViewModelStore()
        val editor=withContext(Dispatchers.Main){EditorController(app,core).also{store.put("image-output",it)}}
        try {
            waitUntil{!editor.busy}
            withContext(Dispatchers.Main){editor.viewport(128,128,1f);editor.newCanvas()}
            waitUntil{!editor.busy && editor.scene?.background!=null}
            block(editor)
        }finally{withContext(NonCancellable){
            withContext(Dispatchers.Main){store.clear()}
            withTimeout(15_000){checkNotNull(editor.scope.coroutineContext[Job]).join()}
            owned?.let{directory->
                assertEquals(File(app.filesDir,"projects").canonicalFile,directory.parentFile)
                assertTrue(directory.name.matches(Regex("[0-9a-fA-F-]{36}")))
                assertTrue(!directory.exists() || directory.deleteRecursively())
            }
        }}
    }
    private suspend fun waitUntil(condition:()->Boolean)=withTimeout(10_000){while(!withContext(Dispatchers.Main){condition()})delay(10)}
}
