package com.visualworkbench.shared

// Closed v1 audit limits intentionally remain independent of producer changes.
private const val AUDIT_MAX_COUNTER: Long = 9_007_199_254_740_991L
private const val AUDIT_MAX_EVENTS: Int = 256
private const val AUDIT_MAX_REPORT_CHARS: Int = 65_536
// Schema 2 adds the core section; its limits are equally fixed here.
private const val AUDIT_MAX_CORE_EVENTS: Int = 256
private const val AUDIT_MAX_CORE_TEXT: Int = 64
private const val AUDIT_MAX_CORE_REPORT_CHARS: Int = 131_072

/** Validates the existing private factory's exact canonical layout (schema 1, or
 * schema 2 when both core counts are given), not
 * arbitrary JSON. Fixed literals reject unknown/duplicate keys, escaped aliases
 * and free text without accepting a last-key-wins parse. There is no recursive
 * descent, arbitrary string allocation, normalization or redaction. The input
 * is bounded before scanning and no content is included in a failure message.
 *
 * This establishes the closed representation and numeric bounds. It cannot
 * establish the provenance of an otherwise valid numeric measurement.
 */
internal fun diagnosticsPrivacyAudit(
    text: String,
    eventCount: Int,
    rotatedEventCount: Long,
    coreEventCount: Int? = null,
    coreDroppedCount: Long? = null,
): Boolean {
    // Schema 1 has no core section; schema 2 has both core counts. Nothing in between.
    if ((coreEventCount == null) != (coreDroppedCount == null)) return false
    val limit = if (coreEventCount == null) AUDIT_MAX_REPORT_CHARS else AUDIT_MAX_CORE_REPORT_CHARS
    if (text.length > limit || text.any { it.code !in 0..127 } ||
        eventCount !in 0..AUDIT_MAX_EVENTS || rotatedEventCount !in 0L..AUDIT_MAX_COUNTER) return false
    if (coreEventCount != null && coreDroppedCount != null &&
        (coreEventCount !in 0..AUDIT_MAX_CORE_EVENTS || coreDroppedCount !in 0L..AUDIT_MAX_COUNTER)) return false
    return DiagnosticReportAudit(text).accepts(eventCount, rotatedEventCount, coreEventCount, coreDroppedCount)
}

private class DiagnosticReportAudit(private val text: String) {
    private var at: Int = 0

    fun accepts(eventCount: Int, rotatedEventCount: Long, coreEventCount: Int?, coreDroppedCount: Long?): Boolean {
        if (!take("{\"schema_version\":") || !take(if (coreEventCount == null) "1" else "2") ||
            !take(",\"platform\":\"") ||
            !(take("android") || take("windows")) || !take("\",\"metrics\":{")) return false
        if (!metric("\"startup_ms\":", AUDIT_MAX_COUNTER) ||
            !metric(",\"heap_used_bytes\":", AUDIT_MAX_COUNTER) ||
            !metric(",\"trace_samples\":", Int.MAX_VALUE.toLong()) ||
            !metric(",\"frames_decoded\":", AUDIT_MAX_COUNTER) ||
            !metric(",\"frames_dropped\":", AUDIT_MAX_COUNTER) ||
            !metric(",\"echo_rtt_ms\":", AUDIT_MAX_COUNTER) ||
            !take("},\"events_rotated\":")) return false
        if (integer(AUDIT_MAX_COUNTER) != rotatedEventCount || !take(",\"events\":[")) return false
        var seen = 0
        if (!take("]")) {
            var lastElapsed = 0L
            while (true) {
                if (seen == AUDIT_MAX_EVENTS || !take("{\"elapsed_ms\":")) return false
                val elapsed = integer(AUDIT_MAX_COUNTER) ?: return false
                if (elapsed < lastElapsed || !take(",\"code\":")) return false
                if (!eventCode() || !take("}")) return false
                lastElapsed = elapsed
                seen++
                if (take("]")) break
                if (!take(",")) return false
            }
        }
        if (seen != eventCount) return false
        if (coreEventCount != null && coreDroppedCount != null && !coreSection(coreEventCount, coreDroppedCount)) return false
        return take("}\n") && at == text.length
    }

    /** `,"core_dropped":N,"core":[{"seq":N,"elapsed_ms":N,"kind":K,"area":T,"name":T,"value":N},...]`
     * where K is one of three fixed words and T is 1 to 64 characters of
     * lower-case letters, digits, space, dot, underscore and hyphen. That
     * alphabet has no quote, backslash, slash or colon, so a path, a URL or an
     * escape sequence cannot appear. Sequence numbers must strictly increase. */
    private fun coreSection(coreEventCount: Int, coreDroppedCount: Long): Boolean {
        if (!take(",\"core_dropped\":") || integer(AUDIT_MAX_COUNTER) != coreDroppedCount ||
            !take(",\"core\":[")) return false
        var seen = 0
        if (!take("]")) {
            var lastSequence = 0L
            while (true) {
                if (seen == AUDIT_MAX_CORE_EVENTS || !take("{\"seq\":")) return false
                val sequence = integer(AUDIT_MAX_COUNTER) ?: return false
                if (sequence <= lastSequence || !take(",\"elapsed_ms\":")) return false
                if (integer(AUDIT_MAX_COUNTER) == null || !take(",\"kind\":")) return false
                if (!(take("\"failure\"") || take("\"duration_us\"") || take("\"count\""))) return false
                if (!take(",\"area\":\"") || !coreText() || !take("\",\"name\":\"") || !coreText()) return false
                if (!take("\",\"value\":") || integer(AUDIT_MAX_COUNTER) == null || !take("}")) return false
                lastSequence = sequence
                seen++
                if (take("]")) break
                if (!take(",")) return false
            }
        }
        return seen == coreEventCount
    }

    private fun coreText(): Boolean {
        val start = at
        while (at < text.length && at - start <= AUDIT_MAX_CORE_TEXT) {
            val c = text[at]
            if (!(c in 'a'..'z' || c in '0'..'9' || c == ' ' || c == '.' || c == '_' || c == '-')) break
            at++
        }
        return at - start in 1..AUDIT_MAX_CORE_TEXT
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
