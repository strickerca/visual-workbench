package com.visualworkbench.shared

import kotlin.test.*

class DocumentFramesTest {
    private val revision = ProjectInfo("project", "title", "device", 1uL, false, false, 1uL, "b".repeat(64), listOf("doc"))
    private val binding = DocumentFrameBinding("a".repeat(64), "doc", 0u, revision, 1L, 1L)
    private fun tile() = DocumentDisplayTile(Point(0.0, 0.0), 1.0, 1u, 1u, byteArrayOf(1, 2, 3, -1))

    @Test fun cancelledGenerationCannotReviveOnAnIdenticalViewport() {
        val frame = DocumentDisplayFrame(binding, listOf(tile()))
        validateDocumentDisplayFrame(frame, binding)
        assertEquals(DocumentFailureKind.Stale,
            assertFailsWith<DocumentFailure> { validateDocumentDisplayFrame(frame, binding.copy(viewportGeneration = 3L)) }.kind)
    }
    @Test fun sourcePageAttachmentAndFullRevisionFenceTheSamePixelShape() {
        val frame = DocumentDisplayFrame(binding, listOf(tile()))
        for (current in listOf(binding.copy(sourceAssetId = "c".repeat(64)), binding.copy(page = 1u),
            binding.copy(attachmentEpoch = 2L), binding.copy(revision = revision.copy(nextLamport = 2uL)),
            binding.copy(revision = revision.copy(stateHash = "d".repeat(64))),
            binding.copy(revision = revision.copy(documentIds = emptyList())))) {
            assertEquals(DocumentFailureKind.Stale,
                assertFailsWith<DocumentFailure> { validateDocumentDisplayFrame(frame, current) }.kind)
        }
    }
    @Test fun zeroOversizedOrTruncatedTilesAndNonfiniteGeometryAreRefused() {
        for (bad in listOf(
            DocumentDisplayTile(Point(0.0, 0.0), 1.0, 0u, 1u, byteArrayOf()),
            DocumentDisplayTile(Point(0.0, 0.0), 1.0, UInt.MAX_VALUE, UInt.MAX_VALUE, byteArrayOf()),
            DocumentDisplayTile(Point(0.0, 0.0), 1.0, 1u, 1u, byteArrayOf(1)),
            DocumentDisplayTile(Point(Double.NaN, 0.0), 1.0, 1u, 1u, byteArrayOf(1, 2, 3, -1)),
            DocumentDisplayTile(Point(0.0, 0.0), Double.POSITIVE_INFINITY, 1u, 1u, byteArrayOf(1, 2, 3, -1)),
        )) assertEquals(DocumentFailureKind.Verification,
            assertFailsWith<DocumentFailure> { validateDocumentDisplayFrame(DocumentDisplayFrame(binding, listOf(bad)), binding) }.kind)
    }
    @Test fun frameTileCountIsAdmittedBeforePlatformCopies() {
        assertEquals(DocumentFailureKind.Limit,
            assertFailsWith<DocumentFailure> { validateDocumentDisplayFrame(DocumentDisplayFrame(binding, List(33) { tile() }), binding) }.kind)
    }
}
