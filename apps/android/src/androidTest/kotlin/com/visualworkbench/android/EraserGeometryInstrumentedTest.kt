@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Path
import android.graphics.RectF
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.visualworkbench.android.editor.*
import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class EraserGeometryInstrumentedTest {
    @Test fun emptyEllipseCenterDoesNotCountAsPaintedHit() {
        val path = androidPaintedPath(item(Shape.Ellipse(Rect(0.0, 0.0, 40.0, 20.0))), 1.0)
        assertFalse(intersects(path, 19f, 9f, 21f, 11f)); assertTrue(intersects(path, 19f, 0f, 21f, 1f))
    }
    @Test fun compoundHoleAndBridgeAreNotPaintedByFillOnlyOutline() {
        val shape = Shape.Polygon(listOf(Point(0.0, 0.0), Point(20.0, 0.0), Point(20.0, 20.0), Point(0.0, 20.0), Point(0.0, 0.0),
            Point(5.0, 5.0), Point(5.0, 15.0), Point(15.0, 15.0), Point(15.0, 5.0), Point(5.0, 5.0), Point(0.0, 0.0)), true)
        val path = androidPaintedPath(item(shape, ObjectStyle(0xff000080u, 0.0, fill = 0xff000080u)), 1.0)
        assertTrue(intersects(path, 1f, 1f, 2f, 2f)); assertFalse(intersects(path, 8f, 8f, 12f, 12f))
        assertFalse(intersects(path, -.2f, 2f, -.1f, 3f))
    }
    @Test fun screenConstantWidthIsNotMultipliedByObjectScale() {
        val source = item(Shape.Line(listOf(Point(0.0, 0.0), Point(10.0, 0.0))), ObjectStyle(0x000000ffu, 4.0, true))
            .copy(transform = Transform(a = 4.0, d = 8.0, c = 2.0, e = 10.0, f = 20.0))
        val path = androidPaintedPath(source, 2.0)
        assertTrue(intersects(path, 25f, 20.5f, 26f, 20.9f)); assertFalse(intersects(path, 25f, 21.1f, 26f, 22f))
    }
    @Test fun independentFillAndOutlineRemainPaintedForEitherWindingAndReflection() {
        val corners = listOf(Point(8.0, 8.0), Point(32.0, 8.0), Point(32.0, 32.0), Point(8.0, 32.0))
        for (points in listOf(corners, corners.reversed())) for (transform in listOf(Transform(), Transform(a = -1.0, e = 48.0))) {
            val source = item(Shape.Polygon(points, true), ObjectStyle(0x000000ffu, 6.0, true, 0xff0000ffu))
                .copy(transform = transform)
            // This point is inside both the red fill and the separately drawn
            // black outline. Opposite winding cannot turn that overlap empty.
            val probe = EditorDrawing.map(transform, Point(8.5, 16.5))
            val path = androidPaintedPath(source, 1.0)
            assertTrue(intersects(path, (probe.x - .1).toFloat(), (probe.y - .1).toFloat(),
                (probe.x + .1).toFloat(), (probe.y + .1).toFloat()))
            val bitmap = rendered(source)
            try { assertEquals(Color.BLACK, bitmap.getPixel(probe.x.toInt(), probe.y.toInt())) }
            finally { bitmap.recycle() }
        }
    }
    @Test fun arrowHeadAndOutlineOverlapRemainPaintedAfterReflection() {
        for (transform in listOf(Transform(), Transform(a = -1.0, e = 48.0))) {
            val source = item(Shape.Arrow(listOf(Point(8.0, 16.0), Point(40.0, 16.0))),
                ObjectStyle(0x000000ffu, 4.0, true)).copy(transform = transform)
            val probe = EditorDrawing.map(transform, Point(34.5, 16.5))
            val path = androidPaintedPath(source, 1.0)
            assertTrue(intersects(path, (probe.x - .1).toFloat(), (probe.y - .1).toFloat(),
                (probe.x + .1).toFloat(), (probe.y + .1).toFloat()))
            val bitmap = rendered(source)
            try { assertEquals(Color.BLACK, bitmap.getPixel(probe.x.toInt(), probe.y.toInt())) }
            finally { bitmap.recycle() }
        }
    }
    @Test fun actualConstantWidthPixelsAndHitAgreeAfterShearAndReflection() {
        for (transform in listOf(Transform(a = 4.0, d = 8.0, c = 2.0, e = 10.0, f = 20.0),
            Transform(a = -4.0, d = 8.0, c = 2.0, e = 50.0, f = 20.0))) {
            val source = item(Shape.Line(listOf(Point(0.0, 0.0), Point(10.0, 0.0))),
                ObjectStyle(0x000000ffu, 4.0, true)).copy(transform = transform)
            val path = androidPaintedPath(source, 2.0)
            val bitmap = rendered(source, scale = 2.0)
            try {
                assertEquals(Color.BLACK, bitmap.getPixel(50, 40))
                assertTrue(intersects(path, 25.1f, 20.1f, 25.4f, 20.4f))
                // The old painter multiplied the outline by d=8 even though
                // hit geometry correctly kept a four-screen-pixel width.
                assertEquals(Color.WHITE, bitmap.getPixel(50, 46))
                assertFalse(intersects(path, 25.1f, 23.1f, 25.4f, 23.4f))
            } finally { bitmap.recycle() }
        }
    }
    @Test fun canonicalGuideAndOpenLineIgnoreStoredFillColor() {
        for (shape in listOf(Shape.Guide(Rect(8.0, 8.0, 32.0, 32.0)),
            Shape.Line(listOf(Point(8.0, 8.0), Point(40.0, 8.0), Point(40.0, 40.0))),
            Shape.Arrow(listOf(Point(8.0, 8.0), Point(40.0, 8.0), Point(40.0, 40.0))))) {
            val source = item(shape, ObjectStyle(0x000000ffu, 2.0, fill = 0xff0000ffu))
            val path = androidPaintedPath(source, 1.0)
            assertFalse(intersects(path, 29f, 18f, 31f, 20f))
            val bitmap = rendered(source)
            try { assertEquals(Color.WHITE, bitmap.getPixel(30, 19)) }
            finally { bitmap.recycle() }
        }
    }
    @Test fun highlighterFillOnlyReplacementHasNoSecondHairlineAlphaPass() {
        val source = item(Shape.Polygon(listOf(Point(8.0, 8.0), Point(24.0, 8.0), Point(24.0, 24.0), Point(8.0, 24.0)), true),
            ObjectStyle(0xff000080u, 0.0, fill = 0xff000080u)).copy(layerBlend = "multiply")
        val info = ProjectInfo("fixture", "fixture", "device", 1u, false, false, 0u, "hash", listOf("document"))
        val document = DocumentSnapshot("document", "fixture", 32u, 32u, 8u,
            listOf(LayerInfo("layer", "layer", true, false, 1.0, "multiply")), RenderList(info, listOf(source)))
        val scene = CanvasScene(1, document, null, Camera(Point(16.0, 16.0), 1.0, 0.0, 32.0, 32.0),
            Transform(), listOf(EditorDrawing.objectPath(source)))
        val bitmap = Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888)
        try {
            EditorDrawing.draw(Canvas(bitmap), scene)
            val center = bitmap.getPixel(16, 16); val edge = bitmap.getPixel(8, 16)
            assertEquals(255, android.graphics.Color.red(center))
            assertTrue(android.graphics.Color.green(center) in 126..128)
            assertEquals(android.graphics.Color.green(center), android.graphics.Color.green(edge))
        } finally { bitmap.recycle() }
    }
    private fun intersects(path: Path, left: Float, top: Float, right: Float, bottom: Float): Boolean {
        val probe = Path().apply { addRect(RectF(left, top, right, bottom), Path.Direction.CW) }
        val output = Path(); check(output.op(path, probe, Path.Op.INTERSECT)); return !output.isEmpty
    }
    private fun rendered(source: RenderItem, scale: Double = 1.0): Bitmap {
        val info = ProjectInfo("fixture", "fixture", "device", 1u, false, false, 0u, "hash", listOf("document"))
        val document = DocumentSnapshot("document", "fixture", 64u, 64u, 8u,
            listOf(LayerInfo("layer", "layer", true, false, 1.0, "normal")), RenderList(info, listOf(source)))
        val dimension = (64 * scale).toInt()
        val scene = CanvasScene(1, document, null, Camera(Point(32.0, 32.0), scale, 0.0, dimension.toDouble(), dimension.toDouble()),
            Transform(a = scale, d = scale), listOf(EditorDrawing.objectPath(source)))
        val bitmap = Bitmap.createBitmap(dimension, dimension, Bitmap.Config.ARGB_8888)
        try { EditorDrawing.draw(Canvas(bitmap), scene); return bitmap }
        catch (error: Throwable) { bitmap.recycle(); throw error }
    }
    private fun item(shape: Shape, style: ObjectStyle = ObjectStyle(0x000000ffu, 2.0)) = RenderItem("object", "layer", 1.0, "normal",
        Rect(0.0, 0.0, 40.0, 20.0), Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(), style, shape, false)
}
