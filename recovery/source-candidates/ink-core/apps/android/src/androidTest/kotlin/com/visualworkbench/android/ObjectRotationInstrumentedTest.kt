package com.visualworkbench.android

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.MotionEvent
import androidx.lifecycle.ViewModelStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.*
import com.visualworkbench.android.input.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.math.cos
import kotlin.math.sin

/** Actual pen router -> editor -> native CAS/undo. Input and images are synthetic;
 * no real stylus, screenshot, clipboard, network or provider is used. */
@RunWith(AndroidJUnit4::class)
class ObjectRotationInstrumentedTest {
    @Test fun drawnRoundHandleRoutesThreeRealSamplesToOneSnappedUndoableEdit() = runBlocking {
        fixture { f ->
            val before=f.snapshot();val item=before.document.render.items.single()
            val knob=checkNotNull(androidRotationHandle(before,item))
            val bitmap=Bitmap.createBitmap(400,400,Bitmap.Config.ARGB_8888)
            try {
                withContext(Dispatchers.Main){EditorDrawing.draw(Canvas(bitmap),before)}
                assertTrue(knob.hit(knob.knob,24.0))
                assertEquals(0xff087dba.toInt(),bitmap.getPixel(knob.knob.x.toInt(),knob.knob.y.toInt()))
            } finally { bitmap.recycle() }
            val end=f.turned(knob,13.0,before.camera)
            withContext(Dispatchers.Main){
                f.editor.rotationSnapping(true)
                f.down(knob.knob);f.move(end);f.up(end);f.up(end)
            }
            f.committed(before.document.render.revision.hostSeq+1uL)
            val transformed=f.snapshot().document.render.items.single().transform
            val radians=Math.PI/12.0
            assertEquals(cos(radians),transformed.a,1e-12)
            assertEquals(sin(radians),transformed.b,1e-12)
            assertEquals(-sin(radians),transformed.c,1e-12)
            assertEquals(cos(radians),transformed.d,1e-12)
            withContext(Dispatchers.Main){f.editor.undo()}
            waitUntil{f.editor.pending==0 && f.editor.info?.stateHash==before.document.render.revision.stateHash}
            assertEquals(item.transform,f.snapshot().document.render.items.single().transform)
        }
    }

    @Test fun cancelledContactAndViewportReplacementCannotPublishTheirPrivateRotation() = runBlocking {
        fixture { f ->
            val before=f.snapshot();val knob=checkNotNull(androidRotationHandle(before,before.document.render.items.single()))
            val end=f.turned(knob,30.0,before.camera)
            withContext(Dispatchers.Main){f.down(knob.knob);f.move(end);f.up(end,cancelled=true)}
            assertUnchanged(f,before)
            withContext(Dispatchers.Main){f.down(knob.knob);f.move(end);f.editor.viewport(420,400,1f);f.up(end)}
            assertUnchanged(f,before)
            assertTrue(f.snapshot().transformed.isEmpty())
        }
    }

    @Test fun finalUpSampleAndCurrentContextSnapChoiceWinAfterAnEarlierPreview() = runBlocking {
        fixture { f ->
            val before=f.snapshot();val knob=checkNotNull(androidRotationHandle(before,before.document.render.items.single()))
            val preview=f.turned(knob,13.0,before.camera);val final=f.turned(knob,26.0,before.camera)
            withContext(Dispatchers.Main){
                f.editor.rotationSnapping(true);f.down(knob.knob);f.move(preview)
                assertEquals(before.document.render.revision.hostSeq,f.editor.info?.hostSeq)
                f.editor.rotationSnapping(false);f.up(final)
            }
            f.committed(before.document.render.revision.hostSeq+1uL)
            val value=f.snapshot().document.render.items.single().transform
            assertEquals(cos(26.0*Math.PI/180.0),value.a,1e-12)
            assertEquals(sin(26.0*Math.PI/180.0),value.b,1e-12)
        }
    }

