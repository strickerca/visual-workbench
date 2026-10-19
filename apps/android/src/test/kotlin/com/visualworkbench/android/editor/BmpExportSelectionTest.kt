package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

class BmpExportSelectionTest {
    private val info = ProjectInfo("project", "fixture", "owner", 8uL, true, false, 7uL, "1".repeat(64), listOf("document"))
    private val document = DocumentSnapshot("document", "fixture", 8u, 6u, 16u, emptyList(), RenderList(info, emptyList()))
    private val options = ExportOptions("document", ImageFormat.Bmp8, true, Rect(1.0, 2.0, 4.0, 3.0),
        null, false, false, false)
    private fun ticket() = ExportTicket("ticket", info.projectId, document.documentId, info.hostSeq, info.stateHash, options, ExportEncoding.Bmp8)

    @Test fun bmpIsAvailableWithItsActualCodecAndNoImplicitDepthColorOrMatteConsent() {
        assertTrue(ExportEncoding.entries.all { it.exportAvailable })
        val selection = ExportSelection(encoding = ExportEncoding.Bmp8)
        assertEquals(ImageFormat.Bmp8, selection.encoding.format(selection.quality))
        assertEquals("image/bmp", selection.encoding.mime); assertEquals("bmp", selection.encoding.extension)
        assertTrue(selection.marked); assertFalse(selection.viewOnly)
        assertFalse(selection.allowDepthReduction); assertFalse(selection.convertToSrgb); assertNull(selection.matteRgb)
        assertNull(selection.copy(whiteMatte = true).matteRgb)
        assertEquals(16u, document.bitDepth)
    }
    @Test fun bmpTicketPreservesThePreparedRegionAndRejectsChangedExportPolicies() {
        val ticket = ticket()
        assertSame(options, ticket.options)
        assertEquals(ImageFormat.Bmp8, ticket.options.format)
        assertEquals(Rect(1.0, 2.0, 4.0, 3.0), ticket.options.region)
        assertTrue(ticket.matches(info, document, options.copy()))
        for (changed in listOf(options.copy(format = ImageFormat.Png8), options.copy(marked = false),
            options.copy(region = null), options.copy(region = Rect(0.0, 0.0, 4.0, 3.0)),
            options.copy(matteRgb = 0xffffffu), options.copy(convertToSrgb = true),
            options.copy(assumeUntaggedSrgb = true), options.copy(allowDepthReduction = true),
            options.copy(memoryBudgetBytes = 128uL * 1024uL * 1024uL))) assertFalse(ticket.matches(info, document, changed))
        assertFalse(ticket.matches(info, document, null))
    }
    @Test fun bmpTicketAndReadyReceiptStillRequireTheExactProjectDocumentAndRevision() {
        val ticket = ticket()
        val ready = ExportReady(info.projectId, document.documentId, info.stateHash, info.hostSeq, 4u, 3u, "2".repeat(64))
        assertTrue(ready.matches(info, document))
        for (changed in listOf(info.copy(projectId = "another"), info.copy(hostSeq = 8uL), info.copy(stateHash = "3".repeat(64)))) {
            assertFalse(ticket.matches(changed, document, options)); assertFalse(ready.matches(changed, document))
        }
        val anotherDocument = document.copy(documentId = "another")
        assertFalse(ticket.matches(info, anotherDocument, options)); assertFalse(ready.matches(info, anotherDocument))
        assertFalse(ticket.matches(null, document, options)); assertFalse(ready.matches(info, null))
    }
    @Test fun bmpViewRegionKeepsOutwardRoundingAndClippingWithoutChangingSelectionPolicies() {
        val points = listOf(Point(-1.5, 1.2), Point(3.8, 1.2), Point(3.8, 9.2), Point(-1.5, 9.2))
        assertEquals(Rect(0.0, 1.0, 4.0, 5.0), viewExportRegion(points, 8u, 6u))
        val selection = ExportSelection(encoding = ExportEncoding.Bmp8, viewOnly = true, marked = false,
            convertToSrgb = true, allowDepthReduction = true)
        assertTrue(selection.viewOnly); assertFalse(selection.marked)
        assertTrue(selection.convertToSrgb); assertTrue(selection.allowDepthReduction)
        assertEquals(ImageFormat.Bmp8, selection.encoding.format(selection.quality)); assertNull(selection.matteRgb)
    }
    @Test fun bmpAvailabilityDoesNotAdmitEmptyOrNonFiniteViewRegions() {
        for (points in listOf(emptyList(), listOf(Point(0.0, 0.0)), List(4) { Point(Double.NaN, 0.0) },
            List(4) { Point(Double.POSITIVE_INFINITY, 0.0) }, List(4) { Point(100.0, 100.0) })) {
            assertThrows(IllegalArgumentException::class.java) { viewExportRegion(points, 8u, 6u) }
        }
    }
}
