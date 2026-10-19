package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.flow.MutableSharedFlow
import org.junit.Assert.*
import org.junit.Test

class ExportProjectionTest {
    private val binding = WorkflowBinding("project", "document", 7uL, "a".repeat(64))
    private val project = object : WorkbenchProject {
        override val changes = MutableSharedFlow<ProjectChange>()
        override suspend fun info(): ProjectInfo = error("unused")
        override suspend fun document(documentId: String): DocumentSnapshot = error("unused")
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
        override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
        override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
        override suspend fun export(options: ExportOptions): ExportResult = error("unused")
        override suspend fun close() = Unit
    }

    @Test fun selectionAndFullOptionsKeepDIntentAndUseBoundAsymmetricOFrame() = runBlocking {
        val original = ExportOptions("document", ImageFormat.Bmp8, false, Rect(1.0, 2.0, 4.0, 3.0),
            0xffffffu, true, true, true, 32uL * 1024uL * 1024uL)
        val output = SelectionRegion(1u, 1u, 2u, 4u)
        val (projected, geometry) = projectExportOptions(project, binding, original) { queriedProject, expected, source, budget ->
            assertSame(project, queriedProject); assertEquals(binding, expected)
            assertEquals(SelectionRegion(1u, 2u, 4u, 3u), source)
            assertEquals(original.memoryBudgetBytes, budget)
            ImageOutputExportGeometry(expected, "b".repeat(64), 8u, 6u, 4u, 6u, output, 2u, 4u)
        }
        assertEquals(Rect(1.0, 2.0, 4.0, 3.0), original.region)
        assertEquals(Rect(1.0, 1.0, 2.0, 4.0), projected.region)
        assertEquals(original.copy(region = projected.region), projected)
        assertEquals(2u, geometry.encodedWidth); assertEquals(4u, geometry.encodedHeight)
        val full = original.copy(region = null)
        val (fullOptions, fullGeometry) = projectExportOptions(project, binding, full) { _, expected, source, _ ->
            assertNull(source)
            ImageOutputExportGeometry(expected, "b".repeat(64), 8u, 6u, 4u, 6u, null, 4u, 6u)
        }
        assertNull(fullOptions.region); assertEquals(4u, fullGeometry.encodedWidth)
        assertEquals(6u, fullGeometry.encodedHeight)
    }

    @Test fun malformedDCoordinatesAreRejectedBeforeNativeQuery() = runBlocking {
        val regions = listOf(
            Rect(Double.NaN, 0.0, 1.0, 1.0), Rect(Double.POSITIVE_INFINITY, 0.0, 1.0, 1.0),
            Rect(-1.0, 0.0, 1.0, 1.0), Rect(0.5, 0.0, 1.0, 1.0),
            Rect(0.0, 0.0, 0.0, 1.0), Rect(0.0, 0.0, UInt.MAX_VALUE.toDouble() + 1.0, 1.0),
        )
        for (region in regions) {
            var queried = false
            try {
                projectExportOptions(project, binding, ExportOptions("document", region = region)) { _, _, _, _ ->
                    queried = true; error("invalid source reached native query")
                }
                fail("Accepted $region")
            } catch (_: IllegalArgumentException) { }
            assertFalse(queried)
        }
    }

    @Test fun staleBindingAndInvalidReturnedDimensionsAreRefused() = runBlocking {
        val source = ExportOptions("document", region = Rect(1.0, 2.0, 4.0, 3.0))
        for (geometry in listOf(
            ImageOutputExportGeometry(binding.copy(hostSeq = 8uL), "b".repeat(64), 8u, 6u, 4u, 6u,
                SelectionRegion(1u, 1u, 2u, 4u), 2u, 4u),
            ImageOutputExportGeometry(binding, "b".repeat(64), 8u, 6u, 4u, 6u,
                SelectionRegion(3u, 1u, 2u, 4u), 2u, 4u),
            ImageOutputExportGeometry(binding, "b".repeat(64), 8u, 6u, 4u, 6u,
                SelectionRegion(1u, 1u, 2u, 4u), 3u, 4u),
        )) {
            try { projectExportOptions(project, binding, source) { _, _, _, _ -> geometry }; fail("Accepted $geometry") }
            catch (_: IllegalArgumentException) { }
        }
    }
}
