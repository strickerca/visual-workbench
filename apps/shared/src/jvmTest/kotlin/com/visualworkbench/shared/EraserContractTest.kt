package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class EraserContractTest {
    private fun id(n: Int): String = "00000000-0000-7000-8000-${n.toString().padStart(12, '0')}"
    private fun options(): VectorEraseEdit = VectorEraseEdit(
        WorkflowBinding(id(1), id(2), 0u, "a".repeat(64)), WorkflowMetadata(id(3), "device", 1u, 1000),
        listOf(VectorEraseTarget(id(4), id(5))), listOf(Point(1.0, 2.0), Point(3.0, 4.0)), 12.0)
    @Test fun mutableInputIsCopiedBeforeNativeQueueSuspension() {
        val targets = options().targets.toMutableList(); val points = options().centers.toMutableList()
        val request = options().copy(targets = targets, centers = points).ownedEraseRequest()
        targets.clear(); points[0] = Point(1000.0, 2000.0)
        assertEquals(1, request.targets.size); assertEquals(Point(1.0, 2.0), request.centers[0])
        assertEquals(0uL, request.binding.hostSeq); assertEquals("a".repeat(64), request.binding.stateHash)
    }
    @Test fun duplicateOrReusedReplacementIdsAreRefused() {
        val request = options()
        val failure = assertThrows(WorkflowFailure::class.java) {
            request.copy(targets = listOf(VectorEraseTarget(id(4), id(4)))).ownedEraseRequest()
        }
        assertEquals(WorkflowFailureKind.Invalid, failure.kind)
        assertThrows(WorkflowFailure::class.java) { request.copy(targets = request.targets + request.targets).ownedEraseRequest() }
    }
    @Test fun nonfiniteCoordinatesAndNoResizeBoundsFailBeforeNativeCall() {
        for (point in listOf(Point(Double.NaN, 0.0), Point(0.0, Double.POSITIVE_INFINITY), Point(1_000_000_001.0, 0.0))) {
            assertEquals(WorkflowFailureKind.Invalid, assertThrows(WorkflowFailure::class.java) {
                options().copy(centers = listOf(point)).ownedEraseRequest()
            }.kind)
        }
        assertEquals(WorkflowFailureKind.Limit, assertThrows(WorkflowFailure::class.java) {
            options().copy(centers = List(4_097) { Point(1.0, 1.0) }).ownedEraseRequest()
        }.kind)
    }
    @Test fun allAcceptedSamplesAndExplicitBudgetsRemainExact() {
        val samples = List(4_096) { Point(it.toDouble(), (it % 7).toDouble()) }
        val value = options().copy(centers = samples, radius = 0.5, maxOutputVertices = 4_096u, maxWorkUnits = 123_456u).ownedEraseRequest()
        assertEquals(samples, value.centers); assertEquals(0.5, value.radius, 0.0)
        assertEquals(4_096u, value.maxOutputVertices); assertEquals(123_456uL, value.maxWorkUnits)
    }
}