    private suspend fun assertUnchanged(f:Fixture,before:CanvasScene) {
        withContext(Dispatchers.Main){
            assertEquals(0,f.editor.pending)
            assertEquals(before.document.render.revision.stateHash,f.editor.info?.stateHash)
            assertEquals(before.document.render.items.single().transform,f.editor.document?.render?.items?.single()?.transform)
        }
    }
    private class Fixture(val editor:EditorController,val core:WorkbenchCore) {
        private val router=CanvasGestureRouter(editor,4.0)
        private var time=100L
        private var down=100L
        private val capabilities=StylusCapabilities(true,false,false,false,false,false)
        fun down(point:Point){down=time+1;packet(MotionEvent.ACTION_DOWN,point)}
        fun move(point:Point)=packet(MotionEvent.ACTION_MOVE,point)
        fun up(point:Point,cancelled:Boolean=false)=packet(MotionEvent.ACTION_UP,point,cancelled)
        private fun packet(action:Int,p:Point,cancelled:Boolean=false) {
            val sample=PenSample(1,PointerTool.Pen,p.x,p.y,++time,1f,1f,null,null,0)
            router.accept(PointerPacket(action,1,cancelled,down,listOf(PointerFrame(time,listOf(sample))),capabilities))
        }
        suspend fun snapshot():CanvasScene=withContext(Dispatchers.Main){checkNotNull(editor.scene)}
        fun turned(handle:ObjectRotationHandle,degrees:Double,camera:Camera):Point {
            val points=core.mapPoints(camera,true,listOf(handle.center,handle.knob))
            val center=points[0];val vector=Point(points[1].x-center.x,points[1].y-center.y)
            val angle=degrees*Math.PI/180.0
            return core.mapPoints(camera,false,listOf(Point(center.x+cos(angle)*vector.x-sin(angle)*vector.y,
                center.y+sin(angle)*vector.x+cos(angle)*vector.y))).single()
        }
        suspend fun committed(sequence:ULong)=waitUntil{editor.pending==0 && editor.document?.render?.revision?.hostSeq==sequence}
        suspend fun seed() {
            val camera=snapshot().camera
            val points=core.mapPoints(camera,false,listOf(Point(18.0,20.0),Point(42.0,44.0),Point(18.0,32.0)))
            withContext(Dispatchers.Main){editor.chooseTool(EditorTool.Rectangle);down(points[0]);up(points[1])}
            waitUntil{editor.pending==0 && editor.document?.render?.items?.size==1 && !editor.semanticEditor.state.value.busy}
            withContext(Dispatchers.Main){editor.chooseTool(EditorTool.Select);down(points[2]);up(points[2])}
            waitUntil{editor.selected!=null && editor.pending==0 && editor.scene?.rotationHandles==true}
        }
    }
    private suspend fun fixture(block:suspend(Fixture)->Unit) {
        val app=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val actual=workbenchCore();val bitmap=Bitmap.createBitmap(64,64,Bitmap.Config.ARGB_8888)
        val png=try{bitmap.eraseColor(android.graphics.Color.WHITE);ByteArrayOutputStream().also{assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG,100,it))}.toByteArray()}finally{bitmap.recycle()}
        var owned:File?=null
        var ownedProject:WorkbenchProject?=null
        val core=object:WorkbenchCore by actual {
            override suspend fun create(options:CreateProject):WorkbenchProject {
                owned=File(options.path).canonicalFile
                return actual.create(options.copy(source=png)).also{ownedProject=it}
            }
        }
        val store=ViewModelStore();val editor=withContext(Dispatchers.Main){EditorController(app,core).also{store.put("object-rotation",it)}}
        var preferences:EditorPreferences?=null
        try {
            waitUntil{!editor.busy}
            withContext(Dispatchers.Main){
                preferences=editor.preferences
                editor.settings(editor.preferences.copy(brushes=editor.preferences.brushes+(EditorTool.Rectangle to BrushPreference(width=2.0))))
                editor.viewport(400,400,1f);editor.newCanvas()
            }
            waitUntil{!editor.busy && editor.scene?.background!=null && !editor.semanticEditor.state.value.busy}
            val f=Fixture(editor,core);f.seed();block(f)
        } finally { withContext(NonCancellable) {
            withContext(Dispatchers.Main){preferences?.let(editor::settings);store.clear()}
            withTimeout(15_000){checkNotNull(editor.scope.coroutineContext[Job]).join()}
            // ViewModel reports close failures to its UI; this idempotent await
            // also observes native settlement before deleting owned fixtures.
            withTimeout(15_000){ownedProject?.close()}
            owned?.let{directory->
                assertEquals(File(app.filesDir,"projects").canonicalFile,directory.parentFile)
                assertTrue(directory.name.matches(Regex("[0-9a-fA-F-]{36}")))
                assertTrue(!directory.exists()||directory.deleteRecursively())
            }
        } }
    }
    private companion object {
        suspend fun waitUntil(condition:()->Boolean)=withTimeout(10_000){while(!withContext(Dispatchers.Main){condition()})delay(10)}
    }
}
