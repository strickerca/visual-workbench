@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import com.visualworkbench.shared.*
import com.visualworkbench.shared.Shape
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class AiResultPainterTest {
    @Test fun materializedSamplesReplaceIncludingTransparency() = runBlocking {
        val bitmap = aiBitmap(AiRegion(0u,0u,2u,1u),2u,1u,byteArrayOf(0,0,255.toByte(),128.toByte(),0,0,0,0))
        try {
            val target = ImageBitmap(2,1)
            val item = RenderItem("object","layer",1.0,"normal",Rect(0.0,0.0,2.0,1.0),
                Contours(longArrayOf(),longArrayOf(),uintArrayOf()),Transform(),ObjectStyle(0xffffffffu,0.0),Shape.Result("c".repeat(64),"result"),false)
            CanvasDrawScope().draw(Density(1f),LayoutDirection.Ltr,Canvas(target),Size(2f,1f)) {
                drawRect(Color.Red.copy(alpha=128f/255f))
                drawAiResult(item,AiResultImage("object","result","c".repeat(64),2u,1u,listOf(AiDisplayTile(AiRegion(0u,0u,2u,1u),bitmap))))
            }
            val pixels=target.toPixelMap()
            assertTrue(kotlin.math.abs(pixels[0,0].alpha-128f/255f)<.005f)
            assertTrue(pixels[0,0].red<.005f); assertTrue(pixels[0,0].blue>.995f)
            assertEquals(0f,pixels[1,0].alpha,0f)
        } finally { bitmap.abandon() }
    }
}
