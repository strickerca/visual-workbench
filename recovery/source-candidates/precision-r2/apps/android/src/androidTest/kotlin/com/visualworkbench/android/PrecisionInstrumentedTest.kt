@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.MotionEvent
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.visualworkbench.android.editor.CanvasScene
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.android.input.*
import com.visualworkbench.android.precision.*
import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Synthetic pixels and input only; no screen capture, real pen or provider. */
@RunWith(AndroidJUnit4::class)
class PrecisionInstrumentedTest {
    private val camera = Camera(Point(64.0,64.0),1.0,0.0,400.0,400.0)
    private fun fixture(body:(WorkbenchCore,PrecisionController)->Unit) {
        val core=workbenchCore();val p=PrecisionController(core,{})
        try{p.synchronize(camera);body(core,p)}finally{p.close()}
    }
    private fun sample(x:Double=200.0,y:Double=200.0,time:Long=100,buttons:Int=0)=
        PenSample(7,PointerTool.Pen,x,y,time,.37f,.51f,.2f,-.1f,buttons)
    private fun packet(sample:PenSample,canceled:Boolean=false)=PointerPacket(MotionEvent.ACTION_MOVE,7,canceled,0,
        listOf(PointerFrame(sample.timeMs,listOf(sample))),StylusCapabilities(true,true,true,true,false,true))

