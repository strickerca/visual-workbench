package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class DiagnosticsBundleTest {
    @Test fun reportHasOnlyClosedSchemaStringsAndTypedMetrics() {
        val log = PrivateDiagnosticLog()
        DiagnosticEventCode.entries.forEachIndexed { i, code -> log.append(i.toLong(), code) }
        val report = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(
            startupMillis = 12, heapUsedBytes = 4096, traceSampleCount = 7,
            framesDecoded = 10, framesDropped = 2, echoRoundTripMillis = 3,
        ))
        val allowed = setOf(
            "schema_version", "platform", "android", "metrics", "startup_ms",
            "heap_used_bytes", "trace_samples", "frames_decoded", "frames_dropped",
            "echo_rtt_ms", "events_rotated", "events", "elapsed_ms", "code",
            "app_started", "trace_started", "trace_saved", "trace_interrupted",
            "trace_rejected", "report_prepared", "export_succeeded",
            "export_failed", "export_cancelled",
        )
        val strings = Regex("\"([^\"]*)\"").findAll(report.jsonText).map { it.groupValues[1] }.toList()
        assertTrue(strings.all { it in allowed })
        assertEquals(9, report.eventCount)
        assertTrue(report.jsonText.contains("\"heap_used_bytes\":4096"))
        assertFalse(report.jsonText.contains("serial"))
        assertFalse(report.jsonText.contains("account"))
        assertFalse(report.jsonText.contains("window_title"))
    }

    @Test fun unavailableMetricsStayUnknownRatherThanBecomingZero() {
        val report = PrivateDiagnosticLog().prepare(DiagnosticPlatform.Windows, DiagnosticMetrics())
        assertTrue(report.jsonText.contains("\"platform\":\"windows\""))
        for (name in listOf("startup_ms", "heap_used_bytes", "trace_samples", "frames_decoded", "frames_dropped", "echo_rtt_ms")) {
            assertTrue(report.jsonText.contains("\"$name\":null"))
        }
        assertEquals(0, report.eventCount)
    }

    @Test fun rotationKeepsNewestEventsAndReportsDiscardedCount() {
        val log = PrivateDiagnosticLog(capacity = 2)
        log.append(10, DiagnosticEventCode.AppStarted)
        log.append(20, DiagnosticEventCode.TraceStarted)
        log.append(30, DiagnosticEventCode.TraceSaved)
        val report = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics())
        assertEquals(2, report.eventCount)
        assertEquals(1L, report.rotatedEventCount)
        assertFalse(report.jsonText.contains("app_started"))
        assertTrue(report.jsonText.indexOf("trace_started") < report.jsonText.indexOf("trace_saved"))
    }

    @Test fun preparedPreviewCannotChangeWhenLaterEventsArrive() {
        val log = PrivateDiagnosticLog(capacity = 1)
        log.append(0, DiagnosticEventCode.AppStarted)
        val preview = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(traceSampleCount = 1))
        val original = preview.jsonText
        log.append(1, DiagnosticEventCode.ExportSucceeded)
        assertEquals(original, preview.jsonText)
        assertEquals(0L, preview.rotatedEventCount)
        assertFalse(preview.jsonText.contains("export_succeeded"))
        assertTrue(log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics()).jsonText.contains("export_succeeded"))
    }

    @Test fun invalidMetricsAndClockOrderAreRejectedBeforeExport() {
        assertFailsWith<IllegalArgumentException> { DiagnosticMetrics(heapUsedBytes = -1) }
        assertFailsWith<IllegalArgumentException> { DiagnosticMetrics(traceSampleCount = -1) }
        assertFailsWith<IllegalArgumentException> { DiagnosticMetrics(framesDecoded = Long.MAX_VALUE) }
        assertFailsWith<IllegalArgumentException> { PrivateDiagnosticLog(0) }
        assertFailsWith<IllegalArgumentException> { PrivateDiagnosticLog(257) }
        val log = PrivateDiagnosticLog()
        log.append(3, DiagnosticEventCode.AppStarted)
        assertFailsWith<IllegalArgumentException> { log.append(2, DiagnosticEventCode.ExportFailed) }
        assertFailsWith<IllegalArgumentException> { log.append(Long.MAX_VALUE, DiagnosticEventCode.ExportFailed) }
        assertEquals(1, log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics()).eventCount)
    }

    @Test fun maximumLogStillProducesBoundedAsciiPayload() {
        val log = PrivateDiagnosticLog(256)
        repeat(512) { log.append(it.toLong(), DiagnosticEventCode.TraceSaved) }
        val report = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(
            startupMillis = 9_007_199_254_740_991, heapUsedBytes = 9_007_199_254_740_991,
            traceSampleCount = Int.MAX_VALUE, framesDecoded = 9_007_199_254_740_991,
            framesDropped = 9_007_199_254_740_991, echoRoundTripMillis = 9_007_199_254_740_991,
        ))
        assertEquals(256, report.eventCount)
        assertEquals(256L, report.rotatedEventCount)
        assertTrue(report.jsonText.length <= 65_536)
        assertTrue(report.jsonText.all { it.code in 0..127 })
        assertTrue(report.jsonText.endsWith("\n"))
    }
}
