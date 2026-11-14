package com.visualworkbench.shared

import com.visualworkbench.bindings.core.CoreDiagnosticKind as NCoreDiagnosticKind
import com.visualworkbench.bindings.core.coreDiagnostics as nativeCoreDiagnostics

private const val MAX_CORE_NUMBER: Long = 9_007_199_254_740_991L

private fun bounded(value: ULong): Long = minOf(value, MAX_CORE_NUMBER.toULong()).toLong()

/** Reads the core's retained diagnostic events newer than [afterSequence].
 * An event whose text falls outside the allowed alphabet is dropped rather
 * than shown. */
public fun readCoreEvents(afterSequence: Long = 0L): CoreEventLog {
    require(afterSequence >= 0L)
    val snapshot = nativeCoreDiagnostics(afterSequence.toULong())
    val events = snapshot.events.mapNotNull { event ->
        if (event.sequence == 0uL || !isCoreEventText(event.area) || !isCoreEventText(event.name)) null
        else CoreEvent(
            sequence = bounded(event.sequence),
            elapsedMillis = bounded(event.elapsedMs),
            kind = when (event.kind) {
                NCoreDiagnosticKind.FAILURE -> CoreEventKind.Failure
                NCoreDiagnosticKind.DURATION_MICROS -> CoreEventKind.DurationMicros
                NCoreDiagnosticKind.COUNT -> CoreEventKind.Count
            },
            area = event.area,
            name = event.name,
            value = bounded(event.value),
        )
    }
    return CoreEventLog(events, bounded(snapshot.dropped))
}
