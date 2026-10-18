package com.visualworkbench.desktop

import java.nio.file.Path

/** EDT-owned request spanning the queued AWT handoff and its nested modal loop.
 * Keep the focus lease until native disposal succeeds; failures remain retryable. */
internal class ProjectFilePickerRequest(
    val owner: Any,
    val route: Any,
    val epoch: Long,
    val returnToConnection: Boolean,
    private val focusLease: AutoCloseable,
) : AutoCloseable {
    private var disposeDialog: (() -> Unit)? = null
    private var started = false
    private var creating = false
    private var closeRequested = false
    private var closing = false
    var retired: Boolean = false
        private set
    var returnEligible: Boolean = false
        private set

    fun <T> choose(
        isCurrent: () -> Boolean,
        create: () -> T,
        show: (T) -> Path?,
        dispose: (T) -> Unit,
        accept: (Path) -> Unit,
    ) {
        if (retired || closeRequested || started) return
        started = true
        if (!isCurrent()) return
        try {
            // A constructor may pump native events before its owner is returned.
            // Defer retirement until the created dialog can be retained/disposed.
            creating = true
            val dialog = try { create() } finally { creating = false }
            disposeDialog = { dispose(dialog) }
            if (closeRequested) { close(); return }
            if (!isCurrent()) return
            val selected = show(dialog)
            // Recheck after the native nested event loop, including Cancel.
            returnEligible = !retired && !closeRequested && isCurrent()
            if (selected != null && returnEligible) accept(selected)
        } catch (error: Exception) {
            returnEligible = !retired && !closeRequested && isCurrent()
            throw error
        }
    }

    override fun close() {
        // Even a failed disposal permanently fences the queued/late result.
        closeRequested = true
        if (retired || closing) return
        if (creating) return
        closing = true
        try {
            disposeDialog?.invoke()
            disposeDialog = null
            focusLease.close()
            retired = true
        } finally {
            closing = false
        }
    }
}
