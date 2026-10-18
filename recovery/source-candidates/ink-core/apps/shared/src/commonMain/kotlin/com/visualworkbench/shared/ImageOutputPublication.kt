package com.visualworkbench.shared

/** The caller must run this on the complete app-private native result, before
 * opening a provider destination, publishing a share URI or creating a grant.
 * A late project change cannot turn an old D-to-output region into a new-frame
 * export. This checks the rendered receipt, not a later project.info() query. */
public fun verifyImageOutputPublication(expected: WorkflowBinding, options: ExportOptions, result: FileExportResult): Unit {
    require(options.documentId == expected.documentId && expected.documentId in result.revision.documentIds &&
        result.revision.projectId == expected.projectId && result.revision.hostSeq == expected.hostSeq &&
        result.revision.stateHash == expected.stateHash) {
        "The completed image has a different document revision. Check the export again before saving or sharing."
    }
}
