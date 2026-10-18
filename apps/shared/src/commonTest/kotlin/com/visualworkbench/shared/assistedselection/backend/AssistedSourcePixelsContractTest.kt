package com.visualworkbench.shared.assistedselection.backend

import com.visualworkbench.shared.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class AssistedSourcePixelsContractTest {
    @Test fun originalAdmissionUsesFullBindingAndPermitsBothCanonicalDepths() {
        val value = source()
        value.requireAssistedOriginal(BUDGET)
        value.copy(document = value.document.copy(sourceBitDepth = 16u)).requireAssistedOriginal(BUDGET)
        assertEquals(24, value.assistedSourceRgbaBytes())
    }

    @Test fun pagesAndUnrecognizedDepthCannotBecomeOriginalImagePixels() {
        for (value in listOf(source().copy(page = 0u), source().let {
            it.copy(document = it.document.copy(sourceBitDepth = 32u))
        })) assertEquals(SelectionFailureKind.Unsupported,
            assertFailsWith<SelectionFailure> { value.requireAssistedOriginal(BUDGET) }.kind)
    }

    @Test fun bindingMustMatchTheFullVisibleRevisionBeforeAnyPixelsAreRead() {
        val value = source()
        for (binding in listOf(value.binding.copy(stateHash = "c".repeat(64)),
            value.binding.copy(hostSeq = 8uL), value.binding.copy(projectId = "other"))) {
            val changed = value.copy(document = value.document.copy(binding = binding))
            assertEquals(SelectionFailureKind.Invalid,
                assertFailsWith<SelectionFailure> { changed.requireAssistedOriginal(BUDGET) }.kind)
        }
    }

    @Test fun byteCountCannotWrapAndLargeOrEmptyImagesRefuse() {
        val value = source()
        for ((width, height) in listOf(0u to 2u, 2u to 0u, (1u shl 31) to (1u shl 31),
            UInt.MAX_VALUE to UInt.MAX_VALUE)) {
            val changed = value.copy(document = value.document.copy(width = width, height = height))
            assertEquals(SelectionFailureKind.Limit,
                assertFailsWith<SelectionFailure> { changed.assistedSourceRgbaBytes() }.kind)
        }
    }

    @Test fun replyMustRetainSourceIdentityAndExactOrientedGrid() {
        val value = source()
        requireAssistedSourceReply(value, value.binding, 3u, 2u, 8u, null, 24uL)
        for (binding in listOf(value.binding.copy(sourceAssetId = "d".repeat(64)),
            value.binding.copy(stateHash = "c".repeat(64)), value.binding.copy(hostSeq = 8uL),
            value.binding.copy(documentId = "other"))) {
            assertEquals(SelectionFailureKind.Conflict,
                assertFailsWith<SelectionFailure> {
                    requireAssistedSourceReply(value, binding, 3u, 2u, 8u, null, 24uL)
                }.kind)
        }
        for (args in listOf(listOf(2u, 3u, 8u), listOf(3u, 2u, 16u))) {
            assertFailsWith<SelectionFailure> {
                requireAssistedSourceReply(value, value.binding, args[0], args[1], args[2], null, 24uL)
            }
        }
        assertFailsWith<SelectionFailure> {
            requireAssistedSourceReply(value, value.binding, 3u, 2u, 8u, 0u, 24uL)
        }
        for (count in listOf(0uL, 23uL, 25uL, ULong.MAX_VALUE)) assertFailsWith<SelectionFailure> {
            requireAssistedSourceReply(value, value.binding, 3u, 2u, 8u, null, count)
        }
    }

    private fun source(): AssistedSelectionSource {
        val revision = ProjectInfo("project", "", "installation", 1uL, false, false,
            7uL, "a".repeat(64), listOf("document"))
        val binding = SelectionBinding("project", "document", "b".repeat(64), 7uL, "a".repeat(64))
        return AssistedSelectionSource(SelectionDocument(binding, 3u, 2u, 8u, revision), null, 1)
    }

    companion object { private const val BUDGET: ULong = 268435456uL }
}
