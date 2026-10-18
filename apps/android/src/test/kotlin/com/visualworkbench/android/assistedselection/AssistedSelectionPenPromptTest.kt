package com.visualworkbench.android.assistedselection

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

class AssistedSelectionPenPromptTest {
    private val revision = ProjectInfo("project", "Test", "device", 1uL, false, false,
        0uL, "a".repeat(64), listOf("document"))
    private val source = AssistedSelectionSource(SelectionDocument(SelectionBinding("project", "document",
        "b".repeat(64), 0uL, revision.stateHash), 128u, 64u, 8u, revision), 0u, 1L)

    @Test fun penTapAppendsOneRealPointWithoutChangingEarlierPromptsOrBox() {
        val before = AssistedSelectionPrompt(positive = listOf(Point(1.0, 1.0)), box = Rect(0.0, 0.0, 32.0, 32.0))
        val included = appendAssistedPenPrompt(before, Point(2.0, 3.0), false, source)!!
        val excluded = appendAssistedPenPrompt(included, Point(4.0, 5.0), true, source)!!
        assertEquals(1, before.positive.size); assertTrue(before.negative.isEmpty())
        assertEquals(listOf(Point(1.0, 1.0), Point(2.0, 3.0)), excluded.positive)
        assertEquals(listOf(Point(4.0, 5.0)), excluded.negative); assertEquals(before.box, excluded.box)
    }
    @Test fun combinedTapLimitDoesNotTruncateOrPublishAPartialPrompt() {
        val full = AssistedSelectionPrompt(positive = List(32) { Point(1.0, 1.0) }, negative = List(32) { Point(2.0, 2.0) })
        assertNull(appendAssistedPenPrompt(full, Point(3.0, 3.0), false, source))
        assertNull(appendAssistedPenPrompt(full, Point(3.0, 3.0), true, source))
        assertEquals(32, full.positive.size); assertEquals(32, full.negative.size)
    }
    @Test fun nonfiniteAndOutsideDocumentPointsAreRefused() {
        val empty = AssistedSelectionPrompt()
        for (point in listOf(Point(Double.NaN, 0.0), Point(0.0, Double.POSITIVE_INFINITY),
            Point(-0.1, 0.0), Point(128.0, 0.0), Point(0.0, 64.0)))
            assertNull(appendAssistedPenPrompt(empty, point, false, source))
        assertNotNull(appendAssistedPenPrompt(empty, Point(127.999, 63.999), false, source))
    }
}
