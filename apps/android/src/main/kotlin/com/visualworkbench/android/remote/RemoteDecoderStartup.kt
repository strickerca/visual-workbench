package com.visualworkbench.android.remote

import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
import kotlin.time.Duration.Companion.nanoseconds

internal enum class RemoteDecoderStartup { Ready, Invalidated, Failed, Deadline }

/** Retain the caller's exact native frame until the independently owned codec
 * worker reaches Ready. Cancellation propagates to the caller's existing ticket
 * cleanup; no second source frame, retry or synthetic callback is required.
 * The absolute deadline starts before open(), within its unchanged 3 s budget.
 * This is startup admission; the unchanged 500 ms input/output/render deadline
 * starts only when the current frame is actually queued after Ready. */
internal suspend fun awaitRemoteDecoderStartup(
    status: StateFlow<DecoderStatus>,
    deadlineNanos: Long,
    isCurrent: () -> Boolean,
): RemoteDecoderStartup {
    if (!isCurrent()) return RemoteDecoderStartup.Invalidated
    val remaining = deadlineNanos - System.nanoTime()
    if (remaining <= 0) return RemoteDecoderStartup.Deadline
    val terminal = withTimeoutOrNull(remaining.nanoseconds) {
        status.first { it != DecoderStatus.Starting }
    } ?: return RemoteDecoderStartup.Deadline
    if (!isCurrent()) return RemoteDecoderStartup.Invalidated
    if (System.nanoTime() >= deadlineNanos) return RemoteDecoderStartup.Deadline
    return if (terminal is DecoderStatus.Ready) RemoteDecoderStartup.Ready else RemoteDecoderStartup.Failed
}

/** The first IDR may arrive before SurfaceView publishes its first valid Surface.
 * Retain only that IDR within the SAME startup deadline and scope/lifecycle fence. */
internal suspend fun awaitRemoteSurfaceStartup(deadlineNanos:Long,isCurrent:()->Boolean,isReady:()->Boolean):RemoteDecoderStartup {
    while(true){
        if(!isCurrent())return RemoteDecoderStartup.Invalidated
        if(System.nanoTime()>=deadlineNanos)return RemoteDecoderStartup.Deadline
        if(isReady())return RemoteDecoderStartup.Ready
        kotlinx.coroutines.delay(5)
    }
}
