@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import com.visualworkbench.desktop.precision.drawPrecision
import com.visualworkbench.desktop.precision.precisionResizeHit
import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** Actual offscreen Compose rendering; no window, capture, clipboard or disk. */
class DesktopPrecisionTest {
    @Test fun editableInsetUsesTheSameCanonicalCompoundGeometryOncePerPane(){
        val vertices=listOf(0 to 0,12 to 0,12 to 12,0 to 12,4 to 4,16 to 4,16 to 16,4 to 16)
        val item=RenderItem("ink","layer",1.0,"normal",Rect(0.0,0.0,16.0,16.0),
            Contours(vertices.map{it.first*256L}.toLongArray(),vertices.map{it.second*256L}.toLongArray(),uintArrayOf(4u,8u)),
            Transform(),ObjectStyle(0xff000080u,8.0),Shape.Stroke("highlighter"),false)
        val geometry=prepareGeometry(item)
        val pane=MagnifierPane(Rect(20.0,20.0,80.0,80.0),Transform(a=4.0,d=4.0,e=24.0,f=24.0),4.0,4,true,true)
        val image=ImageBitmap(160,140)
        var calls=0
        CanvasDrawScope().draw(Density(1f),LayoutDirection.Ltr,Canvas(image),Size(160f,140f)){
            drawPrecision(PrecisionFrame(lens=pane),Camera(Point(0.0,0.0),1.0,0.0,160.0,140.0)){matrix,scale,_->
                calls++;withTransform({transform(matrix.matrix())}){drawGeometry(item,geometry,null,scale)}
            }
        }
        val pixels=image.toPixelMap()
        assertEquals(1,calls);assertEquals(0f,pixels[5,5].alpha,0f)
        assertEquals(128f/255f,pixels[32,32].alpha,1f/255f)
        assertEquals(pixels[32,32].alpha,pixels[48,48].alpha,0f)
    }
    @Test fun inspectionNeverInvokesInputOrChangesTheBaseCameraAndKeepsDrawingClipped(){
        val camera=Camera(Point(11.0,23.0),1.25,.7,180.0,140.0)
        val before=listOf(camera.center.x.toBits(),camera.center.y.toBits(),camera.scale.toBits(),camera.rotation.toBits())
        val pane=MagnifierPane(Rect(30.0,20.0,80.0,80.0),Transform(a=4.0,d=4.0),4.0,4,true,false)
        val image=ImageBitmap(180,140)
        CanvasDrawScope().draw(Density(1f),LayoutDirection.Ltr,Canvas(image),Size(180f,140f)){
            drawPrecision(PrecisionFrame(loupe=pane),camera){_,_,_->drawRect(Color.Red)}
        }
        val pixels=image.toPixelMap()
        assertEquals(0f,pixels[2,2].alpha,0f);assertEquals(1f,pixels[40,30].red,0f)
        assertEquals(before,listOf(camera.center.x.toBits(),camera.center.y.toBits(),camera.scale.toBits(),camera.rotation.toBits()))
    }
    @Test fun actualResizeAdmissionUsesTenPhysicalPixelsAtTwoFourAndEightTimes(){
        val bounds=Rect(10.0,20.0,30.0,40.0)
        for(m in listOf(2.0,4.0,8.0)){
            val physical=Transform(a=m,d=m,e=120.0-40*m,f=100.0-60*m)
            assertTrue(precisionResizeHit(bounds,physical,Point(129.0,109.0),10.0))
            assertFalse(precisionResizeHit(bounds,physical,Point(131.0,100.0),10.0))
            assertFalse(precisionResizeHit(bounds,physical,Point(120.0,111.0),10.0))
        }
    }
    @Test fun rotatedLensUsesRawPhysicalCornerInsteadOfBaseDocumentTolerance(){
        val bounds=Rect(10.0,20.0,30.0,40.0)
        for(m in listOf(2.0,4.0,8.0)){
            val physical=Transform(a=0.0,b=m,c=-m,d=0.0,e=120.0+60*m,f=100.0-40*m)
            assertTrue(precisionResizeHit(bounds,physical,Point(120.0,109.0),10.0))
            assertFalse(precisionResizeHit(bounds,physical,Point(120.0,111.0),10.0))
        }
    }
    @Test fun invalidPhysicalResizeGeometryRefusesWithoutChangingItsDocumentBounds(){
        val bounds=Rect(10.0,20.0,30.0,40.0)
        assertFalse(precisionResizeHit(bounds,Transform(e=Double.NaN),Point(40.0,60.0),10.0))
        assertFalse(precisionResizeHit(bounds,Transform(),Point(Double.POSITIVE_INFINITY,60.0),10.0))
        assertFalse(precisionResizeHit(bounds,Transform(),Point(40.0,60.0),-1.0))
        assertEquals(Rect(10.0,20.0,30.0,40.0),bounds)
    }
}
