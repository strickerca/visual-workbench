package com.visualworkbench.desktop.capture

import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Invoke only after abortable departure checks. Either capture refusal keeps
 * the parent owners alive; cancellation cannot skip a source's actual join. */
internal suspend fun withRetiredCaptureCallers(
    picker: suspend () -> Unit,
    foreground: suspend () -> Unit,
    closeParents: suspend () -> Unit,
) {
    withContext(NonCancellable) {
        picker()
        foreground()
        closeParents()
    }
}

/** Agent requests may hold the editor's existing verification borrow. A failed
 * service retirement must keep that editor and every native parent alive. */
internal suspend fun withRetiredVerificationAccess(
    requests: suspend () -> Unit,
    service: suspend () -> Unit,
    closeEditorAndParents: suspend () -> Unit,
) {
    withContext(NonCancellable) {
        requests()
        service()
        closeEditorAndParents()
    }
}
