package com.visualworkbench.android.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.coroutines.flow.map

/** Native waitSignal reports normal receiver retirement as SessionFailure.Closed.
 * Retain the last identity without querying a closed handle, and terminate after
 * publishing Closed. Cancellation, other failures and downstream callback errors
 * are not reclassified as native lifecycle completion. */
internal suspend fun observeStreamSignals(initial: CaptureStreamSignal, signals: Flow<CaptureStreamSignal>,
    publish: (CaptureStreamSignal) -> Unit): CaptureStreamSignal? {
    var last = initial
    return signals.map { value ->
        if (value.stage == CaptureStreamStage.Closed) value.copy(sourceAvailable = false) else value
    }.catch { error ->
        if (error !is SessionFailure || error.kind != SessionFailureKind.Closed) throw error
        emit(last.copy(stage = CaptureStreamStage.Closed, sourceAvailable = false, failure = null))
    }.firstOrNull { value ->
        last = value; publish(value)
        value.stage == CaptureStreamStage.Closed
    }
}
