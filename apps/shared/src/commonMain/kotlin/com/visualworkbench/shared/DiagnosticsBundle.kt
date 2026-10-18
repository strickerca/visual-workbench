package com.visualworkbench.shared

private const val MAX_DIAGNOSTIC_VALUE: Long = 9_007_199_254_740_991L
private const val MAX_DIAGNOSTIC_EVENTS: Int = 256

public enum class DiagnosticPlatform { Android, Windows }

/** Closed event codes. Do not add an arbitrary message, identifier or path field. */
public enum class DiagnosticEventCode {
    AppStarted, TraceStarted, TraceSaved, TraceInterrupted, TraceRejected,
    ReportPrepared, ExportSucceeded, ExportFailed, ExportCancelled,
}

/** Only counters and measured durations enter a report; absent values stay null.
 * Echo round-trip time is not pen-to-display latency.
 */
public data class DiagnosticMetrics(
    public val startupMillis: Long? = null,
    public val heapUsedBytes: Long? = null,
    public val traceSampleCount: Int? = null,
    public val framesDecoded: Long? = null,
    public val framesDropped: Long? = null,
    public val echoRoundTripMillis: Long? = null,
) {
    init {
        for (value in listOf(startupMillis, heapUsedBytes, framesDecoded, framesDropped, echoRoundTripMillis)) {
            require(value == null || value in 0L..MAX_DIAGNOSTIC_VALUE)
        }
        require(traceSampleCount == null || traceSampleCount >= 0)
    }
}

internal data class PrivateDiagnosticEvent(val elapsedMillis: Long, val code: DiagnosticEventCode) {
    init { require(elapsedMillis in 0L..MAX_DIAGNOSTIC_VALUE) }
}

/** A local bounded rotating event log, owned by the app UI dispatcher.
 * Append and prepare must be serialized by that owner. IO returns its result
 * to the owner before logging; raw logs, trace data and exceptions never enter.
 */
public class PrivateDiagnosticLog(public val capacity: Int = 64) {
    private val events: MutableList<PrivateDiagnosticEvent> = mutableListOf()
    private var lastElapsedMillis: Long = 0L
    private var rotated: Long = 0L

    init { require(capacity in 1..MAX_DIAGNOSTIC_EVENTS) }

    public fun append(elapsedMillis: Long, code: DiagnosticEventCode): Unit {
        require(elapsedMillis in lastElapsedMillis..MAX_DIAGNOSTIC_VALUE)
        if (events.size == capacity) {
            check(rotated < MAX_DIAGNOSTIC_VALUE)
            events.removeAt(0)
            rotated += 1L
        }
        events.add(PrivateDiagnosticEvent(elapsedMillis, code))
        lastElapsedMillis = elapsedMillis
    }

    /** Freezes the exact preview/export payload; subsequent events cannot alter it. */
    public fun prepare(platform: DiagnosticPlatform, metrics: DiagnosticMetrics): PreparedDiagnosticsReport =
        PreparedDiagnosticsReport.create(platform, metrics, events.toList(), rotated)
}

/** The immutable body shown in the preview is the body written by exporters.
 * Its private factory accepts only the closed, typed report schema.
 */
public class PreparedDiagnosticsReport private constructor(
    public val jsonText: String,
    public val eventCount: Int,
    public val rotatedEventCount: Long,
) {
    internal companion object {
        internal fun create(
            platform: DiagnosticPlatform,
            metrics: DiagnosticMetrics,
            events: List<PrivateDiagnosticEvent>,
            rotated: Long,
        ): PreparedDiagnosticsReport {
            require(events.size <= MAX_DIAGNOSTIC_EVENTS && rotated in 0L..MAX_DIAGNOSTIC_VALUE)
            require(events.zipWithNext().all { (before, after) -> before.elapsedMillis <= after.elapsedMillis })
            val text = buildString {
                append("{\"schema_version\":1,\"platform\":\"")
                append(when (platform) { DiagnosticPlatform.Android -> "android"; DiagnosticPlatform.Windows -> "windows" })
                append("\",\"metrics\":{")
                append("\"startup_ms\":").append(metrics.startupMillis)
                append(",\"heap_used_bytes\":").append(metrics.heapUsedBytes)
                append(",\"trace_samples\":").append(metrics.traceSampleCount)
                append(",\"frames_decoded\":").append(metrics.framesDecoded)
                append(",\"frames_dropped\":").append(metrics.framesDropped)
                append(",\"echo_rtt_ms\":").append(metrics.echoRoundTripMillis)
                append("},\"events_rotated\":").append(rotated).append(",\"events\":[")
                events.forEachIndexed { index, event ->
                    if (index != 0) append(',')
                    append("{\"elapsed_ms\":").append(event.elapsedMillis)
                    append(",\"code\":\"").append(event.code.wireCode()).append("\"}")
                }
                append("]}\n")
            }
            check(diagnosticsPrivacyAudit(text, events.size, rotated)) {
                "Diagnostics report failed its privacy audit"
            }
            return PreparedDiagnosticsReport(text, events.size, rotated)
        }
    }
}

private fun DiagnosticEventCode.wireCode(): String = when (this) {
    DiagnosticEventCode.AppStarted -> "app_started"
    DiagnosticEventCode.TraceStarted -> "trace_started"
    DiagnosticEventCode.TraceSaved -> "trace_saved"
    DiagnosticEventCode.TraceInterrupted -> "trace_interrupted"
    DiagnosticEventCode.TraceRejected -> "trace_rejected"
    DiagnosticEventCode.ReportPrepared -> "report_prepared"
    DiagnosticEventCode.ExportSucceeded -> "export_succeeded"
    DiagnosticEventCode.ExportFailed -> "export_failed"
    DiagnosticEventCode.ExportCancelled -> "export_cancelled"
}
