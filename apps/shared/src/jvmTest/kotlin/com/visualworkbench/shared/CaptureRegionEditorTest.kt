package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class CaptureRegionEditorTest {
    private val camera = Camera(Point(50.0, 30.0), 2.0, 0.0, 240.0, 160.0)
    private fun editor() = CaptureRegionEditor { c, inverse, points ->
        require(inverse)
        points.map { Point((it.x - c.viewportWidth / 2) / c.scale + c.center.x,
            (it.y - c.viewportHeight / 2) / c.scale + c.center.y) }
    }.also { it.bind("source-a", 100u, 60u, camera) }
    @Test fun displayLetterboxingAndDpiNeverDoubleScaleFrameCoordinates() {
        val value = editor(); assertTrue(value.begin(Point(40.0, 40.0)))
        assertTrue(value.finish(Point(100.0, 100.0)))
        assertEquals(listOf(10.0, 10.0, 40.0, 40.0), value.completedEdges())
    }
    @Test fun reversedDragRemainsExplicitForNativePixelEdgeAdmission() {
        val value = editor(); value.begin(Point(100.0, 100.0)); value.finish(Point(40.0, 40.0))
        assertEquals(listOf(40.0, 40.0, 10.0, 10.0), value.completedEdges())
    }
    @Test fun outsideImageOrEmptyContactsDoNotBecomeRegionRequests() {
        val value = editor(); assertFalse(value.begin(Point(0.0, 0.0)))
        value.begin(Point(40.0, 40.0)); assertFalse(value.finish(Point(40.0, 60.0)))
        assertNull(value.completedEdges()); value.begin(Point(40.0, 40.0))
        assertFalse(value.finish(Point(500.0, 60.0))); assertNull(value.completedEdges())
    }
    @Test fun sourceViewportAndSizeChangesInvalidateBothDownAndCompletedDrafts() {
        for (change in 0..2) { val value = editor(); value.begin(Point(40.0, 40.0)); value.finish(Point(100.0, 100.0))
            when (change) {
                0 -> value.bind("source-b", 100u, 60u, camera)
                1 -> value.bind("source-a", 99u, 60u, camera)
                else -> value.bind("source-a", 100u, 60u, camera.copy(scale = 3.0))
            }
            assertNull(value.edges); assertFalse(value.finish(Point(100.0, 100.0)))
        }
    }
    @Test fun cancelAndDetachCannotReuseACompletedRectangle() {
        val value = editor(); value.begin(Point(40.0, 40.0)); value.finish(Point(100.0, 100.0))
        value.cancel(); assertNull(value.completedEdges()); value.detach(); assertFalse(value.begin(Point(40.0, 40.0)))
    }
    @Test fun nonfiniteInputAndMappingFailureDoNotEscapeTheDisplayBoundary() {
        val value = editor(); assertFalse(value.begin(Point(Double.NaN, 0.0)))
        val failed = CaptureRegionEditor { _, _, _ -> error("source replaced") }
        failed.bind("a", 1u, 1u, camera); assertFalse(failed.begin(Point(1.0, 1.0)))
    }
}
