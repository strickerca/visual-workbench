package com.visualworkbench.desktop.remote

import com.visualworkbench.shared.RemoteWindowCandidate
import com.visualworkbench.shared.WorkbenchRemoteEdit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** The panel serializes these actions. Selection starts the native video owner;
 * granting that selection must not restart it. Native grant still revalidates
 * the exact target, foreground, canvas and input ownership on every attempt. */
internal class RemoteWindowSelection(private val remote: WorkbenchRemoteEdit) {
    private val mutable = MutableStateFlow<RemoteWindowCandidate?>(null)
    val selected: StateFlow<RemoteWindowCandidate?> = mutable.asStateFlow()

    suspend fun select(candidate: RemoteWindowCandidate) {
        mutable.value = null
        remote.select(candidate)
        mutable.value = candidate
    }

    suspend fun grant() {
        checkNotNull(mutable.value) { "Select a remote window first" }
        remote.grant()
    }
}
