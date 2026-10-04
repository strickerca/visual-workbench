@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import com.visualworkbench.android.editor.*
import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

class AiResultDrawingInstrumentedTest {
    private fun item(opacity: Double = 1.0, pose: Transform = Transform()) = RenderItem("object","layer",opacity,"normal",
        Rect(0.0,0.0,2.0,1.0),Contours(longArrayOf(),longArrayOf(),uintArrayOf()),pose,ObjectStyle(0xffffffffu,0.0),Shape.Result("c".repeat(64),"result"),false)
    private fun tiles(bitmap: Bitmap) = AiResultImage("object","result","c".repeat(64),2u,1u,
        listOf(AiDisplayTile(AiRegion(0u,0u,2u,1u),AiBitmap(bitmap))))
    private fun candidate() = Bitmap.createBitmap(intArrayOf(0x800000ff.toInt(),0),2,1,Bitmap.Config.ARGB_8888)
    @Test fun replacementCopiesAlphaAndClearsFullyTransparentSamples() {
        val original = Bitmap.createBitmap(2,1,Bitmap.Config.ARGB_8888); val result = candidate()
        try {
            original.eraseColor(0x80ff0000.toInt())
            drawAiResult(Canvas(original),item(),tiles(result),Transform())
            assertEquals(result.getPixel(0,0),original.getPixel(0,0))
            assertEquals(0,original.getPixel(1,0))
        } finally { original.recycle(); result.recycle() }
    }
    @Test fun halfOpacityInterpolatesPremultipliedAlphaRatherThanSourceOver() {
        val original = Bitmap.createBitmap(2,1,Bitmap.Config.ARGB_8888); val result = candidate()
        try {
            original.eraseColor(0x80ff0000.toInt())
            drawAiResult(Canvas(original),item(.5),tiles(result),Transform())
            val mixed = original.getPixel(0,0)
            assertTrue(kotlin.math.abs(Color.alpha(mixed)-128)<=1)
            assertTrue(kotlin.math.abs(Color.red(mixed)-127)<=2)
            assertTrue(kotlin.math.abs(Color.blue(mixed)-128)<=2)
            assertTrue(kotlin.math.abs(Color.alpha(original.getPixel(1,0))-64)<=1)
        } finally { original.recycle(); result.recycle() }
    }
    @Test fun reflectedAffineOnlyReplacesItsCoveredRectangle() {
        val original = Bitmap.createBitmap(4,1,Bitmap.Config.ARGB_8888); val result = candidate()
        try {
            original.eraseColor(Color.GREEN)
            drawAiResult(Canvas(original),item(pose=Transform(a=-1.0,e=3.0)),tiles(result),Transform())
            assertEquals(Color.GREEN,original.getPixel(0,0)); assertEquals(Color.GREEN,original.getPixel(3,0))
            assertEquals(0,original.getPixel(1,0)); assertEquals(result.getPixel(0,0),original.getPixel(2,0))
        } finally { original.recycle(); result.recycle() }
    }
    @Test fun actualEditorStackKeepsWhitePageOutsideTransparentResultReplacement() {
        val output = Bitmap.createBitmap(2,1,Bitmap.Config.ARGB_8888)
        val original = Bitmap.createBitmap(2,1,Bitmap.Config.ARGB_8888); val result = candidate()
        try {
            original.eraseColor(0x80ff0000.toInt())
            val value = item(); val info = ProjectInfo("project","fixture","device",2u,false,false,1u,"a".repeat(64),listOf("document"))
            val document = DocumentSnapshot("document","fixture",2u,1u,8u,listOf(LayerInfo("layer","Result",true,false,1.0,"normal")),RenderList(info,listOf(value)))
            val scene = CanvasScene(1,document,original,Camera(Point(1.0,.5),1.0,0.0,2.0,1.0),Transform(),
                listOf(EditorDrawing.objectPath(value)),aiResults=mapOf("object" to tiles(result)),resultDisplayReady=true)
            EditorDrawing.draw(Canvas(output),scene)
            assertEquals(Color.WHITE,output.getPixel(1,0))
            val pixel = output.getPixel(0,0)
            assertEquals(255,Color.alpha(pixel)); assertEquals(255,Color.blue(pixel))
            assertTrue(kotlin.math.abs(Color.red(pixel)-127)<=1)
            assertTrue(kotlin.math.abs(Color.green(pixel)-127)<=1)
        } finally { output.recycle(); original.recycle(); result.recycle() }
    }
}
