package com.visualworkbench.desktop

import com.visualworkbench.shared.AiCompareMode
import com.visualworkbench.shared.AiRegion
import com.visualworkbench.shared.Camera
import com.visualworkbench.shared.Point
import org.junit.Assert.*
import org.junit.Test

class AiViewGeometryTest {
    private val camera = Camera(Point(100.0, 80.0), 2.0, 0.25, 800.0, 600.0)
    private fun corners(left: Double, top: Double, right: Double, bottom: Double) =
        listOf(Point(left, top), Point(right, top), Point(right, bottom), Point(left, bottom))
    private fun refusal(reason: AiDisplayRefusal.Reason, block: () -> Unit) {
        try { block(); fail("Expected bounded display refusal") }
        catch (failure: AiDisplayRefusal) { assertEquals(reason, failure.reason) }
    }
    private fun begin(collector: AiContactCollector, split: Boolean = false, point: Point = Point(10.0, 20.0)) {
        collector.begin("candidate-one", camera, 100u, 80u, split, point, 8.0, false, true)
    }

    @Test fun exactEdgeTilesCoverTheVisibleImageOnce() {
        val plan = aiTilePlan(1000u, 700u, corners(0.0, 0.0, 1000.0, 700.0))
        assertEquals(2_800_000L, plan.rgbaBytes)
        assertEquals(listOf(AiRegion(0u,0u,512u,512u), AiRegion(512u,0u,488u,512u),
            AiRegion(0u,512u,512u,188u), AiRegion(512u,512u,488u,188u)), plan.regions)
    }
    @Test fun exactBudgetBoundaryRefusesBeforeAllocatingTiles() {
        val box = corners(0.0, 0.0, 1000.0, 700.0)
        assertEquals(4, aiTilePlan(1000u, 700u, box, 2_800_000).regions.size)
        refusal(AiDisplayRefusal.Reason.ZoomIn) { aiTilePlan(1000u, 700u, box, 2_799_999) }
        refusal(AiDisplayRefusal.Reason.ZoomIn) {
            aiTilePlan(Int.MAX_VALUE.toUInt(), Int.MAX_VALUE.toUInt(),
                corners(0.0, 0.0, Int.MAX_VALUE.toDouble(), Int.MAX_VALUE.toDouble()))
        }
    }
    @Test fun offDocumentViewIsEmptyAndPartialEdgesStayInBounds() {
        assertTrue(aiTilePlan(1000u, 700u, corners(-100.0,-80.0,-1.0,-1.0)).regions.isEmpty())
        assertEquals(listOf(AiRegion(512u,512u,488u,188u)),
            aiTilePlan(1000u, 700u, corners(999.0,699.0,1200.0,900.0)).regions)
    }
    @Test fun invalidCoordinatesAndSplitOverflowAreRefused() {
        refusal(AiDisplayRefusal.Reason.Invalid) { aiTilePlan(1u, 1u, corners(0.0,0.0,Double.NaN,1.0)) }
        refusal(AiDisplayRefusal.Reason.Invalid) { aiCanvasWidth(Int.MAX_VALUE.toUInt(), AiCompareMode.Split) }
        assertEquals(200u, aiCanvasWidth(100u, AiCompareMode.Split))
        assertEquals(100u, aiCanvasWidth(100u, AiCompareMode.Difference()))
    }
    @Test fun pixelPackingDoesNotSwapChannelsOrDiscardAlpha() {
        val bytes = byteArrayOf(0x12, 0x34, 0x56, 0x78, 0x7f, 0, 0xff.toByte(), 0)
        assertEquals(0x78123456, aiArgb(bytes, 0))
        assertEquals(0x007f00ff, aiArgb(bytes, 4))
        aiCheckPixels(AiRegion(3u, 4u, 2u, 1u), 5u, 5u, bytes)
    }
    @Test fun hostilePixelLengthsAndCoordinatesFailAdmission() {
        refusal(AiDisplayRefusal.Reason.Invalid) { aiCheckPixels(AiRegion(0u,0u,513u,1u), 513u, 1u, ByteArray(0)) }
        refusal(AiDisplayRefusal.Reason.Invalid) { aiCheckPixels(AiRegion(UInt.MAX_VALUE,0u,1u,1u), UInt.MAX_VALUE, 1u, ByteArray(4)) }
        refusal(AiDisplayRefusal.Reason.Invalid) { aiCheckPixels(AiRegion(0u,0u,1u,1u), 1u, 1u, ByteArray(3)) }
        refusal(AiDisplayRefusal.Reason.Invalid) { aiArgb(ByteArray(4), 1) }
    }
    @Test fun splitContactUsesNativeResultPaneCoordinates() {
        val collector = AiContactCollector()
        begin(collector, true, Point(110.0, 20.0))
        collector.append("candidate-one", camera, listOf(Point(115.0, 22.0), Point(115.0,22.0)))
        val contact = checkNotNull(collector.finish("candidate-one", camera))
        assertEquals(listOf(Point(10.0,20.0), Point(15.0,22.0)), contact.points)
        assertTrue(contact.clearFirst)
        assertFalse(collector.active)
        assertNull(collector.finish("candidate-one", camera))
    }
    @Test fun crossingSplitSeamCancelsEntireContact() {
        val collector = AiContactCollector(); begin(collector, true, Point(110.0,20.0))
        refusal(AiDisplayRefusal.Reason.Invalid) { collector.append("candidate-one", camera, listOf(Point(99.0,20.0))) }
        assertNull(collector.finish("candidate-one", camera))
    }
    @Test fun changedCandidateOrViewportCannotPublishCollectedSamples() {
        val collector = AiContactCollector(); begin(collector)
        refusal(AiDisplayRefusal.Reason.Stale) { collector.append("candidate-two", camera, listOf(Point(11.0,20.0))) }
        assertFalse(collector.active)
        begin(collector)
        refusal(AiDisplayRefusal.Reason.Stale) { collector.finish("candidate-one", camera.copy(scale=3.0)) }
        assertFalse(collector.active)
    }
    @Test fun explicitCancelLeavesNoPartialAcceptCommand() {
        val collector = AiContactCollector(); begin(collector)
        collector.append("candidate-one", camera, listOf(Point(11.0,20.0)))
        collector.cancel()
        assertNull(collector.finish("candidate-one", camera))
        assertTrue(collector.points.isEmpty())
    }
    @Test fun contactLimitCancelsInsteadOfThinningOrTruncating() {
        val collector = AiContactCollector(); begin(collector)
        val tooMany = List(AI_CONTACT_POINTS) { Point(11.0,20.0) }
        refusal(AiDisplayRefusal.Reason.ContactLimit) { collector.append("candidate-one", camera, tooMany) }
        assertNull(collector.finish("candidate-one", camera))
    }
    @Test fun immutableFinishedContactDoesNotAcquireLaterSamples() {
        val collector = AiContactCollector(); begin(collector)
        val saved = checkNotNull(collector.finish("candidate-one", camera))
        begin(collector)
        collector.append("candidate-one", camera, listOf(Point(50.0,40.0)))
        assertEquals(listOf(Point(10.0,20.0)), saved.points)
    }
}

