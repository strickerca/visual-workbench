package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class DocumentActionFormTest {
    private fun id(number: Int): String = "00000000-0000-0000-0000-${number.toString().padStart(12, '0')}"
    private fun row(number: Int, kind: String = "image") = DocumentControlRow(id(number), "Original $number", kind, 800u, 600u)
    private val revision = ProjectInfo(id(90), "Project", id(91), 1uL, false, false, 2uL, "a".repeat(64), listOf(id(1), id(2)))
    private val binding = WorkflowBinding(revision.projectId, id(1), revision.hostSeq, revision.stateHash)
    private val navigation = DocumentNavigationState(true, binding, 7L, revision.documentIds, "Original 1")
    private fun controls(rows: List<DocumentControlRow>, fence: WorkflowBinding = binding) = DocumentControlsState(
        active = true, snapshot = DocumentControlSnapshot(fence, revision, rows, emptyList(), emptyList(), emptyList()))

    @Test fun titleCatalogRequiresTheWholeExactVisibleRevision() {
        val rows = listOf(row(1), row(2))
        assertEquals("Original 2", documentActionCatalog(navigation, controls(rows))[id(2)]?.title)
        assertTrue(documentActionCatalog(navigation, controls(rows, binding.copy(stateHash = "b".repeat(64)))).isEmpty())
        assertTrue(documentActionCatalog(navigation, controls(rows.take(1))).isEmpty())
        assertTrue(documentActionCatalog(navigation, controls(listOf(row(1), row(3)))).isEmpty())
        assertTrue(documentActionCatalog(navigation, controls(listOf(row(1), row(1)))).isEmpty())
    }

    @Test fun invalidAndUnchangedTitlesNeverAllocateIdsOrProduceActions() {
        val current = row(1)
        var allocated = 0
        val ids = { allocated++; id(allocated + 10) }
        assertNull(DocumentActionForm("Original 1").rename(current))
        listOf("", "  ", "bad\nname", "🙂".repeat(1025)).forEach { value ->
            val form = DocumentActionForm(value)
            assertFalse(form.valid)
            assertNull(form.rename(current))
            assertNull(form.createFromSource(current, 2, ids))
        }
        assertEquals(0, allocated)
    }

    @Test fun createUsesTwoFreshIdsOnlyForEligibleBoundedImageCatalog() {
        val form = DocumentActionForm("Variation")
        var allocated = 0
        val ids = { allocated++; id(allocated + 10) }
        assertNull(form.createFromSource(row(1, "pdf"), 2, ids))
        assertNull(form.createFromSource(row(1), 128, ids))
        assertEquals(0, allocated)
        val command = form.createFromSource(row(1), 24, ids)!!
        assertEquals(id(11), command.documentId)
        assertEquals(id(12), command.layerId)
        assertEquals("Variation", command.title)
        assertEquals(2, allocated)
        assertEquals("Variation", form.rename(row(1))?.title)
    }
}
