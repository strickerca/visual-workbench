package com.visualworkbench.android.editor

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.visualworkbench.android.stream.AndroidStreamDraftEditor
import com.visualworkbench.android.stream.StreamEditorViewport
import com.visualworkbench.shared.CaptureDraftInfo
import com.visualworkbench.shared.WorkbenchCore
import com.visualworkbench.shared.WorkbenchProject
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Identity belongs to a document/page, not to a changing sync status or revision.
 * A retained durable Save plan remains inspectable after transport loss. */
internal class StreamRootAttachment<P : Any, L : Any>(
    val project: P, val documentId: String, val page: UInt?, val editorEpoch: Long,
    val link: L, val linkEpoch: Long,
) {
    fun matches(project: P?, documentId: String?, page: UInt?, editorEpoch: Long, link: L?, linkEpoch: Long): Boolean =
        this.project === project && this.documentId == documentId && this.page == page &&
            this.editorEpoch == editorEpoch && this.link === link && this.linkEpoch == linkEpoch
}

/** Register before cancellation/stale-result checks. A failed release retains
 * the exact owner and refuses its replacement until a joined retry succeeds. */
internal class StreamRootOwner<T : Any>(private val close: suspend (T) -> Unit) {
    var value by mutableStateOf<T?>(null); private set
    private val retirement = Mutex()
    fun retain(owner: T) { check(value == null || value === owner); value = owner }
    fun transferred(owner: T) { check(value === owner); value = null }
    suspend fun retire(): Unit = withContext(NonCancellable) { retirement.withLock {
        value?.let { owner -> close(owner); if (value === owner) value = null }
    } }
}

/** The existing editor borrows the native temporary project. Only its parent
 * stream session calls close, so child scope retirement never joins itself. */
internal class StreamDraftEditorAdapter private constructor(
    internal val controller: EditorController,
    private val retired: (StreamDraftEditorAdapter) -> Unit,
) : AndroidStreamDraftEditor {
    private var initializationFailure: Exception? = null
    override val scenes get() = initializationFailure?.let { throw it } ?: controller.streamScenes
    override val gestures get() = controller
    override fun viewport(width: Int, height: Int, density: Float) = controller.viewport(width, height, density)
    override suspend fun sealEdits() = controller.settleStreamEdits()
    override suspend fun close() {
        controller.disposeStreamEditor()
        retired(this)
    }

    companion object {
        /** Retain the adapter before install suspends; failed cleanup must keep
         * the child borrow alive until the parent can retry retirement. */
        suspend fun create(application: Application, core: WorkbenchCore, project: WorkbenchProject,
            draft: CaptureDraftInfo, viewport: StreamEditorViewport,
            retain: (StreamDraftEditorAdapter) -> Unit, retired: (StreamDraftEditorAdapter) -> Unit): StreamDraftEditorAdapter {
            val adapter = StreamDraftEditorAdapter(EditorController(application, core, streamDraftOnly = true), retired)
            retain(adapter)
            try { adapter.controller.installStreamDraft(project, draft.documentId, viewport.initialCamera) }
            catch (error: Exception) {
                // Return the actual failed child to the module's child slot.
                // Its first scene read fails there, after ownership transfer,
                // so resume/close still joins this borrow before native disposal.
                adapter.initializationFailure = error
            }
            return adapter
        }
    }
}
