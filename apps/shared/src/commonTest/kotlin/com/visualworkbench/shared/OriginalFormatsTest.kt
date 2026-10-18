package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class OriginalFormatsTest {
    private val binding = WorkflowBinding("018bcfe5-6800-7000-8000-000000000001", "018bcfe5-6800-7000-8000-000000000002", 19uL, "1".repeat(64))
    private fun receipt(format: String = "tiff", warning: Boolean = true, depth: UByte = 16u): OriginalFormatInfo =
        OriginalFormatInfo(binding, "2".repeat(64), format, depth, false, warning)
    private fun info(): ProjectInfo = ProjectInfo(binding.projectId, "fixture", "local", 20uL, false, false,
        binding.hostSeq, binding.stateHash, listOf(binding.documentId))

    @Test fun multipageWarningRemainsVisibleForHighDepthAndUntaggedOriginals() {
        val value = receipt().copy(untagged = true)
        validateOriginalFormatInfo(value, binding)
        val notice = assertNotNull(originalFormatNotice(value))
        assertTrue(notice.contains("first TIFF page"))
        assertTrue(notice.contains("unchanged source file"))
        assertEquals(16u.toUByte(), value.sourceBitDepth)
        assertTrue(value.untagged)
    }
    @Test fun singlePageAndBmpHaveNoMisleadingMultipageWarning() {
        assertNull(originalFormatNotice(receipt(warning = false)))
        val value = receipt("bmp", false, 8u)
        validateOriginalFormatInfo(value, binding)
        assertNull(originalFormatNotice(value))
    }
    @Test fun mismatchedFullRevisionCannotBeAccepted() {
        assertFailsWith<IllegalArgumentException> { validateOriginalFormatInfo(receipt().copy(binding = binding.copy(stateHash = "3".repeat(64))), binding) }
        assertFalse(originalFormatRevisionMatches(info().copy(stateHash = "3".repeat(64)), binding))
        assertFalse(originalFormatRevisionMatches(info().copy(hostSeq = 20uL), binding))
    }
    @Test fun replacementProjectOrMissingDocumentCannotAdoptAnOldNotice() {
        assertFalse(originalFormatRevisionMatches(info().copy(projectId = "018bcfe5-6800-7000-8000-000000000003"), binding))
        assertFalse(originalFormatRevisionMatches(info().copy(documentIds = emptyList()), binding))
        assertTrue(originalFormatRevisionMatches(info(), binding))
    }
    @Test fun malformedAssetHashOrUnknownDepthCannotEnterTheUi() {
        assertFailsWith<IllegalArgumentException> { validateOriginalFormatInfo(receipt().copy(sourceAssetId = "x".repeat(64)), binding) }
        assertFailsWith<IllegalArgumentException> { validateOriginalFormatInfo(receipt().copy(sourceAssetId = "2".repeat(63)), binding) }
        assertFailsWith<IllegalArgumentException> { validateOriginalFormatInfo(receipt(depth = 12u), binding) }
    }
    @Test fun tiffWarningCannotBeAttachedToBmpOrUnknownFormat() {
        assertFailsWith<IllegalArgumentException> { validateOriginalFormatInfo(receipt("bmp", true), binding) }
        assertFailsWith<IllegalArgumentException> { validateOriginalFormatInfo(receipt("unknown", false), binding) }
    }
}