    @Test fun acceptedAndPredictedRoutingPreservesEveryNonCoordinateField()=fixture{_,p->
        p.toggle();p.down(Point(200.0,200.0));val input=sample(212.0,208.0)
        val mapped=p.routePen(listOf(input)).single()
        assertEquals(203.0,mapped.x,1e-9);assertEquals(202.0,mapped.y,1e-9)
        assertEquals(input,mapped.copy(x=input.x,y=input.y))
        p.toggle();assertTrue(p.state.value.pending)
        assertEquals(mapped,p.routePen(listOf(input)).single())
        p.end();assertNull(p.frame().lens)
    }
    @Test fun inspectionOnAndOffPreservesAllAcceptedSampleBits()=fixture{_,p->
        val samples=(0..199).map{sample(x=it*.375,y=100.0+it*.125,time=it.toLong())}
        p.inspection(false);p.down(Point(0.0,100.0));val off=p.routePen(samples);p.end()
        p.inspection(true);p.down(Point(0.0,100.0));val on=p.routePen(samples);p.end()
        assertEquals(samples,off);assertEquals(off,on)
    }
    @Test fun physicalHoldAcrossHoverToContactDoesNotBecomeAShortClick()=fixture{_,p->
        val buttons=PrecisionButtons(p){true}
        p.hover(Point(200.0,200.0));buttons.accept(packet(sample(time=100,buttons=MotionEvent.BUTTON_STYLUS_PRIMARY)))
        // Production beginTool cancels the old hover/gesture, then latches.
        p.cancel();p.down(Point(200.0,200.0))
        buttons.accept(packet(sample(time=500,buttons=MotionEvent.BUTTON_STYLUS_PRIMARY)))
        buttons.accept(packet(sample(time=550)))
        assertFalse(p.state.value.pending);assertNull(p.frame().lens);p.end()
    }
    @Test fun shortReleaseBeforeUpQueuesExactlyOneToggleAndCancellationNeverClicks()=fixture{_,p->
        val buttons=PrecisionButtons(p){true};p.down(Point(200.0,200.0))
        buttons.accept(packet(sample(time=100,buttons=MotionEvent.BUTTON_STYLUS_PRIMARY)))
        buttons.accept(packet(sample(time=200)));assertTrue(p.state.value.pending);assertNull(p.frame().lens)
        p.end();assertNotNull(p.frame().lens)
        buttons.accept(packet(sample(time=300,buttons=MotionEvent.BUTTON_STYLUS_PRIMARY)))
        buttons.accept(packet(sample(time=350),canceled=true));p.cancel();buttons.reset()
        buttons.accept(packet(sample(time=400)));assertNotNull(p.frame().lens)
    }
    @Test fun rendererClipsTheInsetAndUsesActualImmutableSourcePixels()=fixture{core,p->
        val source=Bitmap.createBitmap(128,128,Bitmap.Config.ARGB_8888)
        val plain=Bitmap.createBitmap(400,400,Bitmap.Config.ARGB_8888)
        val lens=Bitmap.createBitmap(400,400,Bitmap.Config.ARGB_8888)
        try{
            source.eraseColor(0xffe12462.toInt())
            val info=ProjectInfo("project","synthetic","device",1uL,false,false,0uL,"hash",listOf("doc"))
            val document=DocumentSnapshot("doc","synthetic",128u,128u,8u,emptyList(),RenderList(info,emptyList()))
            val scene=CanvasScene(1,document,source,camera,core.cameraMatrix(camera),emptyList())
            EditorDrawing.draw(Canvas(plain),scene)
            p.toggle();p.smooth(true);PrecisionDrawing.draw(Canvas(lens),scene.copy(precision=p.frame()))
            assertEquals(plain.getPixel(2,2),lens.getPixel(2,2))
            assertEquals(source.getPixel(64,64),lens.getPixel(200,200))
            assertEquals(0xffe12462.toInt(),source.getPixel(64,64))
        }finally{source.recycle();plain.recycle();lens.recycle()}
    }
    @Test fun snapCalibrationCapturesLensPhysicalPixelsBeforeQueuedClose()=fixture{_,p->
        p.toggle();p.down(Point(200.0,200.0));val matrix=p.inputMatrix()
        assertEquals(4.0,matrix.a,0.0)
        assertNotNull(semanticDropBox(Point(0.0,0.0),Point(1.0,1.0),matrix))
        p.toggle();p.end();assertNull(p.frame().lens)
        assertEquals(4.0,matrix.a,0.0);assertEquals(1.0,p.inputMatrix().a,0.0)
    }
    private fun hitItem(text: Boolean = false, at: Point = Point(64.0,64.0), size: Double = 4.0): RenderItem {
        val shape = if(text) Shape.Text(at,"fixture","Inter",20.0,listOf(Outline.Move(0f,0f),
            Outline.Line(size.toFloat(),0f),Outline.Line(size.toFloat(),size.toFloat()),Outline.Line(0f,size.toFloat()),Outline.Close))
        else Shape.Rectangle(Rect(at.x,at.y,size,size))
        return RenderItem(if(text)"text" else "rectangle","layer",1.0,"normal",Rect(at.x,at.y,size,size),
            Contours(longArrayOf(),longArrayOf(),uintArrayOf()),Transform(),ObjectStyle(0xffffffffu,0.0,fill=0xffffffffu),shape,false)
    }
    private fun hitScene(core: WorkbenchCore, item: RenderItem): CanvasScene {
        val info=ProjectInfo("project","fixture","device",1uL,false,false,0uL,"hash",listOf("doc"))
        val doc=DocumentSnapshot("doc","fixture",512u,512u,8u,emptyList(),RenderList(info,listOf(item)))
        return CanvasScene(1,doc,null,camera,core.cameraMatrix(camera),listOf(EditorDrawing.objectPath(item)))
    }
    @Test fun actualSelectionAndTextHitSlopStaysEightPhysicalPixelsAtEveryLensMagnification()=fixture{core,p->
        for(magnification in listOf(2,4,8)){
            p.reset();p.magnification(magnification);p.toggle();p.down(Point(200.0,200.0))
            for(text in listOf(false,true)){
                val item=hitItem(text);val scene=precisionHitScene(hitScene(core,item),p.inputMatrix())
                val edge=EditorDrawing.map(scene.matrix,Point(68.0,66.0))
                assertEquals(item.objectId,EditorDrawing.hit(scene,Point(edge.x+7.0,edge.y),8f)?.item?.objectId)
                assertNull(EditorDrawing.hit(scene,Point(edge.x+10.0,edge.y),8f))
                assertEquals(magnification.toDouble(),scene.camera.scale,0.0)
            }
            p.end()
        }
    }
    @Test fun actualResizeCornerAdmissionStaysTwentyFourPhysicalPixelsAtTwoFourAndEightTimes()=fixture{core,p->
        for(magnification in listOf(2,4,8)){
            p.reset();p.magnification(magnification);p.toggle();p.down(Point(200.0,200.0))
            val item=hitItem();val scene=precisionHitScene(hitScene(core,item),p.inputMatrix())
            val corner=EditorDrawing.screenBounds(scene,item)[2]
            assertEquals(2,precisionResizeHandle(scene,item,Point(corner.x+23.0,corner.y),24f))
            assertNull(precisionResizeHandle(scene,item,Point(corner.x+25.0,corner.y),24f))
            p.end()
        }
    }
    @Test fun lensVisibleGeometryOutsideTheBaseViewportRemainsSelectable()=fixture{core,p->
        for(magnification in listOf(2,4,8)){
            p.reset();p.magnification(magnification);p.hover(Point(399.0,200.0));p.toggle()
            val raw=Point(200.0+3*magnification,200.0)
            p.down(raw)
            val item=hitItem(at=Point(265.0,63.0),size=3.0);val base=hitScene(core,item)
            val remapped=p.route(listOf(raw)).single()
            assertTrue(remapped.x>=camera.viewportWidth)
            assertNull(EditorDrawing.hit(base,remapped,0f))
            val physical=precisionHitScene(base,p.inputMatrix())
            assertEquals(item.objectId,EditorDrawing.hit(physical,raw,0f)?.item?.objectId)
            p.end()
        }
    }
}
