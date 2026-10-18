package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class ImageOutputPublicationTest {
    private val info = ProjectInfo("project", "Synthetic", "device", 9uL, true, false, 4uL, "a".repeat(64), listOf("document"))
    private val binding = WorkflowBinding(info.projectId, "document", info.hostSeq, info.stateHash)
    private val options = ExportOptions("document")
    private fun receipt(value: ProjectInfo = info) = FileExportResult(4uL, "b".repeat(64), "{}", value, 1024uL)

    @Test fun exact_rendered_binding_is_admitted_without_a_later_project_query() {
        verifyImageOutputPublication(binding, options, receipt())
    }
    @Test fun any_changed_document_project_or_visible_revision_refuses_before_publication() {
        for (changed in listOf(info.copy(projectId="other"), info.copy(hostSeq=5uL),
            info.copy(stateHash="c".repeat(64)), info.copy(documentIds=listOf("other")))) {
            assertThrows(IllegalArgumentException::class.java) { verifyImageOutputPublication(binding, options, receipt(changed)) }
        }
        assertThrows(IllegalArgumentException::class.java) { verifyImageOutputPublication(binding, options.copy(documentId="other"), receipt()) }
    }
}
