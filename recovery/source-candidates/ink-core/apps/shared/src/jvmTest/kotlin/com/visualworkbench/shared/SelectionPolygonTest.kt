package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class SelectionPolygonTest {
    @Test fun boundedVertexListRefusesMoreContactsBeforeGrowing() {
        val draft = SelectionPolygon(3)
        for (p in listOf(Point(0.0, 0.0), Point(10.0, 0.0), Point(10.0, 10.0))) {
            assertTrue(draft.begin(p)); assertTrue(draft.end())
        }
        assertFalse(draft.begin(Point(0.0, 10.0))); assertEquals(3, draft.count)
        assertEquals(3, draft.completed()?.size)
    }
    @Test fun movingContactNeverAddsVerticesAndCannotPublishWhileDown() {
        val draft = SelectionPolygon()
        assertTrue(draft.begin(Point(0.0, 0.0)))
        repeat(100) { assertTrue(draft.move(Point(it.toDouble(), 0.0))) }
        assertEquals(0, draft.count); assertEquals(listOf(Point(99.0, 0.0)), draft.preview())
        assertNull(draft.completed()); assertFalse(draft.undo()); assertFalse(draft.begin(Point(1.0, 1.0)))
        assertTrue(draft.end()); assertEquals(1, draft.count)
    }
    @Test fun duplicateConsecutiveVertexDoesNotConsumeBudgetAndSnapshotsAreOwned() {
        val draft = SelectionPolygon(3)
        repeat(5) { assertTrue(draft.begin(Point(0.0, 0.0))); assertTrue(draft.end()) }
        assertEquals(1, draft.count)
        for (p in listOf(Point(10.0, 0.0), Point(5.0, 10.0))) { draft.begin(p); draft.end() }
        val saved = draft.completed()!!
        assertTrue(draft.undo()); assertNull(draft.completed()); assertEquals(3, saved.size)
    }
    @Test fun invalidCoordinatesCannotEnterTheDraft() {
        val draft = SelectionPolygon()
        for (p in listOf(Point(Double.NaN, 0.0), Point(0.0, Double.POSITIVE_INFINITY), Point(8_388_609.0, 0.0))) {
            assertFalse(draft.begin(p)); assertEquals(0, draft.count)
        }
        assertTrue(draft.begin(Point(2.0, 3.0)))
        assertFalse(draft.move(Point(Double.NaN, 0.0)))
        assertEquals(listOf(Point(2.0, 3.0)), draft.preview())
    }
}
