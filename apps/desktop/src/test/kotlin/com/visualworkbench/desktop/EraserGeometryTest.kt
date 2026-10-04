@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import com.visualworkbench.shared.Shape
import org.junit.Assert.*
import org.junit.Test

class EraserGeometryTest {
    @Test fun outlinedEllipseDoesNotEraseItsEmptyBoundingBoxInterior() {
        val area = desktopPaintedArea(item(Shape.Ellipse(Rect(0.0, 0.0, 40.0, 20.0))), 1.0)
        assertFalse(area.contains(20.0, 10.0)); assertTrue(area.contains(20.0, .25))
    }
    @Test fun fillOnlyCompoundHoleAndDisconnectedPieceRemainEmptyBetween() {
        val points = listOf(Point(0.0, 0.0), Point(20.0, 0.0), Point(20.0, 20.0), Point(0.0, 20.0), Point(0.0, 0.0),
            Point(5.0, 5.0), Point(5.0, 15.0), Point(15.0, 15.0), Point(15.0, 5.0), Point(5.0, 5.0), Point(0.0, 0.0),
            Point(30.0, 0.0), Point(40.0, 0.0), Point(40.0, 10.0), Point(30.0, 10.0), Point(30.0, 0.0), Point(0.0, 0.0))
        val area = desktopPaintedArea(item(Shape.Polygon(points, true), ObjectStyle(0xff000080u, 0.0, fill = 0xff000080u)), 1.0)
        assertTrue(area.contains(2.0, 2.0)); assertFalse(area.contains(10.0, 10.0))
        assertFalse(area.contains(25.0, 5.0)); assertTrue(area.contains(35.0, 5.0))
        assertFalse(area.contains(-.1, 2.0)) // width zero must not become a hairline
    }
    @Test fun constantScreenWidthIsStrokedAfterShearWhileDocumentWidthIsTransformed() {
        val transform = Transform(a = 4.0, d = 8.0, c = 2.0, e = 10.0, f = 20.0)
        val line = item(Shape.Line(listOf(Point(0.0, 0.0), Point(10.0, 0.0))), ObjectStyle(0xff0000ffu, 4.0, true)).copy(transform = transform)
        val fixed = desktopPaintedArea(line, 2.0)
        assertTrue(fixed.contains(25.0, 20.9)); assertFalse(fixed.contains(25.0, 21.1))
        val local = desktopPaintedArea(line.copy(style = line.style.copy(screenConstantWidth = false)), 2.0)
        assertTrue(local.contains(25.0, 24.0))
    }
    @Test fun transparentFillAndZeroWidthHaveNoPaintedTarget() {
        val shape = Shape.Rectangle(Rect(0.0, 0.0, 20.0, 20.0))
        assertTrue(desktopPaintedArea(item(shape, ObjectStyle(0xff000000u, 0.0, fill = 0xff000000u)), 1.0).isEmpty)
    }
    private fun item(shape: Shape, style: ObjectStyle = ObjectStyle(0x000000ffu, 2.0)) = RenderItem("object", "layer", 1.0, "normal",
        Rect(0.0, 0.0, 40.0, 20.0), Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(), style, shape, false)
}
