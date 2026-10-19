package com.visualworkbench.shared

// Closed v1 audit limits intentionally remain independent of producer changes.
private const val AUDIT_MAX_COUNTER: Long = 9_007_199_254_740_991L
private const val AUDIT_MAX_EVENTS: Int = 256
private const val AUDIT_MAX_REPORT_CHARS: Int = 65_536

/** Validates the existing private factory's exact canonical v1 layout, not
 * arbitrary JSON. Fixed literals reject unknown/duplicate keys, escaped aliases
 * and free text without accepting a last-key-wins parse. There is no recursive
 * descent, arbitrary string allocation, normalization or redaction. The input
 * is bounded before scanning and no content is included in a failure message.
 *
 * This establishes the closed representation and numeric bounds. It cannot
 * establish the provenance of an otherwise valid numeric measurement.
 */
internal fun diagnosticsPrivacyAudit(text: String, eventCount: Int, rotatedEventCount: Long): Boolean {
    if (text.length > AUDIT_MAX_REPORT_CHARS || text.any { it.code !in 0..127 } ||
        eventCount !in 0..AUDIT_MAX_EVENTS || rotatedEventCount !in 0L..AUDIT_MAX_COUNTER) return false
    return DiagnosticReportAudit(text).accepts(eventCount, rotatedEventCount)
}

private class DiagnosticReportAudit(private val text: String) {
    private var at: Int = 0

    fun accepts(eventCount: Int, rotatedEventCount: Long): Boolean {
        if (!take("{\"schema_version\":1,\"platform\":\"") ||
            !(take("android") || take("windows")) || !take("\",\"metrics\":{")) return false
        if (!metric("\"startup_ms\":", AUDIT_MAX_COUNTER) ||
            !metric(",\"heap_used_bytes\":", AUDIT_MAX_COUNTER) ||
            !metric(",\"trace_samples\":", Int.MAX_VALUE.toLong()) ||
            !metric(",\"frames_decoded\":", AUDIT_MAX_COUNTER) ||
            !metric(",\"frames_dropped\":", AUDIT_MAX_COUNTER) ||
            !metric(",\"echo_rtt_ms\":", AUDIT_MAX_COUNTER) ||
            !take("},\"events_rotated\":")) return false
        if (integer(AUDIT_MAX_COUNTER) != rotatedEventCount || !take(",\"events\":[")) return false
        if (take("]}\n")) return eventCount == 0 && at == text.length
        var seen = 0
        var lastElapsed = 0L
        while (seen < AUDIT_MAX_EVENTS) {
            if (!take("{\"elapsed_ms\":")) return false
            val elapsed = integer(AUDIT_MAX_COUNTER) ?: return false
            if (elapsed < lastElapsed || !take(",\"code\":")) return false
            if (!eventCode() || !take("}")) return false
            lastElapsed = elapsed
            seen++
            if (take("]}\n")) return seen == eventCount && at == text.length
            if (!take(",")) return false
        }
        return false
    }

    private fun metric(key: String, maximum: Long): Boolean = take(key) &&
        (take("null") || integer(maximum) != null)

    /** Nonnegative decimal integers only; no sign, leading zeroes, exponent,
     * fractional value or overflow. Delimiters are checked by the fixed schema. */
    private fun integer(maximum: Long): Long? {
        if (at >= text.length || text[at] !in '0'..'9') return null
        if (text[at] == '0') { at++; return 0L }
        var value = 0L
        while (at < text.length && text[at] in '0'..'9') {
            val digit = (text[at] - '0').toLong()
            if (value > maximum / 10L || (value == maximum / 10L && digit > maximum % 10L)) return null
            value = value * 10L + digit
            at++
        }
        return value
    }

    // Deliberately independent of the producer's enum-to-string mapping. An
    // accidental new wire string must not silently broaden the v1 export set.
    private fun eventCode(): Boolean = take("\"app_started\"") || take("\"trace_started\"") ||
        take("\"trace_saved\"") || take("\"trace_interrupted\"") || take("\"trace_rejected\"") ||
        take("\"report_prepared\"") || take("\"export_succeeded\"") || take("\"export_failed\"") ||
        take("\"export_cancelled\"")

    private fun take(literal: String): Boolean {
        if (!text.startsWith(literal, at)) return false
        at += literal.length
        return true
    }
}
