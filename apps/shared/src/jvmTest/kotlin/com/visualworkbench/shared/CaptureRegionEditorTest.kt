package com.visualworkbench.shared

import kotlinx.coroutines.CancellationException
import org.junit.Assert.*
import org.junit.Test

class CaptureRegionEditorTest {
    private val camera = Camera(Point(50.0, 30.0), 2.0, 0.0, 240.0, 160.0)
    private fun mapped(camera: Camera, point: Point): Point = Point(
        (point.x - camera.viewportWidth / 2) / camera.scale + camera.center.x,
        (point.y - camera.viewportHeight / 2) / camera.scale + camera.center.y,
    )
    private fun editor() = CaptureRegionEditor { c, inverse, points ->
        require(inverse)
        points.map { mapped(c, it) }
    }.also { it.bind("source-a", 100u, 60u, camera) }
    private fun completed(value: CaptureRegionEditor) {
        assertTrue(value.begin(Point(40.0, 40.0)))
        assertTrue(value.finish(Point(100.0, 100.0)))
    }

    @Test fun displayLetterboxingAndDpiNeverDoubleScaleFrameCoordinates() {
        for (density in listOf(1.0, 1.25, 1.5, 2.0, 3.0)) {
            val value = CaptureRegionEditor { c, _, points -> points.map { mapped(c, it) } }
            value.bind("source-a", 100u, 60u, camera.copy(scale = camera.scale * density,
                viewportWidth = camera.viewportWidth * density, viewportHeight = camera.viewportHeight * density))
            assertTrue(value.begin(Point(40.0 * density, 40.0 * density)))
            assertTrue(value.finish(Point(100.0 * density, 100.0 * density)))
            assertEquals(listOf(10.0, 10.0, 40.0, 40.0), value.completedEdges())
        }
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
        for (finished in listOf(false, true)) for (change in 0..4) {
            val value = editor(); assertTrue(value.begin(Point(40.0, 40.0)))
            if (finished) assertTrue(value.finish(Point(100.0, 100.0)))
            when (change) {
                0 -> value.bind("source-b", 100u, 60u, camera)
                1 -> value.bind("source-a", 99u, 60u, camera)
                2 -> value.bind("source-a", 100u, 59u, camera)
                3 -> value.bind("source-a", 100u, 60u, camera.copy(scale = 3.0))
                else -> value.bind("source-a", 100u, 60u, camera.copy(viewportWidth = 241.0))
            }
            assertNull(value.edges); assertFalse(value.contacting)
            assertFalse(value.finish(Point(100.0, 100.0)))
        }
    }
    @Test fun cancelAndDetachCannotReuseACompletedRectangle() {
        val value = editor(); completed(value)
        value.cancel(); assertNull(value.completedEdges()); value.detach()
        assertFalse(value.begin(Point(40.0, 40.0)))
    }
    @Test fun nonfiniteInputAndMappingFailureDoNotEscapeTheDisplayBoundary() {
        val value = editor(); assertFalse(value.begin(Point(Double.NaN, 0.0)))
        val failed = CaptureRegionEditor { _, _, _ -> error("source replaced") }
        failed.bind("a", 1u, 1u, camera); assertFalse(failed.begin(Point(1.0, 1.0)))
    }
    @Test fun invalidMoveRetiresContactRatherThanReusingOldEndpoints() {
        for (invalid in listOf(Point(-1.0, 40.0), Point(500.0, 60.0), Point(40.0, Double.NaN),
            Point(Double.POSITIVE_INFINITY, 40.0))) {
            val value = editor(); assertTrue(value.begin(Point(40.0, 40.0)))
            assertFalse(value.move(invalid)); assertFalse(value.contacting); assertNull(value.edges)
            assertFalse(value.finish(Point(100.0, 100.0))); assertNull(value.completedEdges())
        }
    }
    @Test fun invalidCameraReplacementDetachesThePreviousCompletedSelection() {
        for (invalid in listOf(camera.copy(scale = 0.0), camera.copy(scale = -1.0),
            camera.copy(scale = Double.NaN), camera.copy(scale = Double.POSITIVE_INFINITY),
            camera.copy(scale = Double.MIN_VALUE), camera.copy(scale = Double.MAX_VALUE),
            camera.copy(rotation = .1), camera.copy(rotation = Double.NaN),
            camera.copy(center = Point(Double.NaN, 30.0)), camera.copy(center = Point(50.0, Double.NEGATIVE_INFINITY)),
            camera.copy(center = Point(Double.MAX_VALUE, 30.0)),
            camera.copy(viewportWidth = 0.0), camera.copy(viewportHeight = -1.0),
            camera.copy(viewportWidth = Double.POSITIVE_INFINITY))) {
            val value = editor(); completed(value)
            assertTrue(runCatching { value.bind("source-b", 100u, 60u, invalid) }.exceptionOrNull() is IllegalArgumentException)
            assertNull(value.completedEdges()); assertFalse(value.contacting)
            assertFalse(value.begin(Point(40.0, 40.0)))
        }
    }
    @Test fun invalidSourceOrExtentReplacementAlsoDetachesThePreviousSelection() {
        for ((source, width, height) in listOf(Triple("", 100u, 60u), Triple("x".repeat(257), 100u, 60u),
            Triple("source-b", 0u, 60u), Triple("source-b", 100u, 0u),
            Triple("source-b", 32769u, 60u), Triple("source-b", 100u, 32769u))) {
            val value = editor(); completed(value)
            assertTrue(runCatching { value.bind(source, width, height, camera) }.isFailure)
            assertNull(value.edges); assertFalse(value.begin(Point(40.0, 40.0)))
        }
    }
    @Test fun identicalBindingAndDuplicateFinishCannotAlterCompletedEdges() {
        val value = editor(); completed(value)
        val edges = value.completedEdges()
        value.bind("source-a", 100u, 60u, camera.copy())
        assertEquals(edges, value.completedEdges())
        assertFalse(value.finish(Point(120.0, 120.0))); assertEquals(edges, value.completedEdges())
        assertFalse(value.move(Point(120.0, 120.0))); assertEquals(edges, value.completedEdges())
    }
    @Test fun offViewportInputCannotSelectUnseenPixelsOfAZoomedSource() {
        val value = CaptureRegionEditor { c, _, points -> points.map { mapped(c, it) } }
        value.bind("source", 100u, 60u, camera.copy(viewportWidth = 100.0, viewportHeight = 80.0))
        // (-1, 20) maps to (24.5, 20), inside the image but outside the Canvas.
        assertFalse(value.begin(Point(-1.0, 20.0)))
        assertTrue(value.begin(Point(20.0, 20.0)))
        assertFalse(value.finish(Point(101.0, 60.0))); assertNull(value.completedEdges())
    }
    @Test fun exactFrameBordersAndFractionalReversedEdgesStayUnrounded() {
        val value = editor()
        assertTrue(value.begin(Point(20.0, 20.0))); assertTrue(value.finish(Point(220.0, 140.0)))
        assertEquals(listOf(0.0, 0.0, 100.0, 60.0), value.completedEdges())
        assertTrue(value.begin(Point(100.5, 100.25))); assertTrue(value.finish(Point(40.25, 40.5)))
        assertEquals(listOf(40.25, 40.125, 10.125, 10.25), value.completedEdges())
    }
    @Test fun cancellationFromTheMapperPropagatesAndRetiresTheContact() {
        var cancelled = false
        val failure = CancellationException("picker cancelled")
        val value = CaptureRegionEditor { c, _, points ->
            if (cancelled) throw failure
            points.map { mapped(c, it) }
        }
        value.bind("source-a", 100u, 60u, camera)
        assertTrue(value.begin(Point(40.0, 40.0))); cancelled = true
        assertSame(failure, runCatching { value.move(Point(100.0, 100.0)) }.exceptionOrNull())
        assertFalse(value.contacting); assertNull(value.edges)
        cancelled = false
        assertFalse(value.finish(Point(100.0, 100.0)))
    }
    @Test fun malformedMapperResultsRetireTheActiveContact() {
        for (reply in listOf(emptyList(), listOf(Point(1.0, 1.0), Point(2.0, 2.0)),
            listOf(Point(Double.NaN, 1.0)), listOf(Point(101.0, 1.0)))) {
            var moving = false
            val value = CaptureRegionEditor { c, _, points -> if (moving) reply else points.map { mapped(c, it) } }
            value.bind("source-a", 100u, 60u, camera)
            assertTrue(value.begin(Point(40.0, 40.0))); moving = true
            assertFalse(value.move(Point(100.0, 100.0))); assertNull(value.edges)
            moving = false
            assertFalse(value.finish(Point(100.0, 100.0)))
        }
    }
    @Test fun mappingCannotPublishAfterDetachAndRebindingTheSameSource() {
        lateinit var value: CaptureRegionEditor
        value = CaptureRegionEditor { c, _, points ->
            value.detach(); value.bind("source-a", 100u, 60u, camera)
            points.map { mapped(c, it) }
        }
        value.bind("source-a", 100u, 60u, camera)
        assertFalse(value.begin(Point(40.0, 40.0))); assertNull(value.edges)
    }
    @Test fun obsoleteFinishCannotCancelAReplacementGestureStartedInsideMapping() {
        var replacing = false
        lateinit var value: CaptureRegionEditor
        value = CaptureRegionEditor { c, _, points ->
            if (replacing) {
                replacing = false
                value.detach(); value.bind("source-b", 100u, 60u, camera)
                assertTrue(value.begin(Point(60.0, 60.0)))
            }
            points.map { mapped(c, it) }
        }
        value.bind("source-a", 100u, 60u, camera)
        assertTrue(value.begin(Point(40.0, 40.0))); replacing = true
        assertFalse(value.finish(Point(100.0, 100.0)))
        assertTrue(value.contacting)
        assertTrue(value.finish(Point(120.0, 120.0)))
        assertEquals(listOf(20.0, 20.0, 50.0, 50.0), value.completedEdges())
    }
    @Test fun unchangedBindingPreservesTheActiveContact() {
        val value = editor()
        assertTrue(value.begin(Point(40.0, 40.0)))
        value.bind("source-a", 100u, 60u, camera.copy())
        assertTrue(value.contacting)
        assertTrue(value.finish(Point(100.0, 100.0)))
        assertEquals(listOf(10.0, 10.0, 40.0, 40.0), value.completedEdges())
    }
    @Test fun maximumAdmittedExtentCanSelectItsExactInclusivePixelEdges() {
        val value = CaptureRegionEditor { c, _, points -> points.map { mapped(c, it) } }
        value.bind("s".repeat(256), 32768u, 32768u,
            Camera(Point(16384.0, 16384.0), 1.0 / 256.0, 0.0, 128.0, 128.0))
        assertTrue(value.begin(Point(0.0, 0.0)))
        assertTrue(value.finish(Point(128.0, 128.0)))
        assertEquals(listOf(0.0, 0.0, 32768.0, 32768.0), value.completedEdges())
    }
    @Test fun ordinaryMappingFailureDuringMoveCannotLeaveAFinishableContact() {
        var failed = false
        val value = CaptureRegionEditor { c, _, points ->
            if (failed) error("mapping unavailable")
            points.map { mapped(c, it) }
        }
        value.bind("source-a", 100u, 60u, camera)
        assertTrue(value.begin(Point(40.0, 40.0))); failed = true
        assertFalse(value.move(Point(100.0, 100.0))); assertNull(value.edges)
        failed = false
        assertFalse(value.finish(Point(100.0, 100.0)))
    }

}
