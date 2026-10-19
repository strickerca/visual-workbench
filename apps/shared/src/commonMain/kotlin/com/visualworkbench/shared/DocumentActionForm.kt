package com.visualworkbench.shared

/** Private chooser text. No identity is allocated until a valid action is sent. */
public data class DocumentActionForm(public val title: String) {
    public val valid: Boolean get() = title.isNotBlank() && title.length <= 4096 &&
        title.encodeToByteArray().size <= 4096 && title.none { it.isISOControl() }

    public fun rename(current: DocumentControlRow): DocumentControlCommand.RenameDocument? =
        if (valid && title != current.title) DocumentControlCommand.RenameDocument(title) else null

    /** Creates an independent empty annotation document over the same original.
     * It does not duplicate the current document's annotations or original bytes. */
    public fun createFromSource(current: DocumentControlRow, documentCount: Int,
        newId: () -> String): DocumentControlCommand.CreateDocumentFromSource? {
        if (!valid || current.kind !in setOf("image", "capture") || documentCount !in 1..127) return null
        return DocumentControlCommand.CreateDocumentFromSource(newId(), newId(), title)
    }
}

/** A read-only title catalog is usable only for the chooser's exact revision. */
public fun documentActionCatalog(navigation: DocumentNavigationState,
    controls: DocumentControlsState): Map<String, DocumentControlRow> {
    val snapshot = controls.snapshot ?: return emptyMap()
    val ids = snapshot.documents.map { it.documentId }
    if (navigation.binding != snapshot.binding || navigation.documentIds.size != snapshot.documents.size ||
        ids.distinct().size != ids.size || navigation.documentIds.distinct().size != ids.size ||
        navigation.documentIds.toSet() != ids.toSet() || snapshot.revision.documentIds.toSet() != ids.toSet()) return emptyMap()
    return snapshot.documents.associateBy { it.documentId }
}
