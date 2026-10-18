package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class PrecisionPlacementTest {
    private val camera = Camera(Point(0.0, 0.0), 1.0, 0.0, 1000.0, 800.0)

    @Test fun subpixelViewportHidesAnUnsafePaneWithoutInvalidClamping() {
        val tiny = camera.copy(viewportWidth = .5, viewportHeight = .75)
        assertNull(inspectionRectangle(tiny, Point(.25, .375), 200.0, false))
    }

    @Test fun invalidViewportAndDraggedCoordinatesAreRejected() {
        for (bad in listOf(Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY)) {
            assertFailsWith<IllegalArgumentException> {
                inspectionRectangle(camera.copy(viewportWidth = bad), Point(500.0, 400.0), 100.0, false)
            }
            assertFailsWith<IllegalArgumentException> {
                inspectionRectangle(camera, Point(500.0, 400.0), 100.0, false, Point(bad, 30.0))
            }
        }
        for (bad in listOf(0.0, -1.0)) {
            assertFailsWith<IllegalArgumentException> {
                inspectionRectangle(camera.copy(viewportHeight = bad), Point(500.0, 400.0), 100.0, false)
            }
        }
    }

    @Test fun draggedPaneClampsAllEdgesAndRetainsPhysicalNibClearance() {
        val nib = Point(500.0, 400.0)
        for (left in listOf(false, true)) {
            for (x in listOf(-10_000.0, 10_000.0)) for (y in listOf(-10_000.0, 10_000.0)) {
                val pane = assertNotNull(inspectionRectangle(camera, nib, 250.0, left, Point(x, y)))
                assertTrue(pane.x >= 0 && pane.y >= 0)
                assertTrue(pane.x + pane.width <= camera.viewportWidth && pane.y + pane.height <= camera.viewportHeight)
                val closestX = nib.x.coerceIn(pane.x, pane.x + pane.width)
                val closestY = nib.y.coerceIn(pane.y, pane.y + pane.height)
                assertTrue(kotlin.math.hypot(nib.x - closestX, nib.y - closestY) >= 16.0)
            }
        }
    }

    @Test fun gridRefusesMalformedOrEmptyPaneAndOversizedWork() {
        val pane = MagnifierPane(Rect(0.0, 0.0, 512.0, 512.0), Transform(a = 8.0, d = 8.0), 8.0, 8, false, true)
        for (rectangle in listOf(Rect(0.0, 0.0, 0.0, 10.0), Rect(0.0, 0.0, -1.0, 10.0),
            Rect(Double.NaN, 0.0, 10.0, 10.0), Rect(0.0, 0.0, 10.0, Double.POSITIVE_INFINITY),
            Rect(0.0, 0.0, 1e9, 1e9))) {
            assertTrue(precisionPixelGrid(pane.copy(rectangle = rectangle)).isEmpty())
        }
    }

    @Test fun gridAtWorkBoundaryHasAtMost258FiniteDocumentEdgeLines() {
        val pane = MagnifierPane(Rect(0.0, 0.0, 1024.0, 1024.0), Transform(a = 8.0, d = 8.0), 8.0, 8, false, true)
        val grid = precisionPixelGrid(pane)
        assertTrue(grid.size == 258)
        assertTrue(grid.all { (a, b) -> a.x.isFinite() && a.y.isFinite() && b.x.isFinite() && b.y.isFinite() })
        assertTrue(precisionPixelGrid(pane.copy(rectangle = pane.rectangle.copy(width = 1032.0))).isEmpty())
    }
    @Test fun nibAtEveryViewportCornerKeepsExactlySixteenPixelsOfClearance() {
        for (left in listOf(false, true)) {
            for (x in listOf(0.0, camera.viewportWidth)) for (y in listOf(0.0, camera.viewportHeight)) {
                val nib = Point(x, y)
                val pane = assertNotNull(inspectionRectangle(camera, nib, 250.0, left))
                val closestX = nib.x.coerceIn(pane.x, pane.x + pane.width)
                val closestY = nib.y.coerceIn(pane.y, pane.y + pane.height)
                assertTrue(kotlin.math.hypot(nib.x - closestX, nib.y - closestY) >= 16.0)
                assertTrue(pane.x >= 0 && pane.y >= 0 && pane.x + pane.width <= camera.viewportWidth && pane.y + pane.height <= camera.viewportHeight)
            }
        }
    }

}
