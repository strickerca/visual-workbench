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

/** Real app controller -> shared ownership -> native canonical store.
 * Synthetic UI method calls; no real input/clipboard/network is used. */
@RunWith(AndroidJUnit4::class)
class DocumentControlsInstrumentedTest {
    @Test fun newDocumentSwitchKeepsEachLocalCameraAndTheProjectRemainsOpen() = runBlocking {
        fixture { editor ->
            val old=withContext(Dispatchers.Main){checkNotNull(editor.document).documentId}
            withContext(Dispatchers.Main){editor.pan(12.0,7.0);editor.documentControls(true)}
            ready(editor)
            val camera=withContext(Dispatchers.Main){checkNotNull(editor.scene).camera}
            val next=withContext(Dispatchers.Main){editor.documentControlId()}
            withContext(Dispatchers.Main){assertTrue(editor.documentControls.execute(DocumentControlCommand.CreateDocumentFromSource(next,editor.documentControlId(),"Second")))}
            waitUntil{editor.documentControls.state.value.ready && editor.documentControls.state.value.snapshot?.documents?.size==2}
            withContext(Dispatchers.Main){assertNotNull(editor.selectDocument(next))}
            waitUntil{!editor.busy && editor.document?.documentId==next && editor.scene?.background!=null}
            val second=withContext(Dispatchers.Main){editor.pan(-6.0,9.0);checkNotNull(editor.scene).camera}
            withContext(Dispatchers.Main){assertNotNull(editor.selectDocument(old))}
            waitUntil{!editor.busy && editor.document?.documentId==old && editor.scene?.background!=null}
            withContext(Dispatchers.Main){assertEquals(camera,editor.scene?.camera);assertNotNull(editor.selectDocument(next))}
            waitUntil{!editor.busy && editor.document?.documentId==next && editor.scene?.background!=null}
            withContext(Dispatchers.Main){assertEquals(second,editor.scene?.camera);assertEquals(2,editor.info?.documentIds?.size)}
        }
    }

    @Test fun layerVisibilityAndLockUseCanonicalUndoWithoutChangingOriginalDimensions() = runBlocking {
        fixture { editor ->
            withContext(Dispatchers.Main){editor.documentControls(true)};ready(editor)
            val layer=withContext(Dispatchers.Main){editor.documentControls.state.value.snapshot!!.layers.single().layerId}
            val before=withContext(Dispatchers.Main){editor.info!!.stateHash}
            withContext(Dispatchers.Main){assertTrue(editor.documentControls.execute(DocumentControlCommand.UpdateLayer(LayerControlUpdate(layer,locked=true))))}
            waitUntil{editor.documentControls.state.value.ready && editor.documentControls.state.value.snapshot!!.layers.single().locked}
            withContext(Dispatchers.Main){
                assertTrue(editor.documentControls.execute(DocumentControlCommand.UpdateLayer(LayerControlUpdate(layer,visible=false))))
            }
            waitUntil{!editor.documentControls.state.value.busy}
            withContext(Dispatchers.Main){
                assertTrue(editor.documentControls.state.value.snapshot!!.layers.single().visible)
                assertTrue(editor.documentControls.state.value.issue.orEmpty().contains("Unlock"))
                editor.documentControls(false);editor.undo()
            }
            waitUntil{editor.pending==0 && editor.info?.stateHash==before}
            withContext(Dispatchers.Main){assertEquals(64u,editor.document?.width);assertEquals(64u,editor.document?.height)}
        }
    }

    @Test fun semanticCatalogReactivatesAfterSwitchAndAfterAnEarlyDocumentLoadRefusal() = runBlocking {
        fixture { editor ->
            val old=withContext(Dispatchers.Main){checkNotNull(editor.document).documentId}
            waitUntil{editor.semanticEditor.state.value.catalog?.binding?.documentId==old && !editor.semanticEditor.state.value.busy}
            withContext(Dispatchers.Main){editor.documentControls(true)};ready(editor)
            val next=withContext(Dispatchers.Main){editor.documentControlId()}
            withContext(Dispatchers.Main){assertTrue(editor.documentControls.execute(DocumentControlCommand.CreateDocumentFromSource(next,editor.documentControlId(),"Semantic switch")))}
            waitUntil{editor.documentControls.state.value.ready && editor.documentControls.state.value.snapshot?.documents?.size==2}
            withContext(Dispatchers.Main){assertNotNull(editor.selectDocument(next))}
            waitUntil{!editor.busy && editor.semanticEditor.state.value.catalog?.binding?.documentId==next && !editor.semanticEditor.state.value.busy}
            val prior=withContext(Dispatchers.Main){checkNotNull(editor.semanticEditor.state.value.catalog)}
            assertFalse(prior.isCapture)
            val refused=withContext(Dispatchers.Main){checkNotNull(editor.selectDocument(editor.documentControlId()))}
            withTimeout(10_000){refused.join()}
            waitUntil{!editor.busy && !editor.semanticEditor.state.value.busy &&
                editor.semanticEditor.state.value.catalog?.let{it !== prior && it.binding.documentId==next}==true}
            withContext(Dispatchers.Main){
                assertEquals(next,editor.document?.documentId)
                assertNotNull(editor.selectDocument(old))
            }
            waitUntil{!editor.busy && editor.semanticEditor.state.value.catalog?.binding?.documentId==old && !editor.semanticEditor.state.value.busy}
        }
    }

    private suspend fun ready(editor:EditorController)=waitUntil{editor.documentControls.state.value.ready}
    private suspend fun fixture(block:suspend(EditorController)->Unit) {
        val app=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val actual=workbenchCore();val bitmap=Bitmap.createBitmap(64,64,Bitmap.Config.ARGB_8888)
        val source=try { bitmap.eraseColor(android.graphics.Color.WHITE);ByteArrayOutputStream().also{assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG,100,it))}.toByteArray() } finally { bitmap.recycle() }
        var owned:File?=null
        val core=object:WorkbenchCore by actual {
            override suspend fun create(options:CreateProject):WorkbenchProject {
                owned=File(options.path).canonicalFile
                return actual.create(options.copy(source=source))
            }
        }
        val store=ViewModelStore()
        val editor=withContext(Dispatchers.Main){EditorController(app,core).also{store.put("document-controls",it)}}
        try {
            waitUntil{!editor.busy}
            withContext(Dispatchers.Main){editor.viewport(128,128,1f);editor.newCanvas()}
            waitUntil{!editor.busy && editor.scene?.background!=null}
            block(editor)
        } finally { withContext(NonCancellable) {
            withContext(Dispatchers.Main){store.clear()}
            withTimeout(15_000){checkNotNull(editor.scope.coroutineContext[Job]).join()}
            owned?.let { directory ->
                assertEquals(File(app.filesDir,"projects").canonicalFile,directory.parentFile)
                assertTrue(directory.name.matches(Regex("[0-9a-fA-F-]{36}")))
                assertTrue(!directory.exists() || directory.deleteRecursively())
            }
        } }
    }
    private suspend fun waitUntil(condition:()->Boolean)=withTimeout(10_000){while(!withContext(Dispatchers.Main){condition()})delay(10)}
}
