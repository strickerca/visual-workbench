@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** Offscreen pixels only: these tests never create a window or use clipboard. */
class DesktopRenderTest {
    @Test fun constantWidthSurvivesObjectShearNonuniformScaleAndCameraZoom(){
        val objectTransform=Transform(a=3.0,b=.5,c=1.5,d=2.0,e=2.0,f=3.0)
        val line=item(Shape.Line(listOf(Point(4.0,10.0),Point(20.0,10.0))),ObjectStyle(0xffffffffu,6.0,true),objectTransform)
        val geometry=prepareGeometry(line)
        val actual=render{withTransform({transform(Transform(a=1.5,d=1.5).matrix())}){drawGeometry(line,geometry,null,1.5)}}
        // Independently evaluated affine endpoints: (29,25), (77,33) in D,
        // then a 1.5x camera. The physical pen remains exactly six pixels.
        val expected=render{drawPath(Path().apply{moveTo(43.5f,37.5f);lineTo(115.5f,49.5f)},Color.White,style=Stroke(6f,cap=StrokeCap.Round,join=StrokeJoin.Round))}
        assertPixelsClose(expected,actual)
        val ordinary=line.copy(style=line.style.copy(screenConstantWidth=false))
        val scaled=render{withTransform({transform(Transform(a=1.5,d=1.5).matrix())}){drawGeometry(ordinary,prepareGeometry(ordinary),null,1.5)}}
        assertTrue("ordinary object-space width should scale",alphaMass(scaled)>alphaMass(actual)*2)
    }

    @Test fun previewUsesItsOwnTransformedConstantWidthPath(){
        val line=item(Shape.Line(listOf(Point(10.0,10.0),Point(40.0,10.0))),ObjectStyle(0xffffffffu,4.0,true),Transform(a=3.0,d=4.0))
        val actual=render{drawGeometry(line,prepareGeometry(line),Transform(e=9.0,f=17.0),1.0)}
        val expected=render{drawPath(Path().apply{moveTo(19f,27f);lineTo(49f,27f)},Color.White,style=Stroke(4f,cap=StrokeCap.Round,join=StrokeJoin.Round))}
        assertPixelsClose(expected,actual)
    }

    @Test fun markerCircleAndBoxCompositeSeparatelyWithoutZeroWidthHairline(){
        val marker=item(Shape.Marker(1u,Point(32.0,32.0),Rect(24.0,24.0,20.0,20.0)),ObjectStyle(0x00ff00ffu,0.0,fill=0xff000080u))
        val geometry=prepareGeometry(marker)
        assertEquals(listOf(PaintKind.ObjectFill,PaintKind.ObjectFill),geometry.passes.map{it.kind})
        val pixels=render{drawGeometry(marker,geometry,null,1.0)}.toPixelMap()
        assertEquals(192f/255f,pixels[32,32].alpha,1f/255f)
        assertEquals(128f/255f,pixels[22,32].alpha,1f/255f)
        assertEquals(1f,pixels[32,32].red,1f/255f)
        assertEquals(0f,pixels[32,32].green,0f)
    }

    @Test fun overlappingHighlighterContoursPaintAsOneCompoundFill(){
        val vertices=listOf(10 to 10,40 to 10,40 to 40,10 to 40,20 to 20,50 to 20,50 to 50,20 to 50)
        val ink=item(Shape.Stroke("highlighter"),ObjectStyle(0xff000080u,10.0)).copy(contours=Contours(vertices.map{it.first*256L}.toLongArray(),vertices.map{it.second*256L}.toLongArray(),uintArrayOf(4u,8u)))
        val geometry=prepareGeometry(ink);assertEquals(1,geometry.passes.size)
        val pixels=render{drawGeometry(ink,geometry,null,1.0)}.toPixelMap()
        assertEquals(128f/255f,pixels[15,15].alpha,1f/255f)
        assertEquals(pixels[15,15].alpha,pixels[30,30].alpha,0f)
    }
}

private fun item(shape:Shape,style:ObjectStyle,transform:Transform=Transform())=RenderItem("object","layer",1.0,"normal",Rect(0.0,0.0,128.0,96.0),Contours(longArrayOf(),longArrayOf(),uintArrayOf()),transform,style,shape,false)
private fun render(block:DrawScope.()->Unit):ImageBitmap=ImageBitmap(128,96).also{image->CanvasDrawScope().draw(Density(1f),LayoutDirection.Ltr,Canvas(image),Size(128f,96f),block)}
private fun alphaMass(image:ImageBitmap):Double{val pixels=image.toPixelMap();var sum=0.0;for(y in 0 until image.height)for(x in 0 until image.width)sum+=pixels[x,y].alpha;return sum}
private fun assertPixelsClose(expected:ImageBitmap,actual:ImageBitmap){val a=expected.toPixelMap();val b=actual.toPixelMap();for(y in 0 until expected.height)for(x in 0 until expected.width)assertEquals("alpha at $x,$y",a[x,y].alpha,b[x,y].alpha,2f/255f)}
