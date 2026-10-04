@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** Owned in-memory Canvas pixels only; no screenshot, file or owner image. */
class MarkerDrawingInstrumentedTest {
    @Test fun hitIncludesBoxEdgeOutsideCircleWithConstantOutlineAndNoInvisibleHairline() {
        val (scene, obj) = fixture(ObjectStyle(0xff0000ffu, 2.0, true), 1.0)
        val marker = MarkerPaint(circle(), Path().apply { addRect(20f, -5f, 25f, 5f, Path.Direction.CCW) }, Path())
        val boxed = obj.copy(marker = marker)
        val withBox = scene.copy(objects = listOf(boxed))
        // Local (20,0) -> D(104,64) -> view(208,128), well outside circle.
        assertEquals(boxed, EditorDrawing.hit(withBox, Point(208.0, 128.0), 1f))
        assertEquals(boxed, EditorDrawing.hit(withBox, Point(208.0, 128.0), 0f))
        assertEquals(boxed, EditorDrawing.hit(withBox, Point(80.0, 128.0), 1f))
        assertNull(EditorDrawing.hit(withBox, Point(214.0, 128.0), 1f))
        val invisible = boxed.copy(item = boxed.item.copy(style = ObjectStyle(0xff0000ffu, 0.0, true)))
        assertNull(EditorDrawing.hit(scene.copy(objects = listOf(invisible)), Point(208.0, 128.0), 4f))
        // Opposite winding may not cancel the painted overlap in a filled marker.
        val filled = boxed.copy(item = boxed.item.copy(style = ObjectStyle(0xff0000ffu, 0.0, true, 0xff0000ffu)),
            marker = MarkerPaint(circle(), Path().apply { addRect(-6f, -6f, 6f, 6f, Path.Direction.CCW) }, Path()))
        assertEquals(filled, EditorDrawing.hit(scene.copy(objects = listOf(filled)), Point(128.0, 128.0), 0f))
    }

    @Test fun widthZeroProducesNoOutlineInsteadOfAnAndroidHairline() {
        for (constant in listOf(false, true)) {
            val (scene, obj) = fixture(ObjectStyle(0xff0000ffu, 0.0, constant))
            bitmap { pixels ->
                checkNotNull(obj.marker).draw(Canvas(pixels), scene, obj)
                assertTrue(read(pixels).all { it == Color.TRANSPARENT })
            }
        }
    }
    @Test fun constantWidthSurvivesNonuniformAndShearedObjectTransforms() {
        for (shear in listOf(0.0, 1.0)) {
            val (scene, obj) = fixture(ObjectStyle(0xff0000ffu, 2.0, true), shear)
            bitmap { actual -> bitmap { expected ->
                checkNotNull(obj.marker).draw(Canvas(actual), scene, obj)
                // Independent view-space oracle: the complete transformed
                // geometry receives a two-device-pixel outline exactly once.
                val shape = circle().apply { transform(Matrix().apply { setValues(floatArrayOf(4f, (2 * shear).toFloat(), 128f, 0f, 2f, 128f, 0f, 0f, 1f)) }) }
                Canvas(expected).drawPath(shape, Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.RED; style = Paint.Style.STROKE; strokeWidth = 2f; strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND })
                assertArrayEquals(read(expected), read(actual))
            } }
        }
    }
    @Test fun circleAndBoxRetainIndependentAlphaPassesAndLabelTransform() {
        val (scene, obj) = fixture(ObjectStyle(0x00ff00ffu, 0.0, true, 0xff000080u))
        val box = Path().apply { addRect(-6f, -6f, 6f, 6f, Path.Direction.CW) }
        val label = Path().apply { addRect(2f, 2f, 3f, 3f, Path.Direction.CW) }
        val marker = MarkerPaint(circle(), box, label)
        bitmap { pixels ->
            marker.draw(Canvas(pixels), scene, obj)
            assertEquals(192, Color.alpha(pixels.getPixel(128, 128)))
            // Object x scale 2 then camera scale 2 maps local label (2,2)
            // to view (136,132); no camera-only transform may move the label.
            assertEquals(Color.GREEN, pixels.getPixel(138, 133))
        }
    }
    private fun circle() = Path().apply { addCircle(0f, 0f, 12f, Path.Direction.CW) }
    private fun fixture(style: ObjectStyle, shear: Double = 0.0): Pair<CanvasScene, DrawObject> {
        val item = RenderItem("marker", "layer", 1.0, "normal", Rect(30.0, 30.0, 80.0, 80.0),
            Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(a = 2.0, c = shear, e = 64.0, f = 64.0), style,
            Shape.Marker(1u, Point(0.0, 0.0), null), false)
        val obj = DrawObject(item, Point(0.0, 0.0), circle(), false, marker = MarkerPaint(circle(), null, Path()))
        val info = ProjectInfo("fixture", "fixture", "device", 1uL, false, false, 0uL, "fixture", listOf("document"))
        val document = DocumentSnapshot("document", "fixture", 128u, 128u, 8u, listOf(LayerInfo("layer", "Annotations", true, false, 1.0, "normal")), RenderList(info, listOf(item)))
        return CanvasScene(1, document, null, Camera(Point(64.0, 64.0), 2.0, 0.0, 256.0, 256.0), Transform(a = 2.0, d = 2.0), listOf(obj)) to obj
    }
    private fun read(value: Bitmap) = IntArray(value.width * value.height).also { value.getPixels(it, 0, value.width, 0, 0, value.width, value.height) }
    private fun bitmap(block: (Bitmap) -> Unit) { val value = Bitmap.createBitmap(256, 256, Bitmap.Config.ARGB_8888); try { block(value) } finally { value.recycle() } }
}
