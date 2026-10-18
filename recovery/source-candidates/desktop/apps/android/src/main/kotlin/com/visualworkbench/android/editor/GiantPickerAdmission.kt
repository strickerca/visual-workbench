package com.visualworkbench.android.editor

import android.net.Uri

/** Main-confined picker -> confirmation ownership. The scoped focus lease is
 * acquired before launching or publishing a returned URI. It survives a
 * project-screen change; a replacement Activity restores its own lease while
 * the retiring Activity can release only its own token. */
internal class GiantPickerAdmission(
    restoredPending: Boolean,
    private val acquire: () -> AutoCloseable,
    private val changed: (pending: Boolean, selected: Uri?) -> Unit,
) : AutoCloseable {
    private var pending = restoredPending
    private var selected: Uri? = null
    private var closed = false
    private var lease: AutoCloseable? = if (restoredPending) acquire() else null

    fun launch(open: () -> Unit) {
        check(!closed)
        if (pending || selected != null) return
        lease = acquire()
        pending = true; changed(true, null)
        try { open() } catch (error: Throwable) { dismiss(); throw error }
    }
    fun result(uri: Uri?) {
        if (closed || !pending) return
        if (uri?.scheme != "content") { dismiss(); return }
        check(lease != null)
        pending = false; selected = uri
        changed(false, uri) // Existing lease remains through the confirmation.
    }
    fun accept(import: (Uri) -> Unit) {
        if (closed) return
        val value = selected ?: return
        try { import(value) } finally { dismiss() }
    }
    fun dismiss() {
        if (closed) return
        pending = false; selected = null; changed(false, null)
        lease?.close(); lease = null
    }
    override fun close() {
        if (closed) return
        closed = true
        lease?.close(); lease = null; selected = null
        // Keep the saved awaiting-result flag for Activity recreation. It
        // contains no URI or permission; a returned result still needs Confirm.
    }
}
